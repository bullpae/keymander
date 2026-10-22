//! kmd dojo 수련 기록 저장소 (docs/10 §10).
//!
//! 판정·점수 계산은 TUI 쪽(`src/tui/dojo`)이 하고, 여기는 결과를 남기고 읽는
//! 얇은 API만 둔다. 띠는 저장하지 않는다 — `dojo_progress`에서 매번 계산한다
//! (진실 소스 하나).

use std::collections::BTreeMap;

use crate::db::{Database, DbError};

/// 수련 한 판의 기록
#[derive(Debug, Clone, Default)]
pub struct DojoRun {
    /// 과 id — `white.1`, `white.boss` …
    pub lesson: String,
    /// 모드 — summon | golf | spar | boss …
    pub kind: String,
    pub strokes: Option<u32>,
    pub par: Option<u32>,
    pub score: u32,
    /// 0~3
    pub stars: u8,
    pub max_combo: Option<u32>,
    pub mistakes: u32,
    pub hints: u32,
    pub duration_ms: u64,
    pub os: String,
    pub terminal: Option<String>,
    /// 배치 지문 (docs/10 §6-5)
    pub keymap_fp: Option<String>,
}

/// 과별 최고 기록
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DojoProgress {
    pub best_stars: u8,
    pub best_score: Option<u32>,
    pub best_strokes: Option<u32>,
    pub cleared_at: Option<i64>,
}

/// 비급 해금 상태
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DojoScroll {
    pub unlocked_at: i64,
    /// 비급 시험 정답 횟수 (0~3)
    pub mastery: u8,
}

/// 최근 기록 목록용 요약
#[derive(Debug, Clone)]
pub struct DojoRunSummary {
    pub lesson: String,
    pub stars: u8,
    pub score: u32,
    pub strokes: Option<u32>,
    pub par: Option<u32>,
    pub created_at: i64,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Database {
    /// 기록을 남기고 과별 최고 기록을 갱신한다. 별·점수·타수 중 하나라도
    /// 좋아졌으면 `true` (결과 화면의 "최고 기록" 표시용).
    ///
    /// 별 0개(완주 실패)도 기록은 남기되 최고 기록은 건드리지 않는다.
    pub fn dojo_record_run(&self, run: &DojoRun) -> Result<bool, DbError> {
        let now = now_secs();
        let tx = self.conn().unchecked_transaction()?;
        tx.execute(
            "INSERT INTO dojo_runs(lesson, kind, strokes, par, score, stars, max_combo,
                                   mistakes, hints, duration_ms, os, terminal, keymap_fp, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                run.lesson,
                run.kind,
                run.strokes,
                run.par,
                run.score,
                run.stars,
                run.max_combo,
                run.mistakes,
                run.hints,
                run.duration_ms as i64,
                run.os,
                run.terminal,
                run.keymap_fp,
                now,
            ],
        )?;

        let mut improved = false;
        if run.stars > 0 {
            let prev = progress_row(&tx, &run.lesson)?;
            let next = match &prev {
                None => {
                    improved = true;
                    DojoProgress {
                        best_stars: run.stars,
                        best_score: Some(run.score),
                        best_strokes: run.strokes,
                        cleared_at: Some(now),
                    }
                }
                Some(p) => {
                    let better_strokes = match (run.strokes, p.best_strokes) {
                        (Some(s), Some(b)) => s < b,
                        (Some(_), None) => true,
                        _ => false,
                    };
                    let better_score = p.best_score.is_none_or(|b| run.score > b);
                    improved = run.stars > p.best_stars || better_score || better_strokes;
                    DojoProgress {
                        best_stars: p.best_stars.max(run.stars),
                        best_score: if better_score {
                            Some(run.score)
                        } else {
                            p.best_score
                        },
                        best_strokes: if better_strokes {
                            run.strokes
                        } else {
                            p.best_strokes
                        },
                        cleared_at: p.cleared_at.or(Some(now)),
                    }
                }
            };
            tx.execute(
                "INSERT INTO dojo_progress(lesson, best_stars, best_score, best_strokes, cleared_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(lesson) DO UPDATE SET
                   best_stars = ?2, best_score = ?3, best_strokes = ?4, cleared_at = ?5",
                rusqlite::params![
                    run.lesson,
                    next.best_stars,
                    next.best_score,
                    next.best_strokes,
                    next.cleared_at,
                ],
            )?;
        }
        tx.commit()?;
        Ok(improved)
    }

    /// 과 id → 최고 기록
    pub fn dojo_progress(&self) -> BTreeMap<String, DojoProgress> {
        let mut out = BTreeMap::new();
        let Ok(mut stmt) = self.conn().prepare(
            "SELECT lesson, best_stars, best_score, best_strokes, cleared_at FROM dojo_progress",
        ) else {
            return out;
        };
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                DojoProgress {
                    best_stars: row.get(1)?,
                    best_score: row.get(2)?,
                    best_strokes: row.get(3)?,
                    cleared_at: row.get(4)?,
                },
            ))
        });
        if let Ok(rows) = rows {
            out.extend(rows.flatten());
        }
        out
    }

    /// 비급을 해금한다. 새로 해금됐으면 `true`.
    pub fn dojo_unlock_scroll(&self, scroll: &str) -> Result<bool, DbError> {
        let changed = self.conn().execute(
            "INSERT OR IGNORE INTO dojo_scrolls(scroll, unlocked_at) VALUES (?1, ?2)",
            rusqlite::params![scroll, now_secs()],
        )?;
        Ok(changed > 0)
    }

    /// 비급 id → 해금 상태
    pub fn dojo_scrolls(&self) -> BTreeMap<String, DojoScroll> {
        let mut out = BTreeMap::new();
        let Ok(mut stmt) = self
            .conn()
            .prepare("SELECT scroll, unlocked_at, mastery FROM dojo_scrolls")
        else {
            return out;
        };
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                DojoScroll {
                    unlocked_at: row.get(1)?,
                    mastery: row.get(2)?,
                },
            ))
        });
        if let Ok(rows) = rows {
            out.extend(rows.flatten());
        }
        out
    }

    /// 최근 기록 (새것부터)
    pub fn dojo_recent_runs(&self, limit: usize) -> Vec<DojoRunSummary> {
        let Ok(mut stmt) = self.conn().prepare(
            "SELECT lesson, stars, score, strokes, par, created_at FROM dojo_runs
             ORDER BY id DESC LIMIT ?1",
        ) else {
            return Vec::new();
        };
        stmt.query_map([limit as i64], |row| {
            Ok(DojoRunSummary {
                lesson: row.get(0)?,
                stars: row.get(1)?,
                score: row.get(2)?,
                strokes: row.get(3)?,
                par: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }
}

fn progress_row(
    conn: &rusqlite::Connection,
    lesson: &str,
) -> Result<Option<DojoProgress>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT best_stars, best_score, best_strokes, cleared_at FROM dojo_progress WHERE lesson = ?1",
    )?;
    let mut rows = stmt.query([lesson])?;
    Ok(match rows.next()? {
        Some(row) => Some(DojoProgress {
            best_stars: row.get(0)?,
            best_score: row.get(1)?,
            best_strokes: row.get(2)?,
            cleared_at: row.get(3)?,
        }),
        None => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lesson: &str, stars: u8, score: u32, strokes: Option<u32>) -> DojoRun {
        DojoRun {
            lesson: lesson.into(),
            kind: "summon".into(),
            stars,
            score,
            strokes,
            os: "macos".into(),
            ..Default::default()
        }
    }

    #[test]
    fn 첫_완주는_최고기록이_되고_더_나쁜_판은_덮어쓰지_않는다() {
        let db = Database::open_in_memory().unwrap();
        assert!(db
            .dojo_record_run(&run("white.1", 2, 800, Some(20)))
            .unwrap());
        assert!(!db
            .dojo_record_run(&run("white.1", 1, 500, Some(30)))
            .unwrap());
        let p = &db.dojo_progress()["white.1"];
        assert_eq!(p.best_stars, 2);
        assert_eq!(p.best_score, Some(800));
        assert_eq!(p.best_strokes, Some(20));
    }

    #[test]
    fn 항목별로_따로_갱신된다() {
        let db = Database::open_in_memory().unwrap();
        db.dojo_record_run(&run("white.1", 2, 800, Some(20)))
            .unwrap();
        // 별은 늘고 점수는 줄었다 — 별만 올라가야 한다
        assert!(db
            .dojo_record_run(&run("white.1", 3, 700, Some(25)))
            .unwrap());
        let p = &db.dojo_progress()["white.1"];
        assert_eq!(
            (p.best_stars, p.best_score, p.best_strokes),
            (3, Some(800), Some(20))
        );
    }

    #[test]
    fn 별_0개_판은_기록만_남는다() {
        let db = Database::open_in_memory().unwrap();
        assert!(!db.dojo_record_run(&run("white.boss", 0, 0, None)).unwrap());
        assert!(db.dojo_progress().is_empty());
        assert_eq!(db.dojo_recent_runs(10).len(), 1);
    }

    #[test]
    fn 비급은_한_번만_해금된다() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.dojo_unlock_scroll("七").unwrap());
        assert!(!db.dojo_unlock_scroll("七").unwrap());
        assert_eq!(db.dojo_scrolls()["七"].mastery, 0);
    }
}
