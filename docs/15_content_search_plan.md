# 문서 본문 검색 (Content Search) 계획

> 상태: P0 완료 (2026-08-17), P1 구현 중.
> 배경: 파일명/폴더 검색을 넘어 "문서 안의 내용"으로 찾는 기능.
> 참고 분석 대상: [Docufinder(Anything)](https://github.com/chrisryugj/Docufinder) —
> **BSL 1.1 라이선스라 코드 재사용 불가, 설계 아이디어만 자체 구현으로 채택.**

## 1. Docufinder 검토 결론 (2026-08-17)

42,750줄 Rust 백엔드(FTS5+Lindera 형태소+KoSimCSE 벡터+RRF, OCR, HWP)를 조사한 결과:

**채택하는 설계 (자체 구현):**
- SQLite **FTS5 본문 인덱스** — keymander의 `rusqlite (bundled)`에 FTS5가 이미
  컴파일돼 있음을 확인(의존성 추가 0). 기존 `kmd.db` + `migrate()` 훅에 접합.
- **"NULL 컬럼 = 작업 큐"** — `indexed_at IS NULL`이 곧 남은 일 목록. 별도 잡
  테이블 없이 재시작·크래시 복구·재인덱싱이 쿼리 하나로 수렴.
- **증분 판정 mtime+size AND** — 해시 계산 없이 저렴하게, FAT 2초 해상도 등
  플랫폼 차이에 보수적으로 대응.
- **가시성 계단** — 메타데이터(파일명)는 즉시, 본문은 뒤이어. keymander는 이미
  파일명 인덱스가 있으므로 자연 충족.
- **방어 계층 철학** — 파일당 크기 상한, 읽기 실패는 조용히 스킵, 배치
  트랜잭션. (Docufinder의 catch_unwind 파서 격리는 외부 파서 도입 시 채택)

**채택하지 않는 것:**
- 벡터/시맨틱 검색(ONNX 모델 420MB, 콜드 683ms) — 런처 키스트로크 예산과 불일치
- HWP(Node 사이드카)·OCR·AI 질의응답 — 제품 정체성(미니멀 런처) 위배
- Lindera 형태소(사전 ~23MB) — 1차 보류. 한국어 재현율 실측 후 P4에서
  "형태소 토큰을 원문 뒤에 이어붙여 unicode61로 색인"하는 트릭으로 도입 검토

**Docufinder가 못 푼 문제에서 얻은 런처 특화 지침:**
- 타이핑 경로에 무거운 연산 금지 → 본문 검색은 **prefix 명시 진입**(P3)
- 한글 1~2자 쿼리의 LIKE 풀스캔 함정 → 최소 질의 길이 2자
- 결과 enrich가 검색보다 비쌈 → 첫 화면은 경로+스니펫만

## 2. P0 — 선결 조사 (완료)

macOS 데몬 파일 인덱싱 "0건" 기록(2026-07-29, 포터블 시절)은 **해소 확인**:
안정 경로(`~/.keymander/bin/kmd-daemon`)+launchd 데몬이 Desktop/Documents/Downloads
254건 인덱싱, 실제 디스크와 일치. TCC 폴더 권한이 안정 서명 구조에 귀속됨.

## 3. P1 — FTS5 본문 인덱스 (이번 구현)

**범위**: 플레인 텍스트 계열만. 3-OS 공통 코드(kmd-core + kmd-daemon)로 구현.

- **스키마** (`kmd.db`, `db.rs` migrate 훅):
  - `content_files(id, path UNIQUE, size, mtime, indexed_at)`
  - `content_fts` — `fts5(body, path UNINDEXED..., tokenize='unicode61')` 계열,
    rowid = content_files.id 수동 동기화 (Docufinder 방식, 트리거 없음)
- **대상 파일**: 확장자 허용 목록(`txt md markdown rst org csv log json yaml yml
  toml ini` + 주요 소스 확장자). 파일당 상한 기본 1MB (`max_file_kb`).
- **인코딩**: UTF-8 → 실패 시 EUC-KR/CP949 폴백(`encoding_rs`, U+FFFD 비율로 수용
  판정) — Windows 한국어 legacy 텍스트 대응.
- **스캔 범위**: `launcher.search_paths` + `ignore_patterns` 재사용, `search_depth` 공유.
- **증분**: path 기준 upsert, mtime+size 일치 시 스킵. 사라진 파일은 스캔 후 일괄
  삭제(임시 경로 셋 대조). 배치 트랜잭션(200건).
- **검색**: FTS5 MATCH(마지막 토큰 prefix `*`) + `bm25()` 랭킹 + `snippet()`
  (마커 → 하이라이트 범위). 최소 질의 2자.
- **설정** (`[launcher.content_search]`): `enabled`(기본 true), `max_file_kb`(1024),
  `extensions`(허용 목록 덮어쓰기).
- **데몬**: 기존 `spawn_index_refresher` 주기에 본문 sync 추가(파일명 인덱스 갱신
  직후). 데몬 없는 환경은 `kmd index --rebuild`가 함께 수행.
- **검증**: CLI `kmd grep <질의>`(경로+라인 스니펫 출력) + 단위 테스트 + 실기기
  스모크(데몬 경유 인덱싱 후 검색).

## 4. P2 — 증분 감시 + "자주 변하는 폴더" 제안 (완료 2026-08-17)

- **notify 감시 완료**: 데몬이 search_paths를 재귀 감시(`spawn_content_watcher`,
  notify 8) — 이벤트 500ms 잠잠 후 증분 sync, 최소 간격 10초(폭풍 방지),
  최대 지연 60초(연속 이벤트 기아 방지), 60초마다 config 재로드(경로 변경
  재감시·비활성화 반영). 동시 호출(리프레셔/IPC/워처)은 AtomicBool 가드로
  겹침 스킵 — sync가 증분·멱등이라 다음 호출이 따라잡는다.
  이벤트 유실은 기존 6시간 주기 리프레셔가 보정 (이중 안전망).
- **폴더 제안 완료** (자동 추가 금지, 제안만 — `folder_suggest.rs`):
  - 신호: 변경 활동 — 홈 직계 하위 폴더를 깊이 3·엔트리 예산 12k로 훑어
    "최근 14일 수정 문서 5개 이상 + 검색 범위 밖" 폴더 상위 후보 산출
    (숨김·ignore_patterns·범위 중첩 제외). 세션 캐시 TTL 10분.
  - 노출: 런처 `?` 빈 질의 하단 제안 행(**Enter = search_paths 추가 + config
    저장**), `kmd index --suggest`.
  - 실행 이력 신호(자주 연 항목의 부모 폴더)는 후속 여지로 남김.

## 5. P3 — 런처 통합

- **`?` prefix 완료 (2026-08-17)**: `?질의`/`:grep 질의` → 데스크톱·TUI 공통
  본문 검색 (`content_index::launcher_results`, folder_search.rs 패턴).
  결과 = "파일명 — «매치» 스니펫" + 실경로, Enter = 파일 열기.
  데스크톱은 공유 kmd.db를 지연 오픈(자체 desktop/kmd.db와 별개), TUI는 기존
  db 핸들 재사용. `:help` 목록에는 COMMANDS 레지스트리로 자동 노출.
- **완료 (v0.16.3)**: 검색 연산자 서브셋 `"구문"`, `-제외`, `ext:`, `path:`.
  `ParsedQuery`가 질의를 분해한다 — 본문 조건(`terms`/`phrases`/`excludes`)은
  FTS5 MATCH 식으로, 파일 필터(`extensions`/`path_fragments`)는 메타 테이블
  `f.path`에 거는 LIKE로 나뉜다. 두 종류를 섞지 않는 게 요점이다.
  - 필터만 있는 질의(`ext:rs`)는 빈 결과 — 본문 조건 없이 전체를 훑지 않는다.
  - LIKE의 `%`/`_`/`\`는 이스케이프한다(`path:%`가 전체 매칭이 되면 필터가
    무력화된다). 닫히지 않은 따옴표는 끝까지를 구문으로 본다 — 타이핑 중간
    상태에서도 결과가 나와야 한다.

## 6. P4 — 조건부 확장 (실측 후 결정)

- 한국어 재현율 부족 시: Lindera 형태소를 본문 뒤에 이어붙여 같은 FTS 행에 색인
  (커스텀 토크나이저 없이 부분어 재현율 확보, 실패 시 graceful degradation).
- 포맷 확장: docx(quick-xml)·pdf(순수 Rust 크레이트) — 외부 프로세스/사이드카 금지.
- 대안 경로 메모: ripgrep 실시간 grep(`:grep`)은 무인덱스라 매력 있으나 대형
  트리 지연·랭킹 부재로 기본 경로로는 비채택.
