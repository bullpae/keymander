//! 간격 반복 (라이트너 상자) — 입버릇 `src/srs.ts` 를 그대로 옮겼다.
//!
//! 폰과 같은 간격을 써야 S4 동기화 때 두 기기의 복습 일정이 어긋나지 않는다.

use super::Sentence;

const HOUR: i64 = 60 * 60 * 1000;

/// 상자별 다음 복습까지의 간격 (ms). 초반 상자는 몇 시간 간격이라 같은 날 여러 번 다시 나온다.
pub const INTERVALS_MS: [i64; 8] = [
    0,
    3 * HOUR,
    8 * HOUR,
    24 * HOUR,
    3 * 24 * HOUR,
    7 * 24 * HOUR,
    14 * 24 * HOUR,
    30 * 24 * HOUR,
];

pub const MAX_BOX: u8 = (INTERVALS_MS.len() - 1) as u8;

/// 이 상자부터는 영어를 가리고 한국어만 보고 떠올리게 한다
pub const RECALL_FROM_BOX: u8 = 2;

pub fn is_recall_stage(s: &Sentence) -> bool {
    s.r#box >= RECALL_FROM_BOX
}

fn interval(b: u8) -> i64 {
    INTERVALS_MS[usize::from(b.min(MAX_BOX))]
}

/// 말해서 통과 — 상자를 올린다. 컴퓨터는 이 함수를 쓰지 않는다 (docs/21 §4).
pub fn grade_pass(s: &mut Sentence, now: i64) {
    s.r#box = (s.r#box + 1).min(MAX_BOX);
    s.due_at = now + interval(s.r#box);
}

/// 틀림 — 노출 단계로. 1시간 뒤에 다시 나와 같은 날 반복한다.
pub fn grade_fail(s: &mut Sentence, now: i64) {
    s.r#box = 0;
    s.due_at = now + HOUR;
}

/// 정답을 보고 말했거나 컴퓨터에서 맞힘 — 상자는 그대로, 3시간 뒤 다시
pub fn grade_hold(s: &mut Sentence, now: i64) {
    s.due_at = now + 3 * HOUR;
}

/// 복습 시각이 지난 문장, 오래 밀린 순
pub fn due_sentences(all: &[Sentence], now: i64) -> Vec<&Sentence> {
    let mut due: Vec<&Sentence> = all.iter().filter(|s| s.due_at <= now).collect();
    due.sort_by_key(|s| (s.due_at, s.r#box));
    due
}

/// 지금 보여 줄 문장 하나 — 복습할 차례인 것 중 가장 밀린 것, 없으면 상자가 낮은 것.
/// 같은 문장이 연달아 나오지 않도록 `recent` 를 피한다 (다 최근이면 무시).
pub fn pick_for_time<'a>(all: &'a [Sentence], at: i64, recent: &[String]) -> Option<&'a Sentence> {
    let fresh: Vec<&Sentence> = all.iter().filter(|s| !recent.contains(&s.id)).collect();
    let pool: Vec<&Sentence> = if fresh.is_empty() {
        all.iter().collect()
    } else {
        fresh
    };
    if let Some(s) = pool
        .iter()
        .filter(|s| s.due_at <= at)
        .min_by_key(|s| s.due_at)
    {
        return Some(s);
    }
    pool.into_iter().min_by_key(|s| (s.r#box, s.due_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, b: u8, due_at: i64) -> Sentence {
        Sentence {
            id: id.into(),
            en: format!("Sentence {id}"),
            ko: format!("문장 {id}"),
            builtin: true,
            r#box: b,
            due_at,
            ..Sentence::default()
        }
    }

    // 입버릇 tests/logic.test.ts "간격 반복: 맞히면 단계 상승, 틀리면 처음으로"
    #[test]
    fn pass_raises_fail_resets() {
        let now = 1_000_000;
        let mut a = mk("a", 0, 0);
        grade_pass(&mut a, now);
        assert_eq!(a.r#box, 1);
        assert_eq!(a.due_at, now + INTERVALS_MS[1]);
        grade_pass(&mut a, now);
        assert!(is_recall_stage(&a));
        let mut f = mk("b", 4, 0);
        grade_fail(&mut f, now);
        assert_eq!(f.r#box, 0);
        assert!(f.due_at > now);
        let all = [mk("x", 0, 5), mk("y", 0, 1), mk("z", 0, 99)];
        let ids: Vec<&str> = due_sentences(&all, 10)
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids, ["y", "x"]);
    }

    #[test]
    fn hold_keeps_box() {
        let mut s = mk("h", 3, 0);
        grade_hold(&mut s, 100);
        assert_eq!(s.r#box, 3);
        assert_eq!(s.due_at, 100 + 3 * HOUR);
    }

    #[test]
    fn max_box_caps() {
        let mut s = mk("m", MAX_BOX, 0);
        grade_pass(&mut s, 0);
        assert_eq!(s.r#box, MAX_BOX);
        assert_eq!(s.due_at, INTERVALS_MS[7]);
    }

    #[test]
    fn pick_prefers_due_then_low_box_and_avoids_recent() {
        let all = [mk("a", 0, 50), mk("b", 0, 10), mk("c", 3, 500)];
        assert_eq!(pick_for_time(&all, 100, &[]).unwrap().id, "b");
        assert_eq!(pick_for_time(&all, 100, &["b".into()]).unwrap().id, "a");
        // 밀린 것이 없으면 상자가 낮은 것
        assert_eq!(pick_for_time(&all, 0, &[]).unwrap().id, "b");
        // 전부 최근이면 최근 제외를 무시
        let recent: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(pick_for_time(&all, 100, &recent).unwrap().id, "b");
        assert!(pick_for_time(&[], 0, &[]).is_none());
    }
}
