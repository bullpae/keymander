#!/usr/bin/env bash
# ============================================================
# 텍스트 파일 인코딩 검사 (UTF-8 강제 · BOM 규칙)
# ============================================================
#
# 배경: Windows에서 CP949 혼용 편집 환경 때문에 `Cargo.toml`의 한글이 깨져
# cargo-deb 빌드가 실패한 적이 있다(87955e0에서 복구). 깨진 바이트는 눈으로
# 잘 안 보이고, 릴리스 파이프라인 끝에서야 터진다. 그 사각지대를 CI에서 막는다.
#
# 함께 잡는 것:
#   - UTF-8이 아닌 바이트열 (CP949/EUC-KR 혼입)
#   - UTF-8 BOM — PowerShell의 `Out-File -Encoding utf8`이 붙이는데,
#     TOML 파서와 셰뱅(`#!`)이 이걸 못 견딘다
#
# **예외: `.ps1`은 BOM이 있어야 한다.** Windows PowerShell 5.1은 BOM이 없는
# UTF-8 스크립트를 ANSI 코드페이지로 읽어 한글을 깨뜨린다. 그래서 .ps1은
# UTF-8 유효성만 보고 BOM은 오히려 요구한다.
#
# 사용법:
#   scripts/check-encoding.sh
#
# 종료 코드: 0 = 깨끗, 1 = 문제 발견
set -uo pipefail
cd "$(dirname "$0")/.."

# 검사 대상: 추적 중인 텍스트 파일. 바이너리(png/ico/rpm 등)는 제외한다.
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "git 저장소가 아니거나 git이 접근을 거부했다 — 검사를 수행할 수 없다" >&2
  exit 2
fi

mapfile -t FILES < <(
  git ls-files -- \
    '*.rs' '*.toml' '*.md' '*.sh' '*.ps1' '*.yml' '*.yaml' \
    '*.json' '*.kbd' '*.tsv' '*.txt' '*.plist' \
  | grep -v '^vendor/'
)

# 목록이 비면 "검사할 게 없다"가 아니라 "검사가 실패했다"로 본다.
# 조용한 통과가 가장 위험하다 — 인코딩이 깨져도 CI가 초록으로 지나간다.
if [ "${#FILES[@]}" -eq 0 ]; then
  echo "검사 대상 파일이 0개 — git ls-files가 실패했을 가능성이 높다" >&2
  exit 2
fi

fail=0

for f in "${FILES[@]}"; do
  [ -f "$f" ] || continue

  # 1) UTF-8 유효성 — iconv가 통과 못 하면 깨진 바이트가 있다
  if ! iconv -f UTF-8 -t UTF-8 "$f" >/dev/null 2>&1; then
    echo "❌ UTF-8 아님 (CP949 혼입 의심): $f"
    fail=1
  fi

  # 2) BOM 규칙 — .ps1은 필수, 나머지는 금지
  has_bom=0
  if [ "$(head -c 3 "$f" | od -An -tx1 | tr -d ' \n')" = "efbbbf" ]; then
    has_bom=1
  fi

  case "$f" in
    *.ps1)
      # PS 5.1은 BOM 없는 UTF-8 스크립트를 ANSI로 읽어 한글을 깨뜨린다
      if [ "$has_bom" -eq 0 ] && LC_ALL=C grep -q '[^ -~	]' "$f"; then
        echo "[!] .ps1에 BOM이 없다 (비ASCII 포함): $f"
        echo "    Windows PowerShell 5.1이 ANSI로 읽어 한글이 깨진다. BOM을 유지할 것."
        fail=1
      fi
      ;;
    *)
      if [ "$has_bom" -eq 1 ]; then
        echo "[!] UTF-8 BOM: $f"
        echo "    TOML 파서와 셰뱅(#!)이 BOM을 못 견딘다. BOM 없이 저장할 것."
        fail=1
      fi
      ;;
  esac
done

if [ "$fail" -eq 0 ]; then
  echo "✅ 인코딩 검사 통과 (${#FILES[@]}개 파일)"
else
  echo
  echo "인코딩 문제가 발견됐다. 위 안내대로 다시 저장할 것."
fi

exit "$fail"
