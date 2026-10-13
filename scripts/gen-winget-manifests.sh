#!/usr/bin/env bash
# ============================================================
# winget 매니페스트 3종 생성 (최초 등록용)
# ============================================================
#
# 최초 등록이 microsoft/winget-pkgs에 머지된 뒤부터는 릴리스 워크플로의
# update-winget 잡(winget-releaser)이 새 버전을 자동 제출한다. 이 스크립트는
# **최초 1회 등록**과, 등록이 막혀 재제출해야 할 때를 위한 것이다.
#
# 사용법:
#   gen-winget-manifests.sh <버전> [출력디렉터리]
#   예) gen-winget-manifests.sh 0.12.0 /tmp/winget
#
# SHA256과 릴리스 날짜는 GitHub 릴리스에서 직접 읽는다(gh 필요).
# 출력: <출력디렉터리>/manifests/b/bullpae/keymander/<버전>/ 아래 3개 파일

set -euo pipefail

VERSION="${1:?사용법: gen-winget-manifests.sh <버전> [출력디렉터리]}"
OUT_ROOT="${2:-./winget-manifests}"

# 저장소 이름은 유도한다 — 개명하면 InstallerUrl이 죽은 주소가 되고, 그 사실이
# winget 검증 실패로만 드러난다. PackageIdentifier는 winget 카탈로그의 고유 키라
# 저장소를 개명해도 절대 바뀌면 안 되므로 이쪽만 고정한다.
REPO="${GITHUB_REPOSITORY:-$(git remote get-url origin 2>/dev/null |
    sed -E 's#(git@github\.com:|https://github\.com/)##; s#\.git$##')}"
: "${REPO:?저장소를 알 수 없습니다 — GITHUB_REPOSITORY를 지정하거나 git 저장소 안에서 실행하세요}"
PKG_ID="bullpae.keymander"
# winget-pkgs가 요구하는 스키마 버전. 올릴 때는 3개 파일을 함께 맞춰야 한다.
SCHEMA="1.9.0"

DEST="$OUT_ROOT/manifests/b/bullpae/keymander/$VERSION"
mkdir -p "$DEST"

# 릴리스 자산의 SHA256은 GitHub이 계산해 둔 값을 그대로 쓴다 — 재다운로드하며
# 해시를 다시 구할 이유가 없고, 전송 중 손상을 오해할 여지도 없앤다.
asset_sha() {
    gh api "repos/$REPO/releases/tags/v$VERSION" \
        --jq ".assets[] | select(.name == \"$1\") | .digest" |
        sed 's/^sha256://' | tr '[:lower:]' '[:upper:]'
}

SHA_X64="$(asset_sha "keymander-winget-x86_64-pc-windows-msvc.zip")"
SHA_ARM64="$(asset_sha "keymander-winget-aarch64-pc-windows-msvc.zip")"
RELEASE_DATE="$(gh api "repos/$REPO/releases/tags/v$VERSION" --jq '.published_at[:10]')"

[ -n "$SHA_X64" ] && [ -n "$SHA_ARM64" ] || {
    echo "✗ v$VERSION 릴리스에서 Windows winget zip을 찾지 못했습니다" >&2
    exit 1
}

# InstallerType: zip + NestedInstallerType: portable —
# 압축 안의 여러 exe를 각각 명령으로 노출할 수 있는 유일한 조합이다.
# RelativeFilePath는 zip 내부 최상위 디렉터리를 포함한 경로여야 한다.
cat > "$DEST/$PKG_ID.installer.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.installer.$SCHEMA.schema.json

PackageIdentifier: $PKG_ID
PackageVersion: $VERSION
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
- RelativeFilePath: keymander\\kmd.exe
  PortableCommandAlias: kmd
- RelativeFilePath: keymander\\kmd-desktop.exe
  PortableCommandAlias: kmd-desktop
- RelativeFilePath: keymander\\kmd-daemon.exe
  PortableCommandAlias: kmd-daemon
ReleaseDate: $RELEASE_DATE
Installers:
- Architecture: x64
  InstallerUrl: https://github.com/$REPO/releases/download/v$VERSION/keymander-winget-x86_64-pc-windows-msvc.zip
  InstallerSha256: $SHA_X64
- Architecture: arm64
  InstallerUrl: https://github.com/$REPO/releases/download/v$VERSION/keymander-winget-aarch64-pc-windows-msvc.zip
  InstallerSha256: $SHA_ARM64
ManifestType: installer
ManifestVersion: $SCHEMA
EOF

cat > "$DEST/$PKG_ID.locale.en-US.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.defaultLocale.$SCHEMA.schema.json

PackageIdentifier: $PKG_ID
PackageVersion: $VERSION
PackageLocale: en-US
Publisher: bullpae
PublisherUrl: https://github.com/bullpae
PublisherSupportUrl: https://github.com/$REPO/issues
PackageName: keymander
PackageUrl: https://github.com/$REPO
License: MIT
LicenseUrl: https://github.com/$REPO/blob/main/LICENSE
Copyright: Copyright (c) 2026 bullpae
ShortDescription: Keyboard-driven cross-platform launcher (TUI, desktop, key-remap daemon)
Description: |-
  keymander is a CLI-first cross-platform launcher controlled entirely from the keyboard.
  It ships three binaries: kmd (TUI/CLI launcher), kmd-desktop (GUI launcher window),
  and kmd-daemon (global hotkey and key-remapping daemon).
Moniker: keymander
Tags:
- cli
- keyboard
- launcher
- productivity
- terminal
- tui
ReleaseNotesUrl: https://github.com/$REPO/releases/tag/v$VERSION
ManifestType: defaultLocale
ManifestVersion: $SCHEMA
EOF

cat > "$DEST/$PKG_ID.yaml" <<EOF
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.version.$SCHEMA.schema.json

PackageIdentifier: $PKG_ID
PackageVersion: $VERSION
DefaultLocale: en-US
ManifestType: version
ManifestVersion: $SCHEMA
EOF

echo "▸ 생성 완료: $DEST"
ls -1 "$DEST"
