#!/usr/bin/env python3
"""kmd dojo 화면 와이어프레임 생성기 (docs/10_dojo_plan.md §5).

한 벌의 화면 정의에서 두 가지를 만든다:

  1. 문서용 ASCII — docs/10_dojo_plan.md 의 SCREENS 마커 사이를 교체한다.
  2. 색 미리보기 HTML — 터미널 색·반전까지 재현한 검토용 페이지.

한글(동아시아 Wide/Fullwidth)은 2칸으로 세어 테두리를 맞춘다. 모호폭 문자
(─ │ ★ ■ 등)는 대부분의 터미널 기본값처럼 1칸이다.

사용법:
    python3 scripts/gen_dojo_screens.py                 # 문서 갱신
    python3 scripts/gen_dojo_screens.py --html out.html # + 미리보기 생성
"""

import argparse
import html
import re
import sys
import unicodedata
from pathlib import Path

W = 80  # 화면 폭 — dojo 최소 터미널 크기 80×24 기준

# ── 세그먼트 모델 ─────────────────────────────────────────────────────────────
# 한 줄 = [(텍스트, 스타일클래스)] 리스트. 마크업 "[[cls|텍스트]]" 로 색을 입힌다.

MARK = re.compile(r"\[\[([\w-]+)\|(.*?)\]\]")


def cw(ch):
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def sw(s):
    return sum(cw(c) for c in s)


def L(s="", base=""):
    """마크업 문자열 → 세그먼트 줄"""
    out, pos = [], 0
    for m in MARK.finditer(s):
        if m.start() > pos:
            out.append((s[pos:m.start()], base))
        out.append((m.group(2), m.group(1)))
        pos = m.end()
    if pos < len(s):
        out.append((s[pos:], base))
    return out


def width(line):
    return sum(sw(t) for t, _ in line)


def pad(line, n, cls=""):
    """표시폭 n 까지 공백으로 채운다 (넘치면 오류 — 와이어프레임은 정확해야 한다)"""
    w = width(line)
    if w > n:
        text = "".join(t for t, _ in line)
        raise ValueError(f"줄이 {w}칸으로 {n}칸을 넘침: {text!r}")
    return line + [(" " * (n - w), cls)] if w < n else line


def center(line, n):
    w = width(line)
    left = (n - w) // 2
    return pad([(" " * left, "")] + line, n)


def box(lines, n, title=None, cls="bd", rounded=False, fill=""):
    """폭 n(테두리 포함) 상자. lines 는 마크업 문자열 또는 세그먼트 줄."""
    tl, tr, bl, br = ("╭", "╮", "╰", "╯") if rounded else ("┌", "┐", "└", "┘")
    inner = n - 2
    if title:
        t = L(title) if isinstance(title, str) else title
        top = [(tl + "─ ", cls)] + t + [(" ", cls)]
        top = top + [("─" * (n - 1 - width(top)) + tr, cls)]
    else:
        top = [(tl + "─" * inner + tr, cls)]
    body = []
    for ln in lines:
        seg = L(ln) if isinstance(ln, str) else ln
        body.append([("│", cls)] + pad(seg, inner, fill) + [("│", cls)])
    return [top] + body + [[(bl + "─" * inner + br, cls)]]


def hcat(*blocks, gap=1):
    """블록(줄 리스트)들을 가로로 잇는다"""
    h = max(len(b) for b in blocks)
    widths = [max((width(l) for l in b), default=0) for b in blocks]
    out = []
    for i in range(h):
        row = []
        for j, (b, bw) in enumerate(zip(blocks, widths)):
            seg = b[i] if i < len(b) else []
            row += pad(seg, bw)
            if j < len(blocks) - 1:
                row.append((" " * gap, ""))
        out.append(row)
    return out


def frame(title, right, body, hint, height=24):
    """dojo 공통 프레임 — 머리줄 / 본문 / 키 안내줄, 80×height"""
    head = L(title)
    r = L(right)
    gapw = W - 2 - width(head) - width(r)
    header = [(" ", "hd")] + [(t, c or "hd") for t, c in head] + [(" " * gapw, "hd")] + \
             [(t, c or "hd") for t, c in r] + [(" ", "hd")]
    rows = [header]
    for ln in body:
        seg = L(ln) if isinstance(ln, str) else ln
        rows.append(pad(seg, W))
    while len(rows) < height - 1:
        rows.append(pad([], W))
    if len(rows) > height - 1:
        raise ValueError(f"{title}: 본문이 {len(rows) - 1}줄로 넘침")
    ft = L(hint)
    rows.append([(" ", "ft")] + [(t, c or "ft") for t, c in ft] +
                [(" " * (W - 1 - width(ft)), "ft")])
    return rows


# ── 키보드 다이어그램 ─────────────────────────────────────────────────────────
# 쥐는 손 = hold(파랑), 움직이는 손 = act(주황). 나머지 키는 흐리게.

ROWS = [
    (0, "Q W E R T Y U I O P [ ]"),
    (1, "A S D F G H J K L ; '"),
    (2, "Z X C V B N M , . /"),
]


def keyboard(hold=(), act=(), pressed=(), labels=None, caps=True, space=None, indent=2):
    """[X] = 강조 키. pressed 는 '눌린 순간' 표시(채움)."""
    labels = labels or {}
    out = []
    for i, (off, keys) in enumerate(ROWS):
        line = [(" " * (indent + off * 2), "")]
        if i == 1:
            cap_cls = "hold-on" if "Caps" in pressed else ("hold" if "Caps" in hold else "dim")
            line.append(("[Caps]" if "Caps" in hold else " Caps ", cap_cls))
            line.append((" ", ""))
        elif i == 2:
            sh_cls = "act-on" if "LShift" in pressed else ("act" if "LShift" in act else "dim")
            line.append(("[Shft]" if "LShift" in act else " Shft ", sh_cls))
            line.append((" ", ""))
        else:
            line.append(("      ", ""))
            line.append((" ", ""))
        for k in keys.split():
            if k in pressed:
                cls = "hold-on" if k in hold else "act-on"
                line.append((f"[{k}]", cls))
            elif k in hold:
                line.append((f"[{k}]", "hold"))
            elif k in act:
                line.append((f"[{k}]", "act"))
            else:
                line.append((f" {k} ", "dim"))
            line.append((" ", ""))
        out.append(line)
    if space:
        cls, text = space
        out.append([(" " * (indent + 16), ""), (text, cls)])
    return out


# ── 화면 정의 ─────────────────────────────────────────────────────────────────

SCREENS = []


def screen(sid, name, note):
    def deco(fn):
        SCREENS.append((sid, name, note, fn))
        return fn
    return deco


BRAND = "[[brand|道場]] [[hd-b|kmd dojo]]"
PROFILE = "[[belt-g|■]] 초록띠 · 연속 3일 · 비급 12/24"


@screen("S1", "도장 입구 (로비)",
        "첫 화면. 메뉴 조작 자체가 수련이다 — ↑↓(=CapsLock+K/J)로 움직인다. "
        "맨 j/k로 움직이면 사부가 한 번 넌지시 레이어 사용을 권한다.")
def s_lobby():
    menu = box([
        "",
        " [[sel|▶ 수련          띠 지도 ]]",
        "   오늘의 수련   [[yel|● 2분]]",
        "   비급 도감     12/24",
        "   수련 기록",
        "   입문 의식     키 감응 보정",
        "   하산          종료",
        "",
    ], 34, title="[[acc|문(門)]]", rounded=True)
    prof = box([
        "",
        " 수련생  [[txt-b|atom]]",
        " 띠      [[belt-g|■■■ 초록띠]]",
        "         [[grn|━━━━━━━━━━]][[dim|━━━━]] 다음 띠까지 2과",
        " 연속    [[pch|▮▮▮]][[dim|▯▯▯▯]] 3일 (검은띠 조건 3일)",
        " 골프    축지 코스 최고 [[grn|-2]]",
        " 인장    [[gold|◆]] macOS  [[dim|◇ Windows  ◇ Linux]]",
        "",
    ], 43, title="[[acc|수련생]]", rounded=True)
    body = [
        "",
        center(L("[[dim|━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━]]"), W),
        center(L("[[brand-b|道  場]]      [[txt-b|k m d   d o j o]]"), W),
        center(L("[[sub|손이 떠나면 진다 — 홈로우 위의 무공]]"), W),
        center(L("[[dim|━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━]]"), W),
        "",
    ]
    body += hcat([[(" ", "")]], menu, prof)
    body += [
        "",
        L(" [[sensei|사부]]  [[say|\"메뉴도 CapsLock 을 쥐고 J/K 로 움직여라. 수련은 문 앞에서 시작된다.\"]]"),
    ]
    return frame(BRAND, PROFILE, body, "↑↓ 이동 (CapsLock+J/K)   Enter 들어가기   Esc 하산")


@screen("S2", "입문 의식 (사전 점검 + 키 감응 보정)",
        "최초 진입 시 자동, 이후 메뉴에서 재실행. 터미널마다 같은 수(手)가 다른 "
        "키로 도착하므로(예: macOS 단어← = Alt+Left → iTerm2 는 ESC b) 실제 도착한 "
        "키를 듣고 기록한다. 5초 무입력이면 '관측 불가'로 표시하고 그 수는 이 "
        "터미널의 골프 허용 수에서 빠진다.")
def s_calib():
    checks = box([
        " [[grn|✓]] 키맵       데몬 실행 중 · vim-nav · CapsLock 트리거",
        " [[grn|✓]] 마우스     캡처 지원 (보라띠 수련 가능)",
        " [[yel|!]] 입력기     [[yel|한글 모드 감지 — RAlt 를 톡 쳐서 영문으로]]",
        " [[grn|✓]] 크기       120×34 (최소 80×24)",
    ], 78, title="[[acc|사전 점검]]")
    calib = box([
        "",
        "   [[hold|CapsLock]] 을 쥐고  [[act|I]]  를 한 번 눌러라            (단어 ←)",
        "",
        "   H ←    [[grn|✓]]   J ↓    [[grn|✓]]   K ↑    [[grn|✓]]   L →    [[grn|✓]]",
        "   I 단어← [[sel| 듣는 중… ]]  O 단어→ [[dim|·]]   II 줄처음 [[dim|·]]   OO 줄끝 [[dim|·]]",
        "   N PgUp [[dim|·]]   M PgDn [[dim|·]]   . BkSp [[dim|·]]   / Del  [[dim|·]]",
        "",
        "   방금 도착:  [[teal|Alt+b]]   →  [[grn|'단어←' 로 기록]]",
        "",
    ], 78, title="[[acc|키 감응 보정]]  [[sub|4 / 12]]")
    body = [""] + hcat([[(" ", "")]], checks, gap=0) + [""] + hcat([[(" ", "")]], calib, gap=0) + [
        "",
        L(" [[sensei|사부]]  [[say|\"같은 수라도 터미널마다 다른 소리로 도착한다. 듣는 법부터 익혀라.\"]]"),
    ]
    return frame(BRAND + "  [[sub|입문 의식]]", "[[sub|iTerm2 · macOS]]", body,
                 "5초 무입력 = 관측 불가   S 건너뛰기   Esc 중단")


@screen("S3", "수련 지도 (띠 사다리)",
        "왼쪽은 띠 사다리, 오른쪽은 선택한 띠의 과(課) 목록. 과마다 별과 최고 "
        "기록, 맨 아래는 보스. 잠긴 과는 해금 조건을 그 자리에서 보여준다.")
def s_map():
    belts = box([
        " [[belt-k|■]] 검은띠  [[dim|잠김]]",
        " [[belt-br|■]] 갈색띠  [[dim|잠김]]",
        " [[belt-p|■]] 보라띠  [[dim|잠김]]",
        " [[belt-b|■]] 파란띠  [[dim|잠김]]",
        "[[sel|▶]][[belt-g|■]] [[sel|초록띠  수련 중]]",
        " [[belt-o|■]] 주황띠  [[gold|★ 11]]",
        " [[belt-y|■]] 노란띠  [[gold|★ 9]]",
        " [[belt-w|■]] 흰띠    [[gold|★ 11]]",
        "",
        " [[dim|── 단 ──]]",
        " [[dim|◇ 삼국 통일]]",
    ], 20, title="[[acc|띠]]")
    lessons = box([
        " [[say|\"레이어는 에디터가 아니라 모든 앱에서 동작한다\"]]",
        "",
        "[[sel| ▶ 1과  선택의 수     LShift + 이동    ★★★  골프 -1 ]]",
        "   2과  지우는 수     선택 후 . 한 번   [[gold|★★]][[dim|☆]]  골프 E",
        "   3과  실전: 오타 세 마리               [[gold|★]][[dim|☆☆]]  14타",
        "   4과  실전: 문단 손질                  [[dim|☆☆☆]]  —",
        "",
        "   [[red|⚔]] 보스  화살귀(箭鬼) 3형          [[dim|잠김]]",
        "        [[dim|해금: 1~4과 모두 ★ 1개 이상]]",
        "",
        " [[sub|승급: 모든 과 ★1 + 보스 격파 → 파란띠]]",
    ], 57, title="[[belt-g|■]] [[acc|초록띠 · 문서 수술]]")
    body = [""] + hcat([[(" ", "")]], belts, lessons) + [
        "",
        box([
            " [[act|LShift]] 를 함께 쥐면 이동이 [[txt-b|선택]]이 된다. 지우기·복사는 그 다음 한 수.",
        ], 78, title="[[sub|1과 미리보기]]")[0],
    ]
    pv = box([" [[act|LShift]] 를 함께 쥐면 이동이 [[txt-b|선택]]이 된다. 지우기·복사는 그 다음 한 수."],
             78, title="[[sub|1과 미리보기]]")
    body = body[:-1] + hcat([[(" ", "")]], pv, gap=0)
    return frame(BRAND + "  [[sub|수련 지도]]", "[[gold|★ 37 / 78]]", body,
                 "↑↓ 과 선택   ←→ 띠 이동   Enter 입문   Esc 문으로")


@screen("S4", "과 입문 — 사부의 가르침",
        "모든 과는 '왜'에서 시작한다(납득 → 시연 → 반복 → 실전 → 회고). 두 손 "
        "키보드에서 쥐는 손은 파랑, 움직이는 손은 주황 — '한 손으로 쥐어짜는 "
        "배치는 없다'가 눈으로 보인다.")
def s_intro():
    kb = keyboard(hold={"Caps"}, act=set("HJKL"))
    kbbox = box([""] + kb + [
        "",
        "   [[hold|■ 쥐는 손]]  왼손 새끼          [[act|■ 움직이는 손]]  오른손 검지~약지",
    ], 78, title="[[acc|두 손]]")
    body = [
        "",
        L("  [[belt-y|■]] [[txt-b|노란띠 · 1과   화살표 없는 화살표]]"),
        "",
        box([
            "",
            "  [[say|\"왼손이 쥐고, 오른손이 움직인다.]]",
            "  [[say| 화살표 키까지 오른손을 보내는 순간, 너는 이미 홈로우를 떠난 것이다.\"]]",
            "",
        ], 78, title="[[sensei|사부의 가르침]]", rounded=True),
    ]
    body = [""] + [L("  [[belt-y|■]] [[txt-b|노란띠 · 1과   화살표 없는 화살표]]")] + [""]
    teach = box([
        "",
        "  [[say|\"왼손이 쥐고, 오른손이 움직인다.]]",
        "  [[say| 화살표 키로 오른손을 보내는 순간, 너는 이미 홈로우를 떠난 것이다.\"]]",
        "",
    ], 78, title="[[sensei|사부의 가르침]]", rounded=True)
    body += hcat([[(" ", "")]], teach, gap=0) + [""] + hcat([[(" ", "")]], kbbox, gap=0) + [
        "",
        L("     [[act|H]] ←      [[act|J]] ↓      [[act|K]] ↑      [[act|L]] →"
          "        [[dim|(Vim 의 그 자리, 모든 앱에서)]]"),
    ]
    return frame(BRAND + "  [[sub|입문]]", "[[sub|비급 卷一 · 두 손의 법칙]]", body,
                 "Enter 시연 보기   Space 바로 수련   Esc 지도로")


@screen("S5", "유령 시연",
        "사부의 손(유령 키보드)이 수를 보여준다. 눌린 키는 채워지고, 아래 미니 "
        "버퍼에서 결과가 같이 움직인다. 시연은 2~4수로 짧게, 언제든 Space 로 넘긴다.")
def s_ghost():
    kb = keyboard(hold={"Caps"}, act=set("HJKL"), pressed={"Caps", "L"})
    kbbox = box([""] + kb + [""], 78, title="[[acc|사부의 손]]  [[sub|시연 2 / 4]]")
    buf = box([
        "",
        "   [[dim| 1]]  let index = build_[[cur|i]]ndex(&config);",
        "   [[dim|  ]]  [[dim|                  ⟶ ⟶ ⟶]]",
        "",
    ], 78, title="[[acc|결과]]")
    body = [""] + hcat([[(" ", "")]], kbbox, gap=0) + [
        "",
        L("     [[hold|CapsLock]] 쥔 채   [[act-on|L]] [[act-on|L]] [[act-on|L]]"
          "      커서가 오른쪽으로 세 칸"),
        "",
    ] + hcat([[(" ", "")]], buf, gap=0) + [
        "",
        L(" [[sensei|사부]]  [[say|\"보았느냐. 오른손은 제자리에서 손가락만 움직였다.\"]]"),
    ]
    return frame(BRAND + "  [[sub|시연]]", "[[belt-y|■]] 노란띠 · 1과", body,
                 "Space 다음 수   R 처음부터   Enter 수련 시작")


@screen("S6", "키 골프 — 홀",
        "핵심 화면. 커서(반전)를 깃발 ⚑ 까지 최소 타수로 옮긴다. Par 는 사부의 수"
        "(BFS 최적해)에 여유를 더한 값. 쓴 수는 아래에 기호로 쌓이고, 허용된 수 "
        "목록은 이 터미널의 보정 결과에 따라 달라진다.")
def s_golf():
    buf = box([
        "",
        "  [[dim| 1]]  [[cur|f]]n main() -> Result<()> {",
        "  [[dim| 2]]      let config = load_config()?;",
        "  [[dim| 3]]      let index = [[flag|b]]uild_index(&config);      [[grn|⚑]]",
        "  [[dim| 4]]      run(index)",
        "  [[dim| 5]]  }",
        "",
    ], 78, title="[[acc|buffer.rs]]")
    body = [
        L("  [[belt-o|■]] [[txt-b|축지 코스 · 3번 홀]]"
          "              [[sub|Par]] [[txt-b|4]]     [[sub|타수]] [[yel-b|2]]     [[sub|사부의 수]] [[dim|?]]"),
    ] + hcat([[(" ", "")]], buf, gap=0) + [
        L("  목표  [[grn|⚑]] 3행 [[flag|b]]uild 의 b 로"),
        L("  쓴 수 [[act|↓]] [[act|↓]] [[dim|·]] [[dim|·]]                              [[sub|남은 Par]] 2"),
        "",
        box([
            " [[act|H J K L]] 한 칸   [[act|I O]] 단어   [[act|II OO]] 줄 처음/끝   [[act|N M]] 페이지",
            " [[dim|이 터미널에서 관측 불가: 없음]]",
        ], 78, title="[[sub|허용된 수]]")[0],
    ]
    allowed = box([
        " [[act|H J K L]] 한 칸   [[act|I O]] 단어   [[act|II OO]] 줄 처음/끝   [[act|N M]] 페이지",
        " [[dim|이 터미널에서 관측 불가: 없음]]",
    ], 78, title="[[sub|허용된 수]]")
    body = body[:-1] + hcat([[(" ", "")]], allowed, gap=0) + [
        "",
        L(" [[sensei|사부]]  [[say|\"L 을 열두 번 두드리는 자는 아직 화살귀의 종이다.\"]]"),
    ]
    return frame(BRAND + "  [[sub|키 골프]]", "[[belt-o|■]] 주황띠 · 2과", body,
                 "Esc 일시정지   R 다시 치기   ? 힌트 (별 -1)")


@screen("S7", "키 골프 — 홀 결과 & 스코어카드",
        "홀 아웃 순간의 오버레이. 내 수와 사부의 수를 나란히 리플레이해 '더 적은 "
        "수'를 눈으로 비교시킨다. 코스(9홀) 스코어카드는 실제 골프 기호를 쓴다.")
def s_golf_result():
    card = box([
        "",
        center(L("[[gold-b|B I R D I E]]"), 44),
        center(L("[[txt-b|버디  -1]]   [[grn|◆ 사부와 같은 수]]"), 44),
        "",
        "   내 수      [[act|↓]] [[act|↓]] [[act|O]]           [[txt-b|3타]]",
        "   사부의 수  [[act|↓]] [[act|↓]] [[act|O]]           [[txt-b|3타]]",
        "   Par                            4타",
        "",
        "   [[sub|Space 리플레이 · Enter 다음 홀]]",
        "",
    ], 46, title="[[acc|3번 홀]]", rounded=True)
    sc = box([
        " 홀     1   2   3   4   5   6   7   8   9 │  합",
        " Par    3   4   4   5   4   3   6   4   5 │  38",
        " 타수   [[grn|2]]   4   [[grn|3]]   [[dim|·   ·   ·   ·   ·   ·]] │",
        " 결과   [[grn|◉]]   [[sub|─]]   [[grn|◉]]" + " " * 24 + " │ [[grn|-2]]",
    ], 52, title="[[acc|축지 코스]]")
    legend = box([
        " [[gold|◎]] 이글 -2",
        " [[grn|◉]] 버디 -1",
        " [[sub|─]] 파    E",
        " [[yel|▲]] 보기 +1",
        " [[red|▲▲]] 더블 +2",
    ], 18, title="[[sub|기호]]")
    body = [""] + [center(card[i], W) for i in range(len(card))] + [""] + \
        hcat([[(" ", "")]], sc, legend, gap=1)
    return frame(BRAND + "  [[sub|키 골프]]", "[[belt-o|■]] 주황띠 · 2과", body,
                 "Enter 다음 홀   Space 리플레이   Esc 코스 중단")


@screen("S8", "대련 — 반응 라운드",
        "제시된 방향을 결과 키로 응답한다. 반응시간 막대와 콤보, 최근 기록 줄. "
        "틀리면 화면이 한 칸 흔들리고 콤보가 끊긴다. 보스전의 기본 공격 단위이기도 하다.")
def s_spar():
    target = box([
        "",
        center(L("[[yel-b|↑]]"), 18),
        "",
    ], 20, cls="yel", rounded=True)
    body = [
        L("  [[belt-y|■]] [[txt-b|대련 · 화살귀 1형]]"
          "                        [[sub|콤보]] [[pch-b|×12]]      [[sub|남은 시간]] 0:23"),
        "",
        "",
    ] + [center(t, W) for t in target] + [
        center(L("[[hold|CapsLock]] + [[act|K]]"), W),
        "",
        center(L("[[grn|████████████████]][[dim|░░░░░░░░]]  1.2초"), W),
        "",
        "",
        L("     최근   [[grn|✓ 0.41]]  [[grn|✓ 0.38]]  [[red|✗ → 를 침]]  [[grn|✓ 0.52]]  [[grn|✓ 0.33]]"),
        "",
        L("     평균 0.42초   최고 콤보 18   정확도 94%"),
        "",
        "",
        L(" [[sensei|사부]]  [[say|\"보지 말고 느껴라. 화살표가 아니라 방향이다.\"]]"),
    ]
    return frame(BRAND + "  [[sub|대련]]", "[[belt-y|■]] 노란띠 · 3과", body,
                 "Esc 일시정지   (Ctrl 계열은 판정에 쓰이므로 종료 키가 아니다)")


@screen("S9", "실전 — 문서 수술",
        "가짜 앱 속 실제 작업. 군더더기 글자(빨강)를 nav 이동과 . / 삭제, "
        "LShift 선택으로 제거한다. 실전도 타수/Par 로 채점해 골프 감각을 이어간다.")
def s_surgery():
    doc = box([
        "",
        "  키맨더는 하나의 키맵으로 세 개의 OS 를 다룬다.",
        "  손은 홈로우를 떠나지 않[[typo|는]]는다. 화살표 키도, 마우스도",
        "  필요 없다. 레이어를 쥐는 손과 조작하는 손은 항상",
        "  다르다 — 한 손으로 쥐어짜는 [[cur|배]]치는 [[typo|없없]]다.",
        "",
    ], 78, title="[[acc|보고서.txt]]")
    body = [
        L("  [[belt-g|■]] [[txt-b|실전 · 오타 세 마리]]"
          "                  [[sub|남은 오타]] [[red|2]] / 3    [[sub|타수]] 14 / [[sub|Par]] 18"),
        "",
    ] + hcat([[(" ", "")]], doc, gap=0) + [
        "",
        L("  [[grn|✓]] 1마리 제거됨   '하나의의' → '하나의'"),
        L("  [[typo|빨강]] 군더더기 글자 — 지워야 한다"),
        "",
        L("  쓴 수  [[act|↓]] [[act|↓]] [[act|O]] [[act|←]] [[act|.]] … [[dim|(14)]]"),
        "",
        L("  [[sub|팁]]  [[act|LShift]]+[[act|O]] 로 단어째 잡고 [[act|.]] 한 번 — 글자 하나씩 지우는 것보다 빠르다"),
        "",
        L(" [[sensei|사부]]  [[say|\"칼은 한 번에 들어가야 한다. 두 글자를 두 번 베지 마라.\"]]"),
    ]
    return frame(BRAND + "  [[sub|실전]]", "[[belt-g|■]] 초록띠 · 3과", body,
                 "Esc 일시정지   R 처음부터   ? 힌트 (별 -1)")


@screen("S10", "보이지 않는 마우스 — 조준",
        "터미널 마우스 캡처로 OS 포인터의 도착 좌표를 판정한다. 먼 타겟은 가속, "
        "작은 타겟은 LShift 정밀 없이는 어렵게 배치. 드래그 과는 같은 화면에서 "
        "아이콘을 폴더로 옮긴다(Space 홀드).")
def s_mouse():
    field = [
        "",
        "                                              ┌──────┐",
        "                                              │  [[grn|◎]]   │   [[sub|큰 과녁 · W]]",
        "                                              └──────┘",
        "                                  [[dim|· · · ·]] [[acc|➤]]",
        "                   [[dim|· · · · · · · ·]]",
        "   ┌─┐",
        "   │[[yel|◎]]│  [[sub|작은 과녁 · LShift 정밀]]",
        "   └─┘",
        "",
    ]
    arena = box(field, 78, title="[[acc|과녁장]]")
    body = [
        L("  [[belt-p|■]] [[txt-b|쥐 없는 조준]]"
          "                                 [[sub|명중]] 7 / 10    [[sub|시간]] 0:18"),
    ] + hcat([[(" ", "")]], arena, gap=0) + [
        L("  [[hold|RAlt]] 쥐고   [[act|E]]↑ [[act|S]]← [[act|D]]↓ [[act|F]]→   "
          "[[act|W]] 좌클릭  [[act|R]] 우클릭  [[act|T]]/[[act|G]] 휠  [[act|LShift]] 정밀"),
        L("  [[sub|새어 나온 글자]] [[grn|0]]  [[dim|(마우스 레이어는 미매핑 키를 막는다 — 마구 쳐도 안전)]]"),
        "",
        L(" [[sensei|사부]]  [[say|\"멀리는 빠르게, 가까이는 느리게. 가속은 네 편이다.\"]]"),
    ]
    return frame(BRAND + "  [[sub|조준]]", "[[belt-p|■]] 보라띠 · 2과", body,
                 "Esc 일시정지   포인터는 과녁장 안에서만 판정")


@screen("S11", "한 손가락 두 얼굴 — 한영 리듬",
        "RAlt 짧은 탭 = 한/영, 길게 쥐면 마우스 레이어. 혼합 문장을 치며 전환 "
        "지점에서 탭한다. 도착 글자의 문자 체계(한글/라틴)로 판정한다.")
def s_ime():
    body = [
        L("  [[belt-b|■]] [[txt-b|한영 리듬]]"
          "                                         [[sub|문장]] 3 / 8   [[sub|정확]] 96%"),
        "",
        "",
        L("     제시   [[txt-b|kmd 로 PDF 를 찾는다]]"),
        L("     입력   [[grn|kmd 로 PD]][[cur|F]]"),
        L("                          [[yel|▲]] 다음 전환: '를' 앞에서 [[hold|RAlt]] 톡"),
        "",
        "",
        L("     지금   [[sel| A  영문 ]]   [[dim| 가 한글 ]]"),
        "",
        L("     전환   [[dim|──]][[teal|A]][[dim|──▶──]][[pch|가]][[dim|──▶──]][[teal|A]][[dim|──▶ ·]]"),
        "",
        "",
        box([
            " [[hold|RAlt]] 짧게 [[txt-b|톡]] = 한/영        [[hold|RAlt]] 길게 [[txt-b|쥠]] = 마우스 레이어",
            " [[dim|300ms 안에 두 번 치면 두 번째는 무시된다 — 전환 겹침을 막는 안전장치]]",
        ], 78, title="[[sub|기억할 것]]")[0],
    ]
    rem = box([
        " [[hold|RAlt]] 짧게 [[txt-b|톡]] = 한/영        [[hold|RAlt]] 길게 [[txt-b|쥠]] = 마우스 레이어",
        " [[dim|300ms 안에 두 번 치면 두 번째는 무시된다 — 전환 겹침을 막는 안전장치]]",
    ], 78, title="[[sub|기억할 것]]")
    body = body[:-1] + hcat([[(" ", "")]], rem, gap=0) + [
        "",
        L(" [[sensei|사부]]  [[say|\"수십 년 쓴 그 자리다. 손가락은 이미 알고 있다.\"]]"),
    ]
    return frame(BRAND + "  [[sub|한영 리듬]]", "[[belt-b|■]] 파란띠 · 1과", body,
                 "Esc 일시정지   한글 모드 경고는 이 과에서만 꺼진다")


@screen("S12", "소환술 — 런처 골프",
        "의뢰를 읽고 최소 입력으로 목표를 1순위에 올려 Enter. 실제 kmd-core 검색 "
        "엔진을 고정 연습 목록 위에서 돌린다. '도장 밖 실전'을 켜면 내 실행 "
        "기록에서 의뢰가 나온다.")
def s_summon():
    req = box([
        "  [[say|\"지난달 받은 견적서 PDF 를 열어 달라.\"]]",
    ], 78, title="[[pch|의뢰 4 / 6]]", rounded=True)
    res = box([
        " [[sel| ▸ quote_2026-09.pdf          File   ~/Downloads       ]][[grn-sel| ✓ 목표 ]]",
        "   quarterly_report.pdf         File   ~/Documents",
        "   QuickTime Player             App    /Applications",
        "",
    ], 78, title="[[acc|❯]] [[txt-b|quo]][[cur| ]]  [[sub|fuzzy · 3 / 214]]", rounded=True)
    body = [
        L("  [[belt-w|■]] [[txt-b|소환술]]"
          "                                  [[sub|Par]] 5    [[sub|타수]] [[yel-b|3]] + Enter"),
        "",
    ] + hcat([[(" ", "")]], req, gap=0) + [""] + hcat([[(" ", "")]], res, gap=0) + [
        "",
        L("  [[sub|사부의 수]]  [[dim|q u o ⏎  (4타)]]     [[sub|접두 비급]]  [[act|.pdf]] 확장자  [[act|@gh]] 웹  [[act|!]] 셸"),
        "",
        L("  [[sub|모드]]  [[sel| 연습 목록 ]]  [[dim| 도장 밖 실전 (내 기계) ]]"),
        "",
        L(" [[sensei|사부]]  [[say|\"찾지 말고 불러라. 세 글자면 충분하다.\"]]"),
    ]
    return frame(BRAND + "  [[sub|소환술]]", "[[belt-w|■]] 흰띠 · 3과", body,
                 "Enter 소환   Tab 모드 전환 (과 시작 전)   Esc 일시정지")


@screen("S13", "보스전 — 화살귀(箭鬼)",
        "띠의 마지막 관문. 그 띠의 모든 수를 섞은 연속 공격(대련 + 미니 홀). "
        "명중은 보스 HP 를 깎고, 실수는 내 기(氣)를 깎는다. 보스 대사는 '이탈'을 유혹한다.")
def s_boss():
    art = [
        "              [[red|  ╱╲    ╱╲  ]]",
        "              [[red| ╱  ╲__╱  ╲ ]]",
        "              [[red|│  ◣    ◢  │]]    [[red-b|화살귀(箭鬼)]]",
        "              [[red|│    ▼▼    │]]    [[say|\"네 오른손은 결국 화살표로 돌아올 것이다!\"]]",
        "              [[red| ╲ ⟵ ⟶ ⟰  ╱ ]]",
    ]
    body = [
        L("  [[red|⚔]] [[txt-b|보스전]]"
          "                         [[sub|보스]] [[red|██████████████]][[dim|░░░░░░]] 70%"),
        L("                                         [[sub|기]]   [[grn|██████████████████]][[dim|░░]] 90%"),
        "",
    ] + art + [
        "",
        L("     [[yel|공격 3 / 10]]   ⚑ 를 향해!   [[sub|Par]] 3"),
        box([
            "  [[dim| 7]]  return [[cur|O]]k(items.into_iter().collect())  [[grn|⚑]]",
        ], 70, title="[[acc|미니 홀]]")[0],
    ]
    mini = box([
        "  [[dim| 7]]  return [[cur|O]]k(items.into_iter().[[flag|c]]ollect())   [[grn|⚑]]",
    ], 70, title="[[acc|미니 홀]]")
    body = body[:-1] + hcat([[("     ", "")]], mini, gap=0) + [
        "",
        L("     최근  [[grn|✓ 명중 -8%]]  [[grn|✓ 명중 -8%]]  [[red|✗ 보기 — 기 -10%]]"),
    ]
    return frame(BRAND + "  [[sub|보스전]]", "[[belt-o|■]] 주황띠 관문", body,
                 "Esc 일시정지 (보스전 중 일시정지는 1회)")


@screen("S14", "승급식",
        "보스 격파 후. 띠가 위에서 내려와 묶이는 짧은 애니메이션(감소 모션 설정 시 "
        "정지 화면), 새로 해금된 비급 카드, 다음 띠 예고.")
def s_promo():
    belt = center(L("[[belt-o|████████████████████████████████████]]"), W)
    card = box([
        "",
        "  [[gold|卷七]]  [[txt-b|완충지대 — 왜 , 는 비어 있는가]]",
        "  [[sub|가장 자주 쓰는 삭제 키 . 의 바로 옆은 비워 둔다.]]",
        "",
    ], 56, title="[[gold|새 비급]]", rounded=True)
    body = [
        "",
        center(L("[[dim|━━━━━━━━━━━━━━━]]   [[gold-b|승  급]]   [[dim|━━━━━━━━━━━━━━━]]"), W),
        "",
        belt,
        center(L("[[txt-b|주황띠를 수여한다]]"), W),
        "",
        center(L("[[say|\"적게 누르는 것이 곧 빠른 것이다.\"]]"), W),
        "",
        center(L("축지 코스 [[grn|-2]]   화살귀 격파 0:48   별 [[gold|11]] / 12"), W),
        "",
    ] + [center(c, W) for c in card] + [
        "",
        center(L("[[sub|다음]]  [[belt-g|■]] 초록띠 · 문서 수술 — LShift 로 선택하는 법"), W),
    ]
    return frame(BRAND, "[[belt-o|■]] 주황띠", body,
                 "Enter 비급 읽기   Space 다음 띠로   C 결과 카드 저장")


@screen("S15", "비급 도감",
        "철학 학습의 회고 장치. 권(卷)마다 배치 근거 한 가지(출처 docs/16 등). "
        "해금은 과 클리어, 숙련 별은 비급 시험 정답 횟수. 미해금은 제목도 ??? .")
def s_codex():
    lst = box([
        " 一 두 손의 법칙     [[gold|★★★]]",
        " 二 굴근은 신근보다  [[gold|★★]]",
        " 三 위 행은 긴 손가락 [[gold|★]]",
        " 四 레이어 불변식    [[gold|★★]]",
        " 五 축지법           [[gold|★]]",
        " 六 P 의 안전지대    [[gold|★]]",
        "[[sel| 七 완충지대        ★★★ ]]",
        " 八 [[dim|???]]",
        " 九 [[dim|???]]",
        " [[dim|…]]",
    ], 27, title="[[acc|권]]")
    det = box([
        "",
        " nav 레이어의 [[act|,]] 는 일부러 비워 두었다.",
        "",
        " 가장 자주 쓰는 삭제 키 [[act|.]] 바로 옆이라,",
        " 손가락이 한 칸 미끄러져도 아무 일도",
        " 일어나지 않게 하는 [[txt-b|완충지대]]다.",
        "",
        " [[sub|레이어에는 오발사해도 무해한 수만]]",
        " [[sub|둔다 — 卷四 레이어 불변식과 이어진다.]]",
        "",
        " [[dim|출처 docs/16 §1-2 · 해금 주황띠 관문]]",
        " [[dim|숙련 ★★★ (시험 3회 정답)]]",
    ], 50, title="[[gold|卷七]] [[txt-b|완충지대]]")
    body = [""] + hcat([[(" ", "")]], lst, det)
    return frame(BRAND + "  [[sub|비급 도감]]", "[[gold|12 / 24 해금]]", body,
                 "↑↓ 권 선택   Enter 비급 시험   Esc 문으로")


@screen("S16", "비급 시험",
        "가끔 과 사이에, 또는 도감에서 직접. 외운 답이 아니라 납득을 묻는다. "
        "정답이면 숙련 별, 오답이면 해당 권을 바로 펼쳐 보여준다.")
def s_quiz():
    q = box([
        "",
        "  nav 레이어에서 [[act|,]] 자리를 비워 둔 까닭은?",
        "",
        "[[sel|  ▶ 1   가장 자주 쓰는 삭제 키 . 옆의 완충지대라서                  ]]",
        "    2   쉼표는 한글 입력에 꼭 필요해서",
        "    3   검지가 이미 과부하라서",
        "    4   아직 기능을 정하지 못해서",
        "",
    ], 78, title="[[gold|卷七]] [[txt-b|비급 시험]]  [[sub|숙련 ★★ → ★★★]]", rounded=True)
    body = [""] * 3 + hcat([[(" ", "")]], q, gap=0) + [
        "",
        "",
        L(" [[sensei|사부]]  [[say|\"외운 자는 잊고, 납득한 자는 쓴다.\"]]"),
    ]
    return frame(BRAND + "  [[sub|비급 시험]]", "", body, "↑↓ 고르기   1~4 바로 답   Enter 제출")


@screen("S17", "결과 카드 (kmd dojo card)",
        "공유용 한 장. `kmd dojo card` 또는 승급식에서 C. 띠·연속일·코스 기록·"
        "OS 인장·가장 많이 쓴 수 히트맵. 터미널 그대로 스크린샷 찍기 좋은 크기(48칸).")
def s_card():
    card = box([
        "",
        "  [[brand|道場]] [[txt-b|atom]]          [[belt-g|■ 초록띠]]",
        "",
        "  연속 수련  [[pch|12일]]       비급  [[gold|18 / 24]]",
        "  축지 코스  [[grn|-4]]         소환술  [[grn|-2]]",
        "  화살귀     [[grn|격파]] 0:41",
        "",
        "  인장  [[gold|◆]] macOS  [[gold|◆]] Windows  [[dim|◇ Linux]]",
        "",
        "  많이 쓴 수",
        "    J  [[act|████████████]]",
        "    L  [[act|████████]]",
        "    O  [[act|█████]]  [[sub|← 단어 점프가 늘었다]]",
        "",
        "  [[dim|keymander · 손이 떠나면 진다]]",
        "",
    ], 48, title="[[brand|kmd dojo]]", rounded=True)
    body = [""] * 2 + [center(c, W) for c in card]
    return frame(BRAND + "  [[sub|결과 카드]]", "", body, "C 클립보드로 복사 (텍스트)   Esc 닫기")


@screen("S18", "일시정지 오버레이",
        "라운드 중 Esc 한 번 = 일시정지, 다시 Esc = 그만두기. Ctrl+C 는 tap-hold "
        "판정에 쓰일 수 있어 종료 키로 쓰지 않는다(raw mode 라 SIGINT 도 아님).")
def s_pause():
    ov = box([
        "",
        center(L("[[txt-b|숨 고르기]]"), 36),
        "",
        "  [[sel| ▶ 계속             Enter ]]",
        "    처음부터          R",
        "    그만두기          Esc",
        "",
        "  [[dim|기록되지 않는다 · 별 변화 없음]]",
        "",
    ], 38, title="[[acc|일시정지]]", rounded=True)
    dimrow = L("[[dim|  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ ]]")
    body = [dimrow] * 5 + [center(o, W) for o in ov] + [dimrow] * 6
    return frame(BRAND + "  [[sub|키 골프]]", "[[belt-o|■]] 주황띠 · 2과", body,
                 "Enter 계속   R 처음부터   Esc 그만두기")


@screen("S19", "사부의 배치 진단",
        "배치 지문이 바뀐 뒤 첫 입장, 또는 메뉴에서. 표준과 네 배치를 수 단위로 "
        "나란히 놓고, 비급 규칙으로 진단한다. 경고도 아무것도 잠그지 않는다 — "
        "안내·시연·Par 는 이미 네 배치로 다시 계산되었다.")
def s_diagnose():
    diff = box([
        " [[sub|수]]          [[sub|도장 표준]]          [[sub|네 배치]]",
        " 레이어      [[hold|CapsLock]] 홀드      [[hold|LAlt]] 홀드        [[yel|! 卷九]]",
        " 단어 ←      [[hold|CapsLock]]+[[act|I]]         [[hold|LAlt]]+[[act|Y]]",
        " 페이지 ↑    [[hold|CapsLock]]+[[act|N]]         [[dim|없음]]             [[sub|→ 주황 3과 제외]]",
        " (수 아님)   [[dim|—]]                  [[hold|LAlt]]+[[act|P]] 붙여넣기  [[red|! 卷四]]",
    ], 78, title="[[acc|바뀐 수]]  [[sub|3 / 21]]")
    diag = box([
        " [[red|!]] [[gold|卷四]] nav [[act|P]] = 붙여넣기 — 잘못 누르면 무해하지 않다",
        " [[yel|i]] [[gold|卷九]] 트리거가 modifier — 앱의 Alt 단축키를 가리는 대신",
        "            HHKB 모드탭(탭 Caps / 홀드 Ctrl)이 살아난다",
        " [[grn|✓]] [[gold|卷一]] 쥐는 손(왼손)과 움직이는 손(오른손)이 다르다",
    ], 78, title="[[sensei|사부의 진단]]")
    body = [
        "",
        L(" [[sensei|사부]]  [[say|\"배치가 바뀌었구나. 도장은 네 손에 맞춰 다시 차린다.\"]]"),
        "",
    ] + hcat([[(" ", "")]], diff, gap=0) + [""] + hcat([[(" ", "")]], diag, gap=0) + [
        "",
        L("  [[sub|안내 문구 · 시연 · Par 는 네 배치로 다시 계산했다.]]"),
        L("  [[sub|예전 기록은 남고 '다른 배치' 표식이 붙는다.]]"),
    ]
    return frame(BRAND + "  [[sub|배치 진단]]", "[[yel|배치가 바뀌었다]]", body,
                 "Enter 입문 의식 다시 (권장)   Space 그대로 수련   ↑↓ 진단 펼치기")


# ── 출력 ─────────────────────────────────────────────────────────────────────

def to_text(rows):
    out = []
    for row in rows:
        s = ""
        for t, cls in row:
            if cls == "cur":
                t = "▮" * len(t) if t.strip() else "▮"
            s += t
        out.append(s.rstrip())
    return "\n".join(out)


def to_html_rows(rows):
    lines = []
    for row in rows:
        parts = []
        for t, cls in row:
            chunk = []
            for ch in t:
                e = html.escape(ch)
                if ord(ch) < 128:
                    chunk.append(e)
                else:
                    chunk.append(f'<i class="w{cw(ch)}">{e}</i>')
            text = "".join(chunk)
            parts.append(f'<span class="{cls}">{text}</span>' if cls else text)
        lines.append("".join(parts))
    return "\n".join(lines)


START = "<!-- SCREENS:BEGIN (scripts/gen_dojo_screens.py 가 생성 — 직접 고치지 말 것) -->"
END = "<!-- SCREENS:END -->"


def markdown():
    md = []
    for sid, name, note, fn in SCREENS:
        md.append(f"#### {sid}. {name}\n\n{note}\n\n```text\n{to_text(fn())}\n```\n")
    return "\n".join(md)


def update_doc(doc):
    text = doc.read_text(encoding="utf-8")
    if START not in text or END not in text:
        sys.exit(f"{doc}: SCREENS 마커가 없다")
    head, rest = text.split(START, 1)
    _, tail = rest.split(END, 1)
    doc.write_text(head + START + "\n\n" + markdown() + "\n" + END + tail, encoding="utf-8")


HTML_HEAD = """<title>kmd dojo 화면</title>
<meta name="claude:viewport-fit-sides" content="cover">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Gowun+Batang:wght@700&family=IBM+Plex+Sans+KR:wght@400;600&family=JetBrains+Mono:wght@400;700&family=Nanum+Gothic+Coding:wght@400;700&display=swap" rel="stylesheet">
<style>
/* 레이아웃: 왼쪽 설명 레일 + 오른쪽 실제 크기 터미널, 좁으면 한 줄로 쌓는다 */
:root{--bg:#eceff4;--fg:#181a23;--muted:#565b6e;--line:#cfd4df;--chip:#e2e6ee;--accent:#0b6fa4;
  --term:#1a1b26;--term-bar:#24253a;--shadow:rgba(20,24,40,.18);
  --f-display:"Gowun Batang",serif;--f-body:"IBM Plex Sans KR",system-ui,sans-serif;
  --f-mono:"JetBrains Mono",ui-monospace,monospace;--f-mono-ko:"Nanum Gothic Coding","JetBrains Mono",monospace}
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){--bg:#0f1017;--fg:#e4e6f0;--muted:#9a9fb4;--line:#262838;--chip:#191a26;--accent:#56d2ff;--term-bar:#1f2031;--shadow:rgba(0,0,0,.5);color-scheme:dark}}
:root[data-theme="dark"]{--bg:#0f1017;--fg:#e4e6f0;--muted:#9a9fb4;--line:#262838;--chip:#191a26;--accent:#56d2ff;--term-bar:#1f2031;--shadow:rgba(0,0,0,.5);color-scheme:dark}
*{box-sizing:border-box}
body{background:var(--bg);color:var(--fg);font-family:var(--f-body);line-height:1.6;font-size:15px;
  padding-left:max(16px,env(safe-area-inset-left,0px));padding-right:max(16px,env(safe-area-inset-right,0px))}
.wrap{max-width:1120px;margin:0 auto}
header.top{padding-block:48px 8px}
header.top h1{font-family:var(--f-display);font-size:clamp(28px,5vw,44px);margin:0;letter-spacing:-.01em;text-wrap:balance}
header.top h1 b{color:var(--accent)}
header.top p{color:var(--muted);max-width:68ch;margin:8px 0 0}
.legend{display:flex;flex-wrap:wrap;gap:8px;margin-top:16px}
.legend span{font-size:13px;padding:2px 10px;border-radius:999px;background:var(--chip);border:1px solid var(--line)}
.legend i{display:inline-block;width:10px;height:10px;border-radius:2px;margin-right:6px}
nav.toc{margin-top:20px;display:flex;flex-wrap:wrap;gap:6px}
nav.toc a{font-size:13px;color:var(--fg);text-decoration:none;padding:3px 10px;border:1px solid var(--line);border-radius:6px}
nav.toc a:hover,nav.toc a:focus-visible{border-color:var(--accent);color:var(--accent);outline:none}
main{padding-block:8px 80px}
section.scr{margin-block:40px;display:grid;grid-template-columns:minmax(0,1fr);gap:12px}
@media (min-width:1060px){section.scr{grid-template-columns:240px minmax(0,1fr);gap:28px}}
section.scr>div{min-width:0}
section.scr h2{font-family:var(--f-display);font-size:20px;margin:0}
section.scr h2 small{display:block;font-family:var(--f-mono);color:var(--accent);font-size:12px;letter-spacing:.08em}
section.scr p{color:var(--muted);font-size:14px;margin:6px 0 0}
.termwrap{overflow-x:auto;border-radius:10px}
.term{background:var(--term);border-radius:10px;width:max-content;box-shadow:0 10px 30px var(--shadow);padding-bottom:6px}
.term .bar{height:26px;display:flex;align-items:center;gap:6px;padding-inline:10px;background:var(--term-bar);border-radius:10px 10px 0 0;margin-bottom:6px}
.term .bar b{width:10px;height:10px;border-radius:50%;background:#3b3d52}
pre{margin:0;padding-inline:10px;font-family:var(--f-mono);font-size:13px;line-height:1.38;color:#dce4ff;white-space:pre}
pre i{font-style:normal;display:inline-block;text-align:center;overflow:visible}
pre i.w1{width:1ch}
pre i.w2{width:2ch;font-family:var(--f-mono-ko)}
.hd{background:#141420;color:#b4bedc}.hd-b{background:#141420;color:#dce4ff;font-weight:700}
.ft{background:#141420;color:#828caa}
.brand{color:#ffa560}.brand-b{color:#ffa560;font-weight:700}
.bd{color:#4a4d68}.dim{color:#5a5f80}.sub{color:#9aa3c7}.txt-b{color:#fff;font-weight:700}
.acc{color:#56d2ff;font-weight:700}.grn{color:#50fa7b}.yel{color:#ffe678}.yel-b{color:#ffe678;font-weight:700}
.pch{color:#ffa560}.pch-b{color:#ffa560;font-weight:700}.red{color:#ff6e8c}.red-b{color:#ff6e8c;font-weight:700}
.teal{color:#50f0d2}.gold{color:#f5c542}.gold-b{color:#f5c542;font-weight:700;letter-spacing:.1em}
.sensei{color:#141420;background:#f5c542;font-weight:700}.say{color:#e8d9a8;font-style:italic}
.sel{background:#444862;color:#fff;font-weight:700}.grn-sel{background:#444862;color:#50fa7b;font-weight:700}
.hold{color:#7aa2ff;font-weight:700}.act{color:#ffa560;font-weight:700}
.hold-on{background:#7aa2ff;color:#141420;font-weight:700}.act-on{background:#ffa560;color:#141420;font-weight:700}
.cur{background:#dce4ff;color:#1a1b26}
.flag{color:#50fa7b;text-decoration:underline;text-underline-offset:3px;font-weight:700}
.typo{color:#ff6e8c;text-decoration:line-through;font-weight:700}
.belt-w{color:#f2f2f2}.belt-y{color:#ffd84a}.belt-o{color:#ff9a3c}.belt-g{color:#4cd964}
.belt-b{color:#4aa3ff}.belt-p{color:#b77dff}.belt-br{color:#b07a4f}.belt-k{color:#8a8a8a}
</style>
<div class="wrap">
<header class="top">
<h1><b>道場</b> kmd dojo 화면 설계</h1>
<p>docs/10_dojo_plan.md §5 의 화면 19장을 터미널 색 그대로 렌더한 검토용 페이지. 모든 화면은 80×24 기준이고, 문서의 ASCII 판과 같은 소스(scripts/gen_dojo_screens.py)에서 생성된다.</p>
<div class="legend">
<span><i style="background:#7aa2ff"></i>쥐는 손</span>
<span><i style="background:#ffa560"></i>움직이는 손</span>
<span><i style="background:#50fa7b"></i>성공 · 깃발</span>
<span><i style="background:#ff6e8c"></i>실패 · 군더더기</span>
<span><i style="background:#f5c542"></i>별 · 비급 · 사부</span>
<span><i style="background:#dce4ff"></i>커서(반전)</span>
</div>
"""


def write_html(path):
    toc = "".join(f'<a href="#{sid}">{sid} {html.escape(name)}</a>' for sid, name, _, _ in SCREENS)
    secs = []
    for sid, name, note, fn in SCREENS:
        secs.append(
            f'<section class="scr" id="{sid}"><div><h2><small>{sid}</small>{html.escape(name)}</h2>'
            f'<p>{html.escape(note)}</p></div>'
            f'<div class="termwrap"><div class="term"><div class="bar"><b></b><b></b><b></b></div>'
            f'<pre>{to_html_rows(fn())}</pre></div></div></section>')
    Path(path).write_text(HTML_HEAD + f'<nav class="toc">{toc}</nav></header><main>' + "".join(secs) +
                          "</main></div>", encoding="utf-8")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--doc", default=str(Path(__file__).resolve().parent.parent / "docs/10_dojo_plan.md"))
    ap.add_argument("--html")
    ap.add_argument("--print", action="store_true", help="문서를 고치지 않고 ASCII 만 출력")
    a = ap.parse_args()
    if a.print:
        print(markdown())
        return
    update_doc(Path(a.doc))
    if a.html:
        write_html(a.html)


if __name__ == "__main__":
    main()
