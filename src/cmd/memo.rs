//! `kmd memo` — 메모 CLI (docs/23)
//!
//! 런처 `:m` 과 같은 폴더를 쓴다. 여러 줄 메모(`-` 로 표준 입력)와 폴더 설정은 여기서 한다.
//! `kmd todo`·`kmd timer` 도 여기 있다 — 판단은 런처와 같은 [`NoteSession`] 이 한다.

use std::io::Read;

use color_eyre::eyre::{bail, eyre};
use color_eyre::Result;

use kmd_core::notes::{self, Note, NoteSession, NoteStore};

pub enum Action {
    Add { text: Vec<String> },
    List { query: Vec<String>, limit: usize },
    Show { query: String },
    Edit { query: String },
    Remove { query: String },
    Dir { path: Option<String> },
}

pub fn run(action: Option<Action>) -> Result<()> {
    let config = super::load_config()?;
    let store = NoteStore::from_config(&config.notes);
    let action = action.unwrap_or(Action::List {
        query: Vec::new(),
        limit: 20,
    });
    match action {
        Action::Add { text } => {
            let body = if text.len() == 1 && text[0] == "-" {
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf
            } else {
                text.join(" ")
            };
            let id = store.add_now(&body).map_err(|e| eyre!(e))?;
            println!("저장: {id}");
        }
        Action::List { query, limit } => {
            let all = store.load().map_err(|e| eyre!(e))?;
            let found = notes::search(&all, &query.join(" "));
            if found.is_empty() {
                println!("메모가 없습니다. 추가: kmd memo add <내용>");
            }
            for note in found.iter().take(limit) {
                println!("{}  {}", note.id, note.title());
            }
            if found.len() > limit {
                println!("… {}개 더 (--limit)", found.len() - limit);
            }
        }
        Action::Show { query } => {
            let all = store.load().map_err(|e| eyre!(e))?;
            println!("{}", resolve(&all, &query)?.body);
        }
        Action::Edit { query } => {
            let all = store.load().map_err(|e| eyre!(e))?;
            let path = store
                .file_path(&resolve(&all, &query)?.id)
                .map_err(|e| eyre!(e))?;
            let editor = config
                .general
                .editor
                .clone()
                .or_else(|| std::env::var("EDITOR").ok())
                .or_else(|| std::env::var("VISUAL").ok())
                .unwrap_or_else(|| {
                    if cfg!(target_os = "windows") {
                        "notepad".to_string()
                    } else {
                        "vi".to_string()
                    }
                });
            std::process::Command::new(&editor).arg(path).status()?;
        }
        Action::Remove { query } => {
            let all = store.load().map_err(|e| eyre!(e))?;
            let note = resolve(&all, &query)?;
            store.trash(&note.id).map_err(|e| eyre!(e))?;
            println!(
                "지움: {} ({} 로 옮겼습니다)",
                note.title(),
                store.dir().join(".trash").display()
            );
        }
        Action::Dir { path: None } => {
            println!("{}", store.dir().display());
            if !store.is_configured() {
                println!("이 기기에만 저장됩니다. 공유 폴더로 옮기기: kmd memo dir <폴더>");
            }
        }
        Action::Dir { path: Some(path) } => set_dir(&path, &store)?,
    }
    Ok(())
}

/// id 또는 검색어 → 메모 하나. 여럿이 걸리면 잘못 지우지 않도록 거절한다
fn resolve<'a>(all: &'a [Note], query: &str) -> Result<&'a Note> {
    if let Some(note) = all.iter().find(|n| n.id == query) {
        return Ok(note);
    }
    let found = notes::search(all, query);
    match found.as_slice() {
        [] => bail!("맞는 메모가 없습니다: {query}"),
        [one] => Ok(one),
        many => {
            for note in many.iter().take(10) {
                eprintln!("{}  {}", note.id, note.title());
            }
            bail!("{}개가 걸립니다 — id 로 지정하세요", many.len())
        }
    }
}

/// 폴더를 바꾸고 메모를 옮긴다. `off` 면 이 기기의 기본 폴더로
fn set_dir(path: &str, store: &NoteStore) -> Result<()> {
    let config_path = kmd_core::Config::default_config_dir().join(kmd_core::CONFIG_FILENAME);
    let target = if path.eq_ignore_ascii_case("off") {
        None
    } else {
        Some(kmd_core::keystats::expand_sync_path(path).map_err(|e| eyre!(e))?)
    };
    let dest = match &target {
        Some(p) => p.clone(),
        None => NoteStore::from_config(&Default::default())
            .dir()
            .to_path_buf(),
    };
    let same_dir = dest == store.dir();
    // 설정을 먼저 저장하면 옮기다 실패했을 때 메모가 안 보인다 — 옮긴 뒤에 저장한다
    let (moved, skipped) = if same_dir {
        std::fs::create_dir_all(&dest)?;
        (0, 0)
    } else {
        store.move_all_to(&dest).map_err(|e| eyre!(e))?
    };
    kmd_core::Config::update_and_save(&config_path, move |c| c.notes.dir = target)
        .map_err(|e| eyre!("설정 저장 실패: {e}"))?;
    println!("메모 폴더: {}", dest.display());
    if moved > 0 {
        println!("메모 {moved}개를 옮겼습니다");
    }
    if skipped > 0 {
        println!(
            "같은 이름이 있어 {skipped}개는 {} 에 남겨 뒀습니다",
            store.dir().display()
        );
    }
    Ok(())
}

pub enum TodoAction {
    Add { text: Vec<String> },
    List,
    Done { query: String },
}

/// `kmd todo`
pub fn run_todo(action: Option<TodoAction>) -> Result<()> {
    let config = super::load_config()?;
    let store = NoteStore::from_config(&config.notes);
    let mut session = NoteSession::new(store.clone());
    match action.unwrap_or(TodoAction::List) {
        TodoAction::Add { text } => {
            let applied = session
                .commit(&format!(":td {}", text.join(" ")), None)
                .map_err(|e| eyre!(e))?;
            println!("{}", applied.message);
        }
        TodoAction::List => {
            let rows = session.results(":td", false);
            for item in rows.iter().filter(|r| notes::selected_id(r).is_some()) {
                println!("{} {}  ({})", item.icon, item.name, item.path);
            }
        }
        TodoAction::Done { query } => {
            let all = store.load().map_err(|e| eyre!(e))?;
            let todos: Vec<Note> = all.into_iter().filter(Note::is_todo).collect();
            let id = resolve(&todos, &query)?.id.clone();
            let applied = session.commit(":td", Some(&id)).map_err(|e| eyre!(e))?;
            println!("{}", applied.message);
        }
    }
    Ok(())
}

pub enum TimerAction {
    Start { text: Vec<String> },
    Pomo { text: Vec<String> },
    List,
    Stop { query: String },
}

/// `kmd timer`
pub fn run_timer(action: Option<TimerAction>) -> Result<()> {
    let config = super::load_config()?;
    let store = NoteStore::from_config(&config.notes);
    let mut session = NoteSession::new(store.clone());
    let timers = store.timers();
    match action.unwrap_or(TimerAction::List) {
        TimerAction::Start { text } => {
            let applied = session
                .commit(&format!(":timer {}", text.join(" ")), None)
                .map_err(|e| eyre!(e))?;
            println!("{}", applied.message);
        }
        TimerAction::Pomo { text } => {
            let applied = session
                .commit(&format!(":pomo {}", text.join(" ")), None)
                .map_err(|e| eyre!(e))?;
            println!("{}", applied.message);
        }
        TimerAction::List => {
            let rows = session.results(":timer", false);
            for item in &rows {
                println!("{}  {}", item.name, item.path);
            }
        }
        TimerAction::Stop { query } => {
            let found: Vec<_> = timers
                .list()
                .into_iter()
                .filter(|t| t.id == query || t.label.contains(&query))
                .collect();
            let [timer] = found.as_slice() else {
                bail!("{}개가 걸립니다 — 이름을 더 적으세요", found.len());
            };
            timers.cancel(&timer.id).map_err(|e| eyre!(e))?;
            println!("껐습니다: {}", timer.label);
        }
    }
    Ok(())
}
