//! `:say` 명령 — 입력 해석, 미리보기 항목, Enter 확정 (docs/21 §2.2).
//!
//! `:prompt` 와 같은 규칙: 키를 칠 때는 [`items`] 가 **부수효과 없이** 미리보기만 만들고,
//! 저장은 Enter 의 [`commit`] 에서만 한다. 데스크톱과 TUI가 이 둘을 같이 쓴다.
//!
//! ```text
//! :say                      연습 — 오늘 복습할 문장 하나 (빈 입력 Enter = 정답 보기)
//! :say I'm running late     답 — 채점 미리보기, Enter 로 채점
//! :say add                  클립보드의 문장 담기
//! :say add English | 한국어  직접 담기
//! :say import / export      입버릇 일괄 붙여넣기 형식으로 주고받기
//! :say list / stats / next / on / off
//! ```
//!
//! 답이 `add` 같은 낱말로 시작해도 되도록, 하위 명령은 **그 낱말 하나만** 있을 때
//! (`add` 는 `|` 가 있을 때도) 하위 명령으로 본다. 나머지는 모두 답이다.

use crate::index::{IndexItem, ItemKind, Source};

use super::{bulk, scoring, srs, Deck, PracticePin, SayConfig, SayState, SayStore, Sentence};

/// Enter 확정 마커 — 뒤에 `:<문장 id>` 가 붙을 수 있다
pub const SAY_COMMIT_MARKER: &str = "kmd:say:commit";
/// 입력을 바꾸는 안내 항목 — 뒤에 `:<새 입력>`
pub const SAY_SEED_MARKER: &str = "kmd:say:seed:";
/// 아무것도 안 하는 안내 항목
pub const SAY_NOOP_MARKER: &str = "kmd:say:noop";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SayCommand {
    /// 연습 — 답(빈 문자열이면 아직 안 침)
    Practice(String),
    Add {
        en: String,
        ko: String,
    },
    /// `add` 뒤에 `|` 가 있지만 영어나 한국어가 비었다
    AddInvalid,
    AddClipboard,
    Import,
    Export,
    List,
    Stats,
    Next,
    On,
    Off,
}

/// `:say ...` 입력을 해석한다. 별칭이 아니면 빈 연습으로 본다.
pub fn parse(query: &str) -> SayCommand {
    let rest = super::SAY_ALIASES
        .iter()
        .find_map(|a| query.strip_prefix(a))
        .unwrap_or("")
        .trim();
    let (head, tail) = match rest.split_once(char::is_whitespace) {
        Some((h, t)) => (h, t.trim()),
        None => (rest, ""),
    };
    let alone = tail.is_empty();
    match head.to_ascii_lowercase().as_str() {
        "add" if alone => SayCommand::AddClipboard,
        "add" if tail.contains(['|', '\t']) => match bulk::parse_bulk(tail).into_iter().next() {
            Some((en, ko)) => SayCommand::Add { en, ko },
            None => SayCommand::AddInvalid,
        },
        "import" if alone => SayCommand::Import,
        "export" if alone => SayCommand::Export,
        "list" if alone => SayCommand::List,
        "stats" if alone => SayCommand::Stats,
        "next" | "skip" if alone => SayCommand::Next,
        "on" if alone => SayCommand::On,
        "off" if alone => SayCommand::Off,
        _ => SayCommand::Practice(rest.to_string()),
    }
}

/// 미리보기·확정에 필요한 바깥 값
#[derive(Debug, Clone, Copy)]
pub struct Ctx<'a> {
    pub cfg: &'a SayConfig,
    pub now: i64,
    /// 이번 런처 창에서 정답을 연 문장 id (빈 입력 Enter)
    pub revealed: Option<&'a str>,
    pub use_emoji: bool,
}

/// Enter 확정 결과 — UI가 적용할 일
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    /// 상태 줄에 보일 문구
    pub message: String,
    /// 입력을 이것으로 바꾸고 다시 검색
    pub next_query: Option<String>,
    /// 클립보드에 넣을 텍스트 (export)
    pub copy: Option<String>,
    /// 설정 `say.enabled` 를 이 값으로 저장
    pub set_enabled: Option<bool>,
    /// 이 문장의 정답을 연다
    pub reveal: Option<String>,
}

/// 지금 연습할 문장 — 붙잡아 둔 문장이 있으면 그것, 없으면 복습 차례인 것
pub fn practice_target<'a>(deck: &'a Deck, state: &SayState, now: i64) -> Option<&'a Sentence> {
    state
        .practice
        .as_ref()
        .and_then(|p| deck.get(&p.id))
        .or_else(|| srs::pick_for_time(&deck.sentences, now, &state.recent))
}

fn practice_failed(state: &SayState, id: &str) -> bool {
    state
        .practice
        .as_ref()
        .is_some_and(|p| p.id == id && p.failed)
}

/// 미리보기 항목 — 파일은 읽기만 한다
pub fn items(query: &str, store: &SayStore, ctx: Ctx<'_>) -> Vec<IndexItem> {
    let ue = ctx.use_emoji;
    let cmd = parse(query);
    if !ctx.cfg.enabled && cmd != SayCommand::Off {
        return vec![
            item(
                ic(ue, "\u{1F4AC}", "[EN]"),
                "입버릇 켜기 — 런처를 열 때마다 영어 한 줄".to_string(),
                "Enter: 켜기 · 기본 문장 40개로 시작합니다 · 끄기는 :say off".to_string(),
                commit_marker(None),
            ),
            notice(
                ue,
                "컴퓨터는 영어를 자주 보여 주고, 폰(입버릇)은 입으로 굳힙니다",
                "빈 입력일 때 결과 아래에 문장이 보이고, :say 로 타이핑 연습을 합니다 (docs/21)",
            ),
        ];
    }
    let deck = match store.load_deck(ctx.now) {
        Ok(d) => d,
        Err(e) => return vec![notice(ue, "입버릇 문장 파일을 읽지 못했습니다", &e)],
    };
    let state = store.load_state();
    match cmd {
        SayCommand::Practice(answer) => practice_items(&deck, &state, &answer, ctx),
        SayCommand::Add { en, ko } => {
            if deck.contains_en(&en) {
                vec![notice(ue, "이미 있는 문장입니다", &en)]
            } else {
                vec![item(
                    ic(ue, "\u{2795}", "[+]"),
                    format!("담기: {en}"),
                    format!("{ko} — Enter: 저장"),
                    commit_marker(None),
                )]
            }
        }
        SayCommand::AddInvalid => vec![notice(
            ue,
            "영어와 한국어를 | 로 나눠 적어 주세요",
            "예: :say add I'm on it. | 지금 하고 있어요.",
        )],
        SayCommand::AddClipboard => add_clipboard_items(&deck, ue),
        SayCommand::Import => {
            let pairs = clipboard_pairs();
            if pairs.is_empty() {
                vec![notice(
                    ue,
                    "클립보드에 'English | 한국어' 줄이 없습니다",
                    "엑셀·시트에서 두 열을 복사하거나, 한 줄에 English | 한국어 로 적어 복사하세요",
                )]
            } else {
                let dup = pairs.iter().filter(|(en, _)| deck.contains_en(en)).count();
                vec![item(
                    ic(ue, "\u{1F4E5}", "[IN]"),
                    format!("클립보드의 {}줄 담기", pairs.len()),
                    format!(
                        "이미 있는 문장 {dup}개는 건너뜁니다 · 첫 줄: {} | {}",
                        pairs[0].0, pairs[0].1
                    ),
                    commit_marker(None),
                )]
            }
        }
        SayCommand::Export => vec![item(
            ic(ue, "\u{1F4E4}", "[OUT]"),
            format!("문장 {}개를 클립보드로", deck.sentences.len()),
            "Enter: 복사 → 폰 입버릇 문장 탭 '일괄 붙여넣기'에 그대로 붙이세요".to_string(),
            commit_marker(None),
        )],
        SayCommand::List => list_items(&deck, ctx),
        SayCommand::Stats => stats_items(&deck, ctx),
        SayCommand::Next => vec![item(
            ic(ue, "\u{23ED}\u{FE0F}", "[>>]"),
            "다른 문장으로".to_string(),
            "Enter: 이번 문장은 채점 없이 넘깁니다".to_string(),
            commit_marker(None),
        )],
        SayCommand::On => vec![notice(ue, "이미 켜져 있습니다", "끄려면 :say off")],
        SayCommand::Off => vec![item(
            ic(ue, "\u{1F515}", "[OFF]"),
            "입버릇 끄기".to_string(),
            "Enter: 런처 카드가 사라집니다 (모은 문장은 그대로 남습니다)".to_string(),
            commit_marker(None),
        )],
    }
}

fn practice_items(deck: &Deck, state: &SayState, answer: &str, ctx: Ctx<'_>) -> Vec<IndexItem> {
    let ue = ctx.use_emoji;
    let Some(target) = practice_target(deck, state, ctx.now) else {
        return vec![notice(
            ue,
            "문장이 없습니다",
            "예: :say add I'm on it. | 지금 하고 있어요.  또는 :say import",
        )];
    };
    let failed = practice_failed(state, &target.id);
    let revealed = failed || ctx.revealed == Some(target.id.as_str());
    let hidden = srs::is_recall_stage(target) && !revealed;
    let due = srs::due_sentences(&deck.sentences, ctx.now).len();
    let meta = format!("상자 {} · 복습 대기 {due}개", target.r#box);
    let marker = commit_marker(Some(&target.id));

    let mut out = vec![if hidden {
        item(
            ic(ue, "\u{1F5E3}\u{FE0F}", "[KO]"),
            target.ko.clone(),
            format!("영어로 쳐 보세요 · 빈 입력에서 Enter = 정답 보기 · {meta}"),
            marker.clone(),
        )
    } else {
        let how = if failed {
            "틀린 문장 — 한 번 따라 치면 넘어갑니다"
        } else {
            "그대로 따라 쳐 보세요"
        };
        item(
            ic(ue, "\u{1F501}", "[EN]"),
            target.en.clone(),
            format!("{} — {how} · {meta}", target.ko),
            marker.clone(),
        )
    }];

    if answer.is_empty() {
        for (seed, label) in [
            (":say next", "다른 문장으로"),
            (":say add ", "문장 담기 (클립보드 또는 English | 한국어)"),
            (":say list", "모은 문장 보기"),
            (":say export", "폰 입버릇으로 보내기"),
        ] {
            out.push(item(
                ic(ue, "\u{2192}", "->"),
                seed.trim().to_string(),
                label.to_string(),
                format!("{SAY_SEED_MARKER}{seed}"),
            ));
        }
        return out;
    }

    let r = scoring::score_utterance(&target.en, answer);
    let pct = (r.score * 100.0).round() as u32;
    let pass = r.score >= ctx.cfg.pass_score;
    let mark = if pass {
        ic(ue, "\u{2705}", "[OK]")
    } else {
        ic(ue, "\u{274C}", "[X]")
    };
    // 떠올리기 단계에서 정답 표시를 하면 답을 알려 주는 셈이다 — 맞은 단어 수만 보인다
    let name = if hidden {
        let ok = r.marks.iter().filter(|m| m.ok).count();
        format!("{pct}%  ·  단어 {ok}/{} 맞음", r.marks.len())
    } else {
        format!("{pct}%  {}", scoring::marked_line(&r.marks))
    };
    let path = if failed {
        "Enter: 따라 치기 확인 ([ ] 는 빠진 단어)".to_string()
    } else {
        format!(
            "Enter: 채점 — 통과 기준 {}%",
            (ctx.cfg.pass_score * 100.0).round()
        )
    };
    out.push(item(mark, name, path, marker));
    out
}

fn add_clipboard_items(deck: &Deck, ue: bool) -> Vec<IndexItem> {
    let clip = crate::clipboard::get_text().unwrap_or_default();
    let pairs = bulk::parse_bulk(&clip);
    if !pairs.is_empty() {
        let new = pairs.iter().filter(|(en, _)| !deck.contains_en(en)).count();
        return vec![item(
            ic(ue, "\u{2795}", "[+]"),
            format!("클립보드에서 {new}개 담기"),
            format!("첫 줄: {} | {}", pairs[0].0, pairs[0].1),
            commit_marker(None),
        )];
    }
    match single_english_line(&clip) {
        Some(en) if deck.contains_en(&en) => vec![notice(ue, "이미 있는 문장입니다", &en)],
        Some(en) => vec![item(
            ic(ue, "\u{2795}", "[+]"),
            format!("클립보드: {}", short(&en, 70)),
            "Enter: 한국어 뜻을 적으러 갑니다".to_string(),
            commit_marker(None),
        )],
        None => vec![notice(
            ue,
            "클립보드에 담을 영어 문장이 없습니다",
            "영어 문장을 복사한 뒤 :say add, 또는 :say add English | 한국어",
        )],
    }
}

fn list_items(deck: &Deck, ctx: Ctx<'_>) -> Vec<IndexItem> {
    let ue = ctx.use_emoji;
    if deck.sentences.is_empty() {
        return vec![notice(
            ue,
            "문장이 없습니다",
            "예: :say add I'm on it. | 지금 하고 있어요.",
        )];
    }
    let mut sorted: Vec<&Sentence> = deck.sentences.iter().collect();
    sorted.sort_by_key(|s| (s.due_at, s.r#box));
    sorted
        .into_iter()
        .map(|s| {
            item(
                box_icon(s.r#box, ue),
                s.en.clone(),
                format!(
                    "{} · 상자 {} · {} · 본 {} · 친 {} · 말함 {}",
                    s.ko,
                    s.r#box,
                    due_text(s.due_at, ctx.now),
                    s.seen_count,
                    s.typed_count,
                    s.spoken_count
                ),
                commit_marker(Some(&s.id)),
            )
        })
        .collect()
}

fn stats_items(deck: &Deck, ctx: Ctx<'_>) -> Vec<IndexItem> {
    let ue = ctx.use_emoji;
    let s = &deck.sentences;
    let mut boxes = [0usize; 8];
    for x in s {
        boxes[usize::from(x.r#box.min(srs::MAX_BOX))] += 1;
    }
    let sum = |f: fn(&Sentence) -> u32| s.iter().map(f).sum::<u32>();
    let recall = s.iter().filter(|x| srs::is_recall_stage(x)).count();
    let lines = [
        (
            format!(
                "문장 {}개 · 복습 대기 {}개",
                s.len(),
                srs::due_sentences(s, ctx.now).len()
            ),
            "복습 대기 = 지금 복습할 때가 된 문장".to_string(),
        ),
        (
            format!("떠올리기 단계 {recall}개"),
            "상자 2 이상 — 한국어만 보고 영어를 떠올리는 문장".to_string(),
        ),
        (
            format!(
                "상자별  0:{} 1:{} 2:{} 3:{} 4:{} 5:{} 6:{} 7:{}",
                boxes[0], boxes[1], boxes[2], boxes[3], boxes[4], boxes[5], boxes[6], boxes[7]
            ),
            "상자는 폰에서 말해 통과해야 올라갑니다".to_string(),
        ),
        (
            format!(
                "컴퓨터에서 본 횟수 {} · 타이핑 통과 {} · 폰에서 말함 {}",
                sum(|x| x.seen_count),
                sum(|x| x.typed_count),
                sum(|x| x.spoken_count)
            ),
            "본 횟수 = 런처 카드에 보인 횟수".to_string(),
        ),
    ];
    lines
        .into_iter()
        .map(|(name, path)| {
            item(
                ic(ue, "\u{1F4CA}", "[#]"),
                name,
                path,
                SAY_NOOP_MARKER.to_string(),
            )
        })
        .collect()
}

/// Enter 확정 — 현재 입력을 다시 해석해 적용하고 저장한다.
///
/// `target` 은 선택한 항목 마커에 실린 문장 id. 미리보기와 Enter 사이에 시각이 흘러
/// 다른 문장이 골라지는 일을 막는다.
pub fn commit(
    query: &str,
    target: Option<&str>,
    store: &SayStore,
    ctx: Ctx<'_>,
) -> Result<Applied, String> {
    let cmd = parse(query);
    if !ctx.cfg.enabled && cmd != SayCommand::Off {
        return Ok(Applied {
            message: "입버릇을 켰습니다 — 런처를 열 때마다 결과 아래에 영어 한 줄이 보입니다"
                .to_string(),
            next_query: Some(":say ".to_string()),
            set_enabled: Some(true),
            ..Applied::default()
        });
    }
    let now = ctx.now;
    let mut deck = store.load_deck(now)?;
    let mut state = store.load_state();
    let back = Some(":say ".to_string());

    let applied = match cmd {
        SayCommand::Practice(answer) => {
            let id = match target {
                Some(id) if deck.get(id).is_some() => id.to_string(),
                _ => practice_target(&deck, &state, now)
                    .map(|s| s.id.clone())
                    .ok_or("문장이 없습니다 — :say add 로 담으세요")?,
            };
            let failed = practice_failed(&state, &id);
            let Some(s) = deck.get_mut(&id) else {
                return Err("문장을 찾지 못했습니다".to_string());
            };
            if answer.is_empty() {
                if srs::is_recall_stage(s) && ctx.revealed != Some(id.as_str()) && !failed {
                    return Ok(Applied {
                        message: format!("정답: {}", s.en),
                        reveal: Some(id),
                        ..Applied::default()
                    });
                }
                return Ok(Applied {
                    message: "영어 문장을 쳐 보세요".to_string(),
                    ..Applied::default()
                });
            }
            let r = scoring::score_utterance(&s.en, &answer);
            let pct = (r.score * 100.0).round() as u32;
            let pass = r.score >= ctx.cfg.pass_score;
            let en = s.en.clone();
            let applied = if failed {
                if pass {
                    state.practice = None;
                    state.push_recent(&id);
                    Applied {
                        message: format!("따라 치기 완료 ({pct}%) — 1시간 뒤 다시 나옵니다"),
                        next_query: back,
                        ..Applied::default()
                    }
                } else {
                    Applied {
                        message: format!("{pct}% — 다시 따라 쳐 보세요: {en}"),
                        reveal: Some(id.clone()),
                        ..Applied::default()
                    }
                }
            } else if pass {
                // 컴퓨터에서 맞혀도 상자는 그대로 — 말해야 올라간다 (docs/21 §4)
                srs::grade_hold(s, now);
                s.typed_count += 1;
                s.updated_at = now;
                state.practice = None;
                state.push_recent(&id);
                Applied {
                    message: format!(
                        "통과 {pct}% — 3시간 뒤 다시 나옵니다 (상자는 폰에서 말해야 올라갑니다)"
                    ),
                    next_query: back,
                    ..Applied::default()
                }
            } else {
                srs::grade_fail(s, now);
                s.updated_at = now;
                state.practice = Some(PracticePin {
                    id: id.clone(),
                    failed: true,
                });
                Applied {
                    message: format!("{pct}% — 정답을 보고 한 번 따라 쳐 보세요: {en}"),
                    next_query: back,
                    reveal: Some(id.clone()),
                    ..Applied::default()
                }
            };
            store.save_deck(&deck)?;
            store.save_state(&state)?;
            return Ok(applied);
        }
        SayCommand::Add { en, ko } => {
            let (added, _) = deck.add_pairs(&[(en.clone(), ko)], now);
            if added == 0 {
                return Err(format!("이미 있는 문장입니다: {en}"));
            }
            store.save_deck(&deck)?;
            Applied {
                message: format!("담았습니다: {en}"),
                next_query: back,
                ..Applied::default()
            }
        }
        SayCommand::AddInvalid => {
            return Err("영어와 한국어를 | 로 나눠 적어 주세요".to_string());
        }
        SayCommand::AddClipboard => {
            let clip = crate::clipboard::get_text().unwrap_or_default();
            let pairs = bulk::parse_bulk(&clip);
            if pairs.is_empty() {
                let en =
                    single_english_line(&clip).ok_or("클립보드에 담을 영어 문장이 없습니다")?;
                if deck.contains_en(&en) {
                    return Err(format!("이미 있는 문장입니다: {en}"));
                }
                // 한국어 뜻은 사람이 적는다 — 서버 없이 번역하지 않는다 (입버릇 원칙)
                return Ok(Applied {
                    message: "한국어 뜻을 | 뒤에 적고 Enter".to_string(),
                    next_query: Some(format!(":say add {en} | ")),
                    ..Applied::default()
                });
            }
            let (added, dup) = deck.add_pairs(&pairs, now);
            store.save_deck(&deck)?;
            Applied {
                message: format!("{added}개 담았습니다 (이미 있던 {dup}개 건너뜀)"),
                next_query: back,
                ..Applied::default()
            }
        }
        SayCommand::Import => {
            let pairs = clipboard_pairs();
            if pairs.is_empty() {
                return Err("클립보드에 'English | 한국어' 줄이 없습니다".to_string());
            }
            let (added, dup) = deck.add_pairs(&pairs, now);
            store.save_deck(&deck)?;
            Applied {
                message: format!("{added}개 담았습니다 (이미 있던 {dup}개 건너뜀)"),
                next_query: back,
                ..Applied::default()
            }
        }
        SayCommand::Export => Applied {
            message: format!(
                "문장 {}개를 복사했습니다 — 폰 입버릇 문장 탭 '일괄 붙여넣기'에 붙이세요",
                deck.sentences.len()
            ),
            copy: Some(bulk::export_bulk(&deck.sentences)),
            ..Applied::default()
        },
        SayCommand::List => {
            let id = target
                .filter(|id| deck.get(id).is_some())
                .ok_or("문장을 고르세요")?;
            state.practice = Some(PracticePin {
                id: id.to_string(),
                failed: false,
            });
            store.save_state(&state)?;
            Applied {
                message: "이 문장으로 연습합니다".to_string(),
                next_query: back,
                ..Applied::default()
            }
        }
        SayCommand::Stats => Applied::default(),
        SayCommand::Next => {
            if let Some(id) = practice_target(&deck, &state, now).map(|s| s.id.clone()) {
                state.push_recent(&id);
            }
            state.practice = None;
            store.save_state(&state)?;
            Applied {
                message: "다른 문장으로 넘어갑니다".to_string(),
                next_query: back,
                ..Applied::default()
            }
        }
        SayCommand::On => Applied {
            message: "이미 켜져 있습니다".to_string(),
            ..Applied::default()
        },
        SayCommand::Off => Applied {
            message: "입버릇을 껐습니다 — 다시 켜려면 :say".to_string(),
            next_query: Some(String::new()),
            set_enabled: Some(false),
            ..Applied::default()
        },
    };
    Ok(applied)
}

/// 확정 마커에서 문장 id 를 꺼낸다 — `kmd:say:commit[:<id>]`
pub fn commit_target(keywords: &str) -> Option<Option<String>> {
    let rest = keywords.strip_prefix(SAY_COMMIT_MARKER)?;
    Some(
        rest.strip_prefix(':')
            .filter(|s| !s.is_empty())
            .map(String::from),
    )
}

fn clipboard_pairs() -> Vec<(String, String)> {
    crate::clipboard::get_text()
        .map(|t| bulk::parse_bulk(&t))
        .unwrap_or_default()
}

/// 클립보드가 영어 한 줄이면 그 줄 — 여러 줄이거나 영어가 아니면 None
fn single_english_line(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.contains('\n') || t.chars().count() > 300 {
        return None;
    }
    let letters = t.chars().filter(|c| c.is_alphabetic()).count();
    let ascii = t.chars().filter(|c| c.is_ascii_alphabetic()).count();
    (letters > 0 && ascii * 10 >= letters * 8).then(|| t.to_string())
}

fn due_text(due_at: i64, now: i64) -> String {
    let mins = (due_at - now) / 60_000;
    match mins {
        m if m <= 0 => "지금 복습".to_string(),
        m if m < 60 => format!("{m}분 뒤"),
        m if m < 48 * 60 => format!("{}시간 뒤", m / 60),
        m => format!("{}일 뒤", m / (24 * 60)),
    }
}

fn box_icon(b: u8, ue: bool) -> String {
    if b >= srs::RECALL_FROM_BOX {
        ic(ue, "\u{1F5E3}\u{FE0F}", "[KO]")
    } else {
        ic(ue, "\u{1F501}", "[EN]")
    }
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

fn ic(use_emoji: bool, emoji: &str, ascii: &str) -> String {
    if use_emoji { emoji } else { ascii }.to_string()
}

fn commit_marker(id: Option<&str>) -> String {
    match id {
        Some(id) => format!("{SAY_COMMIT_MARKER}:{id}"),
        None => SAY_COMMIT_MARKER.to_string(),
    }
}

fn item(icon: String, name: String, path: String, keywords: String) -> IndexItem {
    IndexItem {
        name,
        path,
        kind: ItemKind::SystemCommand,
        source: Source::Plugin,
        icon,
        keywords,
        icon_path: None,
    }
}

fn notice(ue: bool, name: &str, path: &str) -> IndexItem {
    item(
        ic(ue, "\u{2139}\u{FE0F}", "[i]"),
        name.to_string(),
        path.to_string(),
        SAY_NOOP_MARKER.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> SayStore {
        let dir = std::env::temp_dir().join(format!("kmd-saycmd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        SayStore::new(dir)
    }

    fn on() -> SayConfig {
        SayConfig {
            enabled: true,
            ..SayConfig::default()
        }
    }

    fn ctx(cfg: &SayConfig, now: i64) -> Ctx<'_> {
        Ctx {
            cfg,
            now,
            revealed: None,
            use_emoji: true,
        }
    }

    /// 한 문장짜리 묶음 — 상자를 정해 둔다
    fn one(store: &SayStore, b: u8) -> String {
        let mut s = Sentence::new(
            "Could you give me a hand?",
            "좀 도와주실 수 있어요?",
            false,
            0,
        );
        s.r#box = b;
        let id = s.id.clone();
        store.save_deck(&Deck::new(vec![s])).unwrap();
        id
    }

    #[test]
    fn parse_subcommands_only_when_alone() {
        assert_eq!(parse(":say"), SayCommand::Practice(String::new()));
        assert_eq!(parse(":say list"), SayCommand::List);
        assert_eq!(
            parse(":say list the items"),
            SayCommand::Practice("list the items".into())
        );
        assert_eq!(parse(":say add"), SayCommand::AddClipboard);
        assert_eq!(
            parse(":say add me to the meeting"),
            SayCommand::Practice("add me to the meeting".into())
        );
        assert_eq!(
            parse(":say add I'm on it. | 하고 있어요"),
            SayCommand::Add {
                en: "I'm on it.".into(),
                ko: "하고 있어요".into()
            }
        );
        assert_eq!(parse(":say add hi |"), SayCommand::AddInvalid);
        assert_eq!(parse(":입버릇 export"), SayCommand::Export);
        assert_eq!(parse(":say NEXT"), SayCommand::Next);
    }

    #[test]
    fn disabled_offers_enable_and_commit_enables() {
        let store = temp_store("off");
        let cfg = SayConfig::default();
        let items = items(":say", &store, ctx(&cfg, 0));
        assert_eq!(items[0].keywords, SAY_COMMIT_MARKER);
        let a = commit(":say", None, &store, ctx(&cfg, 0)).unwrap();
        assert_eq!(a.set_enabled, Some(true));
        assert!(!store.dir().exists(), "켜기만으로는 파일을 만들지 않는다");
    }

    #[test]
    fn recall_stage_hides_english_until_reveal() {
        let store = temp_store("recall");
        let cfg = on();
        let id = one(&store, 2);
        let rows = items(":say", &store, ctx(&cfg, 0));
        assert_eq!(rows[0].name, "좀 도와주실 수 있어요?");
        assert!(rows.iter().all(|r| !r.name.contains("hand")));
        // 답을 쳐도 단어를 보여 주지 않는다
        let rows = items(":say could you", &store, ctx(&cfg, 0));
        assert!(!rows[1].name.contains("hand"), "{}", rows[1].name);
        // 빈 입력 Enter = 정답 보기
        let a = commit(":say", Some(&id), &store, ctx(&cfg, 0)).unwrap();
        assert_eq!(a.reveal.as_deref(), Some(id.as_str()));
        let revealed = Ctx {
            revealed: Some(&id),
            ..ctx(&cfg, 0)
        };
        assert!(items(":say", &store, revealed)[0].name.contains("hand"));
    }

    #[test]
    fn pass_is_hold_not_box_up() {
        let store = temp_store("pass");
        let cfg = on();
        let id = one(&store, 2);
        let now = 10_000;
        let a = commit(
            ":say could you give me a hand",
            Some(&id),
            &store,
            ctx(&cfg, now),
        )
        .unwrap();
        assert_eq!(a.next_query.as_deref(), Some(":say "));
        let deck = store.load_deck(0).unwrap();
        let s = deck.get(&id).unwrap();
        assert_eq!(s.r#box, 2, "컴퓨터는 상자를 올리지 않는다");
        assert_eq!(s.due_at, now + 3 * 3_600_000);
        assert_eq!((s.typed_count, s.updated_at), (1, now));
        assert_eq!(store.load_state().recent, [id]);
    }

    #[test]
    fn fail_resets_then_copy_typing_clears() {
        let store = temp_store("fail");
        let cfg = on();
        let id = one(&store, 3);
        let a = commit(":say what time is it", Some(&id), &store, ctx(&cfg, 0)).unwrap();
        assert_eq!(a.reveal.as_deref(), Some(id.as_str()));
        let s = store.load_deck(0).unwrap().get(&id).unwrap().clone();
        assert_eq!((s.r#box, s.due_at), (0, 3_600_000));
        // 틀린 문장은 붙잡혀 있고 영어가 보인다
        let rows = items(":say", &store, ctx(&cfg, 5));
        assert!(rows[0].name.contains("hand"));
        // 따라 치기 — 일정은 그대로, 붙잡기만 푼다
        let a = commit(
            ":say could you give me a hand",
            Some(&id),
            &store,
            ctx(&cfg, 5),
        )
        .unwrap();
        assert!(a.message.contains("따라 치기"), "{}", a.message);
        let after = store.load_deck(0).unwrap().get(&id).unwrap().clone();
        assert_eq!((after.due_at, after.typed_count), (3_600_000, 0));
        assert!(store.load_state().practice.is_none());
    }

    #[test]
    fn add_import_export() {
        let store = temp_store("add");
        let cfg = on();
        one(&store, 0);
        commit(
            ":say add I'm on it. | 하고 있어요",
            None,
            &store,
            ctx(&cfg, 0),
        )
        .unwrap();
        assert!(commit(":say add i am on it | x", None, &store, ctx(&cfg, 0)).is_err());
        let a = commit(":say export", None, &store, ctx(&cfg, 0)).unwrap();
        let text = a.copy.unwrap();
        assert_eq!(bulk::parse_bulk(&text).len(), 2);
        assert!(text.contains("I'm on it. | 하고 있어요"));
    }

    #[test]
    fn list_pick_pins_practice_and_next_unpins() {
        let store = temp_store("list");
        let cfg = on();
        let mut deck = Deck::seeded(0);
        let last = deck.sentences.last().unwrap().id.clone();
        deck.sentences.last_mut().unwrap().due_at = i64::MAX;
        store.save_deck(&deck).unwrap();
        commit(":say list", Some(&last), &store, ctx(&cfg, 0)).unwrap();
        assert_eq!(
            practice_target(&store.load_deck(0).unwrap(), &store.load_state(), 0)
                .unwrap()
                .id,
            last
        );
        commit(":say next", None, &store, ctx(&cfg, 0)).unwrap();
        let st = store.load_state();
        assert!(st.practice.is_none());
        assert!(st.recent.contains(&last));
    }

    #[test]
    fn commit_target_parses_marker() {
        assert_eq!(commit_target("kmd:say:commit"), Some(None));
        assert_eq!(
            commit_target("kmd:say:commit:ab-12"),
            Some(Some("ab-12".into()))
        );
        assert_eq!(commit_target("kmd:say:noop"), None);
    }

    #[test]
    fn english_line_detection() {
        assert_eq!(
            single_english_line(" I'm on it. "),
            Some("I'm on it.".into())
        );
        assert_eq!(single_english_line("안녕하세요"), None);
        assert_eq!(single_english_line("a\nb"), None);
    }
}
