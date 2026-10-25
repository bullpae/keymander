//! `kmd say` — 입버릇 문장 관리 CLI (docs/21)
//!
//! 연습은 런처 `:say` 에서 한다. CLI 는 문장을 넣고 빼고 옮기는 일만 맡는다.

use std::io::Read;

use color_eyre::eyre::{bail, eyre};
use color_eyre::Result;

use kmd_core::say::{self, bulk, srs, SayStore};

pub enum Action {
    List,
    Add {
        en: String,
        ko: String,
    },
    /// 파일 또는 `-`(표준 입력)에서 `English | 한국어` 줄을 읽는다
    Import {
        path: String,
    },
    Export,
    Stats,
    Remove {
        query: String,
    },
    Enable(bool),
}

pub fn run(action: Option<Action>) -> Result<()> {
    let store = SayStore::default_location();
    let now = say::now_ms();

    match action.unwrap_or(Action::List) {
        Action::List => {
            let deck = store.load_deck(now).map_err(|e| eyre!(e))?;
            let mut list: Vec<&say::Sentence> = deck.sentences.iter().collect();
            list.sort_by_key(|s| (s.due_at, s.r#box));
            for s in list {
                let due = if s.due_at <= now { "지금" } else { "" };
                println!("[{}] {:<45} {}  {due}", s.r#box, s.en, s.ko);
            }
            println!(
                "\n문장 {}개 — 파일: {}",
                deck.sentences.len(),
                store.dir().display()
            );
        }
        Action::Add { en, ko } => {
            if en.trim().is_empty() || ko.trim().is_empty() {
                bail!(
                    "영어와 한국어를 모두 적어 주세요: kmd say add \"I'm on it.\" \"하고 있어요\""
                );
            }
            let mut deck = store.load_deck(now).map_err(|e| eyre!(e))?;
            let (added, _) = deck.add_pairs(&[(en.clone(), ko)], now);
            if added == 0 {
                bail!("이미 있는 문장입니다: {en}");
            }
            store.save_deck(&deck).map_err(|e| eyre!(e))?;
            println!("✅ 담았습니다: {en}");
        }
        Action::Import { path } => {
            let text = if path == "-" {
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                std::fs::read_to_string(&path)?
            };
            let pairs = bulk::parse_bulk(&text);
            if pairs.is_empty() {
                bail!("'English | 한국어' 또는 탭으로 나눈 줄이 없습니다");
            }
            let mut deck = store.load_deck(now).map_err(|e| eyre!(e))?;
            let (added, dup) = deck.add_pairs(&pairs, now);
            store.save_deck(&deck).map_err(|e| eyre!(e))?;
            println!("✅ {added}개 담았습니다 (이미 있던 {dup}개 건너뜀)");
        }
        Action::Export => {
            let deck = store.load_deck(now).map_err(|e| eyre!(e))?;
            println!("{}", bulk::export_bulk(&deck.sentences));
        }
        Action::Stats => {
            let deck = store.load_deck(now).map_err(|e| eyre!(e))?;
            let s = &deck.sentences;
            let mut boxes = [0usize; 8];
            for x in s {
                boxes[usize::from(x.r#box.min(srs::MAX_BOX))] += 1;
            }
            println!(
                "문장 {}개 · 복습 대기 {}개 · 떠올리기 단계 {}개",
                s.len(),
                srs::due_sentences(s, now).len(),
                s.iter().filter(|x| srs::is_recall_stage(x)).count()
            );
            println!("상자별 {boxes:?}  (상자는 폰에서 말해야 올라갑니다)");
            println!(
                "컴퓨터에서 본 횟수 {} · 타이핑 통과 {} · 폰에서 말함 {}",
                s.iter().map(|x| x.seen_count).sum::<u32>(),
                s.iter().map(|x| x.typed_count).sum::<u32>(),
                s.iter().map(|x| x.spoken_count).sum::<u32>()
            );
        }
        Action::Remove { query } => {
            let mut deck = store.load_deck(now).map_err(|e| eyre!(e))?;
            let key = kmd_core::say::scoring::normalize_words(&query);
            let before = deck.sentences.len();
            deck.sentences
                .retain(|s| s.id != query && kmd_core::say::scoring::normalize_words(&s.en) != key);
            let removed = before - deck.sentences.len();
            if removed == 0 {
                bail!("문장을 찾지 못했습니다: {query} (영어 문장 그대로 또는 id)");
            }
            store.save_deck(&deck).map_err(|e| eyre!(e))?;
            println!("✅ {removed}개 지웠습니다");
        }
        Action::Enable(on) => {
            let config = super::load_config()?;
            let path = config
                .config_path
                .clone()
                .ok_or_else(|| eyre!("설정 파일 경로를 알 수 없습니다"))?;
            kmd_core::Config::update_and_save(&path, |c| c.say.enabled = on)?;
            if on {
                println!("✅ 입버릇을 켰습니다 — 런처를 열면 결과 아래에 영어 한 줄이 보입니다");
            } else {
                println!("✅ 입버릇을 껐습니다 (문장은 그대로 남습니다)");
            }
        }
    }
    Ok(())
}
