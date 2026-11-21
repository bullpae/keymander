#!/usr/bin/env bash
# ============================================================
# APT / YUM 저장소 트리 생성
# ============================================================
#
# 릴리스 자산으로 흩어져 있는 .deb/.rpm 을 실제 패키지 저장소로 묶는다.
# 자산만 올려두면 `apt install`/`dnf install` 은 되어도 `apt update` 로
# **갱신을 받아올 수는 없다** — 인덱스(Packages/repodata)와 서명이 있어야 한다.
#
# 사용법:
#   build-linux-repos.sh <패키지디렉터리> <출력디렉터리>
#
#   <패키지디렉터리>  *.deb / *.rpm 이 평평하게 들어 있는 디렉터리
#   <출력디렉터리>    apt/ 와 yum/ 트리를 만들 위치 (GitHub Pages 루트)
#
# 필요 도구: dpkg-scanpackages(dpkg-dev), apt-ftparchive(apt-utils),
#            createrepo_c, rpmsign(rpm), gpg
# 서명 키:   GPG_KEY_ID 환경변수 (없으면 기본 비밀키 하나를 자동 선택)

set -euo pipefail

PKG_DIR="${1:?사용법: build-linux-repos.sh <패키지디렉터리> <출력디렉터리>}"
OUT_DIR="${2:?사용법: build-linux-repos.sh <패키지디렉터리> <출력디렉터리>}"

PKG_DIR="$(cd "$PKG_DIR" && pwd)"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

# 저장소 메타데이터 — 배포판 관례상 Suite/Codename 은 stable 하나만 쓴다.
ORIGIN="keymander"
SUITE="stable"
COMPONENT="main"
DEB_ARCH="amd64"
RPM_ARCH="x86_64"

info() { echo "▸ $1"; }

# 서명 키 결정 — 명시 지정이 없으면 키링의 첫 비밀키를 쓴다.
GPG_KEY_ID="${GPG_KEY_ID:-$(gpg --list-secret-keys --with-colons 2>/dev/null |
    awk -F: '/^fpr/{print $10; exit}')}"
[ -n "$GPG_KEY_ID" ] || {
    echo "✗ 서명할 GPG 비밀키가 없습니다 (GPG_KEY_ID 미지정 + 키링 비어 있음)" >&2
    exit 1
}
info "서명 키: $GPG_KEY_ID"

# ── APT 저장소 ───────────────────────────────────────────────────────────────
#
# 레이아웃:
#   apt/pool/main/k/keymander/*.deb
#   apt/dists/stable/main/binary-amd64/Packages{,.gz}
#   apt/dists/stable/{Release,InRelease,Release.gpg}
#
# InRelease(클리어서명)와 Release.gpg(분리서명)를 둘 다 만든다. 최신 apt 는
# InRelease 만 보지만 오래된 클라이언트는 Release.gpg 를 찾는다.

build_apt() {
    local root="$OUT_DIR/apt"
    local pool="$root/pool/$COMPONENT/k/keymander"
    local dist="$root/dists/$SUITE"
    local bindir="$dist/$COMPONENT/binary-$DEB_ARCH"

    rm -rf "$root"
    mkdir -p "$pool" "$bindir"

    local n=0
    shopt -s nullglob
    for deb in "$PKG_DIR"/*.deb; do
        cp "$deb" "$pool/"
        n=$((n + 1))
    done
    shopt -u nullglob
    [ "$n" -gt 0 ] || { echo "✗ .deb 이 하나도 없습니다: $PKG_DIR" >&2; exit 1; }
    info "APT: .deb $n개 배치"

    # Packages 안의 Filename 은 저장소 루트 기준 상대 경로여야 한다.
    #
    # --multiversion 이 없으면 dpkg-scanpackages 는 패키지당 **최고 버전 하나만**
    # 인덱스에 넣는다. pool 에 이전 릴리스를 같이 두는 의미(핀 고정·롤백)가
    # 사라지므로 반드시 켠다 — `apt install keymander=0.11.3-1` 이 가능해진다.
    ( cd "$root" && dpkg-scanpackages --multiversion --arch "$DEB_ARCH" pool/ \
        > "$bindir/Packages" )
    gzip -9kf "$bindir/Packages"
    info "APT: 인덱스 항목 $(grep -c '^Package:' "$bindir/Packages")개"

    ( cd "$root" && apt-ftparchive \
        -o "APT::FTPArchive::Release::Origin=$ORIGIN" \
        -o "APT::FTPArchive::Release::Label=$ORIGIN" \
        -o "APT::FTPArchive::Release::Suite=$SUITE" \
        -o "APT::FTPArchive::Release::Codename=$SUITE" \
        -o "APT::FTPArchive::Release::Architectures=$DEB_ARCH" \
        -o "APT::FTPArchive::Release::Components=$COMPONENT" \
        release "dists/$SUITE" > "$dist/Release" )

    gpg --batch --yes --default-key "$GPG_KEY_ID" \
        --clearsign -o "$dist/InRelease" "$dist/Release"
    gpg --batch --yes --default-key "$GPG_KEY_ID" \
        --armor --detach-sign -o "$dist/Release.gpg" "$dist/Release"

    info "APT: 인덱스 생성 + 서명 완료"
}

# ── YUM 저장소 ───────────────────────────────────────────────────────────────
#
# 레이아웃:
#   yum/x86_64/*.rpm
#   yum/x86_64/repodata/{repomd.xml,repomd.xml.asc,...}
#
# 패키지 자체(rpmsign)와 repomd.xml 둘 다 서명한다. 각각 .repo 의
# gpgcheck=1 / repo_gpgcheck=1 에 대응한다.

build_yum() {
    local root="$OUT_DIR/yum/$RPM_ARCH"

    rm -rf "$OUT_DIR/yum"
    mkdir -p "$root"

    local n=0
    shopt -s nullglob
    for rpm in "$PKG_DIR"/*.rpm; do
        cp "$rpm" "$root/"
        n=$((n + 1))
    done
    shopt -u nullglob
    [ "$n" -gt 0 ] || { echo "✗ .rpm 이 하나도 없습니다: $PKG_DIR" >&2; exit 1; }
    info "YUM: .rpm $n개 배치"

    # rpmsign 은 %_gpg_name 매크로로 키를 고른다. gpg2 는 --batch 로 비대화 서명.
    # %__gpg_sign_cmd 를 통째로 덮어쓰지 않는다 — 명령 형식이 rpm 버전마다 달라
    # (옛 형식은 실행 파일 뒤에 argv[0] "gpg" 를 한 번 더 쓴다) ubuntu-26.04 의
    # rpm 에서 "gpg gpg --batch …" 가 되어 서명이 죽었다 (v0.21.0). 추가 인자만 넘긴다.
    cat > "$HOME/.rpmmacros" <<EOF
%_gpg_name $GPG_KEY_ID
%_gpg_sign_cmd_extra_args --batch --pinentry-mode loopback
EOF
    rpmsign --addsign "$root"/*.rpm
    info "YUM: 패키지 서명 완료"

    createrepo_c --quiet "$root"
    gpg --batch --yes --default-key "$GPG_KEY_ID" \
        --armor --detach-sign -o "$root/repodata/repomd.xml.asc" "$root/repodata/repomd.xml"

    info "YUM: repodata 생성 + 서명 완료"
}

build_apt
build_yum

info "완료: $OUT_DIR"
