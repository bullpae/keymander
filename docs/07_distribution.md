# 배포 채널 셋업 가이드

keymander를 winget / Homebrew / apt / yum으로 배포하기 위해 구축한 파이프라인의
**남은 수동 작업**과, **다른 프로젝트에 동일하게 적용하기 위한 체크리스트**.

갱신: 2026-08-08 (v0.12.0 기준)

---

## 1. 현재 상태 요약

| 채널 | 사용자 명령 | 상태 | 남은 일 |
|---|---|---|---|
| GitHub Releases + SHA256SUMS | — | ✅ 자동 | 없음 |
| Homebrew | `brew install bullpae/tap/keymander` | ✅ 설치·갱신 자동 | 없음 |
| apt (Debian/Ubuntu) | `apt install keymander` | ✅ 설치·갱신 자동 | 없음 |
| yum/dnf (Fedora/RHEL) | `dnf install keymander` | ✅ 설치·갱신 자동 | 없음 |
| winget (Windows) | `winget install keymander` | 🔶 등록 PR 검증·승인 대기 | 없음 — CLA 서명 완료(2026-08-08), 모더레이터 승인 대기 |

사용자 관점의 채널별 설치 배치(바이너리·예시 config 경로, 포터블/표준 모드,
런타임 파일 위치)는 README의 **"Install layout by channel"** 절이 정본이다.

**주의 — 자산 첨부 ≠ 저장소.** `.deb`/`.rpm`을 릴리스 자산으로 올리는 것만으로는
`apt update`/`dnf upgrade`가 새 버전을 찾지 못한다. 인덱스(`Packages`/`repodata`)와
서명을 갖춘 저장소가 있어야 하며, 그 발행이 `publish-repos.yml` 잡이다.

릴리스 자동화 흐름 (태그 push 시):

```
git tag vX.Y.Z && git push origin vX.Y.Z
  → build-cli / build-desktop / build-bundle / build-packages (deb·rpm)
  → release (SHA256SUMS.txt 생성 + GitHub Release 발행)
  → publish-repos (gh-pages에 APT/YUM 저장소 재발행)  ← GPG_PRIVATE_KEY
  → update-tap    (homebrew-tap의 formula 자동 갱신)   ← TAP_DEPLOY_KEY
  → update-winget (winget-pkgs에 갱신 PR 자동 제출)     ← WINGET_GITHUB_TOKEN
```

`-`가 포함된 태그(`v1.0.0-rc1` 등)는 세 갱신 잡을 모두 건너뛴다.

### 1.1 Linux 저장소 구조

`https://bullpae.github.io/keymander/` — GitHub Pages, **Actions 아티팩트 배포**
(브랜치 방식 아님. 저장소 Settings → Pages → Source = GitHub Actions)

```
apt/pool/main/k/keymander/*.deb
apt/dists/stable/main/binary-amd64/Packages{,.gz}
apt/dists/stable/{Release,InRelease,Release.gpg}
yum/x86_64/*.rpm + repodata/{repomd.xml,repomd.xml.asc}
keymander-archive-keyring.{asc,gpg}   # 저장소 서명 공개키
keymander.repo                        # dnf/yum 설정 파일
```

- 아키텍처는 `amd64`/`x86_64`만. CI가 리눅스용 deb/rpm을 그것만 만든다.
- 최근 **안정 릴리스 2개**만 담는다 (`publish-repos.yml`의 `KEEP_RELEASES`).
  현재+직전 버전이면 `apt install keymander=<직전버전>` 롤백이 되고, 사이트가
  작아야 Pages 배포가 제때 끝난다(아래 참고).
- 저장소는 매 발행마다 릴리스에서 통째로 재생성한다. 상태를 누적하지 않아
  자기 치유적이다 — 한 번 깨져도 다음 발행이 정상화한다.
- 릴리스 없이 재발행/검증하려면: `gh workflow run publish-repos.yml`.

**`Deploy to GitHub Pages` 스텝이 빨간불이어도 실패가 아니다.**
`actions/deploy-pages`는 배포 완료를 최대 10분만 기다리고 포기한다(`timeout`
입력을 더 크게 줘도 `600000`ms로 잘린다). 이 사이트는 패키지 바이너리 때문에
그보다 오래 걸리지만 배포 자체는 뒤이어 성공한다. 그래서 그 스텝은
`continue-on-error`로 두고, **바로 다음 스모크 테스트가 실제 게이트**다.
스모크 테스트는 URL이 200을 주는지가 아니라 *이번 빌드의 버전이 인덱스에
들어 있는지*로 판정한다 — 배포가 실패하면 이전 저장소가 그대로 200을 주기 때문에
단순 도달성 검사로는 조용한 실패를 못 잡는다.

---

## 2. 직접 해야 하는 작업 (1회성)

### 2.1 winget 최초 등록 — CLA 서명 ✅ 완료 (승인 대기 중)

등록 PR: **[winget-pkgs#413755](https://github.com/microsoft/winget-pkgs/pull/413755)**
(`bullpae.keymander` 0.12.0). 2026-08-08에 CLA 서명이 확인됐다
(`license/cla` = success, `Needs-CLA` 라벨 해제).

CLA는 **PR 본인 계정으로** 아래 코멘트를 다는 것이다. 법적 동의라 대리 서명 불가:

```
@microsoft-github-policy-service agree
```

계정당 1회면 되고 이후 모든 PR(자동 갱신 포함)에 적용된다. 그다음:

1. 자동 검증(Azure) → `Validation-Completed` 라벨
2. 모더레이터 승인 대기 (신규 패키지는 보통 며칠)
3. 머지 확인: Windows에서 `winget search keymander` → `winget install keymander`

문제가 생기면 PR에 봇이 라벨/코멘트로 원인을 남긴다
(예: `Validation-Installation-Error`, `Manifest-Validation-Error`).

**이전 시도가 왜 실패했나** — [#401304](https://github.com/microsoft/winget-pkgs/pull/401304)은
매니페스트 검증(`Validation-Completed`, `Azure-Pipeline-Passed`)을 다 통과하고도
CLA가 끝내 서명되지 않아 `Needs-CLA` 라벨이 붙은 채 CLOSED됐다. 닫힌 PR은
되살릴 수 없다. **PR을 열면 바로 CLA부터 달 것.**

**재제출이 필요해지면**:

```bash
scripts/gen-winget-manifests.sh <버전> /tmp/winget
```
로 매니페스트 3종을 만들고, fork한 `winget-pkgs`의 새 브랜치에
`manifests/b/bullpae/keymander/<버전>/` 으로 올린 뒤 PR을 낸다.
저장소가 거대해서 clone보다 GitHub contents API로 올리는 게 훨씬 빠르다.
PR 제목 관례: `New package: bullpae.keymander version X.Y.Z`

### 2.2 winget 자동 갱신용 PAT

최초 등록이 머지된 **뒤에야** 의미가 있다. winget-releaser는 winget-pkgs를
fork하고 PR을 내야 하므로 deploy key로는 안 되고 사용자 PAT가 필요하다.

1. GitHub → Settings → Developer settings → Personal access tokens →
   **Tokens (classic)** → Generate new token (classic)
2. Note: `keymander-winget-automation`, Expiration: 1년 권장
3. Scopes: **`public_repo`** 하나만
4. ```bash
   gh secret set WINGET_GITHUB_TOKEN --repo bullpae/keymander
   ```

토큰 만료 시 같은 명령으로 재등록. 만료가 다가오면 GitHub가 메일로 알려준다.

### 2.3 이미 등록된 시크릿 (재작업 불필요)

| 시크릿 | 용도 | 형태 |
|---|---|---|
| `TAP_DEPLOY_KEY` | homebrew-tap formula 갱신 | homebrew-tap에 등록된 쓰기 가능 **deploy key**의 비밀키 |
| `GPG_PRIVATE_KEY` | APT/YUM 저장소 서명 | 패스프레이즈 없는 armored 비밀키 |

- **deploy key를 쓴 이유**: classic PAT(`public_repo`)는 계정의 모든 공개 저장소에
  쓸 수 있고 만료 갱신이 필요하다. deploy key는 tap 저장소 하나에만 유효하고
  만료가 없다. 교체하려면 tap 저장소 Settings → Deploy keys에서 지우고
  새로 만들어 시크릿을 덮어쓴다.
- **서명 키 백업**: `~/.keymander-release/`에 비밀키(`apt-signing-key.private.asc`)와
  키링이 있다. 공개키는 저장소의 `dist/keymander-archive-keyring.asc`로 추적된다.
  **이 비밀키를 잃으면 모든 사용자가 새 키를 다시 임포트해야 한다** — 백업 필수.
  키 교체 시: 새 키 생성 → `GPG_PRIVATE_KEY` 갱신 → `dist/*.asc` 갱신 →
  `publish-repos.yml` 재실행 → 사용자에게 재임포트 안내.

### 2.4 git 커미터 정보 ✅ 설정 완료

한동안 로컬 커밋이 `ATOM <atom@ATOM-MacBook-Pro.local>`로 기록돼 기기 이름이
이력에 남았다(커밋 182개). 2026-08-08에 아래로 설정해 이후 커밋은 정상이다:

```bash
git config --global user.name  "bullpae"
git config --global user.email "bullpae@gmail.com"
```

이미 남은 182개는 **되돌리지 않기로 했다.** 이력을 재작성하면 모든 커밋 해시가
바뀌어 배포된 태그·릴리스·포크와 어긋나는데, 얻는 것은 기기 이름을 가리는 것뿐이라
비용이 이득을 넘는다.

---

## 3. 신규 프로젝트에 동일 적용하는 체크리스트

전제: GitHub 저장소 + 태그 push로 도는 릴리스 CI + 오픈소스 라이선스.

### 3.1 이 저장소에서 복사할 것

| 원본 | 내용 | 수정할 부분 |
|---|---|---|
| `.github/workflows/release.yml`의 `Generate checksums` 스텝 | SHA256SUMS.txt 생성 | 없음 (그대로) |
| `.github/workflows/release.yml`의 `update-tap` 잡 | tap formula 자동 갱신 | 저장소/formula 이름, 아티팩트 이름 |
| `.github/workflows/release.yml`의 `update-winget` 잡 | winget 갱신 PR 자동화 | `identifier`, `installers-regex` |
| `.github/workflows/release.yml`의 `build-packages` 잡 | .deb/.rpm 빌드 | 패키지/바이너리 이름 |
| `scripts/gen-homebrew-formula.sh` | formula 생성기 | 클래스명, URL, 바이너리 목록, caveats |
| `Cargo.toml`의 `[package.metadata.deb]`/`[package.metadata.generate-rpm]` | 패키지 메타데이터 | 이름·설명·assets 경로 |

### 3.2 채널별 절차

**① 체크섬** — release 잡에 스텝 복사. 끝.

**② Homebrew** — tap 저장소는 **계정당 하나**(`bullpae/homebrew-tap`)를 공유한다.
새 프로젝트는 `Formula/<이름>.rb`만 추가하면 된다:

1. 프로젝트에 `scripts/gen-homebrew-formula.sh` 복사 후 수정
2. 릴리스 산출물의 SHA를 넣어 초기 formula 생성 → tap 저장소에 커밋
3. `brew install bullpae/tap/<이름>`으로 설치 검증 (`brew test <이름>`까지)
4. `update-tap` 잡 복사. 인증은 **PAT가 아니라 tap 저장소 deploy key**를 쓴다 —
   계정 전체에 쓰이는 classic PAT보다 범위가 좁고 만료가 없다:
   ```bash
   ssh-keygen -t ed25519 -N "" -C "<프로젝트> release automation" -f /tmp/tapkey
   gh api -X POST repos/<계정>/homebrew-tap/keys \
     -f title="<프로젝트> release automation" -f key="$(cat /tmp/tapkey.pub)" -F read_only=false
   gh secret set TAP_DEPLOY_KEY --repo <계정>/<저장소> < /tmp/tapkey
   shred -u /tmp/tapkey /tmp/tapkey.pub
   ```

**③ winget** — 최초 1회 수동 등록 + 이후 자동:

1. 안정 URL의 릴리스 zip(또는 msi/exe)과 SHA256 준비
2. manifest 3종 작성: `manifests/<첫글자>/<계정>/<이름>/<버전>/` 아래
   `*.installer.yaml`, `*.locale.en-US.yaml`, `*.yaml` (버전 파일).
   keymander의 실제 예: [winget-pkgs#401304](https://github.com/microsoft/winget-pkgs/pull/401304)
   - 단독 exe zip이면 `InstallerType: zip` + `NestedInstallerType: portable`.
     **portable일 때만 NestedInstallerFiles에 여러 exe를 넣을 수 있다.**
   - `InstallerSha256`은 대문자 관례
3. 제출은 둘 중 하나:
   - `komac new <계정>.<이름>` (TTY 필요 — CI가 아닌 로컬 터미널에서 직접 실행)
   - 수동: winget-pkgs를 fork → 브랜치에 파일 3개 추가 → PR
     (거대 저장소라 clone 없이 GitHub contents API로 올리는 게 빠르다)
4. PR 제목 관례: `New package: <계정>.<이름> version X.Y.Z`
5. CLA는 계정당 1회 (§2.1)
6. 머지 후 `update-winget` 잡(winget-releaser)이 이후 버전을 자동 제출

**④ .deb/.rpm 패키지** — Rust 프로젝트면 cargo-deb/cargo-generate-rpm 메타데이터
복사·수정 후 `build-packages` 잡 복사. Rust가 아니면
[nfpm](https://nfpm.goreleaser.com/)이 같은 역할(설정 파일 하나로 deb/rpm/apk 생성)을
한다.

**⑤ apt/yum 저장소** — ④까지는 "다운로드해서 설치"만 되고 `apt update`로 **갱신을
받을 수는 없다**. 저장소를 따로 발행해야 한다.

1. 서명 키 1회 생성 (패스프레이즈 없이 — CI에서 비대화로 서명해야 한다):
   ```bash
   export GNUPGHOME=~/.<프로젝트>-release/gnupg   # 경로가 길면 gpg-agent 소켓이 실패한다
   mkdir -p "$GNUPGHOME" && chmod 700 "$GNUPGHOME"
   gpg --batch --gen-key <<'EOF'
   %no-protection
   Key-Type: RSA
   Key-Length: 4096
   Key-Usage: sign
   Name-Real: <프로젝트> archive signing key
   Expire-Date: 0
   %commit
   EOF
   ```
   RSA를 쓰는 이유는 오래된 apt/rpm 호환이다 (ed25519는 rpm 4.15+ 필요).
   만료를 두지 않는 이유는 만료된 저장소 키가 조용한 고장을 만들기 때문이다.
2. 공개키는 저장소에 커밋, 비밀키는 `gh secret set GPG_PRIVATE_KEY`.
   **비밀키 백업 필수** — 잃으면 전 사용자가 키를 다시 임포트해야 한다.
3. `scripts/build-linux-repos.sh` + `.github/workflows/publish-repos.yml` 복사
   (저장소 이름·아키텍처만 수정). GitHub Pages를 gh-pages 브랜치로 활성화:
   ```bash
   gh api -X POST repos/<계정>/<저장소>/pages -f 'source[branch]=gh-pages' -f 'source[path]=/'
   ```
4. `gh workflow run publish-repos.yml`로 릴리스 없이 검증한 뒤 릴리스 워크플로에
   `uses:`로 연결한다.

**릴리스 전 로컬 검증법** (CI가 막히는 걸 예방):

```bash
# deb/rpm 메타데이터 — 바이너리를 빌드해 두고 실제 패키징까지
cargo build --release && mkdir -p target/package && <config 준비>
cargo deb -p <pkg> --no-build -o /tmp/pkg && cargo generate-rpm -o /tmp/pkg

# 워크플로우 YAML
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"

# formula
brew install --formula ./Formula/<이름>.rb && brew test <이름> && brew uninstall <이름>
```

### 3.3 다음 단계 채널 (필요해지면)

- **AUR**(Arch): PKGBUILD 하나, 요건 최소 — 가장 쉬움
- **COPR**(Fedora/RHEL): spec 파일로 dnf 저장소 제공
- **homebrew-core / 공식 배포판**: 인지도 요건(별 75+ 등) 충족 후

---

## 4. 트러블슈팅 메모 (keymander에서 겪은 것)

- **komac이 TTY를 요구** — 파이프/스크립트 환경에서는 대화형 프롬프트(중첩 파일
  선택)가 막힌다. 로컬 터미널에서 직접 실행하거나 manifest를 수동 작성할 것.
- **winget portable + zip**: zip 안 최상위 디렉터리까지 포함한 상대 경로를
  `RelativeFilePath`에 적는다 (`keymander\kmd.exe`).
- **cargo-deb `$auto` 의존성**은 `dpkg-shlibdeps`가 필요 — CI apt 목록에
  `dpkg-dev` 포함 (macOS 로컬 테스트에서는 경고만 뜨고 의존성 없이 패키징됨).
- **Homebrew 바이너리 배포**는 코드사인/노터라이즈 없이도 Gatekeeper에 걸리지
  않는다 (brew 설치 경로는 quarantine 미부착). 직접 다운로드와 다른 점.
- **MSVC 배포 전제**: Windows 바이너리는 CRT 정적 링크(`crt-static`) 상태여야
  깨끗한 시스템에서 VCRUNTIME140.dll 없이 동작한다 (v0.9.1에서 적용).
- **winget CLA를 미루면 PR이 닫힌다** — #401304은 매니페스트 검증을 다 통과하고도
  CLA 미서명으로 CLOSED됐다. PR을 열자마자 CLA 코멘트부터 달 것.
  **CLA 코멘트는 `@microsoft-github-policy-service agree` 한 줄을 그대로** 달아야
  한다. 다른 문구(예: "Ver 0.12.0")를 달면 봇이 인식하지 못하고 `Needs-CLA`가
  그대로 남는다 — #413755에서 실제로 이렇게 헛돌았다. 서명 여부는 라벨이 아니라
  `license/cla` 체크로 확인하는 게 확실하다:
  ```bash
  gh api repos/microsoft/winget-pkgs/commits/$(gh api repos/microsoft/winget-pkgs/pulls/<번호> --jq .head.sha)/check-runs \
    --jq '.check_runs[] | select(.name=="license/cla") | .conclusion'   # null = 미서명
  ```
  (닫힌 PR은 브랜치가 남아 있으면 `gh pr reopen`으로 되살릴 수 있다.)
- **`Validation-Installation-Error`가 곧 패키지 문제는 아니다** — 실패 상세는 PR
  코멘트에 안 붙고 Azure 파이프라인 아티팩트에만 있다. 반드시 받아서 볼 것:
  ```bash
  # PR 첫 코멘트의 파이프라인 링크에서 buildId 확인 후
  BASE=https://dev.azure.com/shine-oss/8b78618a-7973-49d8-9174-4360829d979b/_apis/build/builds/<buildId>
  curl -s "$BASE/timeline?api-version=7.0"      # 실패한 태스크와 log id
  curl -sfL -o ivl.zip "$BASE/artifacts?artifactName=InstallationVerificationLogs&api-version=7.0&\$format=zip"
  ```
  #413755(0.12.0)의 실패는 검증 VM의 Defender 시그니처 업데이트 실패
  (`hr=0x80070652`) → MotW 첨부 스캔이 `0x80004005` 반환 → winget이
  `0x8A15002D`(보안 검사 실패)로 중단한 **환경 문제**였다. 위협 탐지 기록은
  전혀 없었고 매니페스트/URL/정책/카탈로그 검증은 모두 통과했다.
  `0x8A15002D`는 "악성 판정"이 아니라 "스캔을 수행하지 못함"이다 — 탐지
  판정인지 아닌지는 로그에 threat/quarantine 기록이 있는지로 가른다.
- **gpg-agent "File name too long"** — `GNUPGHOME` 경로가 길면 agent 소켓 생성이
  실패한다(유닉스 소켓 경로 길이 제한). 홈 디렉터리 바로 아래처럼 짧은 경로를 쓴다.
- **apt는 서명 없는 저장소를 거부한다** — `Release`에 `InRelease`(클리어서명)나
  `Release.gpg`가 없으면 `apt update`가 실패한다. `[trusted=yes]`로 우회할 수는
  있지만 사용자에게 검증 없는 설치를 시키는 것이라 쓰지 않는다.
- **Jekyll이 저장소 파일을 먹는다** — 배포 산출물에 `.nojekyll`이 없으면 Pages가
  `repodata/` 같은 디렉터리를 임의로 처리할 수 있다. 반드시 넣을 것.
- **`dpkg-scanpackages`는 기본이 최신 버전 1개** — `--multiversion` 없이는 pool에
  이전 릴리스를 둬도 인덱스에 안 들어가 버전 고정·롤백이 안 된다.
- **dnf는 repo 키를 따로 묻는다** — `rpm --import`를 미리 했어도 `repo_gpgcheck=1`
  때문에 첫 사용 시 키 수락 프롬프트가 뜬다(정상). 비대화 환경은 `-y` 필요.
- **컨테이너로 끝까지 검증할 것** — 서명 검증만으로는 `Filename` 경로 오류나
  아키텍처 불일치를 못 잡는다. 실제 설치까지 돌려보는 게 확실하다:
  ```bash
  podman run --rm --platform linux/amd64 ubuntu:24.04 bash -c '<위 apt 설치 절차>'
  podman run --rm --platform linux/amd64 fedora:41   bash -c '<위 dnf 설치 절차>'
  ```

---

## 5. 공개 저장소 위생

이 저장소는 2026-02-11 생성 시점부터 **public**이다. 즉 커밋된 순간 노출이고,
나중에 파일을 지우거나 이력을 재작성해도 이미 클론·포크·캐시된 것은 되돌릴 수
없다. **비밀정보가 들어갔다면 삭제가 아니라 폐기·교체가 답이다.**

### 5.1 방어선

| 층 | 무엇을 막나 | 상태 |
|---|---|---|
| GitHub push protection | 공급자 토큰(`ghp_`, AWS 키 등) 푸시 차단 | ✅ 켜짐 |
| GitHub secret scanning | 위와 동일 패턴 사후 탐지 | ✅ 켜짐 |
| 비공급자 패턴 스캔 | **개인키 블록** 등 | ⚠️ 꺼짐 — §5.2 |
| `scripts/scan-secrets.sh` (CI `secrets` 잡) | 개인키·토큰, **이력 전수** | ✅ 켜짐 |
| `.gitignore` | `*.pem` `*.key` `*.private.asc` `.env` 등 | ✅ |

CI 잡은 `--history`로 돌아 **삭제된 파일까지** 검사한다. 지운다고 노출이 사라지지
않으므로 현재 트리만 보는 것은 의미가 약하다.

로컬에서도 같은 검사를 돌릴 수 있다:

```bash
scripts/scan-secrets.sh            # 추적 파일만 (빠름)
scripts/scan-secrets.sh --history  # 전체 이력 (느림, CI와 동일)
```

> 스크립트 패턴은 `-----`로 시작하므로 grep에 반드시 `-e`로 넘겨야 한다.
> 그냥 넘기면 옵션으로 파싱돼 **아무것도 못 잡고 조용히 통과한다** — 실제로
> 그렇게 만들었다가 심어둔 개인키를 놓쳤다. 가드를 고칠 때는 반드시
> 가짜 비밀정보를 심어 잡히는지부터 확인할 것.

### 5.2 남은 수동 작업 — 비공급자 패턴 스캔 켜기

REST API(`PATCH /repos/{o}/{r}`)는 이 설정을 200으로 받고도 **조용히 무시한다**.
웹 UI에서만 켜진다:

**Settings → Advanced Security → Secret Protection →
"Scan for non-provider patterns" → Enable**

켜면 GitHub이 개인키 블록 같은 범용 패턴도 푸시 차단 대상으로 잡아준다.
CI 잡이 이미 같은 역할을 하므로 중복 방어이지, 이게 없다고 뚫리는 건 아니다.

### 5.3 공개돼 있는 신원 정보 (의도된 것)

- `bullpae@gmail.com` — `Cargo.toml`의 maintainer, GPG 키 UID, 커밋 작성자.
  패키지 메인테이너 연락처는 `apt show`/`dnf info`에 노출되는 게 정상이다.
- `dist/keymander-archive-keyring.asc` — 저장소 서명 **공개키**. 배포하라고 있는
  것이라 추적이 정상이며 `.gitignore` 예외로 지정돼 있다.
- 서명 **비밀키**는 `~/.keymander-release/`에만 있고 저장소에 없다. 절대 넣지 말 것.

---

## 6. 저장소 개명 런북 (`keymander-cli` → `keymander`)

이름이 사실과 어긋나 있다 — 이 저장소에는 GPU 데스크톱 앱(`kmd-desktop`)과
시스템 데몬(`kmd-daemon`)이 들어 있어 "cli"가 아니다. 개명은 정확성 교정이다.

**언제 하나** — winget 최초 등록 PR이 **머지된 뒤**. 심사 중에 릴리스 자산 URL이
바뀌면 검증이 흔들린다. 반대로 **미룰수록 비싸진다**: 아래 ⚠️ 항목의 영향 범위가
사용자 수에 비례한다. 스타·포크가 붙기 전이 가장 싼 시점이다.

### 6.1 개명이 깨는 것 / 안 깨는 것

| 대상 | 개명 후 |
|---|---|
| git remote, 이슈, PR, 릴리스 다운로드 URL | ✅ GitHub이 리다이렉트 |
| winget 기존 매니페스트의 `InstallerUrl` | ✅ 리다이렉트로 계속 동작 |
| `PackageIdentifier` (`bullpae.keymander`) | ✅ 불변 — 저장소명과 무관 |
| `publish-repos.yml`의 Pages 경로 | ✅ `github.event.repository.name`으로 자동 적응 |
| **GitHub Pages URL** | ⚠️ **리다이렉트 안 됨** — `…/keymander-cli/` 가 죽는다 |
| **사용자의 apt `sources.list` / yum `.repo`** | ⚠️ 위 때문에 **사용자가 직접 고쳐야 함** |

### 6.2 절차

```bash
# 1) 개명 (keymander 이름은 keymander-tui-prototype 이 비워둠)
gh api -X PATCH repos/bullpae/keymander-cli -f name=keymander

# 2) 로컬 리모트 갱신
git remote set-url origin https://github.com/bullpae/keymander.git

# 3) 문서·메타데이터의 하드코딩 치환
#    (생성기 2종은 GITHUB_REPOSITORY/origin에서 유도하므로 손댈 필요 없다)
git grep -l keymander-cli | xargs sed -i '' 's#keymander-cli#keymander#g'
git grep -n keymander-cli   # 남은 게 없어야 한다

# 4) Pages 재발행 → 새 URL로 apt/yum 저장소 생성
gh workflow run publish-repos.yml

# 5) Homebrew formula 재생성 (URL이 새 저장소를 가리키게)
#    최신 릴리스 SHA는 SHA256SUMS.txt 또는 릴리스 자산의 digest에서 가져온다
scripts/gen-homebrew-formula.sh <버전> <sha_arm_mac> <sha_x86_mac> <sha_x86_linux>
#    → bullpae/homebrew-tap 의 Formula/keymander.rb 에 반영
```

### 6.3 개명 후 반드시 공지할 것

apt/yum 사용자는 저장소 주소를 직접 바꿔야 한다. README와 릴리스 노트에 넣을 문구:

```bash
# apt
sudo sed -i 's#/keymander-cli/#/keymander/#' /etc/apt/sources.list.d/keymander.list
sudo apt update

# yum/dnf
sudo curl -fsSL -o /etc/yum.repos.d/keymander.repo \
  https://bullpae.github.io/keymander/keymander.repo
```

낡은 `keymander-cli` Pages 주소는 404가 되므로, 옛 주소를 쓰는 클라이언트는
`apt update`가 실패한다. 조용히 낡은 버전에 머무는 게 아니라 눈에 띄게 실패하므로
사용자가 알아차리기는 한다.

### 6.4 저장소 가족 정리 상태 (2026-09-10 갱신)

| 저장소 | 상태 | 비고 |
|---|---|---|
| `keymander` | PUBLIC, 활성 | 본체. **2026-09-10 `keymander-cli`에서 개명 완료** (winget 최초 머지 직후) |
| `keymander-tui-prototype` | PRIVATE, archived | 원형(2026-02-07~02-11, 44커밋). 이름을 비우려고 개명 |
| `keymander-desktop` | PRIVATE, archived | 커밋 0개 빈 저장소. 삭제하려면 `delete_repo` 스코프 필요 |

`keymander-tui-prototype`은 git 조상을 공유하지 않는 **별개 계보**다. 다만 실행
이름 `kmd`와 태그라인 "키보드 하나로 모든 것을 지휘한다"는 원형부터 이어져 있다.

---

## 7. 로컬 개발 서명 — macOS 전용 키체인

로컬 재배포마다 접근성 권한이 풀리는 것(ad-hoc cdhash churn, §5 아님 —
`deploy-local.sh` 주석 참조)을 막으려면 안정된 서명 신원이 필요하다.

**로그인 키체인에 서명 키를 두면 안 된다.** codesign이 그 키를 쓸 때마다
macOS가 암호 프롬프트를 띄우는데, 스크립트/에이전트 같은 비대화 세션에서는
응답할 수 없어 `errSecInternalComponent`로 실패하고, 사용자 화면에는 맥락 없는
서명 요청 창만 뜬다 — 2026-08-08 키보드 먹통 사고의 한 원인이었다.

대신 **전용 키체인**을 만들어 암호 파일(0600)로 스크립트가 직접 잠금해제한다.
프롬프트가 원천적으로 없고, 로그인 키체인은 일절 건드리지 않는다.

1회 구성 (재현 절차):

```bash
DIR="$HOME/.keymander-release"; KC="$DIR/kmd-codesign.keychain-db"
mkdir -p "$DIR" && openssl rand -hex 24 > "$DIR/keychain-pass" && chmod 600 "$DIR/keychain-pass"
PASS="$(cat "$DIR/keychain-pass")"

security create-keychain -p "$PASS" "$KC"
security set-keychain-settings "$KC"                     # 자동잠금 없음

# 인증서 생성 — p12는 legacy 옵션이 없으면 Apple Security가 임포트를 거부한다
openssl req -x509 -newkey rsa:2048 -keyout "$DIR/key.pem" -out "$DIR/cert.pem" \
  -days 3650 -nodes -subj "/CN=keymander-local-codesign" \
  -addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning"
openssl pkcs12 -export -inkey "$DIR/key.pem" -in "$DIR/cert.pem" -out "$DIR/kmd.p12" \
  -passout "pass:$PASS" -name keymander-local-codesign \
  -legacy -macalg sha1 -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES

security import "$DIR/kmd.p12" -k "$KC" -P "$PASS" -T /usr/bin/codesign
# 핵심 — 이게 없으면 codesign이 키를 쓸 때마다 프롬프트가 뜬다
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$PASS" "$KC"
# codesign은 검색 목록에 있는 키체인만 뒤진다 (--keychain 만으로는 못 찾음)
security list-keychains -d user -s login.keychain-db "$KC"
rm -f "$DIR/kmd.p12" "$DIR/key.pem"
```

이후는 자동: `deploy-local.sh`의 `codesign_binaries()`가 키체인·암호 파일을
감지해 잠금해제 후 서명한다. 잠금해제에 실패하면 **프롬프트를 띄우지 않기 위해**
ad-hoc으로 강등하고 경고한다.

키 교체/폐기: `security delete-keychain "$KC"` 후 위 절차 재실행 → 접근성 1회
재부여. 위협 모델: 이 키의 유일한 권능은 로컬 TCC 신원 연속성이다 — 배포 자산
서명에는 쓰이지 않으며(그건 CI/GPG), 유출돼도 이 머신의 TCC 항목 외엔 영향 없다.

## 8. macOS 로컬 운영 — brew 설치 + 고정 경로 동기화 (2026-08-10 전환)

포터블 번들(`~/keymander/`) 운영을 폐기하고 brew 설치로 일원화하되, TCC
접근성 권한 유지를 위해 **데몬은 항상 고정 경로에서 실행**한다.

**구조** (개발 머신 기준):

| 구성 요소 | 경로 | 역할 |
|---|---|---|
| brew 설치본 | `/opt/homebrew/Cellar/keymander/<ver>/bin/` | 원본 (업그레이드마다 경로 변경) |
| 고정 실행 경로 | `~/.keymander/bin/{kmd,kmd-daemon,kmd-desktop}` | 실제 실행 대상. PATH 최우선, 데몬 plist가 가리킴 |
| 동기화 스크립트 | `~/.keymander/sync.sh` (원본: `scripts/brew-sync-local.sh`) | Cellar → 고정 경로 복사 + §7 안정 서명 + 데몬 재시작. 같은 버전이면 no-op |
| 자동 트리거 | `~/Library/LaunchAgents/com.keymander.sync.plist` (원본: `scripts/`) | WatchPaths=`/opt/homebrew/opt/keymander` — brew 업그레이드 감지 시 sync.sh 실행 |
| 데몬 LaunchAgent | `~/Library/LaunchAgents/com.keymander.daemon.plist` | `~/.keymander/bin/kmd-daemon` 지정. `kmd daemon install`로 재생성 가능 |

**왜 고정 경로인가**: TCC 접근성 항목은 실행 파일 경로 기준이다. brew Cellar
경로는 버전마다 바뀌므로 직접 실행하면 업그레이드마다 손쉬운 사용 재부여가
필요하다. 고정 경로 + §7 안정 서명이면 경로·신원이 모두 불변이다.

**한계 (2026-08-10 관측)**: 같은 경로·같은 인증서·같은 identifier로
재서명해도 **바이너리 내용이 바뀌면 TCC가 허용을 자동 해제**하는 사례를
확인했다(tccd가 Unknown 반환 + 항목 Modify + universalAccessAuthWarn →
설정에서 체크 해제됨). 설정 UI에서 드래그로 추가한 항목의 cdhash 고정 또는
자체서명 인증서의 DR 평가 실패로 추정 — 즉 동기화 자동화가 재부여를 완전히
없애주지 못할 수 있다. 업그레이드 후 훅이 죽어 있으면: 손쉬운 사용에서
`~/.keymander/bin/kmd-daemon` 토글 재활성화 → `kmd daemon restart`.

**데몬 기동 규칙**: 데몬은 반드시 launchd가 띄워야 한다 — 터미널에서 직접
spawn하면 TCC 책임 프로세스가 터미널로 귀속되어 권한을 부여해도
AXIsProcessTrusted=false가 난다. `kmd daemon start`/`restart`는 LaunchAgent가
등록돼 있으면 자동으로 `launchctl bootout→bootstrap` 경유로 기동한다
(kickstart는 바이너리 교체 후 서명 pin 불일치로 즉사하므로 쓰지 않는다).

**주의**: `deploy-local.sh`는 여전히 포터블 경로(`~/keymander/`)를 타깃으로
한다 — 이 워크플로를 다시 쓰려면 DEPLOY_DIR을 `~/.keymander/bin`으로 맞추고
plist 충돌을 정리해야 한다(미정합 상태).
