//! 배치 해석기 — 사용자 키맵에서 "수 → 물리 키"를 읽는다 (docs/10 §6-5).
//!
//! 출처는 데몬·치트시트와 같은 [`kmd_core::keymap::effective_keymap`] 하나다.
//! 여기서 키맵 기본값을 다시 정의하지 않는다.

use std::collections::BTreeMap;

use kmd_core::config::KeymapConfig;

use super::action::{Action, Os};

/// 수 하나를 누르는 법
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// 쥐고 있어야 하는 키 — 레이어 트리거(`CapsLock`) 또는 콤보 수정자(`Shift`)
    pub hold: Option<String>,
    /// 누르는 키
    pub key: String,
    /// 1 = 한 번, 2 = 더블탭 (II)
    pub taps: u8,
    /// 트리거 키 자체를 짧게 친다 (RAlt 톡 = 한/영)
    pub tap_trigger: bool,
    /// 레이어 바인딩이면 그 레이어 이름
    pub layer: Option<String>,
}

impl Binding {
    /// 화면 표기 — `CapsLock+I`, `CapsLock+I I`, `RAlt 톡`, `Shift+Space`
    pub fn chord(&self) -> String {
        if self.tap_trigger {
            return format!("{} 톡", self.key);
        }
        let key = if self.taps >= 2 {
            format!("{0} {0}", self.key)
        } else {
            self.key.clone()
        };
        match &self.hold {
            Some(h) => format!("{h}+{key}"),
            None => key,
        }
    }

    /// 우선순위 — 레이어 > 트리거 탭 > 콤보 > 단순 리맵. 작을수록 먼저.
    fn rank(&self) -> u8 {
        match (&self.layer, self.tap_trigger, &self.hold) {
            (Some(_), false, _) => 0,
            (_, true, _) => 1,
            (None, false, Some(_)) => 2,
            _ => 3,
        }
    }
}

/// 배치를 얼마나 믿을 수 있는가 (docs/10 §6-5 백엔드별 신뢰도)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// 내장 데몬, 또는 kanata + 내장 프리셋 원본
    Full,
    /// kanata + 직접 고친 `.kbd` — 키 대신 수 이름으로 안내한다
    Unknown,
}

#[derive(Debug, Clone)]
pub struct LayerInfo {
    pub name: String,
    pub trigger: String,
}

/// 해석 결과
#[derive(Debug, Clone)]
pub struct Bindings {
    by_action: BTreeMap<Action, Vec<Binding>>,
    /// 표준 배치(vim-nav 기본값)에서 각 수의 키 — 바인딩이 여럿일 때 표준 키를 먼저 고른다
    standard_keys: BTreeMap<Action, String>,
    layers: Vec<LayerInfo>,
    pub trust: Trust,
    /// 수 → 바인딩 표의 지문. config의 무관한 항목이 바뀌어도 그대로다.
    pub fingerprint: String,
}

impl Bindings {
    /// 사용자 effective 키맵을 해석한다. `standard`는 표준 배치의 effective 키맵.
    pub fn resolve(
        effective: &KeymapConfig,
        standard: &KeymapConfig,
        os: Os,
        trust: Trust,
    ) -> Self {
        let standard_keys = collect(standard, os)
            .into_iter()
            .filter_map(|(a, mut v)| {
                v.sort_by_key(Binding::rank);
                v.first().map(|b| (a, b.key.clone()))
            })
            .collect();
        let mut by_action = collect(effective, os);
        for list in by_action.values_mut() {
            list.sort_by_key(Binding::rank);
        }
        let layers = effective
            .layers
            .iter()
            .map(|(name, l)| LayerInfo {
                name: name.clone(),
                trigger: l.trigger.clone(),
            })
            .collect();
        let mut b = Self {
            by_action,
            standard_keys,
            layers,
            trust,
            fingerprint: String::new(),
        };
        b.fingerprint = b.compute_fingerprint();
        b
    }

    /// 현재 config로 해석한다 (표준 = 기본 config의 vim-nav)
    pub fn from_config(config: &kmd_core::Config, trust: Trust) -> Self {
        let effective = kmd_core::keymap::effective_keymap(&config.launcher.keymap);
        let standard = kmd_core::keymap::effective_keymap(&KeymapConfig::default());
        Self::resolve(&effective, &standard, Os::current(), trust)
    }

    /// 대표 바인딩 — 우선순위가 같으면 표준 키를 고른다
    pub fn primary(&self, a: Action) -> Option<&Binding> {
        let list = self.by_action.get(&a)?;
        let best_rank = list.first()?.rank();
        let std_key = self.standard_keys.get(&a);
        list.iter()
            .filter(|b| b.rank() == best_rank)
            .find(|b| Some(&b.key) == std_key)
            .or_else(|| list.first())
    }

    /// 이 배치에 그 수가 있는가. 배치 미상이면 있다고 본다(판정은 보정이 맡는다).
    pub fn has(&self, a: Action) -> bool {
        self.trust == Trust::Unknown || self.by_action.contains_key(&a)
    }

    /// 수 하나의 화면 표기
    pub fn label(&self, a: Action) -> String {
        if self.trust == Trust::Unknown {
            return format!("「{}」 수", a.label());
        }
        match self.primary(a) {
            Some(b) => b.chord(),
            None => "없음".into(),
        }
    }

    /// 쥐는 키를 뺀 키만 — `{key:Left}` → `H`
    pub fn key(&self, a: Action) -> String {
        if self.trust == Trust::Unknown {
            return a.label().to_string();
        }
        self.primary(a)
            .map(|b| {
                if b.taps >= 2 {
                    format!("{0} {0}", b.key)
                } else {
                    b.key.clone()
                }
            })
            .unwrap_or_else(|| "없음".into())
    }

    /// 레이어 트리거 이름
    pub fn trigger(&self, layer: &str) -> Option<&str> {
        self.layers
            .iter()
            .find(|l| l.name == layer)
            .map(|l| l.trigger.as_str())
            .filter(|t| !t.is_empty())
    }

    /// 콘텐츠 문구의 자리표시자를 채운다.
    ///
    /// - `{WordLeft}` → `CapsLock+I` (수 전체)
    /// - `{key:WordLeft}` → `I` (쥐는 키 제외)
    /// - `{nav}` / `{mouse}` → 레이어 트리거
    ///
    /// 모르는 자리표시자는 그대로 둔다 — 콘텐츠 테스트가 잡는다.
    pub fn render(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let Some(end) = after.find('}') else {
                out.push_str(&rest[start..]);
                return out;
            };
            let token = &after[..end];
            match self.render_token(token) {
                Some(s) => out.push_str(&s),
                None => {
                    out.push('{');
                    out.push_str(token);
                    out.push('}');
                }
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        out
    }

    fn render_token(&self, token: &str) -> Option<String> {
        if let Some(id) = token.strip_prefix("key:") {
            return Action::from_id(id).map(|a| self.key(a));
        }
        if token == "nav" || token == "mouse" {
            return Some(match (self.trust, self.trigger(token)) {
                (Trust::Full, Some(t)) => t.to_string(),
                _ if token == "nav" => "nav 레이어 키".into(),
                _ => "mouse 레이어 키".into(),
            });
        }
        Action::from_id(token).map(|a| self.label(a))
    }

    /// 자리표시자가 모두 알려진 것인지 (콘텐츠 검증용)
    #[cfg(test)]
    pub fn unknown_tokens(text: &str) -> Vec<String> {
        let mut bad = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('}') else { break };
            let token = &after[..end];
            let known = token == "nav"
                || token == "mouse"
                || Action::from_id(token.strip_prefix("key:").unwrap_or(token)).is_some();
            if !known {
                bad.push(token.to_string());
            }
            rest = &after[end + 1..];
        }
        bad
    }

    fn compute_fingerprint(&self) -> String {
        // FNV-1a 64 — std 해시는 릴리스 간 안정성이 보장되지 않는다
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |s: &str| {
            for b in s.bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        for (a, list) in &self.by_action {
            feed(a.id());
            feed("=");
            for b in list {
                feed(&b.chord());
                feed(";");
            }
            feed("\n");
        }
        format!("{h:016x}")
    }
}

/// effective 키맵에서 수별 바인딩을 모은다
fn collect(km: &KeymapConfig, os: Os) -> BTreeMap<Action, Vec<Binding>> {
    let mut map: BTreeMap<Action, Vec<Binding>> = BTreeMap::new();
    let mut push = |a: Action, b: Binding| {
        let list = map.entry(a).or_default();
        if !list.contains(&b) {
            list.push(b);
        }
    };
    for (name, layer) in &km.layers {
        let hold = (!layer.trigger.is_empty()).then(|| layer.trigger.clone());
        for (key, target) in &layer.mappings {
            if let Some(a) = Action::from_target(target, os) {
                push(
                    a,
                    Binding {
                        hold: hold.clone(),
                        key: key.clone(),
                        taps: 1,
                        tap_trigger: false,
                        layer: Some(name.clone()),
                    },
                );
            }
        }
        for (key, dt) in &layer.double_taps {
            for (target, taps) in [(&dt.single, 1u8), (&dt.double, 2u8)] {
                if let Some(a) = Action::from_target(target, os) {
                    push(
                        a,
                        Binding {
                            hold: hold.clone(),
                            key: key.clone(),
                            taps,
                            tap_trigger: false,
                            layer: Some(name.clone()),
                        },
                    );
                }
            }
        }
        if let Some(a) = layer
            .tap_action
            .as_deref()
            .and_then(|t| Action::from_target(t, os))
        {
            push(
                a,
                Binding {
                    hold: None,
                    key: layer.trigger.clone(),
                    taps: 1,
                    tap_trigger: true,
                    layer: Some(name.clone()),
                },
            );
        }
    }
    for combo in &km.combos {
        if let (Some(a), Some((hold, key))) = (
            Action::from_target(&combo.action, os),
            combo.trigger.rsplit_once('+'),
        ) {
            push(
                a,
                Binding {
                    hold: Some(hold.trim().to_string()),
                    key: key.trim().to_string(),
                    taps: 1,
                    tap_trigger: false,
                    layer: None,
                },
            );
        }
    }
    for dt in &km.double_taps {
        if let Some(a) = Action::from_target(&dt.action, os) {
            push(
                a,
                Binding {
                    hold: None,
                    key: dt.key.clone(),
                    taps: 2,
                    tap_trigger: false,
                    layer: None,
                },
            );
        }
    }
    for (key, target) in &km.remaps {
        if let Some(a) = Action::from_target(target, os) {
            push(
                a,
                Binding {
                    hold: None,
                    key: key.clone(),
                    taps: 1,
                    tap_trigger: false,
                    layer: None,
                },
            );
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use kmd_core::config::LayerToml;
    use kmd_core::keymap::effective_keymap;

    fn standard() -> KeymapConfig {
        effective_keymap(&KeymapConfig::default())
    }

    fn bindings_for(user: KeymapConfig, os: Os) -> Bindings {
        Bindings::resolve(&effective_keymap(&user), &standard(), os, Trust::Full)
    }

    fn user_nav(trigger: &str, mappings: &[(&str, &str)]) -> KeymapConfig {
        let mut km = KeymapConfig::default();
        let layer = LayerToml {
            trigger: trigger.into(),
            mappings: mappings
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        };
        km.layers.insert("nav".into(), layer);
        km
    }

    #[test]
    fn 기본_배치는_capslock_hjkl이다() {
        let b = bindings_for(KeymapConfig::default(), Os::current());
        assert_eq!(b.label(Action::Left), "CapsLock+H");
        assert_eq!(b.key(Action::Down), "J");
        assert_eq!(b.trigger("nav"), Some("CapsLock"));
        assert_eq!(b.trigger("mouse"), Some("RAlt"));
        assert_eq!(b.label(Action::ToggleHangul), "RAlt 톡");
        assert_eq!(b.label(Action::LineStart), "CapsLock+I I");
    }

    #[test]
    fn 트리거를_바꾸면_안내가_따라_바뀐다() {
        let b = bindings_for(user_nav("LAlt", &[]), Os::current());
        assert_eq!(b.label(Action::Left), "LAlt+H");
        assert_eq!(b.render("{nav} 을 쥐고 {key:Left}"), "LAlt 을 쥐고 H");
    }

    #[test]
    fn 키를_옮기면_새_키가_보이고_표준_키도_남는다() {
        // 사용자가 Y 에도 Left 를 둔다 — 표준 키 H 가 대표로 남는다
        let b = bindings_for(user_nav("", &[("Y", "Left")]), Os::current());
        assert_eq!(b.key(Action::Left), "H");
        // 표준 키를 다른 수로 덮으면 새 키가 대표가 된다
        let b = bindings_for(
            user_nav("", &[("H", "Ctrl+V"), ("Y", "Left")]),
            Os::current(),
        );
        assert_eq!(b.key(Action::Left), "Y");
    }

    #[test]
    fn 수가_없으면_없음이고_has가_거짓이다() {
        let km = KeymapConfig {
            active_profile: "none".into(),
            ..Default::default()
        };
        let b = bindings_for(km, Os::current());
        assert!(!b.has(Action::PageUp));
        assert_eq!(b.label(Action::PageUp), "없음");
    }

    #[test]
    fn 배치_미상이면_수_이름으로_안내한다() {
        let b = Bindings::resolve(&standard(), &standard(), Os::current(), Trust::Unknown);
        assert_eq!(b.label(Action::WordLeft), "「단어 ←」 수");
        assert!(b.has(Action::PageUp));
        assert_eq!(b.render("{nav}"), "nav 레이어 키");
    }

    #[test]
    fn 지문은_배치가_바뀔_때만_바뀐다() {
        let a = bindings_for(KeymapConfig::default(), Os::current());
        // 배치와 무관한 항목만 바꾼다
        let same = KeymapConfig {
            kanata_path: Some("/opt/kanata".into()),
            ..Default::default()
        };
        let b = bindings_for(same, Os::current());
        let c = bindings_for(user_nav("LAlt", &[]), Os::current());
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_ne!(a.fingerprint, c.fingerprint);
    }

    #[test]
    fn 모르는_자리표시자는_그대로_두고_검출된다() {
        let b = bindings_for(KeymapConfig::default(), Os::current());
        assert_eq!(b.render("{Nope} {Left"), "{Nope} {Left");
        assert_eq!(
            Bindings::unknown_tokens("{Left} {key:Up} {nav} {Nope}"),
            vec!["Nope"]
        );
    }
}
