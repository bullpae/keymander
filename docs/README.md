# keymander 문서

문서마다 **성격이 다르다.** 어떤 것은 현재 동작의 정본이고, 어떤 것은 이미
구현된 기능의 설계 이력이며, 어떤 것은 아직 구현되지 않은 계획이다. 이걸 구분하지
않으면 "계획서를 읽고 구현된 줄 알았다" 또는 그 반대의 오해가 생긴다.

## 참조 — 현재 동작의 정본

바꾸려면 코드와 함께 고쳐야 하는 문서들.

| 문서 | 내용 |
|---|---|
| [01_prd.md](01_prd.md) | 제품 요구사항 — 무엇을 만드는가 |
| [02_architecture.md](02_architecture.md) | 시스템 구조, 크레이트 경계, 데이터 흐름 |
| [03_plugin_spec.md](03_plugin_spec.md) | 플러그인 인터페이스 |
| [04_database_schema.md](04_database_schema.md) | DB 스키마 |
| [05_theming.md](05_theming.md) | 테마 규격 |
| [06_config_reference.md](06_config_reference.md) | `config.toml` 전체 레퍼런스 |
| [16_keymap_ergonomics.md](16_keymap_ergonomics.md) | 키맵 전체 지도(nav/mouse/전역) + 손가락 배치 근거 · 오발사 지도 · 결정 기록 |
| [07_distribution.md](07_distribution.md) | 배포 채널(brew/apt/yum/winget) 운영 가이드 + 공개 저장소 위생 |
| [versioning-and-release.md](versioning-and-release.md) | 버전·릴리스 규칙 |

## 설계 이력 — 구현 완료된 기능의 "왜"

기능은 이미 릴리스됐다. 이 문서들이 남아 있는 이유는 **검토한 대안과 기각한
이유**가 코드에는 안 남기 때문이다. 해당 코드를 고치기 전에 읽으면 같은 함정을
다시 밟지 않는다. 단, 서술이 미래형이라도 계획이 아니라 기록이다.

| 문서 | 상태 | 구현 위치 |
|---|---|---|
| [08_layer_passthrough_plan.md](08_layer_passthrough_plan.md) | ✅ v0.9.3 (P0–P3) · 실기기 검증·P4 대기 | `crates/kmd-daemon/src/keybind/engine.rs` |
| [09_llm_autopilot_plan.md](09_llm_autopilot_plan.md) | ✅ v0.10.0 (Windows 전용) | `crates/kmd-daemon/src/autopilot.rs` |
| [13_capslock_trigger.md](13_capslock_trigger.md) | ✅ v0.13.0부터 **기본 트리거** | `crates/kmd-core/src/keymap.rs`, `keybind/macos.rs` |
| [15_content_search_plan.md](15_content_search_plan.md) | ✅ P0~P3 (v0.15.0~v0.16.9) · P4 조건부 | `crates/kmd-core/src/content_index.rs` |
| [12_clipboard_plan.md](12_clipboard_plan.md) | ✅ v0.13.x (P1–P3) | `crates/kmd-daemon/src/clipboard.rs` |
| [14_testing_plan.md](14_testing_plan.md) | ✅ Tier 1 + Tier 2(E2E 키 주입) | `crates/kmd-daemon/tests/`, `.github/workflows/ci.yml` |

## 계획 — 아직 구현되지 않음

| 문서 | 상태 |
|---|---|
| [18_search_scope_ux.md](18_search_scope_ux.md) | **설계안** (2026-10-19) — `:scope` 검색 범위 관리(추가·삭제·즉시 인덱싱) + 포터블 프로그램 폴더 모드 |
| [17_feature_backlog.md](17_feature_backlog.md) | **기능 백로그** — 계획·누락 기능 전수 조사(2026-10-05, v0.16.8). 어떤 "향후"가 진짜 미구현이고 어떤 게 이미 됐는지, 무엇을 영구 보류할지 |
| [11_refactor_backlog.md](11_refactor_backlog.md) | **리팩토링·코드건강 백로그** — 코드 모양·블로킹 지점 (2026-10-18 전수 리뷰 반영, 진행 중) |
| [10_dojo_plan.md](10_dojo_plan.md) | **구현 (M1~M5, 브랜치 `bullpae/dojo`)** — 흰띠~단, 오늘의 수련, 결과 카드, 인장. 남은 것: v2 hard mode |
| [19_key_stats.md](19_key_stats.md) | **구현 (기본 꺼짐)** — 손버릇 통계: 레이어 수·맨 화살표·트리거 홀드 횟수만 모아 분석 |
| [deferred-ideas.md](deferred-ideas.md) | v0.3 로드맵에서 보류한 아이디어 모음 — 재검토용 |

## 변경 이력

- [CHANGELOG.md](../CHANGELOG.md) — 0.4.0 이후 전 버전
- [v03-changelog.md](v03-changelog.md) — 0.3.x 상세 (CHANGELOG의 `[0.3.x] and earlier`가 가리킨다)

---

**두 백로그의 차이**: [17](17_feature_backlog.md)은 *"약속했는데 없는 기능"*,
[11](11_refactor_backlog.md)은 *"있는데 모양이 나쁜 코드"*. 새 항목을 적을 때
어느 쪽인지 먼저 정한다.

**문서를 추가하거나 기능을 릴리스할 때**: 계획 문서가 구현되면 이 표에서 "계획"
→ "설계 이력"으로 옮기고 문서 상단에 상태 배너를 단다. 그대로 두면 다음 사람이
이미 있는 걸 다시 만들려 든다.
