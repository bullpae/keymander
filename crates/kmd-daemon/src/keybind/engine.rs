//! 키 바인딩 결정 엔진 — OS 훅과 분리된 순수 상태 머신
//!
//! 플랫폼 훅 콜백(unsafe, 테스트 불가)에서 로직을 분리해 단위 테스트를
//! 가능하게 한다. 입력은 (VKey, down/up, tick_ms), 출력은 [`KeyDecision`].
//! 타이밍은 호출자가 넘겨주는 밀리초 tick(u32, wrapping)만 사용하므로
//! 테스트에서 시간을 자유롭게 시뮬레이션할 수 있다.

use std::collections::HashSet;

use super::{
    is_modifier_key, modifier_satisfied, BindAction, ComboTrigger, KeybindConfig, MouseBind,
    UnmappedBehavior, VKey,
};

/// 키 이벤트 처리 결정
#[derive(Debug, Clone)]
pub enum KeyDecision {
    /// OS에 그대로 전달
    PassThrough,
    /// 이벤트 억제 (실행할 액션 없음)
    Suppress,
    /// 이벤트 억제 + 액션 실행.
    ///
    /// `layer_trigger`는 활성 레이어 매핑에서 발동된 경우 그 레이어의
    /// 트리거 키다. macOS는 물리적으로 눌린 트리거 수정자(Alt 등)의 잔여
    /// 플래그가 합성 이벤트에 간섭하므로, 실행 전에 해제해야 한다.
    /// Windows는 SendInput이 modifier 간섭을 받지 않아 소비하지 않는다.
    Execute {
        action: BindAction,
        // macOS 어댑터에서만 소비 (레이어 트리거 modifier를 풀고 주입)
        #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
        layer_trigger: Option<VKey>,
    },
    /// 코드(chord) 모드 진입 — 레이어 패스쓰루에서 미매핑 키가 눌렸다.
    ///
    /// 어댑터는 물리 이벤트를 억제하고 `trigger` down → `key` down을
    /// 순서대로 주입해 OS가 트리거 조합(Alt+Tab 등)으로 인식하게 한다.
    /// 이후 홀드가 끝날 때까지 후속 키는 `PassThrough`로 전달된다
    /// (OS 쪽에는 주입된 트리거가 눌려 있는 상태).
    EngageChord { trigger: VKey, key: VKey },
    /// 코드 모드 종료 — 어댑터는 물리 이벤트를 억제하고 `trigger` up을
    /// 주입한다 (Alt+Tab 스위처 확정 등). `deferred_action`은 코드 진입
    /// 전에 지연돼 있던 레이어 Launch가 있으면 트리거 해제 후 실행한다.
    ReleaseChord {
        trigger: VKey,
        deferred_action: Option<BindAction>,
    },
    /// 마우스 바인딩 키 down — 어댑터는 물리 이벤트를 억제하고
    /// 이동/휠이면 마우스 워커를 시작, 버튼이면 button down을 주입한다.
    MouseEngage(MouseBind),
    /// 마우스 바인딩 키 up — 이동/휠 정지, 버튼 button up 주입.
    MouseRelease(MouseBind),
    /// 활성 마우스 바인딩 전체 정지 — 레이어 트리거 해제나 keymap 토글로
    /// 이동 키의 keyup을 더 받을 수 없을 때 stuck-mouse를 방지한다.
    MouseStopAll,
}

impl KeyDecision {
    fn execute(action: BindAction) -> Self {
        Self::Execute {
            action,
            layer_trigger: None,
        }
    }

    fn execute_in_layer(action: BindAction, trigger: VKey) -> Self {
        Self::Execute {
            action,
            layer_trigger: Some(trigger),
        }
    }
}

fn combo_trigger_matches(
    trigger: &ComboTrigger,
    vkey: VKey,
    modifiers_held: &HashSet<VKey>,
) -> bool {
    trigger.key == vkey
        && trigger
            .modifiers
            .iter()
            .all(|m| modifier_satisfied(m, modifiers_held))
}

/// 키 바인딩 런타임 상태 (플랫폼 공용)
pub struct EngineState {
    pub config: KeybindConfig,
    keymap_enabled: bool,
    /// 현재 홀드 중인 레이어 트리거 키
    active_layer: Option<usize>,
    /// 레이어 트리거 키가 다운된 시각 (tick count)
    trigger_down_tick: u32,
    /// 레이어 활성 중 다른 키가 눌렸는지 (tap vs hold 판정)
    layer_key_used: bool,
    /// 레이어 매핑의 Launch 액션은 트리거 키를 뗄 때까지 지연 실행한다.
    /// 트리거 수정자(Alt 등)가 눌린 채 실행하면 새 프로세스의 초기
    /// 포커스/IME 조합에 간섭한다 (0.5.0 "Layer Launch deferral").
    pending_layer_launch: Option<BindAction>,
    /// 현재 물리적으로 눌린 수정자 키 추적
    modifiers_held: HashSet<VKey>,
    /// 홀드 중 다른 키와 조합되어 "수정자로 사용된" 키 집합.
    /// 사용된 수정자의 keyup은 더블탭의 탭으로 기록하지 않는다.
    /// (예: RShift+ㅅ=ㅆ 입력 직후 RShift+/=? 가 더블탭 오판정되는 것 방지)
    mods_used_while_held: HashSet<VKey>,
    /// 더블탭: 마지막으로 탭 완료(keyup)된 키
    last_tap_key: Option<VKey>,
    /// 더블탭: 마지막 탭 시각 (tick count)
    last_tap_tick: u32,
    /// 콤보로 소비된 키 (해당 키의 keyup도 억제)
    combo_consumed_key: Option<VKey>,
    /// 더블탭으로 소비된 키 (해당 키의 keyup도 억제)
    dt_consumed_key: Option<VKey>,
    /// 레이어 내 더블탭: 마지막으로 실행된 키
    layer_dt_last_key: Option<VKey>,
    /// 레이어 내 더블탭: 마지막 실행 시각 (tick count)
    layer_dt_last_tick: u32,
    /// 레이어 내 더블탭: 현재 물리적으로 눌려 있는 키.
    /// up 없이 반복되는 down은 OS 오토리피트 — 새 탭으로 세지 않는다
    /// (홀드 시 single↔double이 교대 발사되던 오작동 방지).
    layer_dt_held: Option<VKey>,
    /// 레이어 내 더블탭: 오토리피트 down에 반복 실행할 액션.
    /// single 액션은 반복(연속 단어 이동 등), double 액션은 반복 금지
    /// (줄 삭제 매크로 등 파괴적 액션의 연사 방지 — None).
    layer_dt_repeat_action: Option<BindAction>,
    /// 레이어가 소비한(억제/실행) 채 아직 눌려 있는 비수정자 키.
    /// 트리거가 먼저 떨어지면 orphan으로 승격해 잔여 오토리피트가
    /// 맨키로 누출되는 것을 막는다 (Alt를 먼저 떼면 "hhh" 입력되던 문제).
    layer_keys_down: HashSet<VKey>,
    /// 레이어 비활성 후에도 눌린 채 남아 있는 소비된 키 — keyup까지 억제
    orphaned_layer_keys: HashSet<VKey>,
    /// 패스쓰루 코드(chord) 모드 — 트리거가 OS에 주입된 상태.
    /// true인 동안 모든 키는 OS 조합으로 통과하고, 트리거 up에서
    /// [`KeyDecision::ReleaseChord`]로 해제된다 (A안, docs/08 참조).
    chord_engaged: bool,
    /// 활성 tap-hold(모드탭) 인덱스 — 키 down 후 tap/hold 판정 대기 중
    active_tap_hold: Option<usize>,
    /// tap-hold 키가 다운된 시각 (tick count)
    tap_hold_down_tick: u32,
    /// tap-hold의 hold 수정자가 OS에 주입된 상태 (chord)
    tap_hold_engaged: bool,
    /// 현재 눌려 있는 마우스 바인딩 키 — 레이어 트리거 해제 시 전체 정지용
    mouse_keys_held: HashSet<VKey>,
}

impl EngineState {
    pub fn new(config: KeybindConfig) -> Self {
        Self {
            config,
            keymap_enabled: true,
            active_layer: None,
            trigger_down_tick: 0,
            layer_key_used: false,
            pending_layer_launch: None,
            modifiers_held: HashSet::new(),
            mods_used_while_held: HashSet::new(),
            last_tap_key: None,
            last_tap_tick: 0,
            combo_consumed_key: None,
            dt_consumed_key: None,
            layer_dt_last_key: None,
            layer_dt_last_tick: 0,
            layer_dt_held: None,
            layer_dt_repeat_action: None,
            layer_keys_down: HashSet::new(),
            orphaned_layer_keys: HashSet::new(),
            chord_engaged: false,
            active_tap_hold: None,
            tap_hold_down_tick: 0,
            tap_hold_engaged: false,
            mouse_keys_held: HashSet::new(),
        }
    }

    /// 레이어 비활성화 시 키 잔여 상태 정리 — 아직 눌려 있는 소비된 키를
    /// orphan으로 승격해 오토리피트/keyup 누출을 막고, 더블탭 홀드 상태를 리셋
    fn deactivate_layer_keys(&mut self) {
        self.orphaned_layer_keys
            .extend(self.layer_keys_down.drain());
        self.layer_dt_held = None;
        self.layer_dt_repeat_action = None;
    }

    /// keymap toggle 등으로 상태를 초기화할 때 호출
    fn reset_runtime_state(&mut self) {
        self.active_layer = None;
        self.layer_key_used = false;
        self.pending_layer_launch = None;
        self.mods_used_while_held.clear();
        self.last_tap_key = None;
        self.combo_consumed_key = None;
        self.dt_consumed_key = None;
        self.layer_dt_last_key = None;
        self.layer_dt_held = None;
        self.layer_dt_repeat_action = None;
        self.layer_keys_down.clear();
        self.orphaned_layer_keys.clear();
        self.chord_engaged = false;
        self.active_tap_hold = None;
        self.tap_hold_engaged = false;
        self.mouse_keys_held.clear();
    }

    /// 현재 코드 모드로 OS에 주입돼 있는 트리거 키 (레이어 chord 또는
    /// tap-hold의 hold 수정자). 어댑터가 stop 시 stuck-modifier를 방지하기
    /// 위해 해제(up 주입)에 사용한다.
    pub fn engaged_chord_trigger(&self) -> Option<VKey> {
        if self.chord_engaged {
            return self
                .active_layer
                .and_then(|idx| self.config.layers.get(idx))
                .map(|l| l.trigger);
        }
        if self.tap_hold_engaged {
            return self
                .active_tap_hold
                .and_then(|idx| self.config.tap_holds.get(idx))
                .map(|t| t.hold);
        }
        None
    }

    /// 코드 모드가 걸린 상태에서 상태 초기화가 필요할 때, 주입된 트리거를
    /// 해제할 [`KeyDecision::ReleaseChord`]가 필요한지 확인한다.
    /// (keymap 토글 등이 코드 모드를 끊을 때 stuck-modifier 방지)
    fn take_engaged_chord_trigger(&mut self) -> Option<VKey> {
        let trigger = self.engaged_chord_trigger();
        self.chord_engaged = false;
        self.tap_hold_engaged = false;
        trigger
    }

    // ── 플랫폼 어댑터용 헬퍼 ────────────────────────────────────────────
    //
    // 아래 dead_code 허용은 "쓰는 플랫폼이 따로 있다"는 뜻이다. 조건을 실제
    // 사용처에 맞춰 좁혀 두었으니, 어떤 플랫폼에서도 안 쓰이게 되면 컴파일러가
    // 다시 알려준다 — 무조건 allow로 두면 진짜 죽은 코드가 숨는다.

    /// 훅/탭이 타임아웃 등으로 이벤트를 놓쳤을 때 일시 상태 전체 초기화.
    /// macOS 탭 재활성화(macos.rs)와 Windows 훅 워치독(windows.rs) 양쪽에서 쓴다.
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    pub fn reset_transient_state(&mut self) {
        self.modifiers_held.clear();
        self.reset_runtime_state();
    }

    /// 어댑터의 합성 입력이 실패했을 때 이미 계산 과정에서 전진한 일시 상태를
    /// 되돌린다. 물리 modifier 추적과 이미 소비한 keyup 억제 상태는 보존하고,
    /// 실제로 주입되지 않은 chord/tap-hold 및 레이어 실행 상태만 정리한다.
    /// 반환값은 어댑터가 마우스 워커에 StopAll을 보내야 하는지 나타낸다.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn recover_from_injection_failure(&mut self) -> bool {
        let stop_mouse = !self.mouse_keys_held.is_empty();
        self.active_layer = None;
        self.layer_key_used = false;
        self.pending_layer_launch = None;
        self.layer_dt_last_key = None;
        self.deactivate_layer_keys();
        self.chord_engaged = false;
        self.active_tap_hold = None;
        self.tap_hold_engaged = false;
        self.mouse_keys_held.clear();
        stop_mouse
    }

    /// 해당 키가 현재 눌린 수정자로 추적 중인지 (flagsChanged is_down 판정 폴백).
    /// macOS 전용 — Windows 훅은 이벤트에 down/up이 실려 폴백이 필요 없다.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn is_modifier_held(&self, vkey: VKey) -> bool {
        self.modifiers_held.contains(&vkey)
    }

    /// 현재 눌린 것으로 추적 중인 수정자 목록 (stop 시 stuck-modifier 해제용).
    /// macOS 전용 — 탭 종료 시 남은 수정자를 직접 풀어 준다.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn held_modifiers(&self) -> Vec<VKey> {
        self.modifiers_held.iter().copied().collect()
    }

    /// OS가 보고한 수정자 플래그와 내부 추적 상태를 동기화.
    /// 플래그가 꺼진 수정자는 Left/Right 키를 모두 제거한다
    /// (macOS flagsChanged — 놓친 keyup으로 인한 stuck modifier 방지).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn sync_modifier_flags(&mut self, shift: bool, ctrl: bool, alt: bool, win: bool) {
        if !shift {
            self.modifiers_held.remove(&VKey::LShift);
            self.modifiers_held.remove(&VKey::RShift);
        }
        if !ctrl {
            self.modifiers_held.remove(&VKey::LCtrl);
            self.modifiers_held.remove(&VKey::RCtrl);
        }
        if !alt {
            self.modifiers_held.remove(&VKey::LAlt);
            self.modifiers_held.remove(&VKey::RAlt);
        }
        if !win {
            self.modifiers_held.remove(&VKey::LWin);
            self.modifiers_held.remove(&VKey::RWin);
        }
    }

    /// 키 이벤트 하나를 처리하고 억제/실행 여부를 결정한다.
    ///
    /// `tick`은 밀리초 단위 단조 증가 카운터 (wrapping 허용 —
    /// Windows의 GetTickCount / KBDLLHOOKSTRUCT.time과 동일 규약).
    pub fn process_key(&mut self, vkey: VKey, is_down: bool, tick: u32) -> KeyDecision {
        let is_up = !is_down;

        // 수정자 물리 상태는 어떤 단계보다 먼저 갱신한다 — 아래 모든 판정이
        // `modifiers_held`를 본다.
        self.track_modifier_state(vkey, is_down);

        // **이 순서가 곧 의미다.** 한 칸만 어긋나도 특정 조합에서만 조용히
        // 다르게 동작한다. 순서를 바꾸려면 `process_key_동작_스냅샷` 테스트가
        // 먼저 깨져야 하고, 왜 달라졌는지 설명할 수 있어야 한다.
        if let Some(d) = self.step_consumed_key(vkey, is_up) {
            return d;
        }
        if let Some(d) = self.step_keymap_toggle(vkey, is_down) {
            return d;
        }
        if let Some(d) = self.step_orphaned_layer_key(vkey, is_up) {
            return d;
        }
        if let Some(d) = self.step_keymap_disabled(vkey, is_down) {
            return d;
        }
        if let Some(d) = self.step_tap_hold(vkey, is_down, tick) {
            return d;
        }
        if let Some(d) = self.step_layer_trigger(vkey, is_down, tick) {
            return d;
        }
        if let Some(d) = self.step_layer_mapping(vkey, is_down, tick) {
            return d;
        }
        if let Some(d) = self.step_combo(vkey, is_down) {
            return d;
        }
        if let Some(d) = self.step_double_tap(vkey, is_down, tick) {
            return d;
        }
        if let Some(d) = self.step_remap(vkey, is_down) {
            return d;
        }

        self.step_record_tap(vkey, is_up, tick)
    }

    /// 수정자 키의 물리 눌림 상태를 추적한다 (결정을 내리지 않는다).
    fn track_modifier_state(&mut self, vkey: VKey, is_down: bool) {
        // ── 1. 수정자 키 물리 상태 추적 ──
        // (mods_used_while_held와 modifiers_held는 서로 다른 필드라 동시 차용
        // 가능 — 키 이벤트마다 도는 훅 경로이므로 중간 Vec 할당 없이 복사)
        if is_modifier_key(&vkey) {
            if is_down {
                // 새 수정자가 합류하면 기존 홀드 중 수정자들은 "사용됨"으로 표시
                self.mods_used_while_held
                    .extend(self.modifiers_held.iter().copied());
                self.modifiers_held.insert(vkey);
                // 새로 눌린 키 자신은 깨끗한 탭 후보로 시작
                self.mods_used_while_held.remove(&vkey);
            } else {
                self.modifiers_held.remove(&vkey);
            }
        } else if is_down {
            // 일반 키 down과 함께 홀드 중인 수정자는 전부 "사용됨" — 이후 keyup이
            // 더블탭의 탭으로 기록되지 않게 한다 (콤보로 소비되기 전에 마킹)
            self.mods_used_while_held
                .extend(self.modifiers_held.iter().copied());
        }
    }

    /// 콤보·더블탭으로 이미 소비된 키를 억제한다 (오토리피트 재발화 방지).
    ///
    /// `None`이면 다음 단계로 넘어간다 — 반환 순서가 곧 우선순위다
    /// (`process_key` 참조).
    fn step_consumed_key(&mut self, vkey: VKey, is_up: bool) -> Option<KeyDecision> {
        // ── 2. 콤보/더블탭으로 소비된 키 억제 ──
        // keyup은 물론, 홀드 중 오토리피트 down도 막는다 — 안 막으면 Shift+Space
        // 홀드 시 리피트마다 콤보(한/영 토글 등)가 재발화한다. 진짜 재입력은
        // keyup이 먼저 와서 소비 상태를 지우므로 정상 발화한다.
        if self.combo_consumed_key == Some(vkey) {
            if is_up {
                self.combo_consumed_key = None;
            }
            return Some(KeyDecision::Suppress);
        }
        if self.dt_consumed_key == Some(vkey) {
            if is_up {
                self.dt_consumed_key = None;
            }
            return Some(KeyDecision::Suppress);
        }
        None
    }

    /// 키맵 전체 on/off 토글 조합을 처리한다.
    ///
    /// `None`이면 다음 단계로 넘어간다 — 반환 순서가 곧 우선순위다
    /// (`process_key` 참조).
    fn step_keymap_toggle(&mut self, vkey: VKey, is_down: bool) -> Option<KeyDecision> {
        // ── keymap on/off 토글 ──
        if is_down && !is_modifier_key(&vkey) {
            if let Some(toggle) = self.config.toggle_keymap.as_ref() {
                if combo_trigger_matches(toggle, vkey, &self.modifiers_held) {
                    let enabled = !self.keymap_enabled;
                    // 코드 모드를 끊는 경우 주입된 트리거를 해제해야 한다
                    let chord_trigger = self.take_engaged_chord_trigger();
                    let had_mouse = !self.mouse_keys_held.is_empty();
                    self.reset_runtime_state();
                    self.keymap_enabled = enabled;
                    self.combo_consumed_key = Some(vkey);
                    if let Some(trigger) = chord_trigger {
                        return Some(KeyDecision::ReleaseChord {
                            trigger,
                            deferred_action: None,
                        });
                    }
                    if had_mouse {
                        return Some(KeyDecision::MouseStopAll);
                    }
                    return Some(KeyDecision::Suppress);
                }
            }
        }
        None
    }

    /// 트리거를 먼저 뗀 뒤 남은 레이어 키의 누출을 막는다.
    ///
    /// `None`이면 다음 단계로 넘어간다 — 반환 순서가 곧 우선순위다
    /// (`process_key` 참조).
    fn step_orphaned_layer_key(&mut self, vkey: VKey, is_up: bool) -> Option<KeyDecision> {
        // ── 2.1 레이어가 소비한 채 남겨진(orphan) 키 억제 ──
        // 트리거를 먼저 뗀 뒤에도 눌려 있는 키의 오토리피트 down이
        // 맨키로 누출되지 않게 keyup까지 전부 억제한다.
        if self.orphaned_layer_keys.contains(&vkey) {
            if is_up {
                self.orphaned_layer_keys.remove(&vkey);
            }
            return Some(KeyDecision::Suppress);
        }
        None
    }

    /// 키맵이 꺼져 있을 때 — Launch 콤보만 살려 두고 나머지는 통과시킨다.
    ///
    /// `None`이면 다음 단계로 넘어간다 — 반환 순서가 곧 우선순위다
    /// (`process_key` 참조).
    fn step_keymap_disabled(&mut self, vkey: VKey, is_down: bool) -> Option<KeyDecision> {
        // ── keymap 비활성 시: Launch 콤보만 동작 ──
        if !self.keymap_enabled {
            if is_down && !is_modifier_key(&vkey) {
                let combo_action = self
                    .config
                    .combos
                    .iter()
                    .find(|(trigger, action)| {
                        matches!(action, BindAction::Launch(_))
                            && combo_trigger_matches(trigger, vkey, &self.modifiers_held)
                    })
                    .map(|(_, action)| action.clone());

                if let Some(action) = combo_action {
                    self.combo_consumed_key = Some(vkey);
                    return Some(KeyDecision::execute(action));
                }
            }
            return Some(KeyDecision::PassThrough);
        }
        None
    }

    /// tap-hold(모드탭): 짧게 탭 = tap 키, 홀드 중 다른 키 = hold 수정자 chord.
    ///
    /// `None`이면 다음 단계로 넘어간다 — 반환 순서가 곧 우선순위다
    /// (`process_key` 참조).
    fn step_tap_hold(&mut self, vkey: VKey, is_down: bool, tick: u32) -> Option<KeyDecision> {
        // ── 2.5 tap-hold(모드탭) 처리 ──
        // 짧게 탭 = tap 키, 홀드 중 다른 키 down = hold 수정자 chord (즉시 판정).
        // 홀드만 하다 떼면(타임아웃 초과) 아무 동작 없음 — HHKB 동작과 동일.
        if is_down {
            if let Some(idx) = self.config.tap_holds.iter().position(|t| t.key == vkey) {
                if self.active_tap_hold.is_none() {
                    self.active_tap_hold = Some(idx);
                    self.tap_hold_down_tick = tick;
                    self.tap_hold_engaged = false;
                    // 레이어 활성 중이면 "키 사용됨" — 레이어 tap 오발동 방지
                    if self.active_layer.is_some() {
                        self.layer_key_used = true;
                    }
                    return Some(KeyDecision::Suppress);
                }
                if self.active_tap_hold == Some(idx) {
                    // OS 오토리피트 — 억제 (down_tick은 최초 값 유지)
                    return Some(KeyDecision::Suppress);
                }
                // 다른 tap-hold가 활성 중 → 아래 '다른 키' 처리로 폴스루
            }
        } else if let Some(idx) = self.active_tap_hold {
            if self.config.tap_holds[idx].key == vkey {
                let th = self.config.tap_holds[idx].clone();
                let engaged = self.tap_hold_engaged;
                self.active_tap_hold = None;
                self.tap_hold_engaged = false;

                if engaged {
                    // hold 수정자가 주입돼 있음 — up 주입으로 해제
                    return Some(KeyDecision::ReleaseChord {
                        trigger: th.hold,
                        deferred_action: None,
                    });
                }
                let elapsed = tick.wrapping_sub(self.tap_hold_down_tick);
                if elapsed < th.timeout_ms {
                    if let Some(tap_key) = th.tap {
                        return Some(KeyDecision::execute(BindAction::SendKey(tap_key)));
                    }
                }
                return Some(KeyDecision::Suppress);
            }
        }

        // 활성 tap-hold 중 다른 키 처리
        if let Some(th_idx) = self.active_tap_hold {
            if self.tap_hold_engaged {
                // hold 수정자가 주입된 상태 — 모든 키를 OS 조합으로 통과
                return Some(KeyDecision::PassThrough);
            }
            if is_down && !is_modifier_key(&vkey) {
                // 다른 키 down → hold 확정. 수정자 down + 키 down을 원자
                // 주입해 Ctrl+C 등이 타임아웃 대기 없이 즉시 동작한다.
                let hold = self.config.tap_holds[th_idx].hold;
                self.tap_hold_engaged = true;
                return Some(KeyDecision::EngageChord {
                    trigger: hold,
                    key: vkey,
                });
            }
            // 수정자 down/키 up은 아래 일반 흐름으로 (물리 수정자는 통과 —
            // 이후 키 down에서 hold와 함께 OS 조합이 된다: Caps+Shift+K 등)
        }
        None
    }

    /// 레이어 트리거 키의 down/up — 레이어 진입·해제와 탭 액션을 결정한다.
    ///
    /// `None`이면 다음 단계로 넘어간다 — 반환 순서가 곧 우선순위다
    /// (`process_key` 참조).
    fn step_layer_trigger(&mut self, vkey: VKey, is_down: bool, tick: u32) -> Option<KeyDecision> {
        // ── 3. 레이어 트리거 키 처리 ──
        if is_down {
            if let Some(idx) = self
                .config
                .layers
                .iter()
                .position(|l| l.matches_trigger(vkey))
            {
                if self.active_layer.is_none() {
                    self.active_layer = Some(idx);
                    self.trigger_down_tick = tick;
                    self.layer_key_used = false;
                    self.pending_layer_launch = None;
                }
                return Some(KeyDecision::Suppress);
            }
        } else if let Some(active_idx) = self.active_layer {
            let is_active_trigger = self
                .config
                .layers
                .get(active_idx)
                .map(|l| l.matches_trigger(vkey))
                .unwrap_or(false);
            if is_active_trigger {
                // 코드 모드 종료 — 주입된 트리거 해제 (tap 판정 없음)
                if self.chord_engaged {
                    self.chord_engaged = false;
                    self.active_layer = None;
                    self.layer_key_used = false;
                    self.deactivate_layer_keys();
                    let deferred = self.pending_layer_launch.take();
                    return Some(KeyDecision::ReleaseChord {
                        trigger: vkey,
                        deferred_action: deferred,
                    });
                }
                let elapsed = tick.wrapping_sub(self.trigger_down_tick);
                let layer = &self.config.layers[active_idx];
                let tap_hold_ms = layer.tap_hold_ms;
                let tap_action = layer.tap_action;
                let was_used = self.layer_key_used;
                let pending_launch = self.pending_layer_launch.take();

                self.active_layer = None;
                self.layer_key_used = false;
                self.deactivate_layer_keys();

                // 이동/버튼 키가 눌린 채 트리거를 뗐다 — 해당 키들의 keyup은
                // 더 이상 레이어 매핑으로 오지 않으므로 여기서 전부 정지
                if !self.mouse_keys_held.is_empty() {
                    self.mouse_keys_held.clear();
                    return Some(KeyDecision::MouseStopAll);
                }

                if !was_used && elapsed < tap_hold_ms {
                    if let Some(tap_key) = tap_action {
                        return Some(KeyDecision::execute(BindAction::SendKey(tap_key)));
                    }
                } else if let Some(action) = pending_launch {
                    // 지연된 Launch — 트리거 키가 떨어진 지금 실행
                    return Some(KeyDecision::execute(action));
                }
                return Some(KeyDecision::Suppress);
            }
        }
        None
    }

    /// 활성 레이어 안에서의 키 매핑 (더블탭·마우스·미매핑 동작 포함).
    ///
    /// `None`이면 다음 단계로 넘어간다 — 반환 순서가 곧 우선순위다
    /// (`process_key` 참조).
    fn step_layer_mapping(&mut self, vkey: VKey, is_down: bool, tick: u32) -> Option<KeyDecision> {
        // ── 4. 활성 레이어 매핑 확인 ──
        if let Some(layer_idx) = self.active_layer {
            let trigger = self.config.layers[layer_idx].trigger;

            // 4-pre. 코드 모드 중에는 홀드가 끝날 때까지 매핑 키 포함 전부
            // OS 조합으로 통과한다 (A안 — OS에는 주입된 트리거가 눌려 있음)
            if self.chord_engaged {
                return Some(KeyDecision::PassThrough);
            }

            // 4-pre2. 트리거 외의 비-Shift 수정자(Cmd/Ctrl/Win)가 함께 눌린
            // 키 down은 매핑을 건너뛰고 트리거 조합으로 OS에 투과한다
            // (Cmd+Alt+H = "다른 앱 가리기" 등 OS 조합 보존).
            // Shift는 예외 — Shift+네비 키 = 선택 확장이 더 유용하다.
            if is_down
                && !is_modifier_key(&vkey)
                && self.config.layers[layer_idx].unmapped == UnmappedBehavior::Passthrough
                && is_modifier_key(&trigger)
            {
                let other_mod_held = self.modifiers_held.iter().any(|m| {
                    !self.config.layers[layer_idx].matches_trigger(*m)
                        && !matches!(m, VKey::LShift | VKey::RShift)
                });
                if other_mod_held {
                    self.chord_engaged = true;
                    self.layer_key_used = true;
                    return Some(KeyDecision::EngageChord { trigger, key: vkey });
                }
            }

            // 4a. 레이어 내 더블탭 매핑 우선 확인
            let dt_opt = self.config.layers[layer_idx]
                .double_tap_mappings
                .get(&vkey)
                .cloned();

            if let Some(dt) = dt_opt {
                if is_down {
                    // up 없이 반복된 down = OS 오토리피트 — 탭으로 세지 않는다.
                    // single 액션은 반복 실행(연속 단어 이동), double 액션 직후는
                    // 반복 금지(줄 삭제 매크로 연사 방지 → 억제)
                    if self.layer_dt_held == Some(vkey) {
                        return Some(match self.layer_dt_repeat_action.clone() {
                            Some(action) => KeyDecision::execute_in_layer(action, trigger),
                            None => KeyDecision::Suppress,
                        });
                    }

                    self.layer_key_used = true;
                    self.layer_dt_held = Some(vkey);
                    self.layer_keys_down.insert(vkey);

                    // 이전 탭과 동일 키이고 timeout 이내면 → 더블탭 액션
                    if self.layer_dt_last_key == Some(vkey) {
                        let elapsed = tick.wrapping_sub(self.layer_dt_last_tick);
                        if elapsed < dt.timeout_ms {
                            self.layer_dt_last_key = None;
                            self.layer_dt_repeat_action = None;
                            return Some(KeyDecision::execute_in_layer(dt.double_action, trigger));
                        }
                    }

                    // 첫 번째 탭 → 싱글 액션 즉시 실행, 더블탭 대기 기록
                    self.layer_dt_last_key = Some(vkey);
                    self.layer_dt_last_tick = tick;
                    self.layer_dt_repeat_action = Some(dt.single_action.clone());
                    return Some(KeyDecision::execute_in_layer(dt.single_action, trigger));
                }
                if self.layer_dt_held == Some(vkey) {
                    self.layer_dt_held = None;
                }
                // 우리가 소비한 down의 up만 억제 — 레이어 활성 전부터 눌려
                // 있던 키의 up은 통과 (stuck 방지)
                if self.layer_keys_down.remove(&vkey) {
                    return Some(KeyDecision::Suppress);
                }
                return Some(KeyDecision::PassThrough);
            }

            // 4b. 일반 레이어 매핑
            let action_opt = self.config.layers[layer_idx].mappings.get(&vkey).cloned();

            if let Some(action) = action_opt {
                // 마우스 바인딩은 상태형 — down=시작/버튼다운, up=정지/버튼업
                if let BindAction::Mouse(mb) = action {
                    if is_down {
                        if self.mouse_keys_held.contains(&vkey) {
                            // OS 오토리피트 — 이미 활성
                            return Some(KeyDecision::Suppress);
                        }
                        self.layer_key_used = true;
                        self.layer_dt_last_key = None;
                        self.mouse_keys_held.insert(vkey);
                        return Some(KeyDecision::MouseEngage(mb));
                    }
                    if self.mouse_keys_held.remove(&vkey) {
                        return Some(KeyDecision::MouseRelease(mb));
                    }
                    // 레이어 활성 전부터 눌려 있던 키의 up — 통과 (stuck 방지)
                    return Some(KeyDecision::PassThrough);
                }
                if is_down {
                    self.layer_key_used = true;
                    self.layer_dt_last_key = None;
                    self.layer_keys_down.insert(vkey);
                    // Launch는 트리거 키를 뗄 때까지 지연 — 트리거 수정자가
                    // 눌린 채 실행하면 새 프로세스 포커스/IME에 간섭한다
                    if matches!(action, BindAction::Launch(_)) {
                        self.pending_layer_launch = Some(action);
                        return Some(KeyDecision::Suppress);
                    }
                    return Some(KeyDecision::execute_in_layer(action, trigger));
                }
                // 우리가 소비한 down의 up만 억제 (stuck 방지)
                if self.layer_keys_down.remove(&vkey) {
                    return Some(KeyDecision::Suppress);
                }
                return Some(KeyDecision::PassThrough);
            }

            // 4c. 미매핑 키 — 레이어별 unmapped 정책 적용
            match self.config.layers[layer_idx].unmapped {
                // 현행 동작: 아래 콤보/더블탭/리맵 검사로 폴스루 (맨키 통과)
                UnmappedBehavior::Plain => {}
                // VIA KC_NO: 비수정자 키 억제 (수정자는 추적을 위해 통과)
                UnmappedBehavior::Block => {
                    if !is_modifier_key(&vkey) {
                        if is_down {
                            self.layer_key_used = true;
                            self.layer_keys_down.insert(vkey);
                            return Some(KeyDecision::Suppress);
                        }
                        // 우리가 소비한 down의 up만 억제 (stuck 방지)
                        if self.layer_keys_down.remove(&vkey) {
                            return Some(KeyDecision::Suppress);
                        }
                        return Some(KeyDecision::PassThrough);
                    }
                }
                // VIA KC_TRNS: 트리거 조합으로 코드 모드 진입.
                // 트리거가 수정자일 때만 성립 — 아니면 Plain 폴백
                UnmappedBehavior::Passthrough => {
                    if is_down && !is_modifier_key(&vkey) && is_modifier_key(&trigger) {
                        self.chord_engaged = true;
                        self.layer_key_used = true;
                        return Some(KeyDecision::EngageChord { trigger, key: vkey });
                    }
                }
            }
        }
        None
    }

    /// 수정자+키 콤보 리맵.
    ///
    /// `None`이면 다음 단계로 넘어간다 — 반환 순서가 곧 우선순위다
    /// (`process_key` 참조).
    fn step_combo(&mut self, vkey: VKey, is_down: bool) -> Option<KeyDecision> {
        // ── 5. 콤보 리맵 확인 (수정자+키 조합) ──
        if is_down && !is_modifier_key(&vkey) {
            let combo_action = self
                .config
                .combos
                .iter()
                .find(|(trigger, _)| combo_trigger_matches(trigger, vkey, &self.modifiers_held))
                .map(|(_, action)| action.clone());

            if let Some(action) = combo_action {
                self.combo_consumed_key = Some(vkey);
                return Some(KeyDecision::execute(action));
            }
        }
        None
    }

    /// 전역 더블탭 판정.
    ///
    /// `None`이면 다음 단계로 넘어간다 — 반환 순서가 곧 우선순위다
    /// (`process_key` 참조).
    fn step_double_tap(&mut self, vkey: VKey, is_down: bool, tick: u32) -> Option<KeyDecision> {
        // ── 6. 더블탭 확인 ──
        if is_down {
            let dt_binding = self
                .config
                .double_taps
                .iter()
                .find(|dt| dt.key == vkey)
                .cloned();

            if let Some(dt) = dt_binding {
                if self.last_tap_key == Some(vkey) {
                    let elapsed = tick.wrapping_sub(self.last_tap_tick);
                    if elapsed < dt.timeout_ms {
                        self.last_tap_key = None;
                        self.dt_consumed_key = Some(vkey);
                        if is_modifier_key(&vkey) {
                            self.modifiers_held.remove(&vkey);
                        }
                        return Some(KeyDecision::execute(dt.action));
                    }
                }
            } else if !is_modifier_key(&vkey) {
                self.last_tap_key = None;
            }
        }
        None
    }

    /// 단순 1:1 리매핑.
    ///
    /// `None`이면 다음 단계로 넘어간다 — 반환 순서가 곧 우선순위다
    /// (`process_key` 참조).
    fn step_remap(&mut self, vkey: VKey, is_down: bool) -> Option<KeyDecision> {
        // ── 7. 단순 리매핑 확인 ──
        if let Some(action) = self.config.remaps.get(&vkey).cloned() {
            if is_down {
                return Some(KeyDecision::execute(action));
            }
            return Some(KeyDecision::Suppress);
        }
        None
    }

    /// keyup 시 더블탭용 탭 완료를 기록하고, 아무도 처리하지 않은
    /// 이벤트를 OS로 통과시킨다.
    fn step_record_tap(&mut self, vkey: VKey, is_up: bool, tick: u32) -> KeyDecision {
        // ── 8. 더블탭 상태 기록 (keyup 시 탭 완료 기록) ──
        if is_up {
            // 홀드 중 다른 키와 조합된 수정자는 탭으로 기록하지 않는다
            let was_used = self.mods_used_while_held.remove(&vkey);
            let has_dt = self.config.double_taps.iter().any(|dt| dt.key == vkey);
            if has_dt && !was_used {
                self.last_tap_key = Some(vkey);
                self.last_tap_tick = tick;
            }
        }

        KeyDecision::PassThrough
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybind::{DoubleTapBinding, Layer, LayerDoubleTap, Modifier};
    use std::collections::HashMap;

    // ── Characterization: process_key 전체 동작 스냅샷 ──────────────────
    //
    // process_key는 451줄 단일 함수였고 섹션 우선순위(수정자 추적 → 소비 억제
    // → 토글 → orphan → 비활성 → tap-hold → 레이어 트리거 → 레이어 매핑 →
    // 콤보 → 더블탭 → 리맵 → 탭 기록)가 곧 의미다. 함수를 쪼갤 때 이 순서가
    // 한 칸이라도 어긋나면 특정 조합에서만 조용히 다르게 동작한다.
    //
    // 그래서 **분해 전에** 실제 결정 시퀀스를 통째로 박아 둔다. 아래 기대값은
    // 분해 이전 구현이 낸 출력이며, 리팩토링이 의미를 바꾸면 여기서 깨진다.
    // 값을 "고쳐서 통과시키는" 것은 이 테스트의 목적을 무효화한다 —
    // 달라졌다면 먼저 왜 달라졌는지 설명할 수 있어야 한다.

    /// 레이어·콤보·더블탭·탭홀드·리맵·토글을 한 설정에 모은 종합 시나리오용 config
    fn characterization_config() -> KeybindConfig {
        let mut cfg = layer_config();
        cfg.remaps
            .insert(VKey::CapsLock, BindAction::SendKey(VKey::Escape));
        cfg.toggle_keymap = Some(ComboTrigger {
            key: VKey::K,
            modifiers: vec![Modifier::Ctrl, Modifier::Alt],
        });
        cfg.combos.push((
            ComboTrigger {
                key: VKey::Space,
                modifiers: vec![Modifier::Shift],
            },
            BindAction::SendKey(VKey::Hangul),
        ));
        cfg
    }

    /// 키 시퀀스를 흘려 결정 문자열 목록을 만든다.
    fn trace(cfg: KeybindConfig, seq: &[(VKey, bool, u32)]) -> Vec<String> {
        let mut e = EngineState::new(cfg);
        seq.iter()
            .map(|(k, down, t)| format!("{:?}", e.process_key(*k, *down, *t)))
            .collect()
    }

    #[test]
    fn process_key_동작_스냅샷() {
        let seq: Vec<(VKey, bool, u32)> = vec![
            // 평범한 타이핑
            (VKey::A, true, 0),
            (VKey::A, false, 50),
            // 리맵 (CapsLock → Escape)
            (VKey::CapsLock, true, 100),
            (VKey::CapsLock, false, 150),
            // 레이어 홀드 + 매핑 키 (LAlt 홀드 → H = Left)
            (VKey::LAlt, true, 200),
            (VKey::H, true, 250),
            (VKey::H, false, 300),
            (VKey::LAlt, false, 400),
            // 레이어 탭 (tap_hold_ms 안에 뗌 → tap_action)
            (VKey::LAlt, true, 500),
            (VKey::LAlt, false, 550),
            // 레이어 안 더블탭 (I 연타)
            (VKey::LAlt, true, 600),
            (VKey::I, true, 650),
            (VKey::I, false, 680),
            (VKey::I, true, 700),
            (VKey::I, false, 730),
            (VKey::LAlt, false, 800),
            // Shift+Space 콤보
            (VKey::LShift, true, 900),
            (VKey::Space, true, 950),
            (VKey::Space, false, 1000),
            (VKey::LShift, false, 1050),
            // Ctrl+Alt+K 토글 → 키맵 비활성
            (VKey::LCtrl, true, 1100),
            (VKey::LAlt, true, 1150),
            (VKey::K, true, 1200),
            (VKey::K, false, 1250),
            (VKey::LAlt, false, 1300),
            (VKey::LCtrl, false, 1350),
            // 비활성 상태에서는 레이어가 안 먹는다
            (VKey::LAlt, true, 1400),
            (VKey::H, true, 1450),
            (VKey::H, false, 1500),
            (VKey::LAlt, false, 1550),
            // 다시 토글 → 활성 복귀
            (VKey::LCtrl, true, 1600),
            (VKey::LAlt, true, 1650),
            (VKey::K, true, 1700),
            (VKey::K, false, 1750),
            (VKey::LAlt, false, 1800),
            (VKey::LCtrl, false, 1850),
            // 복귀 후 레이어 재확인
            (VKey::LAlt, true, 1900),
            (VKey::H, true, 1950),
            (VKey::H, false, 2000),
            (VKey::LAlt, false, 2050),
        ];

        let got = trace(characterization_config(), &seq);

        // 분해 이전 구현의 출력 (2026-08-27 채취)
        let expected: &[&str] = &[
            "PassThrough",
            "PassThrough",
            "Execute { action: SendKey(Escape), layer_trigger: None }",
            "Suppress",
            "Suppress",
            "Execute { action: SendKey(Left), layer_trigger: Some(LAlt) }",
            "Suppress",
            "Suppress",
            "Suppress",
            "Execute { action: SendKey(Escape), layer_trigger: None }",
            "Suppress",
            "Execute { action: SendKey(Home), layer_trigger: Some(LAlt) }",
            "Suppress",
            "Execute { action: SendKey(End), layer_trigger: Some(LAlt) }",
            "Suppress",
            "Suppress",
            "PassThrough",
            "Execute { action: SendKey(Hangul), layer_trigger: None }",
            "Suppress",
            "PassThrough",
            "PassThrough",
            "Suppress",
            "Suppress",
            "Suppress",
            "PassThrough",
            "PassThrough",
            "PassThrough",
            "PassThrough",
            "PassThrough",
            "PassThrough",
            "PassThrough",
            "PassThrough",
            "Suppress",
            "Suppress",
            "PassThrough",
            "PassThrough",
            "Suppress",
            "Execute { action: SendKey(Left), layer_trigger: Some(LAlt) }",
            "Suppress",
            "Suppress",
        ];

        assert_eq!(
            got.len(),
            expected.len(),
            "시퀀스 길이 불일치 — 테스트를 고치기 전에 왜 달라졌는지 확인할 것"
        );
        for (i, (g, e)) in got.iter().zip(expected.iter()).enumerate() {
            let (key, down, tick) = seq[i];
            assert_eq!(
                g, e,
                "step {i} ({key:?} down={down} tick={tick}) 의 결정이 달라졌다"
            );
        }
    }

    fn empty_config() -> KeybindConfig {
        KeybindConfig::empty()
    }

    /// CapsLock → Escape 리맵 설정
    fn remap_config() -> KeybindConfig {
        let mut cfg = empty_config();
        cfg.remaps
            .insert(VKey::CapsLock, BindAction::SendKey(VKey::Escape));
        cfg
    }

    /// vim-nav 스타일 레이어 (LAlt 홀드, tap=Escape, H→Left, I 더블탭)
    fn layer_config() -> KeybindConfig {
        let mut mappings = HashMap::new();
        mappings.insert(VKey::H, BindAction::SendKey(VKey::Left));

        let mut dt_mappings = HashMap::new();
        dt_mappings.insert(
            VKey::I,
            LayerDoubleTap {
                single_action: BindAction::SendKey(VKey::Home),
                double_action: BindAction::SendKey(VKey::End),
                timeout_ms: 300,
            },
        );

        let mut cfg = empty_config();
        cfg.layers.push(Layer {
            name: "nav".into(),
            trigger: VKey::LAlt,
            trigger_aliases: Vec::new(),
            tap_action: Some(VKey::Escape),
            tap_hold_ms: 200,
            unmapped: UnmappedBehavior::Plain,
            mappings,
            double_tap_mappings: dt_mappings,
        });
        cfg
    }

    /// layer_config에서 unmapped 정책만 바꾼 변형
    fn layer_config_unmapped(behavior: UnmappedBehavior) -> KeybindConfig {
        let mut cfg = layer_config();
        cfg.layers[0].unmapped = behavior;
        cfg
    }

    fn assert_execute_sendkey(decision: KeyDecision, expected: VKey) {
        match decision {
            KeyDecision::Execute {
                action: BindAction::SendKey(k),
                ..
            } => assert_eq!(k, expected),
            other => panic!("Execute(SendKey({expected:?})) 기대, 실제: {other:?}"),
        }
    }

    // ── 단순 리맵 ──

    #[test]
    fn remap_down_executes_up_suppressed() {
        let mut e = EngineState::new(remap_config());
        assert_execute_sendkey(e.process_key(VKey::CapsLock, true, 0), VKey::Escape);
        assert!(matches!(
            e.process_key(VKey::CapsLock, false, 50),
            KeyDecision::Suppress
        ));
    }

    #[test]
    fn unbound_key_passes_through() {
        let mut e = EngineState::new(remap_config());
        assert!(matches!(
            e.process_key(VKey::A, true, 0),
            KeyDecision::PassThrough
        ));
        assert!(matches!(
            e.process_key(VKey::A, false, 30),
            KeyDecision::PassThrough
        ));
    }

    // ── 레이어 tap vs hold ──

    #[test]
    fn layer_quick_tap_sends_tap_action() {
        let mut e = EngineState::new(layer_config());
        // 트리거 down → 억제, 레이어 활성
        assert!(matches!(
            e.process_key(VKey::LAlt, true, 1000),
            KeyDecision::Suppress
        ));
        // 199ms 후 up, 다른 키 미사용 → tap 액션 (Escape)
        assert_execute_sendkey(e.process_key(VKey::LAlt, false, 1199), VKey::Escape);
    }

    #[test]
    fn layer_long_hold_does_not_send_tap() {
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, 1000);
        // tap_hold_ms(200) 초과 → tap 없음
        assert!(matches!(
            e.process_key(VKey::LAlt, false, 1300),
            KeyDecision::Suppress
        ));
    }

    #[test]
    fn layer_mapping_fires_and_cancels_tap() {
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, 1000);
        // 레이어 활성 중 H down → Left
        assert_execute_sendkey(e.process_key(VKey::H, true, 1050), VKey::Left);
        // H up → 억제
        assert!(matches!(
            e.process_key(VKey::H, false, 1080),
            KeyDecision::Suppress
        ));
        // 빠르게 뗐어도 키를 사용했으므로 tap 액션 없음
        assert!(matches!(
            e.process_key(VKey::LAlt, false, 1100),
            KeyDecision::Suppress
        ));
    }

    #[test]
    fn layer_deactivates_after_trigger_up() {
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, 1000);
        e.process_key(VKey::LAlt, false, 1500);
        // 레이어 해제 후 H는 일반 키
        assert!(matches!(
            e.process_key(VKey::H, true, 2000),
            KeyDecision::PassThrough
        ));
    }

    // ── 레이어 내 더블탭 ──

    #[test]
    fn layer_double_tap_single_then_double() {
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, 1000);

        // 첫 탭 → 싱글 액션 (Home)
        assert_execute_sendkey(e.process_key(VKey::I, true, 1100), VKey::Home);
        assert!(matches!(
            e.process_key(VKey::I, false, 1130),
            KeyDecision::Suppress
        ));

        // timeout(300ms) 이내 두 번째 탭 → 더블 액션 (End)
        assert_execute_sendkey(e.process_key(VKey::I, true, 1250), VKey::End);
    }

    #[test]
    fn layer_double_tap_timeout_resets_to_single() {
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, 1000);

        assert_execute_sendkey(e.process_key(VKey::I, true, 1100), VKey::Home);
        e.process_key(VKey::I, false, 1130);

        // timeout 초과 → 다시 싱글 액션
        assert_execute_sendkey(e.process_key(VKey::I, true, 1500), VKey::Home);
    }

    #[test]
    fn layer_double_tap_autorepeat_repeats_single_not_double() {
        // 홀드 중 OS 오토리피트 down(up 없는 반복)이 더블탭으로 오판정되어
        // single↔double이 교대 발사되던 문제 — 리피트는 single만 반복한다
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, 1000);

        assert_execute_sendkey(e.process_key(VKey::I, true, 1100), VKey::Home);
        // timeout(300ms) 이내 오토리피트 down → double(End) 아님, single 반복
        assert_execute_sendkey(e.process_key(VKey::I, true, 1150), VKey::Home);
        assert_execute_sendkey(e.process_key(VKey::I, true, 1180), VKey::Home);
        assert!(matches!(
            e.process_key(VKey::I, false, 1200),
            KeyDecision::Suppress
        ));

        // up 이후의 진짜 재탭(timeout 이내)은 더블탭
        assert_execute_sendkey(e.process_key(VKey::I, true, 1250), VKey::End);
    }

    #[test]
    fn layer_double_tap_double_action_does_not_autorepeat() {
        // double 액션(줄 삭제 매크로 등 파괴적일 수 있음)은 오토리피트로
        // 연사되지 않는다
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, 1000);

        assert_execute_sendkey(e.process_key(VKey::I, true, 1100), VKey::Home);
        e.process_key(VKey::I, false, 1130);
        assert_execute_sendkey(e.process_key(VKey::I, true, 1200), VKey::End); // 더블
                                                                               // 홀드 유지 → 오토리피트 down 전부 억제
        assert!(matches!(
            e.process_key(VKey::I, true, 1250),
            KeyDecision::Suppress
        ));
        assert!(matches!(
            e.process_key(VKey::I, true, 1290),
            KeyDecision::Suppress
        ));
    }

    #[test]
    fn layer_keys_orphaned_on_trigger_release_do_not_leak() {
        // Alt+H 홀드 중 Alt를 먼저 떼면, 계속 눌려 있는 H의 오토리피트가
        // 맨키 'h'로 누출되던 문제 — keyup까지 전부 억제한다
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, 1000);
        assert_execute_sendkey(e.process_key(VKey::H, true, 1050), VKey::Left);

        e.process_key(VKey::LAlt, false, 1300); // 트리거 먼저 해제 (H는 아직 홀드)

        // H 오토리피트 down/최종 up 모두 억제 — 'h' 누출 없음
        assert!(matches!(
            e.process_key(VKey::H, true, 1350),
            KeyDecision::Suppress
        ));
        assert!(matches!(
            e.process_key(VKey::H, false, 1400),
            KeyDecision::Suppress
        ));
        // 완전히 뗀 뒤의 새 입력은 정상 통과
        assert!(matches!(
            e.process_key(VKey::H, true, 1500),
            KeyDecision::PassThrough
        ));
    }

    #[test]
    fn layer_mapped_key_up_from_before_activation_passes_through() {
        // 레이어 활성 전부터 눌려 있던 매핑 키의 up은 억제하지 않는다 (stuck 방지)
        let mut e = EngineState::new(layer_config());
        assert!(matches!(
            e.process_key(VKey::H, true, 900),
            KeyDecision::PassThrough
        ));
        e.process_key(VKey::LAlt, true, 1000);
        assert!(matches!(
            e.process_key(VKey::H, false, 1050),
            KeyDecision::PassThrough
        ));
    }

    #[test]
    fn layer_mapping_with_cmd_held_engages_chord() {
        // Cmd(Win)+Alt+H — 매핑(Left) 대신 OS 조합으로 투과해
        // "다른 앱 가리기"(Cmd+Alt+H) 등이 살아 있어야 한다
        let mut e = EngineState::new(layer_config_unmapped(UnmappedBehavior::Passthrough));
        e.process_key(VKey::LAlt, true, 1000);
        assert!(matches!(
            e.process_key(VKey::LWin, true, 1020),
            KeyDecision::PassThrough
        ));
        assert!(matches!(
            e.process_key(VKey::H, true, 1050),
            KeyDecision::EngageChord {
                trigger: VKey::LAlt,
                key: VKey::H,
            }
        ));
    }

    #[test]
    fn layer_mapping_with_shift_held_still_maps() {
        // Shift+Alt+H는 매핑 유지 — 어댑터가 Shift를 병합해 Shift+Left(선택 확장)
        let mut e = EngineState::new(layer_config_unmapped(UnmappedBehavior::Passthrough));
        e.process_key(VKey::LAlt, true, 1000);
        assert!(matches!(
            e.process_key(VKey::LShift, true, 1020),
            KeyDecision::PassThrough
        ));
        assert_execute_sendkey(e.process_key(VKey::H, true, 1050), VKey::Left);
    }

    // ── 레이어 패스쓰루 (코드 모드) — docs/08 P0/P1 ──

    /// Alt 홀드 + 미매핑 Tab → 코드 진입, Tab up 통과, Alt up → 해제 (tap 없음)
    #[test]
    fn passthrough_unmapped_key_engages_and_releases_chord() {
        let mut e = EngineState::new(layer_config_unmapped(UnmappedBehavior::Passthrough));
        e.process_key(VKey::LAlt, true, 1000);

        match e.process_key(VKey::Tab, true, 1050) {
            KeyDecision::EngageChord {
                trigger: VKey::LAlt,
                key: VKey::Tab,
            } => {}
            other => panic!("EngageChord(LAlt, Tab) 기대, 실제: {other:?}"),
        }
        // 물리 Tab up은 통과 (down은 주입됐지만 up 상태는 일관됨)
        assert!(matches!(
            e.process_key(VKey::Tab, false, 1080),
            KeyDecision::PassThrough
        ));
        // 빠르게 뗐어도 코드 모드였으므로 tap(Escape)이 아니라 ReleaseChord
        match e.process_key(VKey::LAlt, false, 1150) {
            KeyDecision::ReleaseChord {
                trigger: VKey::LAlt,
                deferred_action: None,
            } => {}
            other => panic!("ReleaseChord(LAlt) 기대, 실제: {other:?}"),
        }
    }

    /// 코드 모드 중에는 후속 키(반복 Tab, 매핑 키 H 포함)가 전부 OS로 통과 (A안)
    #[test]
    fn chord_mode_passes_all_keys_until_release() {
        let mut e = EngineState::new(layer_config_unmapped(UnmappedBehavior::Passthrough));
        e.process_key(VKey::LAlt, true, 1000);
        e.process_key(VKey::Tab, true, 1050); // EngageChord

        // Alt 홀드 + Tab Tab (스위처 순회) → 통과
        assert!(matches!(
            e.process_key(VKey::Tab, true, 1200),
            KeyDecision::PassThrough
        ));
        // 매핑된 H도 코드 모드 중엔 레이어 액션이 아니라 OS 조합(Alt+H)
        assert!(matches!(
            e.process_key(VKey::H, true, 1300),
            KeyDecision::PassThrough
        ));
        assert!(matches!(
            e.process_key(VKey::H, false, 1330),
            KeyDecision::PassThrough
        ));
        assert!(matches!(
            e.process_key(VKey::LAlt, false, 1400),
            KeyDecision::ReleaseChord { .. }
        ));
    }

    /// 코드 진입 전의 매핑 키는 정상 실행 — 진입은 미매핑 키에서만
    #[test]
    fn passthrough_mapped_key_executes_before_chord() {
        let mut e = EngineState::new(layer_config_unmapped(UnmappedBehavior::Passthrough));
        e.process_key(VKey::LAlt, true, 1000);
        assert_execute_sendkey(e.process_key(VKey::H, true, 1050), VKey::Left);
        e.process_key(VKey::H, false, 1080);

        // 그 다음 미매핑 키 → 코드 진입
        assert!(matches!(
            e.process_key(VKey::Tab, true, 1100),
            KeyDecision::EngageChord { .. }
        ));
    }

    /// 코드 해제 후 레이어를 다시 활성화하면 매핑이 정상 동작 (상태 오염 없음)
    #[test]
    fn chord_state_clean_after_release() {
        let mut e = EngineState::new(layer_config_unmapped(UnmappedBehavior::Passthrough));
        e.process_key(VKey::LAlt, true, 1000);
        e.process_key(VKey::Tab, true, 1050);
        e.process_key(VKey::Tab, false, 1080);
        e.process_key(VKey::LAlt, false, 1100); // ReleaseChord

        // 재활성화 → 매핑 키 정상, 미매핑 키는 새 코드 진입
        e.process_key(VKey::LAlt, true, 2000);
        assert_execute_sendkey(e.process_key(VKey::H, true, 2050), VKey::Left);
        e.process_key(VKey::H, false, 2080);
        assert!(matches!(
            e.process_key(VKey::Tab, true, 2100),
            KeyDecision::EngageChord { .. }
        ));
    }

    /// plain(기본값) 레이어는 기존 동작 유지 — 코드 진입 없음
    #[test]
    fn plain_layer_unmapped_never_engages_chord() {
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, 1000);
        assert!(matches!(
            e.process_key(VKey::Tab, true, 1050),
            KeyDecision::PassThrough
        ));
    }

    /// block 레이어는 미매핑 키를 억제하고, tap 액션도 발동하지 않는다
    #[test]
    fn block_layer_suppresses_unmapped_and_tap() {
        let mut e = EngineState::new(layer_config_unmapped(UnmappedBehavior::Block));
        e.process_key(VKey::LAlt, true, 1000);
        assert!(matches!(
            e.process_key(VKey::Tab, true, 1050),
            KeyDecision::Suppress
        ));
        assert!(matches!(
            e.process_key(VKey::Tab, false, 1080),
            KeyDecision::Suppress
        ));
        // 억제된 키도 "사용됨" — 빠른 해제에서 tap(Escape) 미발동
        assert!(matches!(
            e.process_key(VKey::LAlt, false, 1150),
            KeyDecision::Suppress
        ));
    }

    /// 비수정자 트리거(CapsLock)의 passthrough는 plain으로 폴백
    #[test]
    fn passthrough_with_non_modifier_trigger_falls_back_to_plain() {
        let mut cfg = layer_config_unmapped(UnmappedBehavior::Passthrough);
        cfg.layers[0].trigger = VKey::CapsLock;
        let mut e = EngineState::new(cfg);

        e.process_key(VKey::CapsLock, true, 1000);
        assert!(matches!(
            e.process_key(VKey::Tab, true, 1050),
            KeyDecision::PassThrough
        ));
    }

    /// keymap 토글이 코드 모드를 끊으면 ReleaseChord로 주입 트리거를 해제
    #[test]
    fn toggle_during_chord_releases_injected_trigger() {
        let mut cfg = layer_config_unmapped(UnmappedBehavior::Passthrough);
        cfg.toggle_keymap = Some(ComboTrigger {
            modifiers: vec![],
            key: VKey::F12,
        });
        let mut e = EngineState::new(cfg);

        e.process_key(VKey::LAlt, true, 1000);
        e.process_key(VKey::Tab, true, 1050); // EngageChord
        e.process_key(VKey::Tab, false, 1080);

        match e.process_key(VKey::F12, true, 1200) {
            KeyDecision::ReleaseChord {
                trigger: VKey::LAlt,
                deferred_action: None,
            } => {}
            other => panic!("ReleaseChord(LAlt) 기대, 실제: {other:?}"),
        }
        // 토글 후 keymap 비활성 — 일반 키는 통과
        assert!(matches!(
            e.process_key(VKey::H, true, 1300),
            KeyDecision::PassThrough
        ));
    }

    /// 코드 진입 전에 지연된 레이어 Launch는 ReleaseChord로 전달된다
    #[test]
    fn pending_launch_delivered_via_release_chord() {
        let mut cfg = layer_config_unmapped(UnmappedBehavior::Passthrough);
        cfg.layers[0]
            .mappings
            .insert(VKey::Space, BindAction::Launch("kmd-desktop".into()));
        let mut e = EngineState::new(cfg);

        e.process_key(VKey::LAlt, true, 1000);
        e.process_key(VKey::Space, true, 1050); // Launch 지연
        e.process_key(VKey::Space, false, 1080);
        e.process_key(VKey::Tab, true, 1100); // EngageChord

        match e.process_key(VKey::LAlt, false, 1200) {
            KeyDecision::ReleaseChord {
                trigger: VKey::LAlt,
                deferred_action: Some(BindAction::Launch(cmd)),
            } => assert_eq!(cmd, "kmd-desktop"),
            other => panic!("ReleaseChord + Launch 기대, 실제: {other:?}"),
        }
    }

    // ── 레이어 Launch 지연 실행 ──

    #[test]
    fn layer_launch_deferred_until_trigger_release() {
        let mut cfg = layer_config();
        cfg.layers[0]
            .mappings
            .insert(VKey::Space, BindAction::Launch("kmd-desktop".into()));

        let mut e = EngineState::new(cfg);
        e.process_key(VKey::LAlt, true, 1000);

        // Launch 매핑 키 down → 즉시 실행하지 않고 억제 (지연)
        assert!(matches!(
            e.process_key(VKey::Space, true, 1100),
            KeyDecision::Suppress
        ));
        assert!(matches!(
            e.process_key(VKey::Space, false, 1150),
            KeyDecision::Suppress
        ));

        // 트리거 키를 떼는 순간 Launch 실행 (tap 아님 — 키 사용됨)
        assert!(matches!(
            e.process_key(VKey::LAlt, false, 1300),
            KeyDecision::Execute {
                action: BindAction::Launch(_),
                layer_trigger: None,
            }
        ));
    }

    #[test]
    fn layer_non_launch_action_carries_trigger_context() {
        // 레이어 매핑 실행 결정에는 트리거 키가 포함되어야 한다
        // (macOS가 잔여 modifier 플래그를 해제하는 데 사용)
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, 1000);
        match e.process_key(VKey::H, true, 1050) {
            KeyDecision::Execute {
                action: BindAction::SendKey(VKey::Left),
                layer_trigger: Some(VKey::LAlt),
            } => {}
            other => panic!("layer_trigger=Some(LAlt) 기대, 실제: {other:?}"),
        }
    }

    #[test]
    fn pending_launch_cleared_on_toggle() {
        let mut cfg = layer_config();
        cfg.layers[0]
            .mappings
            .insert(VKey::Space, BindAction::Launch("kmd-desktop".into()));
        cfg.toggle_keymap = Some(ComboTrigger {
            modifiers: vec![Modifier::Ctrl],
            key: VKey::K,
        });

        let mut e = EngineState::new(cfg);
        e.process_key(VKey::LAlt, true, 1000);
        e.process_key(VKey::Space, true, 1100); // pending launch 저장

        // 토글로 상태 리셋 (LAlt 홀드 중이지만 Ctrl+K)
        e.process_key(VKey::LCtrl, true, 1200);
        e.process_key(VKey::K, true, 1220);
        e.process_key(VKey::K, false, 1240);
        e.process_key(VKey::LCtrl, false, 1260);

        // 트리거 keyup — 리셋됐으므로 pending launch가 실행되면 안 됨
        // (keymap off 상태라 레이어 로직 자체가 비활성 → PassThrough)
        assert!(matches!(
            e.process_key(VKey::LAlt, false, 1300),
            KeyDecision::PassThrough
        ));
    }

    // ── macOS 어댑터 헬퍼 ──

    #[test]
    fn sync_modifier_flags_removes_stale_modifiers() {
        let mut e = EngineState::new(empty_config());
        e.process_key(VKey::LShift, true, 0);
        e.process_key(VKey::LCtrl, true, 10);
        assert!(e.is_modifier_held(VKey::LShift));

        // OS 플래그: shift 꺼짐 (keyup을 놓친 상황) → 동기화로 제거
        e.sync_modifier_flags(false, true, false, false);
        assert!(!e.is_modifier_held(VKey::LShift));
        assert!(e.is_modifier_held(VKey::LCtrl), "플래그 켜진 수정자는 유지");
    }

    #[test]
    fn reset_transient_state_clears_modifiers_and_layer() {
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LShift, true, 0);
        e.process_key(VKey::LAlt, true, 10); // 레이어 활성

        e.reset_transient_state();

        assert!(e.held_modifiers().is_empty());
        // 레이어가 리셋됐으므로 H는 일반 키
        assert!(matches!(
            e.process_key(VKey::H, true, 100),
            KeyDecision::PassThrough
        ));
    }

    #[test]
    fn 주입_실패_복구는_거짓_chord_상태를_제거한다() {
        let mut e = EngineState::new(layer_config_unmapped(UnmappedBehavior::Passthrough));
        e.process_key(VKey::LShift, true, 900);
        e.process_key(VKey::LAlt, true, 1000);
        assert!(matches!(
            e.process_key(VKey::Tab, true, 1050),
            KeyDecision::EngageChord { .. }
        ));
        assert_eq!(e.engaged_chord_trigger(), Some(VKey::LAlt));

        e.recover_from_injection_failure();

        assert_eq!(e.engaged_chord_trigger(), None);
        assert!(e.is_modifier_held(VKey::LShift));
        assert!(e.is_modifier_held(VKey::LAlt));
        assert!(matches!(
            e.process_key(VKey::Tab, false, 1080),
            KeyDecision::PassThrough
        ));
        assert!(matches!(
            e.process_key(VKey::LAlt, false, 1100),
            KeyDecision::PassThrough
        ));
        assert!(!e.is_modifier_held(VKey::LAlt));
        assert!(e.is_modifier_held(VKey::LShift));
    }

    #[test]
    fn 주입_실패_복구는_tap_hold_chord도_제거한다() {
        let mut e = EngineState::new(tap_hold_config());
        e.process_key(VKey::LShift, true, 900);
        e.process_key(VKey::CapsLock, true, 1000);
        assert!(matches!(
            e.process_key(VKey::C, true, 1050),
            KeyDecision::EngageChord { .. }
        ));
        assert_eq!(e.engaged_chord_trigger(), Some(VKey::LCtrl));

        e.recover_from_injection_failure();

        assert_eq!(e.engaged_chord_trigger(), None);
        assert!(e.is_modifier_held(VKey::LShift));
        assert!(matches!(
            e.process_key(VKey::CapsLock, false, 1100),
            KeyDecision::PassThrough
        ));
    }

    #[test]
    fn 주입_실패_복구_후에도_물리_modifier_콤보가_동작한다() {
        let mut e = EngineState::new(combo_config());
        e.process_key(VKey::LShift, true, 1000);
        assert_execute_sendkey(e.process_key(VKey::Space, true, 1050), VKey::Hangul);

        e.recover_from_injection_failure();

        assert!(e.is_modifier_held(VKey::LShift));
        assert!(matches!(
            e.process_key(VKey::Space, false, 1080),
            KeyDecision::Suppress
        ));
        assert_execute_sendkey(e.process_key(VKey::Space, true, 1100), VKey::Hangul);
    }

    // ── 콤보 ──

    fn combo_config() -> KeybindConfig {
        let mut cfg = empty_config();
        cfg.combos.push((
            ComboTrigger {
                modifiers: vec![Modifier::Shift],
                key: VKey::Space,
            },
            BindAction::SendKey(VKey::Hangul),
        ));
        cfg
    }

    #[test]
    fn combo_fires_with_modifier_held_and_suppresses_keyup() {
        let mut e = EngineState::new(combo_config());
        // Shift down → 수정자는 통과
        assert!(matches!(
            e.process_key(VKey::LShift, true, 0),
            KeyDecision::PassThrough
        ));
        // Shift+Space → Hangul
        assert_execute_sendkey(e.process_key(VKey::Space, true, 50), VKey::Hangul);
        // 콤보로 소비된 Space의 keyup도 억제
        assert!(matches!(
            e.process_key(VKey::Space, false, 100),
            KeyDecision::Suppress
        ));
        // Shift up은 통과
        assert!(matches!(
            e.process_key(VKey::LShift, false, 150),
            KeyDecision::PassThrough
        ));
    }

    #[test]
    fn combo_autorepeat_does_not_refire() {
        let mut e = EngineState::new(combo_config());
        e.process_key(VKey::LShift, true, 0);
        assert_execute_sendkey(e.process_key(VKey::Space, true, 50), VKey::Hangul);
        // Space 홀드 → 오토리피트 down은 재발화하지 않고 억제
        assert!(matches!(
            e.process_key(VKey::Space, true, 300),
            KeyDecision::Suppress
        ));
        assert!(matches!(
            e.process_key(VKey::Space, true, 350),
            KeyDecision::Suppress
        ));
        // keyup으로 소비 해제 → 진짜 재입력은 다시 발화
        assert!(matches!(
            e.process_key(VKey::Space, false, 400),
            KeyDecision::Suppress
        ));
        assert_execute_sendkey(e.process_key(VKey::Space, true, 500), VKey::Hangul);
    }

    #[test]
    fn combo_does_not_fire_without_modifier() {
        let mut e = EngineState::new(combo_config());
        assert!(matches!(
            e.process_key(VKey::Space, true, 0),
            KeyDecision::PassThrough
        ));
    }

    // ── 글로벌 더블탭 ──

    fn double_tap_config() -> KeybindConfig {
        let mut cfg = empty_config();
        cfg.double_taps.push(DoubleTapBinding {
            key: VKey::RShift,
            action: BindAction::SendKey(VKey::Hangul),
            timeout_ms: 300,
        });
        cfg
    }

    #[test]
    fn double_tap_fires_within_timeout() {
        let mut e = EngineState::new(double_tap_config());
        // 첫 탭: down 통과, up에서 탭 기록
        assert!(matches!(
            e.process_key(VKey::RShift, true, 1000),
            KeyDecision::PassThrough
        ));
        assert!(matches!(
            e.process_key(VKey::RShift, false, 1050),
            KeyDecision::PassThrough
        ));
        // timeout 이내 두 번째 down → 액션
        assert_execute_sendkey(e.process_key(VKey::RShift, true, 1200), VKey::Hangul);
        // 소비된 keyup 억제
        assert!(matches!(
            e.process_key(VKey::RShift, false, 1250),
            KeyDecision::Suppress
        ));
    }

    #[test]
    fn double_tap_expires_after_timeout() {
        let mut e = EngineState::new(double_tap_config());
        e.process_key(VKey::RShift, true, 1000);
        e.process_key(VKey::RShift, false, 1050);
        // timeout(300ms) 초과 → 발동 안 함
        assert!(matches!(
            e.process_key(VKey::RShift, true, 1400),
            KeyDecision::PassThrough
        ));
    }

    #[test]
    fn modifier_used_in_combo_does_not_register_tap() {
        // RShift+A(대문자 입력) 직후 RShift 재탭이 더블탭으로 오판정되면 안 됨
        let mut e = EngineState::new(double_tap_config());
        e.process_key(VKey::RShift, true, 1000);
        // RShift 홀드 중 A 입력 → RShift는 "수정자로 사용됨"
        e.process_key(VKey::A, true, 1020);
        e.process_key(VKey::A, false, 1050);
        e.process_key(VKey::RShift, false, 1080); // 사용됐으므로 탭 기록 안 됨

        // 곧바로 RShift 다시 → 더블탭 아님 (첫 탭부터 다시 시작)
        assert!(matches!(
            e.process_key(VKey::RShift, true, 1150),
            KeyDecision::PassThrough
        ));
    }

    #[test]
    fn other_key_between_taps_resets_double_tap() {
        let mut e = EngineState::new(double_tap_config());
        e.process_key(VKey::RShift, true, 1000);
        e.process_key(VKey::RShift, false, 1050); // 탭 기록
                                                  // 다른 (비수정자) 키 입력 → 탭 시퀀스 리셋
        e.process_key(VKey::A, true, 1100);
        e.process_key(VKey::A, false, 1130);
        assert!(matches!(
            e.process_key(VKey::RShift, true, 1200),
            KeyDecision::PassThrough
        ));
    }

    // ── keymap 토글 ──

    fn toggle_config() -> KeybindConfig {
        let mut cfg = remap_config();
        cfg.toggle_keymap = Some(ComboTrigger {
            modifiers: vec![Modifier::Ctrl, Modifier::Alt],
            key: VKey::K,
        });
        cfg.combos.push((
            ComboTrigger {
                modifiers: vec![Modifier::Alt],
                key: VKey::Space,
            },
            BindAction::Launch("kmd-desktop".into()),
        ));
        cfg
    }

    #[test]
    fn toggle_disables_remaps_but_keeps_launch_combo() {
        let mut e = EngineState::new(toggle_config());

        // 토글 전: 리맵 동작
        assert_execute_sendkey(e.process_key(VKey::CapsLock, true, 0), VKey::Escape);
        e.process_key(VKey::CapsLock, false, 30);

        // Ctrl+Alt+K → keymap off
        e.process_key(VKey::LCtrl, true, 100);
        e.process_key(VKey::LAlt, true, 120);
        assert!(matches!(
            e.process_key(VKey::K, true, 140),
            KeyDecision::Suppress
        ));
        // 소비된 K keyup 억제
        assert!(matches!(
            e.process_key(VKey::K, false, 160),
            KeyDecision::Suppress
        ));
        e.process_key(VKey::LCtrl, false, 180);
        e.process_key(VKey::LAlt, false, 200);

        // off 상태: 리맵은 통과
        assert!(matches!(
            e.process_key(VKey::CapsLock, true, 300),
            KeyDecision::PassThrough
        ));

        // off 상태에서도 Launch 콤보는 동작
        e.process_key(VKey::LAlt, true, 400);
        assert!(matches!(
            e.process_key(VKey::Space, true, 420),
            KeyDecision::Execute {
                action: BindAction::Launch(_),
                ..
            }
        ));
        e.process_key(VKey::Space, false, 440);
        e.process_key(VKey::LAlt, false, 460);

        // 다시 토글 → on
        e.process_key(VKey::LCtrl, true, 500);
        e.process_key(VKey::LAlt, true, 520);
        e.process_key(VKey::K, true, 540);
        e.process_key(VKey::K, false, 560);
        e.process_key(VKey::LCtrl, false, 580);
        e.process_key(VKey::LAlt, false, 600);

        assert_execute_sendkey(e.process_key(VKey::CapsLock, true, 700), VKey::Escape);
    }

    // ── tap-hold (모드탭) ──

    use crate::keybind::{MouseBind, TapHoldBinding};

    /// HHKB 스타일: CapsLock — tap=CapsLock, hold=LCtrl
    fn tap_hold_config() -> KeybindConfig {
        let mut cfg = empty_config();
        cfg.tap_holds.push(TapHoldBinding {
            key: VKey::CapsLock,
            tap: Some(VKey::CapsLock),
            hold: VKey::LCtrl,
            timeout_ms: 200,
        });
        cfg
    }

    #[test]
    fn tap_hold_quick_tap_sends_tap_key() {
        let mut e = EngineState::new(tap_hold_config());
        assert!(matches!(
            e.process_key(VKey::CapsLock, true, 1000),
            KeyDecision::Suppress
        ));
        // 199ms 후 up, 다른 키 미사용 → tap(CapsLock 토글)
        assert_execute_sendkey(e.process_key(VKey::CapsLock, false, 1199), VKey::CapsLock);
    }

    #[test]
    fn tap_hold_long_hold_alone_does_nothing() {
        let mut e = EngineState::new(tap_hold_config());
        e.process_key(VKey::CapsLock, true, 1000);
        // 타임아웃 초과 단독 해제 → 무동작 (HHKB 동일)
        assert!(matches!(
            e.process_key(VKey::CapsLock, false, 1300),
            KeyDecision::Suppress
        ));
    }

    #[test]
    fn tap_hold_other_key_engages_hold_chord_instantly() {
        let mut e = EngineState::new(tap_hold_config());
        e.process_key(VKey::CapsLock, true, 1000);

        // 타임아웃 이내라도 다른 키 down → 즉시 hold 확정 (Ctrl+C 등)
        match e.process_key(VKey::C, true, 1050) {
            KeyDecision::EngageChord {
                trigger: VKey::LCtrl,
                key: VKey::C,
            } => {}
            other => panic!("EngageChord(LCtrl, C) 기대, 실제: {other:?}"),
        }
        // chord 중 후속 키는 전부 통과 (OS에 Ctrl이 주입돼 있음)
        assert!(matches!(
            e.process_key(VKey::C, false, 1080),
            KeyDecision::PassThrough
        ));
        assert!(matches!(
            e.process_key(VKey::V, true, 1120),
            KeyDecision::PassThrough
        ));
        assert!(matches!(
            e.process_key(VKey::V, false, 1150),
            KeyDecision::PassThrough
        ));
        // 빠르게 뗐어도 chord였으므로 tap이 아니라 ReleaseChord(LCtrl)
        match e.process_key(VKey::CapsLock, false, 1180) {
            KeyDecision::ReleaseChord {
                trigger: VKey::LCtrl,
                deferred_action: None,
            } => {}
            other => panic!("ReleaseChord(LCtrl) 기대, 실제: {other:?}"),
        }
    }

    #[test]
    fn tap_hold_modifier_passes_through_then_key_engages() {
        // Caps+Shift+K → OS는 Ctrl+Shift+K (Shift는 물리 통과, K에서 chord)
        let mut e = EngineState::new(tap_hold_config());
        e.process_key(VKey::CapsLock, true, 1000);
        assert!(matches!(
            e.process_key(VKey::LShift, true, 1030),
            KeyDecision::PassThrough
        ));
        assert!(matches!(
            e.process_key(VKey::K, true, 1060),
            KeyDecision::EngageChord {
                trigger: VKey::LCtrl,
                key: VKey::K,
            }
        ));
    }

    #[test]
    fn tap_hold_autorepeat_suppressed_and_tap_window_preserved() {
        let mut e = EngineState::new(tap_hold_config());
        e.process_key(VKey::CapsLock, true, 1000);
        // OS 오토리피트 down — 억제, down_tick은 최초 값 유지
        assert!(matches!(
            e.process_key(VKey::CapsLock, true, 1100),
            KeyDecision::Suppress
        ));
        // 최초 down 기준 150ms → tap
        assert_execute_sendkey(e.process_key(VKey::CapsLock, false, 1150), VKey::CapsLock);
    }

    #[test]
    fn tap_hold_state_clean_after_release() {
        let mut e = EngineState::new(tap_hold_config());
        // chord 사이클
        e.process_key(VKey::CapsLock, true, 1000);
        e.process_key(VKey::C, true, 1050);
        e.process_key(VKey::C, false, 1080);
        e.process_key(VKey::CapsLock, false, 1100);
        // 이후 일반 키는 정상 통과
        assert!(matches!(
            e.process_key(VKey::C, true, 1200),
            KeyDecision::PassThrough
        ));
        // 새 tap 사이클도 정상
        e.process_key(VKey::CapsLock, true, 1300);
        assert_execute_sendkey(e.process_key(VKey::CapsLock, false, 1400), VKey::CapsLock);
    }

    #[test]
    fn tap_hold_none_tap_suppresses_quick_release() {
        let mut cfg = empty_config();
        cfg.tap_holds.push(TapHoldBinding {
            key: VKey::CapsLock,
            tap: None,
            hold: VKey::LCtrl,
            timeout_ms: 200,
        });
        let mut e = EngineState::new(cfg);
        e.process_key(VKey::CapsLock, true, 1000);
        assert!(matches!(
            e.process_key(VKey::CapsLock, false, 1100),
            KeyDecision::Suppress
        ));
    }

    #[test]
    fn tap_hold_wins_over_remap_on_same_key() {
        // 사용자 misconfig: 같은 키에 리맵+tap-hold — tap-hold 우선
        let mut cfg = tap_hold_config();
        cfg.remaps
            .insert(VKey::CapsLock, BindAction::SendKey(VKey::Escape));
        let mut e = EngineState::new(cfg);
        e.process_key(VKey::CapsLock, true, 1000);
        assert_execute_sendkey(e.process_key(VKey::CapsLock, false, 1100), VKey::CapsLock);
    }

    #[test]
    fn toggle_during_tap_hold_chord_releases_hold_modifier() {
        let mut cfg = tap_hold_config();
        cfg.toggle_keymap = Some(ComboTrigger {
            modifiers: vec![],
            key: VKey::F12,
        });
        let mut e = EngineState::new(cfg);
        e.process_key(VKey::CapsLock, true, 1000);
        e.process_key(VKey::C, true, 1050); // EngageChord(LCtrl, C)
        e.process_key(VKey::C, false, 1080);

        // 토글이 chord를 끊으면 주입된 LCtrl을 해제해야 한다
        // (chord 중 F12는 PassThrough — 토글 검사가 chord 통과보다 먼저다)
        match e.process_key(VKey::F12, true, 1200) {
            KeyDecision::ReleaseChord {
                trigger: VKey::LCtrl,
                deferred_action: None,
            } => {}
            other => panic!("ReleaseChord(LCtrl) 기대, 실제: {other:?}"),
        }
    }

    // ── 마우스 레이어 ──

    /// RAlt(별칭 Hangul) 홀드 → 마우스 레이어: W=위, Space=좌클릭, LShift=저속
    fn mouse_layer_config() -> KeybindConfig {
        let mut mappings = HashMap::new();
        mappings.insert(VKey::W, BindAction::Mouse(MouseBind::MoveUp));
        mappings.insert(VKey::D, BindAction::Mouse(MouseBind::MoveRight));
        mappings.insert(VKey::Space, BindAction::Mouse(MouseBind::BtnLeft));
        mappings.insert(VKey::LShift, BindAction::Mouse(MouseBind::Slow));

        let mut cfg = empty_config();
        cfg.layers.push(Layer {
            name: "mouse".into(),
            trigger: VKey::RAlt,
            trigger_aliases: vec![VKey::Hangul],
            tap_action: Some(VKey::Hangul),
            tap_hold_ms: 200,
            unmapped: UnmappedBehavior::Block,
            mappings,
            double_tap_mappings: HashMap::new(),
        });
        cfg
    }

    #[test]
    fn mouse_move_engages_and_releases_with_key() {
        let mut e = EngineState::new(mouse_layer_config());
        e.process_key(VKey::RAlt, true, 1000);

        assert!(matches!(
            e.process_key(VKey::W, true, 1050),
            KeyDecision::MouseEngage(MouseBind::MoveUp)
        ));
        // 오토리피트 down은 억제
        assert!(matches!(
            e.process_key(VKey::W, true, 1500),
            KeyDecision::Suppress
        ));
        assert!(matches!(
            e.process_key(VKey::W, false, 1600),
            KeyDecision::MouseRelease(MouseBind::MoveUp)
        ));
        // 키를 다 뗀 뒤 트리거 해제 — 이동 잔여 없음 → tap 아님(사용됨), 억제
        assert!(matches!(
            e.process_key(VKey::RAlt, false, 1700),
            KeyDecision::Suppress
        ));
    }

    #[test]
    fn 주입_실패_복구는_활성_마우스를_정지_대상으로_반환하고_상태를_비운다() {
        let mut e = EngineState::new(mouse_layer_config());
        e.process_key(VKey::RAlt, true, 1000);
        assert!(matches!(
            e.process_key(VKey::W, true, 1050),
            KeyDecision::MouseEngage(MouseBind::MoveUp)
        ));

        assert!(
            e.recover_from_injection_failure(),
            "활성 모션이 있으면 어댑터가 StopAll을 보내야 한다"
        );
        assert!(
            !e.recover_from_injection_failure(),
            "복구가 마우스 홀드 상태를 남기면 안 된다"
        );
        assert!(matches!(
            e.process_key(VKey::W, false, 1100),
            KeyDecision::PassThrough
        ));
        assert!(matches!(
            e.process_key(VKey::RAlt, false, 1150),
            KeyDecision::PassThrough
        ));
    }

    #[test]
    fn mouse_button_follows_key_for_drag() {
        let mut e = EngineState::new(mouse_layer_config());
        e.process_key(VKey::RAlt, true, 1000);
        // Space 홀드 = 버튼 다운 유지 (드래그), W로 이동
        assert!(matches!(
            e.process_key(VKey::Space, true, 1050),
            KeyDecision::MouseEngage(MouseBind::BtnLeft)
        ));
        assert!(matches!(
            e.process_key(VKey::W, true, 1100),
            KeyDecision::MouseEngage(MouseBind::MoveUp)
        ));
        assert!(matches!(
            e.process_key(VKey::W, false, 1300),
            KeyDecision::MouseRelease(MouseBind::MoveUp)
        ));
        assert!(matches!(
            e.process_key(VKey::Space, false, 1400),
            KeyDecision::MouseRelease(MouseBind::BtnLeft)
        ));
    }

    #[test]
    fn trigger_release_with_held_mouse_keys_stops_all() {
        let mut e = EngineState::new(mouse_layer_config());
        e.process_key(VKey::RAlt, true, 1000);
        e.process_key(VKey::W, true, 1050);
        e.process_key(VKey::Space, true, 1080);

        // 이동/버튼 키가 눌린 채 트리거 해제 → 전체 정지
        assert!(matches!(
            e.process_key(VKey::RAlt, false, 1200),
            KeyDecision::MouseStopAll
        ));
        // 이후 잔여 keyup은 레이어 밖 — 통과 (stuck 없음, 이미 정지됨)
        assert!(matches!(
            e.process_key(VKey::W, false, 1250),
            KeyDecision::PassThrough
        ));
    }

    #[test]
    fn slow_modifier_key_is_suppressed_and_stateful() {
        let mut e = EngineState::new(mouse_layer_config());
        e.process_key(VKey::RAlt, true, 1000);
        // LShift는 레이어 매핑(mouse:slow) — 물리 Shift는 OS로 안 나간다
        assert!(matches!(
            e.process_key(VKey::LShift, true, 1050),
            KeyDecision::MouseEngage(MouseBind::Slow)
        ));
        assert!(matches!(
            e.process_key(VKey::LShift, false, 1200),
            KeyDecision::MouseRelease(MouseBind::Slow)
        ));
    }

    #[test]
    fn hangul_alias_activates_mouse_layer_and_taps() {
        let mut e = EngineState::new(mouse_layer_config());
        // Windows 한국어 배열: 물리 RAlt = VK_HANGUL
        assert!(matches!(
            e.process_key(VKey::Hangul, true, 1000),
            KeyDecision::Suppress
        ));
        assert!(matches!(
            e.process_key(VKey::D, true, 1050),
            KeyDecision::MouseEngage(MouseBind::MoveRight)
        ));
        e.process_key(VKey::D, false, 1080);
        e.process_key(VKey::Hangul, false, 1100); // MouseStopAll 아님 — D 이미 뗌
                                                  // 짧게 탭만 하면 한/영 전환 유지
        e.process_key(VKey::Hangul, true, 2000);
        assert_execute_sendkey(e.process_key(VKey::Hangul, false, 2100), VKey::Hangul);
    }

    #[test]
    fn mouse_key_held_before_layer_activation_releases_clean() {
        let mut e = EngineState::new(mouse_layer_config());
        // W를 일반 타이핑으로 누른 상태에서 레이어 활성화
        assert!(matches!(
            e.process_key(VKey::W, true, 1000),
            KeyDecision::PassThrough
        ));
        e.process_key(VKey::RAlt, true, 1050);
        // W up — 레이어 매핑이지만 우리가 잡은 down이 아님 → 통과 (stuck 방지)
        assert!(matches!(
            e.process_key(VKey::W, false, 1100),
            KeyDecision::PassThrough
        ));
    }

    #[test]
    fn toggle_with_held_mouse_keys_stops_all() {
        let mut cfg = mouse_layer_config();
        cfg.toggle_keymap = Some(ComboTrigger {
            modifiers: vec![],
            key: VKey::F12,
        });
        let mut e = EngineState::new(cfg);
        e.process_key(VKey::RAlt, true, 1000);
        e.process_key(VKey::W, true, 1050);

        assert!(matches!(
            e.process_key(VKey::F12, true, 1100),
            KeyDecision::MouseStopAll
        ));
    }

    // ── tick wrapping ──

    #[test]
    fn tick_wrapping_does_not_break_timing() {
        // u32 tick이 랩어라운드해도 wrapping_sub로 정상 판정
        let mut e = EngineState::new(layer_config());
        e.process_key(VKey::LAlt, true, u32::MAX - 50);
        // 랩어라운드 후 149ms 경과 (< 200ms) → tap
        assert_execute_sendkey(e.process_key(VKey::LAlt, false, 99), VKey::Escape);
    }
}
