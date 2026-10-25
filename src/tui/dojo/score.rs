//! Par · 골프 기호 · 별 — 순수 함수 (docs/10 §4-1, §8)

/// 사부의 수(최소 타수)에 여유를 더한 Par.
/// 여유: 사부의 수 3 이하 +1, 7 이하 +2, 그보다 크면 +3.
pub fn par_from_sage(sage: u32) -> u32 {
    let slack = match sage {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    };
    sage + slack
}

/// 홀 하나의 결과 기호
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Eagle,
    Birdie,
    Par,
    Bogey,
    Double,
}

impl Mark {
    pub fn of(strokes: u32, par: u32) -> Self {
        match strokes as i64 - par as i64 {
            i64::MIN..=-2 => Mark::Eagle,
            -1 => Mark::Birdie,
            0 => Mark::Par,
            1 => Mark::Bogey,
            _ => Mark::Double,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Mark::Eagle => "◎",
            Mark::Birdie => "◉",
            Mark::Par => "─",
            Mark::Bogey => "▲",
            Mark::Double => "▲▲",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mark::Eagle => "이글",
            Mark::Birdie => "버디",
            Mark::Par => "파",
            Mark::Bogey => "보기",
            Mark::Double => "더블 보기",
        }
    }
}

/// 타수로 채점하는 과(소환술·골프·실전) 한 문항의 결과
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoleResult {
    pub strokes: u32,
    pub par: u32,
    pub sage: u32,
    pub hinted: bool,
}

/// 별: ★ 완주 · ★★ 합계 ≤ Par 합 · ★★★ 모든 문항 사부의 수. 힌트를 쓰면 최대 ★★.
pub fn stroke_stars(holes: &[HoleResult]) -> u8 {
    if holes.is_empty() {
        return 0;
    }
    let strokes: u32 = holes.iter().map(|h| h.strokes).sum();
    let par: u32 = holes.iter().map(|h| h.par).sum();
    let mut stars = 1;
    if strokes <= par {
        stars = 2;
        if holes.iter().all(|h| h.strokes <= h.sage) {
            stars = 3;
        }
    }
    if holes.iter().any(|h| h.hinted) {
        stars = stars.min(2);
    }
    stars
}

/// 점수 = 1000 × (사부의 수 합 / 내 타수 합) − 실수 × 20, 0~1000
pub fn stroke_score(holes: &[HoleResult], mistakes: u32) -> u32 {
    let strokes: u32 = holes.iter().map(|h| h.strokes).sum();
    let sage: u32 = holes.iter().map(|h| h.sage).sum();
    if strokes == 0 {
        return 0;
    }
    let base = (1000 * sage / strokes).min(1000);
    base.saturating_sub(mistakes * 20)
}

/// 보스전 별 — 남은 기(氣) 80% 이상 ★★★, 50% 이상 ★★, 격파만 ★
pub fn boss_stars(ki: i32) -> u8 {
    match ki {
        80.. => 3,
        50.. => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(strokes: u32, par: u32, sage: u32) -> HoleResult {
        HoleResult {
            strokes,
            par,
            sage,
            hinted: false,
        }
    }

    #[test]
    fn par는_사부의_수에_구간별_여유를_더한다() {
        assert_eq!(par_from_sage(3), 4);
        assert_eq!(par_from_sage(4), 6);
        assert_eq!(par_from_sage(8), 11);
    }

    #[test]
    fn 골프_기호() {
        assert_eq!(Mark::of(2, 4), Mark::Eagle);
        assert_eq!(Mark::of(3, 4), Mark::Birdie);
        assert_eq!(Mark::of(4, 4), Mark::Par);
        assert_eq!(Mark::of(5, 4), Mark::Bogey);
        assert_eq!(Mark::of(9, 4), Mark::Double);
    }

    #[test]
    fn 별은_완주_par_사부_세_단계다() {
        assert_eq!(stroke_stars(&[]), 0);
        assert_eq!(stroke_stars(&[h(9, 4, 3), h(4, 4, 3)]), 1);
        assert_eq!(stroke_stars(&[h(4, 4, 3), h(3, 4, 3)]), 2);
        assert_eq!(stroke_stars(&[h(3, 4, 3), h(3, 4, 3)]), 3);
        // Par 합은 넘지 않아도 한 홀이 크게 넘으면 ★★ 가 아니다
        assert_eq!(stroke_stars(&[h(2, 6, 2), h(9, 4, 3)]), 1);
    }

    #[test]
    fn 힌트를_쓰면_별은_최대_둘이다() {
        let mut hole = h(3, 4, 3);
        hole.hinted = true;
        assert_eq!(stroke_stars(&[hole]), 2);
    }

    #[test]
    fn 점수는_효율과_실수로_정해진다() {
        assert_eq!(stroke_score(&[h(3, 4, 3)], 0), 1000);
        assert_eq!(stroke_score(&[h(6, 4, 3)], 0), 500);
        assert_eq!(stroke_score(&[h(6, 4, 3)], 2), 460);
        assert_eq!(stroke_score(&[h(6, 4, 3)], 100), 0);
    }
}
