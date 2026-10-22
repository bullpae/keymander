//! `kmd dojo` — 무협 도장 키맵 트레이너 (docs/10)

use color_eyre::Result;

pub fn run(lesson: Option<String>) -> Result<()> {
    let config = super::load_config()?;
    // DB 가 없어도 수련은 된다 — 기록만 남지 않는다
    let db = match super::open_db() {
        Ok(db) => Some(db),
        Err(e) => {
            tracing::warn!("dojo: DB 를 열 수 없어 기록 없이 진행한다: {e}");
            None
        }
    };
    crate::tui::dojo::run(&config, db, lesson)
}
