//! 비급 시험 (S16) — 외운 답이 아니라 납득을 묻는다.
//!
//! 갈색띠 2과(종합 시험 10문)와 비급 도감의 권별 시험이 같은 라운드를 쓴다. 보기
//! 순서는 판마다 섞는다 — 자리를 외워서는 못 맞힌다. 틀리면 그 권의 본문을 바로
//! 펼쳐 보여 준다.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};

use super::content::Content;
use super::play::{Rng, RoundEvent, Summary};

/// 종합 시험 합격선 — 정답 비율
const PASS: f32 = 0.6;

#[derive(Debug, Clone)]
pub struct Question {
    pub scroll: String,
    pub q: String,
    /// 섞인 보기
    pub choices: Vec<String>,
    /// 섞인 보기에서 정답 자리
    pub answer: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Ask,
    /// 답을 냈다 — 고른 자리
    Reveal(usize),
}

pub struct QuizRound {
    pub lesson_id: String,
    pub questions: Vec<Question>,
    pub idx: usize,
    pub sel: usize,
    pub phase: Phase,
    /// 맞힌 문항의 비급 id
    pub correct: Vec<String>,
    /// 비급 도감에서 연 권별 시험 — 기록 대신 숙련만 올린다
    pub codex: bool,
    pub total_mistakes: u32,
    pub fail_streak: u32,
    pub started: Instant,
    pub flash: Option<(bool, Instant)>,
}

impl QuizRound {
    /// `scrolls` 순서대로 문항을 낸다 (모르는 id 는 건너뛴다)
    pub fn new(
        content: &Content,
        lesson_id: &str,
        scrolls: &[String],
        seed: &str,
        codex: bool,
        now: Instant,
    ) -> Option<Self> {
        let mut rng = Rng::seeded(seed);
        let questions: Vec<Question> = scrolls
            .iter()
            .filter_map(|id| content.scroll(id))
            .map(|s| {
                let mut order: Vec<usize> = (0..s.quiz.choices.len()).collect();
                for i in (1..order.len()).rev() {
                    order.swap(i, rng.below(i as u64 + 1) as usize);
                }
                Question {
                    scroll: s.id.clone(),
                    q: s.quiz.q.clone(),
                    choices: order.iter().map(|&i| s.quiz.choices[i].clone()).collect(),
                    answer: order.iter().position(|&i| i == s.quiz.answer).unwrap_or(0),
                }
            })
            .collect();
        if questions.is_empty() {
            return None;
        }
        Some(Self {
            lesson_id: lesson_id.to_string(),
            questions,
            idx: 0,
            sel: 0,
            phase: Phase::Ask,
            correct: Vec::new(),
            codex,
            total_mistakes: 0,
            fail_streak: 0,
            started: now,
            flash: None,
        })
    }

    pub fn question(&self) -> &Question {
        &self.questions[self.idx.min(self.questions.len() - 1)]
    }

    fn pick(&mut self, i: usize, now: Instant) -> RoundEvent {
        let q = self.question();
        if i >= q.choices.len() {
            return RoundEvent::None;
        }
        let ok = i == q.answer;
        let scroll = q.scroll.clone();
        self.phase = Phase::Reveal(i);
        self.flash = Some((ok, now));
        if ok {
            self.correct.push(scroll);
            self.fail_streak = 0;
            RoundEvent::Hit
        } else {
            self.total_mistakes += 1;
            self.fail_streak += 1;
            RoundEvent::Mistake
        }
    }

    pub fn on_key(&mut self, key: &KeyEvent, now: Instant) -> RoundEvent {
        match self.phase {
            Phase::Ask => {
                let n = self.question().choices.len();
                match key.code {
                    KeyCode::Up => self.sel = self.sel.saturating_sub(1),
                    KeyCode::Down => self.sel = (self.sel + 1).min(n - 1),
                    KeyCode::Enter => return self.pick(self.sel, now),
                    KeyCode::Char(c @ '1'..='9') => {
                        return self.pick(c as usize - '1' as usize, now);
                    }
                    _ => {}
                }
                RoundEvent::None
            }
            Phase::Reveal(_) => {
                if !matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
                    return RoundEvent::None;
                }
                if self.idx + 1 < self.questions.len() {
                    self.idx += 1;
                    self.sel = 0;
                    self.phase = Phase::Ask;
                    RoundEvent::Cleared {
                        over_par: false,
                        sage_equal: false,
                    }
                } else if self.passed() {
                    RoundEvent::Finished
                } else {
                    RoundEvent::Failed
                }
            }
        }
    }

    pub fn passed(&self) -> bool {
        self.codex || self.correct.len() as f32 >= self.questions.len() as f32 * PASS
    }

    pub fn summary(&self, cleared: bool) -> Summary {
        let n = self.questions.len() as u32;
        let ok = self.correct.len() as u32;
        let stars = match (cleared, ok) {
            (false, _) => 0,
            (true, k) if k == n => 3,
            (true, k) if k * 10 >= n * 8 => 2,
            _ => 1,
        };
        let mut stats = vec![("정답".to_string(), format!("{ok} / {n}"))];
        let wrong: Vec<String> = self
            .questions
            .iter()
            .filter(|q| !self.correct.contains(&q.scroll))
            .map(|q| format!("卷{}", q.scroll))
            .collect();
        if !wrong.is_empty() {
            stats.push(("다시 읽을 비급".into(), wrong.join(" ")));
        }
        Summary {
            stars,
            score: ok * 1000 / n.max(1),
            strokes: None,
            par: None,
            mistakes: self.total_mistakes,
            hints: 0,
            max_combo: None,
            rows: Vec::new(),
            stats,
            ki: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::content::content;
    use crossterm::event::KeyModifiers;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn 보기는_섞여도_정답은_따라간다() {
        let c = content();
        let ids: Vec<String> = c.scrolls.scroll.iter().map(|s| s.id.clone()).collect();
        let q = QuizRound::new(c, "brown.2", &ids, "seed", false, Instant::now()).unwrap();
        for (qq, s) in q.questions.iter().zip(&c.scrolls.scroll) {
            assert_eq!(qq.choices[qq.answer], s.quiz.choices[s.quiz.answer]);
        }
        let other = QuizRound::new(c, "brown.2", &ids, "다른 씨앗", false, Instant::now()).unwrap();
        assert!(
            q.questions
                .iter()
                .zip(&other.questions)
                .any(|(a, b)| a.answer != b.answer),
            "판마다 정답 자리가 바뀐다"
        );
    }

    #[test]
    fn 다_맞히면_별_셋_반도_못_맞히면_패배() {
        let c = content();
        let ids: Vec<String> = c
            .scrolls
            .scroll
            .iter()
            .take(5)
            .map(|s| s.id.clone())
            .collect();
        let now = Instant::now();
        let mut q = QuizRound::new(c, "brown.2", &ids, "s", false, now).unwrap();
        let mut last = RoundEvent::None;
        for _ in 0..5 {
            let a = q.question().answer;
            q.on_key(&k(KeyCode::Char(char::from(b'1' + a as u8))), now);
            last = q.on_key(&k(KeyCode::Enter), now);
        }
        assert_eq!(last, RoundEvent::Finished);
        assert_eq!(q.summary(true).stars, 3);

        let mut q = QuizRound::new(c, "brown.2", &ids, "s", false, now).unwrap();
        for _ in 0..5 {
            let wrong = (q.question().answer + 1) % q.question().choices.len();
            assert_eq!(
                q.on_key(&k(KeyCode::Char(char::from(b'1' + wrong as u8))), now),
                RoundEvent::Mistake
            );
            last = q.on_key(&k(KeyCode::Enter), now);
        }
        assert_eq!(last, RoundEvent::Failed);
    }
}
