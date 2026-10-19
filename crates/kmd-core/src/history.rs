//! History tracking — frecency-based scoring for search results
//!
//! Frecency = frequency × recency_weight.
//! 최근 사용한 항목에 높은 가중치를 부여하여 Score Pollution을 방지하고,
//! 상한(CAP)을 설정하여 검색 매칭 품질이 항상 순위에 반영되도록 한다.

use std::collections::HashMap;

use crate::db::Database;
use crate::index::ItemKind;
use crate::search::SearchResult;

/// 부스트 계산에 사용할 최대 히스토리 행 수
const HISTORY_BOOST_LIMIT: usize = 500;
/// Frecency 부스트 상한 — search_score(최대 ~200)와 균형 유지
const FRECENCY_BOOST_CAP: u32 = 200;
/// 한 번이라도 실행한 항목의 기본 부스트 — frecency 위에 더한다.
///
/// 이게 없으면 1회 실행 부스트가 +16(1시간 내)~+1에 그쳐, 같은 이름의 폴더
/// (`D:\Util\VeraCrypt` 110점)가 그 안의 실행 파일(`VeraCrypt.exe` 70점)을
/// 영영 이긴다 — 종류 가중치 차(폴더 80·파일 50)만으로 40점이 벌어진다.
/// "실행한 쪽이 위로"를 1회 실행으로 보장하되, 매칭 품질 차가 큰 결과까지
/// 뒤집지 않게 search_score 범위(~200)의 1/4로 둔다.
const LAUNCHED_BASE_BOOST: u32 = 50;

/// value → (frequency, executed_at) — `load_boost_map`으로 만들어 재사용하는 캐시.
///
/// 데스크톱 런처는 키 입력마다 검색을 수행하므로, 매번 DB를 조회하는 대신
/// 부팅 시 한 번 로드한 맵으로 부스트한다 (`boost_results_with_map`).
pub type FrecencyMap = HashMap<String, (u32, String)>;

/// DB에서 부스트용 frecency 맵을 로드한다.
///
/// 같은 value가 여러 item_type 행으로 있으면 빈도는 합산하고 시각은 최신을 쓴다
/// (예전엔 나중 행이 앞 행을 덮어써 frecency가 낮게 잡혔다).
pub fn load_boost_map(db: &Database) -> FrecencyMap {
    let mut map = FrecencyMap::new();
    for h in db.query_history(HISTORY_BOOST_LIMIT) {
        map.entry(h.value)
            .and_modify(|(freq, at)| {
                *freq = freq.saturating_add(h.frequency);
                if h.executed_at > *at {
                    *at = h.executed_at.clone();
                }
            })
            .or_insert((h.frequency, h.executed_at));
    }
    map
}

/// Frecency 기반으로 검색 결과 점수를 부스트하고 재정렬 (매 호출마다 DB 조회)
pub fn boost_results(results: &mut [SearchResult], db: &Database) {
    let freq_map = load_boost_map(db);
    boost_results_with_map(results, &freq_map);
}

/// 미리 로드한 frecency 맵으로 부스트 — 검색 핫패스에서 DB 조회를 피한다.
pub fn boost_results_with_map(results: &mut [SearchResult], freq_map: &FrecencyMap) {
    for result in results.iter_mut() {
        if let Some((freq, executed_at)) = freq_map.get(&result.item.path) {
            let boost = LAUNCHED_BASE_BOOST
                .saturating_add(frecency_score(*freq, executed_at))
                .min(FRECENCY_BOOST_CAP);
            result.score = result.score.saturating_add(boost);
        }
    }

    // 점수 기준 내림차순 재정렬
    results.sort_by_key(|r| std::cmp::Reverse(r.score));
}

/// Frecency 점수 계산: frequency × recency_weight
///
/// recency_weight는 `executed_at`으로부터 경과 시간에 따라 감쇠:
/// - 1시간 이내: ×16
/// - 24시간 이내: ×8
/// - 1주 이내: ×4
/// - 1달 이내: ×2
/// - 그 이전: ×1
fn frecency_score(frequency: u32, executed_at: &str) -> u32 {
    let recency_weight = match parse_hours_ago(executed_at) {
        Some(hours) => match hours {
            0..=1 => 16,
            2..=24 => 8,
            25..=168 => 4,
            169..=720 => 2,
            _ => 1,
        },
        None => 1, // 파싱 실패 → 순수 frequency fallback
    };
    frequency.saturating_mul(recency_weight)
}

/// ISO 8601 타임스탬프("2026-02-16T10:30:00Z")로부터 현재까지 경과 시간(시간 단위)을 계산.
/// 파싱 실패 시 `None` 반환 → 호출부에서 weight=1 fallback 처리.
fn parse_hours_ago(iso_str: &str) -> Option<u64> {
    use std::time::{SystemTime, UNIX_EPOCH};

    // "2026-02-16T10:30:00Z" 형태 파싱
    let s = iso_str.trim();
    if s.len() < 19 {
        return None;
    }

    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(5..7)?.parse().ok()?;
    let day: u32 = s.get(8..10)?.parse().ok()?;
    let hour: u32 = s.get(11..13)?.parse().ok()?;
    let minute: u32 = s.get(14..16)?.parse().ok()?;
    let second: u32 = s.get(17..19)?.parse().ok()?;

    // 간이 날짜→Unix epoch 변환 (윤년 근사)
    let days = days_from_civil(year, month, day)?;
    let ts = days as u64 * 86400 + hour as u64 * 3600 + minute as u64 * 60 + second as u64;

    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();

    if now >= ts {
        Some((now - ts) / 3600)
    } else {
        Some(0) // 미래 타임스탬프 → 0시간 전으로 처리
    }
}

/// 그레고리력 날짜 → Unix epoch 이후 일 수 변환 (civil date to days)
/// 알고리즘 출처: Howard Hinnant's date algorithms
fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400) as u64;
    let doy = (153 * m as u64 + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe as i64 - 719468)
}

/// Record a launch event — item_type은 [`ItemKind::history_tag`]로 정규화된다.
pub fn record_launch(db: &Database, kind: ItemKind, value: &str, display: Option<&str>) {
    if let Err(e) = db.record_launch(kind.history_tag(), value, display) {
        tracing::warn!("Failed to record launch history: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{IndexItem, ItemKind, Source};
    use crate::search::SearchResult;

    fn make_result(path: &str, score: u32) -> SearchResult {
        SearchResult {
            item: IndexItem {
                name: path.to_string(),
                path: path.to_string(),
                kind: ItemKind::File,
                source: Source::FileProvider,
                icon: "--".to_string(),
                keywords: String::new(),
                icon_path: None,
            },
            score,
        }
    }

    #[test]
    fn test_boost_results_with_frecency() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("test.db")).unwrap();

        // "b"를 여러 번 실행 → frequency 증가, executed_at은 "방금"
        for _ in 0..5 {
            record_launch(&db, ItemKind::File, "b", None);
        }

        let mut results = vec![make_result("a", 500), make_result("b", 100)];
        boost_results(&mut results, &db);

        // frequency=5, 방금 실행 → recency_weight=16
        // boost = 기본 50 + frecency 5*16=80 = 130 (CAP 200 이하)
        // b: 100 + 130 = 230 < a: 500 → a가 여전히 1등
        assert_eq!(results[0].item.path, "a");

        // 더 많이 사용하면 CAP에 도달
        for _ in 0..20 {
            record_launch(&db, ItemKind::File, "b", None);
        }
        // frequency=25, recency=16 → 25*16=400 → capped 200
        // b: 100 + 200 = 300 < a: 500 → CAP 덕분에 여전히 a가 우세
        let mut results = vec![make_result("a", 500), make_result("b", 100)];
        boost_results(&mut results, &db);
        assert_eq!(results[0].item.path, "a", "CAP이 Score Pollution을 방지");
    }

    #[test]
    fn test_boost_results_with_map_equals_db_variant() {
        // 데스크톱 핫패스용 맵 기반 부스트가 DB 직조회 버전과 동일하게 동작
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("test.db")).unwrap();
        for _ in 0..5 {
            record_launch(&db, ItemKind::File, "b", None);
        }

        let map = load_boost_map(&db);
        let mut via_map = vec![make_result("a", 100), make_result("b", 100)];
        boost_results_with_map(&mut via_map, &map);

        let mut via_db = vec![make_result("a", 100), make_result("b", 100)];
        boost_results(&mut via_db, &db);

        assert_eq!(via_map[0].item.path, "b", "이력 있는 b가 부스트되어 1등");
        assert_eq!(via_map[0].item.path, via_db[0].item.path);
        assert_eq!(via_map[0].score, via_db[0].score);
    }

    #[test]
    fn test_single_launch_lifts_exe_over_same_name_folder() {
        // 실측 점수 재현: "vera" → 폴더 D:\Util\VeraCrypt 110, VeraCrypt.exe 70.
        // 실행 파일을 한 번 실행하면 다음 검색부터 폴더보다 위에 와야 한다.
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("test.db")).unwrap();
        record_launch(&db, ItemKind::File, "exe", None);

        let mut results = vec![make_result("folder", 110), make_result("exe", 70)];
        boost_results(&mut results, &db);
        assert_eq!(results[0].item.path, "exe", "1회 실행으로 순위 역전");

        // 한 달 넘게 지난 1회 실행이라도 기본 부스트는 남는다
        let mut map = FrecencyMap::new();
        map.insert("exe".into(), (1, "2000-01-01T00:00:00Z".into()));
        let mut results = vec![make_result("folder", 110), make_result("exe", 70)];
        boost_results_with_map(&mut results, &map);
        assert_eq!(results[0].item.path, "exe");
    }

    #[test]
    fn test_boost_no_history_preserves_order() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("test.db")).unwrap();

        let mut results = vec![make_result("a", 500), make_result("b", 300)];

        boost_results(&mut results, &db);

        assert_eq!(results[0].item.path, "a");
        assert_eq!(results[1].item.path, "b");
    }

    // ── Frecency 함수 단위 테스트 ──────────────────────────

    #[test]
    fn test_frecency_score_recent() {
        // 방금 실행 (0시간 전) → weight=16
        let now_iso = {
            use std::time::{SystemTime, UNIX_EPOCH};
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            // 간이 포매팅
            format_epoch_as_iso(secs)
        };
        let score = frecency_score(10, &now_iso);
        assert_eq!(score, 10 * 16, "1시간 이내 → weight 16");
    }

    #[test]
    fn test_frecency_score_old() {
        // 60일 전 → 1440시간 → weight=1
        let old_iso = {
            use std::time::{SystemTime, UNIX_EPOCH};
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                - 60 * 24 * 3600;
            format_epoch_as_iso(secs)
        };
        let score = frecency_score(10, &old_iso);
        assert_eq!(score, 10, "60일 이전 → weight 1");
    }

    #[test]
    fn test_frecency_score_parse_failure() {
        // 잘못된 형식 → weight=1 fallback
        let score = frecency_score(10, "invalid-date");
        assert_eq!(score, 10, "파싱 실패 → weight 1 fallback");
    }

    #[test]
    fn test_parse_hours_ago_valid() {
        let result = parse_hours_ago("2026-02-16T10:30:00Z");
        assert!(result.is_some(), "유효한 ISO 8601 파싱 성공");
    }

    #[test]
    fn test_parse_hours_ago_invalid() {
        assert!(parse_hours_ago("").is_none());
        assert!(parse_hours_ago("not-a-date").is_none());
        assert!(parse_hours_ago("2026").is_none());
    }

    #[test]
    fn test_days_from_civil() {
        // 2026-01-01 → Unix epoch 이후 일 수 확인
        let days = days_from_civil(1970, 1, 1).unwrap();
        assert_eq!(days, 0, "1970-01-01 = epoch day 0");

        let days = days_from_civil(2000, 3, 1).unwrap();
        assert!(days > 0, "2000-03-01은 양수");

        assert!(
            days_from_civil(2026, 0, 1).is_none(),
            "월 0은 유효하지 않음"
        );
        assert!(
            days_from_civil(2026, 13, 1).is_none(),
            "월 13은 유효하지 않음"
        );
    }

    #[test]
    fn test_frecency_boost_cap_applied() {
        // frequency=100, 방금 실행 → 100*16=1600 >> CAP(200)
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("test.db")).unwrap();

        for _ in 0..100 {
            record_launch(&db, ItemKind::File, "x", None);
        }

        let mut results = vec![make_result("x", 0)];
        boost_results(&mut results, &db);

        // score = 0 + min(100*16, 200) = 200
        assert_eq!(results[0].score, FRECENCY_BOOST_CAP, "CAP이 정확히 적용됨");
    }

    /// 테스트용: Unix epoch 초를 ISO 8601 문자열로 변환
    fn format_epoch_as_iso(secs: u64) -> String {
        let days = secs / 86400;
        let rem = secs % 86400;
        let hour = rem / 3600;
        let minute = (rem % 3600) / 60;
        let second = rem % 60;

        // days → civil date (Howard Hinnant 역변환)
        let z = days as i64 + 719468;
        let era = z.div_euclid(146097);
        let doe = z.rem_euclid(146097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            y, m, d, hour, minute, second
        )
    }
}
