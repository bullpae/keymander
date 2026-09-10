---
name: pre-release-refactor
description: Performs pre-release hardening for keymander by auditing critical bugs, security risks, side effects, large files/functions, dead code, UX quality, and CI/release readiness. Use when the user asks for release preparation, deployment readiness checks, pre-release refactoring, or final QA before shipping.
---

# Pre-release Refactor

## Goal

배포 직전에 안정성/보안/성능/UX를 동시에 점검하고, 실제 릴리스 가능한 상태로 마무리한다.

## Entry Conditions

- 사용자 요청에 `배포`, `릴리즈`, `release`, `pre-release`, `리팩토링`, `최종 점검`이 포함됨
- 또는 CI 실패/플랫폼별 실패를 해결한 뒤 릴리스 품질 검증이 필요한 상황

## Mandatory Workflow

1. **CI 상태 확인**
   - `gh run list --workflow CI --limit 5`
   - 최신 run 실패 시 `gh run view <run-id> --log-failed`로 원인 고정
2. **안정성/보안 점검**
   - `unsafe`, `unwrap`, 프로세스/파일 I/O 경로, OS 분기 로직 우선 확인
   - 플랫폼별(Windows/macOS/Linux) 컴파일 경로 불일치 확인
3. **크기/복잡도 점검**
   - 큰 파일/함수 우선순위 목록 작성
   - 즉시 분리 가능한 책임(파싱/실행/UI 렌더링/설정)을 작은 단위로 추출
4. **통합/재사용 검토**
   - 새 기능과 기존 로직 중복 제거
   - 기존 유틸/헬퍼 재사용 우선
5. **사이드 이펙트 검토**
   - 실행 시 외부 영향(프로세스 실행, 파일 변경, URL 오픈, 상태 저장) 확인
   - 기존 UX 플로우(입력/선택/실행/종료) 회귀 여부 확인
6. **불필요 코드 정리**
   - 죽은 코드, 미사용 변수/분기, 중복 테스트/문구 정리
7. **품질 게이트 실행**
   - `cargo check --all-targets`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo fmt --all -- --check`
   - `cargo test --workspace`
8. **릴리스 판정**
   - Critical/High 이슈 0개일 때만 배포 승인
   - 남은 리스크는 명시하고 차기 릴리스 항목으로 분리

## Prioritization Rules

- P0: 크래시, 데이터 손상, 보안 취약점, 플랫폼 빌드 실패
- P1: UX 회귀, 설정 불일치, 사이드 이펙트 누락
- P2: 구조 개선(함수 분리/중복 제거/가독성 개선)
- P3: 미세 최적화

## Refactor Checklist Template

아래 체크리스트를 복사해 진행 상태를 갱신한다.

```text
Pre-release Refactor Checklist
- [ ] CI 최신 run 성공 확인
- [ ] 잠재 버그/보안 취약점 점검 완료
- [ ] 큰 파일/함수 우선순위 산정 완료
- [ ] 재사용/통합 포인트 반영 완료
- [ ] 사이드 이펙트 회귀 점검 완료
- [ ] 불필요 코드 제거 완료
- [ ] UX 시나리오 수동 점검 완료
- [ ] 배포 승인 여부(Go/No-Go) 기록
```

## Output Format

최종 보고는 반드시 아래 순서를 따른다.

1. `Critical findings` (없으면 없음 명시)
2. `Changes applied` (파일 단위)
3. `Verification` (실행 명령 + 결과)
4. `Residual risks`
5. `Go / No-Go`

