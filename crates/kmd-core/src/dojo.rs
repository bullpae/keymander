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

/// 오늘의 수련 연속일 (docs/10 §4-7)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DojoStreak {
    /// 오늘(또는 어제)까지 이어진 연속일. 어제 끊겼으면 0.
    pub days: u32,
    pub today_done: bool,
    /// 지금까지 가장 긴 연속일
    pub best: u32,
    /// 완주한 날 수
    pub total: u32,
}

/// OS 인장 (docs/10 §8-4)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DojoSeal {
    pub os: String,
    pub earned_at: i64,
    /// local | imported
    pub source: String,
}

impl Database {
    /// 로컬 날짜 'YYYY-MM-DD' — 시간대는 SQLite 의 localtime 이 OS 를 따른다
    pub fn dojo_today(&self) -> String {
        self.conn()
            .query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))
            .unwrap_or_else(|_| "1970-01-01".into())
    }

    /// 오늘의 수련 완주를 남긴다. 오늘 첫 완주면 `true`.
    pub fn dojo_complete_daily(&self, score: u32) -> Result<bool, DbError> {
        let day = self.dojo_today();
        let fresh = self.conn().execute(
            "INSERT OR IGNORE INTO dojo_daily(day, score, completed_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![day, score, now_secs()],
        )? > 0;
        if !fresh {
            self.conn().execute(
                "UPDATE dojo_daily SET score = MAX(score, ?2) WHERE day = ?1",
                rusqlite::params![day, score],
            )?;
        }
        Ok(fresh)
    }

    pub fn dojo_streak(&self) -> DojoStreak {
        // 오늘로부터 며칠 전인지 — 최근 것부터
        let Ok(mut stmt) = self.conn().prepare(
            "SELECT CAST(julianday(date('now', 'localtime')) - julianday(day) AS INTEGER)
             FROM dojo_daily ORDER BY day DESC",
        ) else {
            return DojoStreak::default();
        };
        let ago: Vec<i64> = stmt
            .query_map([], |r| r.get(0))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default();
        streak_of(&ago)
    }

    /// OS → 인장
    pub fn dojo_seals(&self) -> Vec<DojoSeal> {
        let Ok(mut stmt) = self
            .conn()
            .prepare("SELECT os, earned_at, source FROM dojo_seals ORDER BY earned_at")
        else {
            return Vec::new();
        };
        stmt.query_map([], |r| {
            Ok(DojoSeal {
                os: r.get(0)?,
                earned_at: r.get(1)?,
                source: r.get(2)?,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// 인장을 찍는다. 그 OS 인장이 처음이면 `true` (이미 있으면 그대로 둔다).
    pub fn dojo_add_seal(&self, os: &str, earned_at: i64, source: &str) -> Result<bool, DbError> {
        let n = self.conn().execute(
            "INSERT OR IGNORE INTO dojo_seals(os, earned_at, source) VALUES (?1, ?2, ?3)",
            rusqlite::params![os, earned_at, source],
        )?;
        Ok(n > 0)
    }

    /// 비급 시험 정답 — 해금된 권이면 숙련을 하나 올린다(최대 3). 새 숙련을 돌려준다.
    pub fn dojo_master_scroll(&self, scroll: &str) -> Result<Option<u8>, DbError> {
        self.conn().execute(
            "UPDATE dojo_scrolls SET mastery = MIN(3, mastery + 1) WHERE scroll = ?1",
            [scroll],
        )?;
        Ok(self.dojo_scrolls().get(scroll).map(|s| s.mastery))
    }

    /// 라운드에서 쓴 수를 센다 (결과 카드의 "많이 쓴 수")
    pub fn dojo_add_keys(&self, counts: &BTreeMap<String, u32>) -> Result<(), DbError> {
        let tx = self.conn().unchecked_transaction()?;
        for (action, n) in counts {
            tx.execute(
                "INSERT INTO dojo_keys(action, count) VALUES (?1, ?2)
                 ON CONFLICT(action) DO UPDATE SET count = count + ?2",
                rusqlite::params![action, n],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 많이 쓴 수 — 많은 것부터
    pub fn dojo_keys(&self) -> Vec<(String, u32)> {
        let Ok(mut stmt) = self
            .conn()
            .prepare("SELECT action, count FROM dojo_keys ORDER BY count DESC, action")
        else {
            return Vec::new();
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }
}

/// 완주한 날들(오늘로부터 며칠 전, 최근 것부터)에서 연속일을 센다
fn streak_of(ago: &[i64]) -> DojoStreak {
    let today_done = ago.first() == Some(&0);
    // 오늘 아직 안 했어도 어제까지 이어졌으면 연속은 살아 있다
    let mut days = 0;
    if let Some(&first) = ago.first() {
        if first <= 1 {
            days = 1;
            for w in ago.windows(2) {
                if w[1] - w[0] == 1 {
                    days += 1;
                } else {
                    break;
                }
            }
        }
    }
    let mut best = u32::from(!ago.is_empty());
    let mut run = 1;
    for w in ago.windows(2) {
        if w[1] - w[0] == 1 {
            run += 1;
            best = best.max(run);
        } else {
            run = 1;
        }
    }
    DojoStreak {
        days,
        today_done,
        best: best.max(days),
        total: ago.len() as u32,
    }
}

// ── 인장 코드 (docs/10 §8-4) ─────────────────────────────────────────────────
//
// 기기마다 DB 가 따로라 인장을 코드로 옮긴다: `KMD1-` + base32(JSON) + `-` + CRC32.
// CRC 는 옮겨 적다 생긴 오타를 잡을 뿐 위조는 막지 않는다 — 정직 시스템.

const SEAL_PREFIX: &str = "KMD1-";
const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// 인장 코드에 담기는 내용
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SealCode {
    pub os: String,
    pub belt: String,
    /// 달성 시각 (Unix 초)
    pub at: i64,
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn b32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0);
    for &b in data {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(B32[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn b32_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0);
    for c in text.bytes() {
        let v = B32.iter().position(|&x| x == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

pub fn seal_encode(seal: &SealCode) -> String {
    let json = serde_json::to_vec(seal).unwrap_or_default();
    format!("{SEAL_PREFIX}{}-{:08X}", b32_encode(&json), crc32(&json))
}

pub fn seal_decode(code: &str) -> Result<SealCode, String> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let body = code
        .strip_prefix(SEAL_PREFIX)
        .ok_or("KMD1- 로 시작하는 인장 코드가 아니다")?;
    let (data, crc) = body.rsplit_once('-').ok_or("CRC 가 없다")?;
    let bytes = b32_decode(data).ok_or("코드에 쓸 수 없는 글자가 있다")?;
    let want = u32::from_str_radix(crc, 16).map_err(|_| "CRC 형식이 틀렸다")?;
    if crc32(&bytes) != want {
        return Err("CRC 가 맞지 않는다 — 옮겨 적다 틀린 글자가 있다".into());
    }
    let seal: SealCode =
        serde_json::from_slice(&bytes).map_err(|e| format!("내용을 읽을 수 없다: {e}"))?;
    if !["macos", "windows", "linux"].contains(&seal.os.as_str()) {
        return Err(format!("모르는 OS: {}", seal.os));
    }
    Ok(seal)
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

    #[test]
    fn 연속일은_오늘이나_어제까지_이어진_날만_센다() {
        assert_eq!(streak_of(&[]), DojoStreak::default());
        let s = streak_of(&[0, 1, 2, 5, 6]);
        assert_eq!((s.days, s.today_done, s.best, s.total), (3, true, 3, 5));
        // 오늘은 아직 — 어제까지 이어졌으면 산다
        let s = streak_of(&[1, 2]);
        assert_eq!((s.days, s.today_done), (2, false));
        // 그제가 마지막이면 끊겼다
        let s = streak_of(&[2, 3, 4, 5]);
        assert_eq!((s.days, s.best), (0, 4));
    }

    #[test]
    fn 오늘의_수련은_하루에_한_줄이다() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.dojo_complete_daily(500).unwrap());
        assert!(!db.dojo_complete_daily(800).unwrap());
        let s = db.dojo_streak();
        assert_eq!((s.days, s.today_done, s.total), (1, true, 1));
        assert_eq!(db.dojo_today().len(), 10);
    }

    #[test]
    fn 인장_코드는_왕복하고_오타는_잡힌다() {
        let seal = SealCode {
            os: "macos".into(),
            belt: "black".into(),
            at: 1_790_000_000,
        };
        let code = seal_encode(&seal);
        assert!(code.starts_with("KMD1-"), "{code}");
        assert_eq!(seal_decode(&code).unwrap(), seal);
        assert_eq!(
            seal_decode(&code.to_lowercase().replace("kmd1-", "KMD1-")).unwrap(),
            seal
        );
        // 한 글자 바꾸면 CRC 에 걸린다
        let mut broken: Vec<char> = code.chars().collect();
        let i = 8;
        broken[i] = if broken[i] == 'A' { 'B' } else { 'A' };
        let broken: String = broken.into_iter().collect();
        assert!(seal_decode(&broken).is_err());
        assert!(seal_decode("hello").is_err());
    }

    #[test]
    fn 인장은_os_마다_하나고_숙련은_셋까지다() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.dojo_add_seal("macos", 1, "local").unwrap());
        assert!(!db.dojo_add_seal("macos", 2, "imported").unwrap());
        assert!(db.dojo_add_seal("linux", 3, "imported").unwrap());
        assert_eq!(db.dojo_seals().len(), 2);

        assert_eq!(
            db.dojo_master_scroll("七").unwrap(),
            None,
            "해금 전엔 숙련 없음"
        );
        db.dojo_unlock_scroll("七").unwrap();
        for _ in 0..5 {
            db.dojo_master_scroll("七").unwrap();
        }
        assert_eq!(db.dojo_scrolls()["七"].mastery, 3);
    }

    #[test]
    fn 많이_쓴_수는_더해진다() {
        let db = Database::open_in_memory().unwrap();
        let mut m = BTreeMap::new();
        m.insert("Down".to_string(), 3);
        m.insert("WordRight".to_string(), 5);
        db.dojo_add_keys(&m).unwrap();
        db.dojo_add_keys(&m).unwrap();
        assert_eq!(db.dojo_keys()[0], ("WordRight".to_string(), 10));
    }
}
