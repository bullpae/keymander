# 키맵 인체공학 — 전체 지도와 배치 근거

> 상태: **참조 문서**. 현재 배치(v0.15.1 + Unreleased)의 전체 지도와, 각 키가
> 왜 그 자리인지의 근거를 담는다. 배치를 바꾸기 전에 읽으면 이미 기각한 안을
> 다시 제안하지 않는다.
>
> 트리거 선택(왜 LAlt가 아니라 CapsLock인가)의 배경은 [13](13_capslock_trigger.md),
> 레이어 엔진 동작은 [08](08_layer_passthrough_plan.md) 참고.
>
> **"Alt로 되돌리는 게 낫지 않나"를 다시 꺼내기 전에 §5를 먼저 읽을 것** —
> 문헌까지 대조해 재검토했고, 무엇이 살고 무엇이 못 사는지 코드 수준으로
> 정리해 뒀다. 트리거 결정 자체는 **실사용 검증 대기로 보류**(2026-08-27).

## 1. 전체 키맵 지도

### 1-1. 전역 — 레이어와 무관하게 항상 동작

| 조합 | 동작 | 출처 |
|---|---|---|
| `Alt+Space` | kmd-desktop 실행 | `keybindings.global_hotkey` |
| `Shift+Space` | 한/영 전환 | 기본 combo (`effective_keymap`) |
| `Ctrl+Alt+K` | 키맵 전체 on/off | `keybindings.toggle_keymap` |
| `RAlt` 짧게 탭 | 한/영 전환 (주 경로) | mouse 레이어 `tap_action` |
| `CapsLock` 짧게 탭 | **무동작** | nav `tap_action = None` |

Windows HHKB 모드탭(탭=Caps / 홀드=Ctrl)은 트리거가 CapsLock이면 **주입되지
않는다** — `user_defined_capslock()`이 레이어 트리거까지 보고 막는다. 같은 키를
모드탭과 레이어 트리거가 동시에 쓸 수 없기 때문.

### 1-2. nav 레이어 — `CapsLock` 홀드 (`unmapped = plain`)

```
 Q    W    E    R    T  │  Y    U      I         O        P     [  ]  \
 ·    ·    ·    ·    ·  │  ·    ·    단어←     단어→      ·     ·  ·  ·
                        │           (2탭 Home) (2탭 End)

 A    S    D    F    G  │  H    J    K    L    ;    '
 ·    ·    ·    ·    ·  │  ←    ↓    ↑    →    ·    ·

 Z    X    C    V    B  │  N     M      ,      .       /
 ·    ·    ·    ·    ·  │ PgUp  PgDn    ·    BkSp     Del

 LShift  ·(통과) → Shift+네비 = 선택 확장
 Space   kmd-desktop 실행 (트리거를 뗄 때 실행)
 ;       kmd-desktop 을 :m 메모 입력으로 실행 (docs/23)
```

`·` = 매핑 없음 → **맨키 통과**(`unmapped = plain`). 왼손 전체가 비어 있다.

의도적으로 비워 둔 자리:

- `,` — 최빈 삭제 키 `.` 바로 옆의 **완충 빈칸**
- `P` — `ㅖ`(Shift+P) 오타 안전지대 (§3 참고)
- `U` — 검지 과부하 방지 (§4 참고)
- `Y`/`[`/`]`/`\`/`'` — 미할당

제거된 매핑 (v0.14.0, 2026-08-12): `P`(붙여넣기) `Y`(줄 복사) `U`(줄 삭제)
`,`(단어 삭제). **레이어 불변식** — 레이어에는 오발사해도 무해한 액션
(이동 · 글자 삭제 · 런처)만 둔다.

### 1-3. mouse 레이어 — `RAlt` 홀드 (`unmapped = block`)

```
 Q     W       E      R       T   │  ...
 ✕   좌클릭    ↑    우클릭    휠↑  │  ✕

 A     S       D      F       G   │  H    J      K      L
 ✕     ←       ↓      →      휠↓  │  ✕  좌클릭  우클릭  중클릭

 LShift = 저속 정밀        Space(왼엄지) = 좌클릭 (홀드 = 드래그)
```

`✕` = **차단**. 마우스 조작 중 미매핑 키 오타가 글자로 새어나가지 않는다.

- **ESDF** = 포인터 — WASD는 검지를 `D`로 한 칸 왼쪽에 묶어 타이핑 홈포지션
  (검지 `F`)을 깨뜨린다. ESDF는 손을 홈에 둔 채 조작되고, 덤으로
  `Q/W/R/T/A/G/Z/X/C/V/B`가 확장 자리로 열린다.
- **W/R** = 좌/우클릭 — 이동 클러스터(`E`) 왼쪽·오른쪽 = 마우스 버튼 좌/우.
- **T/G** = 휠 ↑/↓ — 위 키는 위로, 아래 키는 아래로 (검지 세로열).
- **Space**(왼엄지) = 좌클릭 — `W`는 `S`(←이동)와 같은 약지라 클릭을 홀드하면
  왼쪽 이동이 불가능하다. **드래그는 엄지가 담당**한다.
- **J/K/L** = 오른손 병행용 별칭.

> ⚠️ WASD와 ESDF는 **병합할 수 없다** — `S`와 `D`의 의미가 서로 뒤집힌다.
> WASD로 되돌리려면 `E`/`F`까지 함께 덮어써야 한다.

### 1-4. 두벌식에서 Shift가 필요한 키

배치 안전성을 판단하려면 이 지도가 같이 있어야 한다.

```
LShift 필요 (오른손 문자):  6 7 8 9 0 - =  →  ^ & * ( ) _ +
                            O P            →  ㅒ ㅖ
                            [ ] \          →  { } |
                            ; '            →  : "
                            , . /          →  < > ?

RShift 필요 (왼손 문자):    1 2 3 4 5      →  ! @ # $ %
                            Q W E R T      →  ㅃ ㅉ ㄸ ㄲ ㅆ
```

## 2. 손가락 배치의 기준

배치 판단에 쓰는 해부학적 사실 두 가지.

**굽히는 힘 > 펴는 힘.** 굴근(FDP/FDS)이 신근(EDC)보다 강하다. 아래 행은
**말아 당기는(굴곡)** 동작, 위 행은 **펴서 뻗는(신전)** 동작이다. 같은 거리라도
아래 행이 덜 부담스럽다.

**위 행은 긴 손가락이 유리하다.** 길이는 대개 중지 > 약지 ≳ 검지 > 새끼.
위 행은 손바닥에서 멀어지는 방향이라 **위 행 적합도 = 중지 ≥ 약지 > 검지**다.
아래 행은 짧아도 불리하지 않다.

여기서 흔한 오판이 나온다 — **"검지가 최강 손가락"은 *누르는 힘*의 이야기지
*뻗는 거리*의 이야기가 아니다.** 위 행의 병목은 힘이 아니라 도달성이다.

세 번째 기준은 배치에서만 나온다:

**같은 손가락의 행 왕복(same-finger bigram)을 늘리지 않는다.** 레이아웃
분석에서 피로·속도 저하의 최대 원인으로 꼽히는 패턴이다.

> **문헌 대조** — 이 세 기준은 피어리뷰된 배치 최적화 연구의 비용 모델과
> 일치한다. Engram 연구(IJHCI, 2026)는 30개 키 위치마다 **어느 손가락이
> 닿는지에 따른 노력 비용**을 매기는데, 그 근거가 (1) 손가락 힘 차이
> (새끼 < 검지), (2) **뻗기보다 말기가 쉽다**는 방향성, (3) 행 스태거
> 기하다. 크라우드소싱한 타이핑 선호 데이터에서 **중지는 홈 행과 그 위
> 행에 활동이 몰린다**는 관찰도 §4의 판단과 같은 방향이다.
> ([Engram Study](https://www.tandfonline.com/doi/full/10.1080/10447318.2026.2665409),
> [프로젝트](https://github.com/binarybottle/engram))

### 표준 운지 (오른손, ANSI)

| 손가락 | 담당 키 |
|---|---|
| 검지 | `Y` `U` `H` `J` `N` `M` (6키 — 원래 최과부하) |
| 중지 | `I` `K` `,` |
| 약지 | `O` `L` `.` |
| 새끼 | `P` `;` `/` 및 오른쪽 끝 전부 |

스태거(ANSI 기준): 위 행은 홈 행보다 **0.25u 왼쪽**(`U`=7.5 vs `J`=7.75),
아래 행은 **0.5u 오른쪽**(`N`=7.25는 `H`7.75와 `J`7.75 사이).

## 3. CapsLock ↔ LShift 인접 — 구조적 비용

트리거를 CapsLock으로 옮기며 **새로 생긴 상시 비용**이다. 트리거 선택 자체는
여전히 옳지만(→ [13](13_capslock_trigger.md)), 비용을 기록해 둔다.

### 3-1. 같은 손가락이다

| 키 | 손가락 | 홈(A)에서 |
|---|---|---|
| LAlt | 왼**엄지** | 손바닥 아래로 말아넣기 |
| CapsLock | 왼**새끼** | 왼쪽 1칸 (같은 행) |
| LShift | 왼**새끼** | 왼쪽아래 대각 (한 행 아래) |

`CapsLock`(1.75u)은 `LShift`(2.25u) **바로 위에 가로로 100% 겹쳐** 있다.
중심 간 ≈ 19.6mm — `A`↔`Z` 수준이고, 사이에 촉각 경계가 없다.

Alt 시절에는 엄지 vs 새끼라 이 충돌 자체가 성립하지 않았다.

> **문헌 대조 — "인접"만으로는 부족하고 "인접 + 같은 손가락"이 조건이다.**
>
> 타이핑 오류 분류 연구의 일관된 결과: 대체(substitution) 오류 대부분은
> **키보드상 인접하면서 동시에 같은 손가락이 담당하는** 두 글자 사이에서
> 일어난다. 반대로 다른 손·다른 손가락이 치는 글자쌍은 순서 오류가 덜한데,
> **독립적인 운동 경로가 타이밍 간섭을 줄이기** 때문이다.
> ([ExpECT — Kano & MacKenzie](https://www.yorku.ca/mack/bhci2007.pdf),
> [오류 유형 개괄](https://likelytypo.com/articles/types-of-typing-errors.html))
>
> `CapsLock`↔`LShift`는 이 조건을 정확히 만족한다. `LAlt`↔`LShift`는 인접성도
> 손가락도 둘 다 어긋난다 — **트리거를 바꾸며 없던 오류 조건을 만든 것**이다.
>
> 커뮤니티에도 알려진 결함이다. Colemak 인체공학 모드 문서는 CapsLock을
> Ctrl로 쓸 때 `Ctrl+Shift+Tab` 같은 조합이 *"real hassle"*이 된다고 적고,
> **권장 해법으로 "모디파이어를 엄지로 분산"**을 든다.
> ([Colemak Mods: Modifiers](https://colemakmods.github.io/ergonomic-mods/modifiers.html))
> — 이 권고가 §5의 판단을 가른다.

### 3-2. 실패가 두 종류다

**(A) 순차 오타 — 연습으로 줄어든다.**
Shift를 노리다 CapsLock을 스치는 것. 같은 손가락 인접 판별이라 학습이 느리지만
빈도는 떨어진다. **다만 0이 되지는 않는다.**

**(B) 동시 입력 충돌 — 연습으로 사라지지 않는다.**
엔진은 `Shift+네비`를 의도적으로 지원한다 (`engine.rs`: *"Shift는 예외 —
Shift+네비 키 = 선택 확장이 더 유용하다"*). 그런데 `CapsLock+Shift+H`는
**한 새끼손가락이 다른 행의 두 키를 동시에** 눌러야 한다. 물리적으로 안 된다.

게다가 네비 키가 전부 오른손이라 표준 운지상 **정답 Shift가 하필 LShift**다.

→ **회피책: RShift.** 반대손 규칙에는 어긋나지만 이 배치에서는 이게 정공법이다.
RShift 더블탭(한/영)이 2026-08-12에 제거돼서 지금 RShift는 순수 Shift다.

> RShift는 **레이어 안에서의 선택 확장에만** 유효하다. 일반 타이핑의
> `ㅖ`(Shift+P) `?`(Shift+/) `"`(Shift+') 는 전부 오른손 문자라 LShift가
> 정답이고 손가락을 바꿔 피할 수 없다. **즉 이 인접 비용은 nav 레이어를
> 쓰지 않는 순간에도 발생하는 상시 세금이다.**

### 3-3. 오발사 지도

레이어는 `tap_hold_ms`와 무관하게 **CapsLock 키다운 즉시** 켜진다
(`engine.rs` §3 — 지연 0을 위한 설계). `tap_hold_ms`는 릴리스 시점의
탭/홀드 판정에만 쓰인다. 따라서 스친 순간부터 바로 네비 모드다.

| 치려던 것 | 정상 | CapsLock 슬립 시 | 등급 |
|---|---|---|---|
| `?` | Shift+`/` | **Delete** | 🔴 삭제 |
| `>` | Shift+`.` | **Backspace** | 🔴 삭제 |
| 한/영 | Shift+Space | **런처 실행** | 🟠 |
| 영문 `H` `J` `K` `L` | Shift+키 | **← ↓ ↑ →** | 🟠 커서 이동 |
| 영문 `N` `M` | Shift+키 | **PgUp/PgDn** | 🟠 |
| 영문 `I` `O` | Shift+키 | **단어 이동**, 2탭 **Home/End** | 🟠 |
| `ㅒ` `ㅖ` | Shift+`O`/`P` | `ㅐ` `ㅔ` | 🟢 평범한 오타 |
| `:` `"` `{` `}` `\|` | Shift+키 | 평범한 오타 | 🟢 |
| `<` | Shift+`,` | 평범한 오타 | 🟢 완충 빈칸 |

**한글에서 특히 비싸다** — 하이재킹되는 `H J K L N M`이 두벌식으로
`ㅗ ㅓ ㅏ ㅣ ㅜ ㅡ`, 최빈 모음이다. 스친 직후 1~2타 안에 걸리고, 글자가 틀리는
게 아니라 **커서가 움직여서** 몇 글자 뒤에야 알아챈다.

🔴 두 칸이 **현재 남아 있는 유일한 파괴적 구멍**이다. v0.14.0의 대응
(P/Y/U/, 제거)은 피해를 줄였을 뿐 충돌을 없애지 않았다.

## 4. 위 행 배치 — `I`/`O` vs `U`/`I`

v0.14.0이 `I`/`O` → `U`/`I`로 옮겼다가 **되돌린** 기록이다 (2026-08-20).

당시 근거는 두 개였다:
1. *"J/K 세로열 위의 검지+중지 — 최빈 기능에 최강 손가락"*
2. *"O/P를 비워 오른손 위 행의 오타 안전지대를 넓힌다"*

**근거 ①은 잘못된 적용이다.** §2 — 위 행의 병목은 힘이 아니라 도달성이고,
검지는 셋 중 가장 짧아 위 행 적합도가 가장 낮다.

**근거 ②는 타당하지만 이득이 작다.** `ㅖ`(Shift+P)는 흔하지만
`ㅒ`(Shift+O)는 드물다 — `P`만 비워도 대부분 지켜진다. 그리고 `O` 오발사는
단어 이동이라 레이어 불변식이 명시적으로 허용하는 🟠 등급이다.

### 손가락 부하 비교 (nav 레이어)

| 손가락 | `U`/`I` (v0.14.0) | **`I`/`O` (현재)** |
|---|---|---|
| 검지 | `H` `J` `N` `M` **`U`** → **5** | `H` `J` `N` `M` → 4 |
| 중지 | `K` `I` → 2 | `K` `I` → 2 |
| 약지 | `L` `.` → 2 | `L` `.` **`O`** → 3 |
| 새끼 | `/` → 1 | `/` → 1 |
| 엄지 | `Space` → 1 | `Space` → 1 |

오른손 검지는 원래 6키를 맡는 최과부하 손가락인데, `U`/`I`는 nav의 **최빈 기능
5개를 전부 검지에** 몰았다.

### same-finger bigram

- `U`/`I`: `U`↔`H`, `U`↔`J`, `U`↔`N`, `U`↔`M` — 검지에 **4쌍 추가**
- `I`/`O`: `I`↔`K`(중지), `O`↔`L`(약지) — 각 1쌍씩 **분산**

"단어 단위로 몇 칸 가다가 한 글자 미세조정"(`U`→`H`)은 편집에서 가장 흔한
동작인데, `U`/`I`에서는 그게 전부 검지 한 손가락의 2행 왕복이었다.

### 습관과 구조를 구분하는 법

배치 변경 뒤의 불편이 **재학습 비용인지 구조적 결함인지** 가리는 기준:

- **습관** 문제는 *오타*로 나타난다 — "어 잘못 눌렀네".
- **구조** 문제는 *힘·뻐근함*으로 나타난다.

후자는 시간이 지나도 사라지지 않는다. `U`/`I`의 신고 증상은 후자였다.

> **문헌 대조** — 자동화된 동작 패턴을 바꾸면 **선행간섭(proactive
> interference)**으로 초기 수행 저하가 생긴다. 옛 표상이 일차운동피질에 남아
> 새 패턴과 경쟁하기 때문이고, `Journal of Motor Behavior`(2020)는 이걸
> **타이핑 맥락에서** 다뤘다. 즉 배치를 바꾼 직후의 불편은 **일정 부분 반드시
> 발생하며, 그 자체는 배치가 나쁘다는 증거가 아니다.** 위 구분 기준이 필요한
> 이유다.
> ([Reducing Proactive Interference in Motor Tasks](https://www.tandfonline.com/doi/abs/10.1080/00222895.2019.1635984))

## 5. 트리거 재검토 — `LAlt` vs `CapsLock`

2026-08-26 재검토 기록. **결론은 "재검토했고 CapsLock을 유지하되, Shift 충돌은
별도로 해결한다"**이다. 같은 논의를 다시 열기 전에 아래를 읽을 것.

### 5-1. `Ctrl+Alt+key`는 이미 보존된다 (구현 완료)

`LAlt` 트리거의 가장 큰 걱정인 "Ctrl+Alt 조합까지 잡아먹는다"는 **해결돼 있다.**
`engine.rs` 4-pre2 — 트리거가 modifier이고 `unmapped = "passthrough"`일 때,
**트리거 외의 비-Shift 수정자가 함께 눌린 키 down은 매핑을 건너뛰고**
`EngageChord`로 OS에 그대로 넘어간다. Windows/macOS 어댑터 양쪽 구현 완료
(v0.9.3, [08](08_layer_passthrough_plan.md)).

**충돌 범위는 nav에 매핑된 11개 키뿐이다.** 매핑되지 않은 키는
`UnmappedBehavior::Passthrough`가 그대로 코드 모드로 넘긴다(`engine.rs` §4의
Passthrough 분기).

| 조합 | `LAlt` 트리거 + `passthrough` | 이유 |
|---|---|---|
| `Ctrl+Alt+key` | ✅ 보존 | 4-pre2가 투과 |
| `Alt+F` `Alt+E` `Alt+V` `Alt+A` `Alt+W` `Alt+P` `Alt+Tab` `Alt+F4` … | ✅ 보존 | 미매핑 → `EngageChord` |
| `Alt+Shift+<미매핑 키>` | ✅ 보존 | 위와 같은 경로 (Shift는 물리 통과) |
| `Alt+` **`H J K L N M I O . / Space`** | ❌ 충돌 | nav 매핑이 이긴다 |
| `Alt+Shift+<위 11개>` | ❌ 충돌 | 가드가 Shift를 제외 → `Shift+네비`가 된다. `Alt+Shift+N`(HWP 자간좁히기) → `Shift+PageUp` |

리본으로 치면 `Alt+H`(홈) `Alt+N`(삽입) `Alt+M`(수식/편지) 정도가 실제 손실이다.

> **정정(2026-08-27)**: 이 절의 초판은 `Alt+key`가 통째로 깨진다고 썼는데
> 과했다. 그리고 `Alt+Shift+key`를 "구조적 불가"라고 단정했지만 **그건 우리
> 설계 선택이다** — AutoHotkey의 수정자 핫키는 등록한 수정자 집합이 *정확히*
> 일치할 때만 발동해서(`!h::`는 `Alt+Shift+H`에 반응하지 않음) 등록 안 한
> 조합이 전부 통과한다. 대신 선택 확장은 `!+h::`로 **따로 등록**해야 한다.
> 흥미롭게도 AHK의 커스텀 조합(`CapsLock & h::`)은 와일드카드가 기본이라
> **우리 레이어와 같게 동작한다** — 차이는 "AHK vs keymander"가 아니라
> **"수정자 핫키 vs 레이어"**다. 우리도 §6 #6처럼 키 단위 정책을 두면 둘 다
> 가질 수 있다.

### 5-1b. 더 큰 함정 — `Alt` 탭이 죽으면 KeyTip 전체가 죽는다

리본 접근 키는 **순차 방식**이다. `Alt`를 눌렀다 떼면 KeyTip이 뜨고 그다음
글자를 누른다. 그런데 레이어 트리거의 키다운은 무조건 억제되고(`engine.rs` §3),
뗄 때 탭으로 판정되면 `tap_action`이 대신 나간다. `tap_action = "Escape"`면:

**`Alt` 탭 → Escape → KeyTip이 아예 안 뜬다.** `Alt+H` 하나가 아니라
`Alt+F`/`Alt+N`/`Alt+Q` 등 **체계 전체**가 막힌다. 위 표의 "미매핑 키는 보존"은
*홀드해서 코드로 치는 경로*에만 해당한다.

완화책: `tap_action = "LAlt"`. `SendKey`는 down+up 완전한 타건을 주입하므로
(`send_key_press`) 앱이 보는 건 평범한 Alt 타건이고 KeyTip이 정상 표시된다.
대신 "탭 = Esc" 편의는 포기한다.

### 5-2. 두 선택지는 서로 다른 곳에서 깨진다

| 축 | `LAlt` 트리거 | `CapsLock` 트리거 |
|---|---|---|
| `Ctrl+Alt+key` | ✅ 보존 | ✅ 무관 |
| `Alt+key` — 미매핑 키 | ✅ 보존 | ✅ 무관 |
| `Alt+key` — nav 매핑 11개 | ❌ 충돌 (`Alt+H`/`N`/`M` 리본) | ✅ 보존 |
| `Alt` 탭 → KeyTip | ⚠️ `tap_action="LAlt"` 필요 (§5-1b) | ✅ 무관 |
| `Alt+Shift+key` | ⚠️ 매핑 키만 충돌 · 설계로 수정 가능 (§6 #6) | ✅ 보존 |
| `LShift` 동시 사용 | ✅ 엄지/새끼 독립 | ❌ 같은 새끼 (물리적 불가) |
| 일반 타이핑 오발사 | ✅ Alt는 평소 안 눌림 | ❌ 인접 + 동일 손가락 (§3) |
| 새끼 부하 | ✅ 엄지로 분산 | ❌ 새끼 집중 |

**이건 인체공학 대 인체공학이 아니라 인체공학 대 앱 호환성이다.**
순수 인체공학만 보면 Alt가 낫고 — Colemak 권고("모디파이어를 엄지로")가
가리키는 방향이 바로 `LAlt`다. Alt의 결함은 신체가 아니라 **소프트웨어 관습**에서
온다.

따라서 정답은 사용 패턴에 달렸다:

- 리본·메뉴를 **Alt 니모닉으로 조작한다** → CapsLock 유지.
- 주로 에디터·터미널·브라우저이고 Alt+글자를 거의 안 쓴다 → **Alt가 낫다.**

### 5-3. 채택 — 레이어 안의 Shift (`CapsLock` 유지)

> **2026-08-27 유보**: §5-1 정정으로 `LAlt` 복귀 비용이 초판 추정보다 작아졌다
> (실제 손실은 리본 `Alt+H`/`Alt+N`/`Alt+M` 정도 + `tap_action` 조정).
> 사용자가 CapsLock을 실사용으로 더 검증하기로 해 **트리거 결정 자체는 보류**
> 상태다. 아래 채택안은 어느 트리거를 고르든 유효하므로 그대로 둔다.

Alt로 돌아갈 유일한 강한 논거가 §3-2 **(B) 동시 입력 충돌**인데, 이건
트리거를 바꾸지 않고도 없앨 수 있다. nav 레이어에서 **왼손이 통째로 비어
있기** 때문이다.

```
CapsLock  +  F  +  H   →  Shift+←  (선택 확장)
 왼새끼     왼검지 오른검지   ← 세 이펙터가 전부 독립
```

새끼는 트리거만 잡고 Shift는 검지가 맡는다. Colemak의 "모디파이어를 분산하라"를
**CapsLock을 버리지 않고** 만족시키는 셈이다. (B)가 사라지면 남는 건 연습으로
줄어드는 (A)뿐이다.

필요한 작업: `BindAction`에 "레이어 키를 누르는 동안 수정자 홀드" 액션 추가.
`mouse:slow`가 이미 비슷한 상태 유지 구조라 크지 않다. → §6 #5

### 5-4. `LAlt`를 시험하려면

되돌리기는 config 세 줄이다. **세 번째 줄이 없으면 `Ctrl+Alt+key`까지 깨져서
"역시 Alt는 안 되는구나"라는 잘못된 결론이 난다.**

```toml
trigger = "LAlt"
tap_action = "Escape"
unmapped = "passthrough"    # ← 필수. 이게 4-pre2를 켠다
```

### 5-5. 인용의 한계

이 절과 §2·§3·§4의 문헌 인용은 **초록과 검색 요약까지만 확인한 것**이다 —
ExpECT PDF, Engram 논문, JMB 논문 본문은 접근이 막혀(403/바이너리) 원문
대조를 못 했다. 결론을 뒤집을 만한 재검토를 할 때는 원문을 먼저 확보할 것.

## 6. 열려 있는 과제

| # | 제안 | 해결하는 것 | 상태 |
|---|---|---|---|
| 1 | `.`/`/`에만 **홀드 게이트** — CapsLock 키다운 후 ~120ms 안에 들어온 `.`/`/`는 레이어 매핑을 건너뛰고 통과 | §3-3의 🔴 두 칸. 빠른 타이핑 롤(수정자→키 30~80ms)과 의도적 레이어 진입(100ms+)을 시간으로 가른다. 이동 키는 무해하므로 즉시 반응 유지 | 미착수 |
| 2 | `Shift+Space` 한/영 combo 제거 검토 | 한/영 주 경로는 이미 RAlt 탭으로 단일화됐다. 남겨두면 슬립 시 런처가 뜨는 경로가 하나 더 생긴다 | 미착수 |
| 3 | `N`/`M`(PgUp/PgDn)과 단어 이동의 자리 교환 | 아래 행이 위 행보다 편한데(§2), 지금은 *덜 쓰는* 페이지 이동이 *더 편한* 자리를 차지한다 | 재학습 비용이 커 보류 |
| 4 | ~~kanata 프리셋(`VIM_NAV_KBD`) 드리프트 정리~~ | 트리거 CapsLock · 금지 액션(`y`/`p`/`u`/`,`) 제거 · `W`/`R` 클릭 · `T`/`G` 휠로 정렬. 열 정렬·금지 액션·트리거 일치를 검사하는 드리프트 가드 테스트 3개 추가 | ✅ **완료 (2026-08-27)**. 문법은 kanata 공식 가이드 대조로 확인했으나 **실행 검증은 미완** — CI에서 kanata config 검사 모드를 돌리는 것이 최종 방어선 |
| 5 | **레이어 로컬 Shift** — nav 레이어의 왼손 키(예: `F`)를 홀드하는 동안 Shift 주입. `BindAction`에 수정자 홀드 액션 추가 | §3-2 (B) 동시 입력 충돌을 트리거 교체 없이 제거한다. 새끼는 트리거만, Shift는 검지가 담당 | **채택, 미착수** (→ §5-3) |
| 6 | **키 단위 Shift 정책** — 레이어에 `shift = "exact"` + `shift_mappings` 도입. 4-pre2의 Shift 제외 조건을 "그 키가 `shift_mappings`에 있는가"로 교체 | `Alt+Shift+H`는 선택 확장, `Alt+Shift+N`은 앱으로 투과 — AHK가 열거 방식으로 자연히 얻는 것을 레이어 모델에서도 얻는다. `LAlt` 복귀를 검토할 때만 필요 (→ §5-1) | 미착수 |

트리거를 **새끼손가락 밖의 새 자리로** 빼는 안은 마땅한 곳이 없어 기각했다 —
`LCtrl`/`Tab`도 왼새끼, 오른손 키는 네비 키와 같은 손, `LWin`은 `Win+L`(화면
잠금) 위험, `Space`는 롤오버 문제로 이미 기각([13](13_capslock_trigger.md)).
`LAlt`로 되돌리는 안은 §5에서 별도로 검토했다 — 인체공학적으로는 가장 낫지만
`Alt+key`·`Alt+Shift+key`를 구조적으로 못 살려서, **CapsLock을 유지하고 #5로
Shift 충돌만 떼어내는** 쪽을 택했다.

## 7. 결정 기록

| 날짜 | 결정 | 근거 |
|---|---|---|
| 2026-08-10 | 트리거 LAlt → CapsLock | modifier 트리거가 앱 단축키를 가리는 구조적 충돌 ([13](13_capslock_trigger.md)) |
| 2026-08-12 | CapsLock 탭 = 무동작, 한/영은 RAlt 탭으로 | 실수 탭이 조용히 입력 소스를 바꿈 |
| 2026-08-12 | 삭제키(`.`/`/`) 더블탭 제거, 줄 삭제 분리 | Backspace/Delete는 연타 빈도 1위 — 탭-탭이 줄 삭제로 오발사 |
| 2026-08-12 | 마우스 WASD → ESDF | 타이핑 홈포지션(검지 `F`) 유지, 확장 자리 확보 |
| 2026-08-12 | nav에서 `P`/`Y`/`U`/`,` 제거 | 레이어 불변식 — CapsLock↔LShift 오발사가 붙여넣기·줄 삭제로 이어짐 |
| 2026-08-12 | 단어 이동 `I`/`O` → `U`/`I` | "검지+중지가 최강 손가락" + O/P 안전지대 |
| **2026-08-20** | **단어 이동 `U`/`I` → `I`/`O` 복원** | **위 행 적합도는 중지 ≥ 약지 > 검지. 검지 부하 5 → 4, SFB 4쌍 해소** |
| 2026-08-26 | 트리거 재검토 → CapsLock 유지 + 레이어 로컬 Shift 채택 | `Ctrl+Alt`는 이미 보존됨을 확인. Alt 복귀의 강한 논거(Shift 동시 사용)는 왼손 Shift로 제거 가능 (§5-3). ⚠️ 이때의 "`Alt+key`·`Alt+Shift+key`는 구조적으로 못 살린다"는 판단은 **다음 날 정정됨** |
| **2026-08-27** | **트리거 결정 보류 — 실사용 검증 후 재판단** | `Alt+key`는 nav 매핑 11개만 충돌하고 나머지는 passthrough로 보존된다. `Alt+Shift`도 구조적 불가가 아니라 **설계 선택**(AHK 대조). KeyTip 탭 경로 문제(§5-1b)는 새로 발견 — §5-1 정정 |

## 참고 문헌

원문 대조 여부는 §5-5 참고 — 아래는 초록·요약 수준에서 확인한 것이다.

- [Optimizing Comfortable Keyboard Layouts Using Human Typing Preferences and Language-Dependent n-Grams: The Engram Study](https://www.tandfonline.com/doi/full/10.1080/10447318.2026.2665409) — *International Journal of Human-Computer Interaction*. 키 위치별 노력 비용 모델(손가락 힘·뻗기 vs 말기·스태거). → §2
- [Engram 프로젝트 (binarybottle)](https://github.com/binarybottle/engram) — 위 논문의 구현·데이터
- [ExpECT: An Expanded Error Categorisation Method for Text Input](https://www.yorku.ca/mack/bhci2007.pdf) — Kano & MacKenzie. 텍스트 입력 오류 분류. → §3-1
- [Types of Typing Errors and What Causes Each One](https://likelytypo.com/articles/types-of-typing-errors.html) — 대체/전치 오류와 같은 손가락 인접의 관계 개괄. → §3-1
- [Reducing Proactive Interference in Motor Tasks](https://www.tandfonline.com/doi/abs/10.1080/00222895.2019.1635984) — *Journal of Motor Behavior* 52(3). 타이핑 맥락의 자동화 패턴 변경과 선행간섭. → §4
- [How effector-specific is the effect of sequence learning by motor execution and motor imagery?](https://link.springer.com/article/10.1007/s00221-017-5096-z) — *Experimental Brain Research*. 시퀀스 학습의 이펙터 특이성은 문헌이 엇갈린다는 근거. → §4
- [Hotkeys — Definition & Usage | AutoHotkey v2](https://www.autohotkey.com/docs/v2/Hotkeys.htm) — 수정자 핫키의 정확 일치 매칭 vs 커스텀 조합의 와일드카드 기본값. → §5-1
- [Use the keyboard to work with the ribbon — Microsoft Support](https://support.microsoft.com/en-us/office/use-the-keyboard-to-work-with-the-ribbon-954cd3f7-2f77-4983-978d-c09b20e31f0e) — KeyTip 순차 동작(`Alt` → 글자). → §5-1b
- [Ergonomic Keyboard Mods: Modifiers](https://colemakmods.github.io/ergonomic-mods/modifiers.html) — CapsLock-모디파이어의 Shift 동일 새끼 충돌과 "엄지로 분산" 권고. → §3-1, §5
- [Chord skill: learning optimized hand postures and bimanual coordination](https://pmc.ncbi.nlm.nih.gov/articles/PMC10224868/) — 코드(동시 누름) 학습의 이펙터 특이성
