//! SQLite database for history, bookmarks, and key-value storage.

use std::path::Path;

use rusqlite::Connection;

/// Database wrapper (history + bookmarks + kv_store)
pub struct Database {
    conn: Connection,
}

/// 기본 위치 DB의 역할.
///
/// 데스크톱 런처는 실행 이력을 자체 파일(`desktop/kmd.db`)에 남기고, TUI·CLI·데몬은
/// 공유 파일(`kmd.db`, 본문 인덱스 포함)을 쓴다. 경로를 곳곳에서 손으로 조합하면
/// 한쪽 파일만 보는 버그가 생긴다(폴더 제안이 데스크톱 이력을 못 보던 문제).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbRole {
    /// 공유 DB — TUI/CLI 이력, 본문 인덱스(데몬이 씀)
    Shared,
    /// 데스크톱 런처 전용 이력 DB
    Desktop,
}

impl DbRole {
    /// 모든 역할 — 이력을 합산해 읽을 때 쓴다.
    pub const ALL: [DbRole; 2] = [DbRole::Shared, DbRole::Desktop];

    /// 기본 데이터 디렉터리 기준 DB 파일 경로
    pub fn default_path(self) -> std::path::PathBuf {
        let dir = crate::Config::default_data_dir();
        match self {
            DbRole::Shared => dir.join(crate::DB_FILENAME),
            DbRole::Desktop => dir.join("desktop").join(crate::DB_FILENAME),
        }
    }
}

impl Database {
    /// 기본 위치의 역할별 DB를 연다 (없으면 생성).
    pub fn open_default(role: DbRole) -> Result<Self, DbError> {
        Self::open(&role.default_path())
    }

    /// Open database file (creates parent dirs if needed)
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            // 이 DB는 실행/검색 히스토리를 담고, WAL 모드라 kmd.db-wal/-shm
            // 사이드카까지 같은 디렉터리에 생긴다. 포터블 설치 위치는 상위 경로가
            // 0755라 다른 사용자가 읽을 수 있으므로, 데이터 디렉터리를 0700으로
            // 막는다. 파일마다 0600을 거는 대신 디렉터리 하나로 사이드카까지
            // 전부 덮는다. 이미 존재하는 0755 디렉터리도 idempotent하게 교정된다.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }

        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // 데몬(본문 인덱싱 쓰기)과 데스크톱/TUI(frecency 읽기)가 같은 파일을
        // 다른 프로세스에서 연다. WAL이라 읽기-쓰기 동시성은 되지만, 쓰기 겹침
        // 순간의 SQLITE_BUSY 즉시 실패를 피하려면 대기 시간이 필요하다.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// 크레이트 내부 모듈(content_index 등)이 같은 DB 파일 위에 자체 테이블을
    /// 다루기 위한 연결 접근자. 외부 크레이트에는 노출하지 않는다.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Open in-memory database (for testing) — 파일 DB와 같은 pragma를 건다
    /// (예전엔 foreign_keys를 건너뛰어 테스트 환경이 실제와 달랐다).
    pub fn open_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Record a launch in history (upsert: increment frequency)
    ///
    /// 주기적으로 history pruning을 수행하여 DB 무한 증식을 방지.
    pub fn record_launch(
        &self,
        item_type: &str,
        value: &str,
        display: Option<&str>,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO history(item_type, value, display, frequency)
                 VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT(item_type, value) DO UPDATE SET
                   frequency = frequency + 1,
                   display = COALESCE(?3, display),
                   executed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            rusqlite::params![item_type, value, display],
        )?;

        // 매 20회 호출당 1회 pruning.
        // 기존의 subsec_nanos() % 20 방식은 Windows 시계 해상도가 100ns
        // 단위라 나노초가 항상 100의 배수 = 항상 20의 배수여서
        // "5% 확률" 의도와 달리 매 호출마다 pruning이 실행됐다.
        use std::sync::atomic::{AtomicU32, Ordering};
        static LAUNCH_COUNTER: AtomicU32 = AtomicU32::new(0);
        if LAUNCH_COUNTER.fetch_add(1, Ordering::Relaxed) % 20 == 19 {
            self.prune_history();
        }

        Ok(())
    }

    /// History aging/pruning — zoxide 알고리즘 차용
    ///
    /// 전체 frequency 합이 MAX_TOTAL_FREQ를 초과하면:
    /// 1. 모든 행의 frequency를 0.9배로 감쇠
    /// 2. frequency < 1 인 행 삭제
    pub fn prune_history(&self) {
        const MAX_TOTAL_FREQ: u32 = 9000;

        let total: u32 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(frequency), 0) FROM history",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if total > MAX_TOTAL_FREQ {
            tracing::debug!(
                "History pruning: total_freq={} > {}, aging 적용",
                total,
                MAX_TOTAL_FREQ
            );
            // frequency * 0.9 감쇠 (SQLite CAST로 정수 변환)
            let _ = self.conn.execute(
                "UPDATE history SET frequency = CAST(frequency * 9 / 10 AS INTEGER)",
                [],
            );
            // frequency < 1 인 행 삭제
            let _ = self
                .conn
                .execute("DELETE FROM history WHERE frequency < 1", []);
        }
    }

    /// Query recent/frequent history entries
    pub fn query_history(&self, limit: usize) -> Vec<HistoryEntry> {
        let mut stmt = match self.conn.prepare(
            "SELECT COALESCE(display, value), value, item_type, frequency, executed_at
             FROM history
             ORDER BY frequency DESC, executed_at DESC
             LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("history 조회 실패: {e}");
                return Vec::new();
            }
        };

        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(HistoryEntry {
                    display: row.get(0)?,
                    value: row.get(1)?,
                    item_type: row.get(2)?,
                    frequency: row.get(3)?,
                    executed_at: row.get(4)?,
                })
            })
            .ok();

        match rows {
            Some(iter) => iter.filter_map(|r| r.ok()).collect(),
            None => Vec::new(),
        }
    }

    /// Search history entries matching a query
    pub fn search_history(&self, query: &str, limit: usize) -> Vec<HistoryEntry> {
        // LIKE 와일드카드(%/_)가 사용자 입력으로 동작하지 않도록 이스케이프
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{}%", escaped);
        let mut stmt = match self.conn.prepare(
            "SELECT COALESCE(display, value), value, item_type, frequency, executed_at
             FROM history
             WHERE value LIKE ?1 ESCAPE '\\' OR display LIKE ?1 ESCAPE '\\'
             ORDER BY frequency DESC, executed_at DESC
             LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("history 검색 실패: {e}");
                return Vec::new();
            }
        };

        let rows = stmt
            .query_map(rusqlite::params![pattern, limit as i64], |row| {
                Ok(HistoryEntry {
                    display: row.get(0)?,
                    value: row.get(1)?,
                    item_type: row.get(2)?,
                    frequency: row.get(3)?,
                    executed_at: row.get(4)?,
                })
            })
            .ok();

        match rows {
            Some(iter) => iter.filter_map(|r| r.ok()).collect(),
            None => Vec::new(),
        }
    }

    /// Clear all history
    pub fn clear_history(&self) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM history", [])?;
        Ok(())
    }

    /// Add a bookmark
    pub fn add_bookmark(&self, name: &str, value: &str, item_type: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO bookmarks(name, value, item_type) VALUES (?1, ?2, ?3)",
            rusqlite::params![name, value, item_type],
        )?;
        Ok(())
    }

    /// Query all bookmarks
    pub fn query_bookmarks(&self) -> Vec<BookmarkEntry> {
        let mut stmt = match self.conn.prepare(
            "SELECT name, value, item_type, created_at FROM bookmarks ORDER BY created_at DESC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("bookmarks 조회 실패: {e}");
                return Vec::new();
            }
        };

        let rows = stmt
            .query_map([], |row| {
                Ok(BookmarkEntry {
                    name: row.get(0)?,
                    value: row.get(1)?,
                    item_type: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })
            .ok();

        match rows {
            Some(iter) => iter.filter_map(|r| r.ok()).collect(),
            None => Vec::new(),
        }
    }

    /// Get a value from kv_store
    pub fn kv_get(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM kv_store WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .ok()
    }

    /// Set a value in kv_store
    pub fn kv_set(&self, key: &str, value: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO kv_store(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET
                   value = ?2,
                   updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Run database migrations
    fn migrate(&self) -> Result<(), DbError> {
        let version: i32 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;

        if version < 1 {
            self.conn
                .execute_batch(
                    "
                    -- Launch history
                    CREATE TABLE IF NOT EXISTS history (
                        id          INTEGER PRIMARY KEY AUTOINCREMENT,
                        item_type   TEXT    NOT NULL,
                        value       TEXT    NOT NULL,
                        display     TEXT,
                        executed_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                        frequency   INTEGER NOT NULL DEFAULT 1
                    );
                    CREATE UNIQUE INDEX IF NOT EXISTS idx_history_unique ON history(item_type, value);

                    -- Bookmarks
                    CREATE TABLE IF NOT EXISTS bookmarks (
                        id          INTEGER PRIMARY KEY AUTOINCREMENT,
                        name        TEXT    NOT NULL,
                        value       TEXT    NOT NULL,
                        item_type   TEXT    NOT NULL,
                        created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                    );

                    -- Key-value store
                    CREATE TABLE IF NOT EXISTS kv_store (
                        key        TEXT PRIMARY KEY,
                        value      TEXT NOT NULL,
                        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                    );
                    ",
                )?;
            self.conn.pragma_update(None, "user_version", 1)?;
        }

        if version < 2 {
            // v2: frecency 쿼리 성능을 위한 executed_at 인덱스
            self.conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_history_executed_at ON history(executed_at);",
            )?;
            self.conn.pragma_update(None, "user_version", 2)?;
        }

        if version < 3 {
            // v3: 문서 본문 검색 인덱스 (docs/15).
            // content_fts의 rowid는 content_files.id와 수동으로 동기화한다
            // (트리거 없음 — 삽입/삭제는 content_index 모듈이 한 트랜잭션에서 수행).
            // bundled SQLite에 FTS5가 포함되어 있어 별도 확장 로드는 불필요.
            self.conn.execute_batch(
                "
                    CREATE TABLE IF NOT EXISTS content_files (
                        id         INTEGER PRIMARY KEY AUTOINCREMENT,
                        path       TEXT    NOT NULL UNIQUE,
                        size       INTEGER NOT NULL,
                        mtime      INTEGER NOT NULL,
                        indexed_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                    );
                    CREATE VIRTUAL TABLE IF NOT EXISTS content_fts
                        USING fts5(body, tokenize='unicode61');
                    ",
            )?;
            self.conn.pragma_update(None, "user_version", 3)?;
        }

        if version < 4 {
            // v4: history.item_type 정규화 (ItemKind::history_tag). 예전엔 UI마다
            // "App"/"app"/"Dir"/"dir"/"Directory"로 갈려 같은 경로가 여러 행이었다.
            // 정규 태그로 묶어 빈도는 합산, 시각은 최신, 표시명은 아무거나 하나.
            // 테이블 교체라 트랜잭션으로 묶는다 — 중간 실패 시 v3 상태 그대로 남는다.
            self.conn
                .execute_batch(
                    "
                    BEGIN;
                    CREATE TABLE history_v4 (
                        id          INTEGER PRIMARY KEY AUTOINCREMENT,
                        item_type   TEXT    NOT NULL,
                        value       TEXT    NOT NULL,
                        display     TEXT,
                        executed_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                        frequency   INTEGER NOT NULL DEFAULT 1
                    );
                    INSERT INTO history_v4(item_type, value, display, executed_at, frequency)
                        SELECT tag, value, MAX(display), MAX(executed_at), SUM(frequency)
                        FROM (
                            SELECT CASE lower(item_type)
                                       WHEN 'directory' THEN 'dir'
                                       WHEN 'executable' THEN 'exe'
                                       ELSE lower(item_type)
                                   END AS tag,
                                   value, display, executed_at, frequency
                            FROM history
                        )
                        GROUP BY tag, value;
                    DROP TABLE history;
                    ALTER TABLE history_v4 RENAME TO history;
                    CREATE UNIQUE INDEX idx_history_unique ON history(item_type, value);
                    CREATE INDEX idx_history_executed_at ON history(executed_at);
                    -- bookmarks: add_bookmark의 INSERT OR REPLACE가 동작하려면 이름이
                    -- 유일해야 한다 (예전엔 제약이 없어 같은 이름이 쌓였다). 최신만 남긴다.
                    DELETE FROM bookmarks
                        WHERE id NOT IN (SELECT MAX(id) FROM bookmarks GROUP BY name);
                    CREATE UNIQUE INDEX IF NOT EXISTS idx_bookmarks_name ON bookmarks(name);
                    PRAGMA user_version = 4;
                    COMMIT;
                    ",
                )?;
        }

        Ok(())
    }
}

/// A history entry
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub display: String,
    pub value: String,
    pub item_type: String,
    pub frequency: u32,
    pub executed_at: String,
}

/// A bookmark entry
#[derive(Debug, Clone)]
pub struct BookmarkEntry {
    pub name: String,
    pub value: String,
    pub item_type: String,
    pub created_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_query_history() {
        let db = Database::open_in_memory().unwrap();
        db.record_launch("app", "firefox", Some("Firefox")).unwrap();
        db.record_launch("app", "firefox", Some("Firefox")).unwrap();
        db.record_launch("app", "code", Some("VS Code")).unwrap();

        let history = db.query_history(10);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].display, "Firefox");
        assert_eq!(history[0].frequency, 2);
    }

    #[test]
    fn test_search_history() {
        let db = Database::open_in_memory().unwrap();
        db.record_launch("app", "firefox", Some("Firefox")).unwrap();
        db.record_launch("app", "code", Some("VS Code")).unwrap();

        let results = db.search_history("fire", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "firefox");
    }

    #[test]
    fn test_search_history_escapes_like_wildcards() {
        let db = Database::open_in_memory().unwrap();
        db.record_launch("app", "abc%def", None).unwrap();
        db.record_launch("app", "abcXdef", None).unwrap();

        // '%'는 리터럴로 취급되어야 함 — 와일드카드였다면 둘 다 매칭
        let results = db.search_history("c%d", 10);
        assert_eq!(results.len(), 1, "%는 리터럴 문자로 검색되어야 함");
        assert_eq!(results[0].value, "abc%def");
    }

    #[test]
    fn test_clear_history() {
        let db = Database::open_in_memory().unwrap();
        db.record_launch("app", "firefox", None).unwrap();
        db.clear_history().unwrap();
        assert!(db.query_history(10).is_empty());
    }

    #[test]
    fn test_kv_store() {
        let db = Database::open_in_memory().unwrap();
        db.kv_set("last_index", "2026-01-01").unwrap();
        assert_eq!(db.kv_get("last_index"), Some("2026-01-01".to_string()));
    }

    #[test]
    fn test_bookmarks() {
        let db = Database::open_in_memory().unwrap();
        db.add_bookmark("Firefox", "firefox", "app").unwrap();
        let bookmarks = db.query_bookmarks();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].name, "Firefox");
        // 같은 이름은 교체된다 (UNIQUE 없던 시절엔 쌓였다)
        db.add_bookmark("Firefox", "firefox-dev", "app").unwrap();
        let bookmarks = db.query_bookmarks();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].value, "firefox-dev");
    }

    // ── Pruning 테스트 ─────────────────────────────────────

    #[test]
    fn test_prune_history_reduces_frequencies() {
        let db = Database::open_in_memory().unwrap();

        // frequency 합이 MAX_TOTAL_FREQ(9000)를 초과하도록 대량 등록
        // 10개 항목 × frequency 1000 = 10000 > 9000
        for i in 0..10 {
            let val = format!("item_{}", i);
            // 직접 SQL로 높은 frequency 설정
            db.conn
                .execute(
                    "INSERT INTO history(item_type, value, display, frequency)
                     VALUES ('app', ?1, ?1, 1000)",
                    rusqlite::params![val],
                )
                .unwrap();
        }

        let total_before: u32 = db
            .conn
            .query_row("SELECT SUM(frequency) FROM history", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total_before, 10000);

        db.prune_history();

        let total_after: u32 = db
            .conn
            .query_row("SELECT SUM(frequency) FROM history", [], |row| row.get(0))
            .unwrap();
        // 각 행: 1000 * 9 / 10 = 900, 합: 9000
        assert!(
            total_after < total_before,
            "pruning 후 총 frequency가 감소해야 함: {} < {}",
            total_after,
            total_before
        );
        assert_eq!(total_after, 9000, "각 행 frequency가 0.9배로 감쇠");
    }

    #[test]
    fn test_prune_history_removes_low_frequency() {
        let db = Database::open_in_memory().unwrap();

        // frequency=1 항목 9001개 → 합 9001 > 9000
        for i in 0..9001 {
            let val = format!("item_{}", i);
            db.conn
                .execute(
                    "INSERT INTO history(item_type, value, display, frequency)
                     VALUES ('app', ?1, ?1, 1)",
                    rusqlite::params![val],
                )
                .unwrap();
        }

        let count_before: u32 = db
            .conn
            .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count_before, 9001);

        db.prune_history();

        // frequency=1 * 9/10 = 0 → 삭제됨
        let count_after: u32 = db
            .conn
            .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count_after, 0, "frequency < 1인 모든 행이 삭제되어야 함");
    }

    #[test]
    fn test_prune_history_noop_below_threshold() {
        let db = Database::open_in_memory().unwrap();

        // frequency 합이 임계값 미만이면 아무 동작 없음
        for i in 0..10 {
            let val = format!("item_{}", i);
            db.conn
                .execute(
                    "INSERT INTO history(item_type, value, display, frequency)
                     VALUES ('app', ?1, ?1, 100)",
                    rusqlite::params![val],
                )
                .unwrap();
        }
        // 합: 10 * 100 = 1000 < 9000

        let total_before: u32 = db
            .conn
            .query_row("SELECT SUM(frequency) FROM history", [], |row| row.get(0))
            .unwrap();

        db.prune_history();

        let total_after: u32 = db
            .conn
            .query_row("SELECT SUM(frequency) FROM history", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            total_before, total_after,
            "임계값 미만이면 frequency 변경 없음"
        );
    }

    #[test]
    fn test_migration_v2_creates_index() {
        let db = Database::open_in_memory().unwrap();
        // migration v2가 실행되었는지 확인
        let version: i32 = db
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4, "DB 버전이 4로 업그레이드");

        // idx_history_executed_at 인덱스 존재 확인
        let idx_exists: bool = db
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='index' AND name='idx_history_executed_at'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(idx_exists, "executed_at 인덱스가 존재해야 함");
    }

    #[test]
    fn v4_마이그레이션은_갈린_item_type_행을_합친다() {
        // v3 DB에 예전 UI들이 남긴 표기 그대로 이력을 심는다
        let conn = Connection::open_in_memory().unwrap();
        let v3 = Database { conn };
        v3.migrate().unwrap();
        v3.conn.execute_batch("DROP TABLE history;").unwrap();
        v3.conn
            .execute_batch(
                "CREATE TABLE history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, item_type TEXT NOT NULL,
                    value TEXT NOT NULL, display TEXT,
                    executed_at TEXT NOT NULL, frequency INTEGER NOT NULL DEFAULT 1);
                 CREATE UNIQUE INDEX idx_history_unique ON history(item_type, value);
                 INSERT INTO history(item_type, value, display, executed_at, frequency) VALUES
                    ('App', '/A.app', 'A', '2026-01-01T00:00:00Z', 2),
                    ('app', '/A.app', 'A', '2026-03-01T00:00:00Z', 5),
                    ('Dir', '/d', NULL, '2026-01-01T00:00:00Z', 1),
                    ('Directory', '/d', 'd', '2026-02-01T00:00:00Z', 1),
                    ('dir', '/d', 'd', '2026-01-05T00:00:00Z', 1);
                 PRAGMA user_version = 3;",
            )
            .unwrap();

        v3.migrate().unwrap();

        let rows = v3.query_history(10);
        assert_eq!(rows.len(), 2, "{rows:?}");
        let app = rows.iter().find(|h| h.value == "/A.app").unwrap();
        assert_eq!(app.item_type, "app");
        assert_eq!(app.frequency, 7, "빈도 합산");
        assert_eq!(app.executed_at, "2026-03-01T00:00:00Z", "최신 시각");
        let dir = rows.iter().find(|h| h.value == "/d").unwrap();
        assert_eq!(dir.item_type, "dir");
        assert_eq!(dir.frequency, 3);
        // 정규화 후에도 유니크 키 upsert가 동작한다
        v3.record_launch("dir", "/d", None).unwrap();
        assert_eq!(v3.query_history(10).len(), 2);
    }
}
