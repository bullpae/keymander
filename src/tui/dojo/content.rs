//! dojo 콘텐츠 — `assets/dojo/*.toml`을 바이너리에 내장한다 (docs/10 §9).
//!
//! 콘텐츠는 코드가 아니라 데이터다. 형식 오류는 아래 테스트가 빌드 단계에서
//! 잡으므로, 런타임 파싱 실패는 버그로 보고 패닉한다.

use std::sync::OnceLock;

use serde::Deserialize;

use super::action::Action;

const CURRICULUM: &str = include_str!("../../../assets/dojo/curriculum.toml");
const SCROLLS: &str = include_str!("../../../assets/dojo/scrolls.toml");
const SENSEI: &str = include_str!("../../../assets/dojo/sensei.toml");
const SUMMON: &str = include_str!("../../../assets/dojo/summon.toml");

/// 이번 판에 구현된 수련 종류. 나머지는 지도에 "공사 중"으로 보인다.
pub const READY_KINDS: &[&str] = &["summon"];

#[derive(Debug, Deserialize)]
pub struct Curriculum {
    pub belt: Vec<Belt>,
}

#[derive(Debug, Deserialize)]
pub struct Belt {
    pub id: String,
    pub name: String,
    pub theme: String,
    pub motto: String,
    #[serde(default)]
    pub lesson: Vec<Lesson>,
    pub boss: Option<Lesson>,
}

/// 과 또는 관문(보스). 보스는 `title` 대신 `name`을 쓴다.
#[derive(Debug, Deserialize)]
pub struct Lesson {
    pub id: String,
    #[serde(alias = "name")]
    pub title: String,
    pub kind: String,
    pub set: Option<String>,
    pub why: String,
    #[serde(default)]
    pub teach: Vec<String>,
    #[serde(default)]
    pub needs: Vec<String>,
    pub scroll: Option<String>,
    /// 보스 대사
    pub line: Option<String>,
    /// 보스 제한 시간
    pub time_limit_s: Option<u32>,
}

impl Lesson {
    pub fn is_boss(&self) -> bool {
        self.id.ends_with(".boss")
    }

    pub fn is_ready(&self) -> bool {
        READY_KINDS.contains(&self.kind.as_str())
    }

    /// 이 과가 요구하는 수 (모르는 이름은 콘텐츠 테스트가 막는다)
    pub fn needed_actions(&self) -> Vec<Action> {
        self.needs
            .iter()
            .filter_map(|n| Action::from_id(n))
            .collect()
    }
}

#[derive(Debug, Deserialize)]
pub struct Scrolls {
    pub scroll: Vec<Scroll>,
}

#[derive(Debug, Deserialize)]
pub struct Scroll {
    pub id: String,
    pub title: String,
    pub body: String,
    pub source: String,
    /// 비급 시험 (M2) — 지금은 형식만 테스트로 검증한다
    #[allow(dead_code)]
    pub quiz: Quiz,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Quiz {
    pub q: String,
    pub choices: Vec<String>,
    pub answer: usize,
}

#[derive(Debug, Deserialize)]
pub struct SenseiLines {
    pub lobby: Vec<String>,
    pub raw_jk: Vec<String>,
    pub first_fail: Vec<String>,
    pub fail_streak: Vec<String>,
    pub over_par: Vec<String>,
    pub sage_equal: Vec<String>,
    pub hangul: Vec<String>,
    pub clear: Vec<String>,
    pub locked: Vec<String>,
    pub building: Vec<String>,
}

impl SenseiLines {
    #[cfg(test)]
    fn all(&self) -> impl Iterator<Item = &String> {
        [
            &self.lobby,
            &self.raw_jk,
            &self.first_fail,
            &self.fail_streak,
            &self.over_par,
            &self.sage_equal,
            &self.hangul,
            &self.clear,
            &self.locked,
            &self.building,
        ]
        .into_iter()
        .flatten()
    }
}

#[derive(Debug, Deserialize)]
pub struct Summon {
    pub items: Vec<FixtureItem>,
    pub set: Vec<SummonSet>,
}

#[derive(Debug, Deserialize)]
pub struct FixtureItem {
    pub name: String,
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct SummonSet {
    pub id: String,
    pub requests: Vec<Request>,
}

/// 소환 의뢰 — `target` 항목을 1순위로 올리거나, 입력이 `accept` 중 하나와 같으면 성공
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub ask: String,
    pub target: Option<String>,
    #[serde(default)]
    pub accept: Vec<String>,
}

pub struct Content {
    pub curriculum: Curriculum,
    pub scrolls: Scrolls,
    pub sensei: SenseiLines,
    pub summon: Summon,
}

impl Content {
    fn parse() -> Result<Self, String> {
        Ok(Self {
            curriculum: toml::from_str(CURRICULUM).map_err(|e| format!("curriculum.toml: {e}"))?,
            scrolls: toml::from_str(SCROLLS).map_err(|e| format!("scrolls.toml: {e}"))?,
            sensei: toml::from_str(SENSEI).map_err(|e| format!("sensei.toml: {e}"))?,
            summon: toml::from_str(SUMMON).map_err(|e| format!("summon.toml: {e}"))?,
        })
    }

    pub fn lesson(&self, id: &str) -> Option<(usize, &Belt, &Lesson)> {
        self.curriculum.belt.iter().enumerate().find_map(|(i, b)| {
            b.lesson
                .iter()
                .chain(b.boss.iter())
                .find(|l| l.id == id)
                .map(|l| (i, b, l))
        })
    }

    pub fn scroll(&self, id: &str) -> Option<&Scroll> {
        self.scrolls.scroll.iter().find(|s| s.id == id)
    }

    pub fn summon_set(&self, id: &str) -> Option<&SummonSet> {
        self.summon.set.iter().find(|s| s.id == id)
    }

    /// 과의 총 개수 (보스 제외) — 별 만점 계산용
    pub fn lesson_count(&self) -> usize {
        self.curriculum.belt.iter().map(|b| b.lesson.len()).sum()
    }
}

/// 내장 콘텐츠 (첫 호출 때 한 번 파싱)
pub fn content() -> &'static Content {
    static CONTENT: OnceLock<Content> = OnceLock::new();
    CONTENT.get_or_init(|| Content::parse().unwrap_or_else(|e| panic!("dojo 콘텐츠 오류: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dojo::binding::Bindings;
    use std::collections::BTreeSet;

    fn c() -> &'static Content {
        content()
    }

    /// 콘텐츠 문구에 물리 키 이름이 직접 들어가면 사용자 배치와 어긋난다 (docs/10 §9)
    const RAW_KEY_NAMES: &[&str] = &[
        "CapsLock", "LAlt", "RAlt", "LShift", "Alt+", "Ctrl+", "Cmd+",
    ];

    fn texts_of(l: &Lesson) -> Vec<&str> {
        let mut v = vec![l.why.as_str(), l.title.as_str()];
        v.extend(l.teach.iter().map(String::as_str));
        v.extend(l.line.as_deref());
        v
    }

    #[test]
    fn 모든_콘텐츠가_파싱된다() {
        Content::parse().unwrap();
    }

    #[test]
    fn 과_id는_유일하고_띠_id로_시작한다() {
        let mut seen = BTreeSet::new();
        for b in &c().curriculum.belt {
            for l in b.lesson.iter().chain(b.boss.iter()) {
                assert!(seen.insert(&l.id), "중복 과 id: {}", l.id);
                assert!(
                    l.id.starts_with(&format!("{}.", b.id)),
                    "{} 는 {} 띠 소속이어야",
                    l.id,
                    b.id
                );
            }
            if let Some(boss) = &b.boss {
                assert!(boss.is_boss(), "보스 id 는 .boss 로 끝나야: {}", boss.id);
            }
        }
    }

    #[test]
    fn 비급은_24권이고_참조가_모두_존재한다() {
        let ids: BTreeSet<_> = c().scrolls.scroll.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.len(), 24);
        assert_eq!(c().scrolls.scroll.len(), 24, "비급 id 중복");
        let mut used = BTreeSet::new();
        for b in &c().curriculum.belt {
            for l in b.lesson.iter().chain(b.boss.iter()) {
                if let Some(s) = &l.scroll {
                    assert!(ids.contains(s.as_str()), "{}: 없는 비급 {s}", l.id);
                    assert!(used.insert(s.clone()), "비급 {s} 를 두 과가 나눠 가짐");
                }
            }
        }
        for s in &c().scrolls.scroll {
            assert!(
                s.quiz.answer < s.quiz.choices.len(),
                "卷{} 시험 정답 범위",
                s.id
            );
            assert!(s.quiz.choices.len() >= 2);
        }
    }

    #[test]
    fn 자리표시자는_모두_알려진_수다() {
        for b in &c().curriculum.belt {
            for l in b.lesson.iter().chain(b.boss.iter()) {
                for t in texts_of(l) {
                    assert!(Bindings::unknown_tokens(t).is_empty(), "{}: {t}", l.id);
                }
                for n in &l.needs {
                    assert!(Action::from_id(n).is_some(), "{}: 모르는 수 {n}", l.id);
                }
            }
        }
        for t in c().sensei.all() {
            assert!(Bindings::unknown_tokens(t).is_empty(), "사부 대사: {t}");
        }
    }

    #[test]
    fn 문구에_물리_키_이름을_쓰지_않는다() {
        let check = |t: &str, ctx: &str| {
            for k in RAW_KEY_NAMES {
                assert!(
                    !t.contains(k),
                    "{ctx}: 물리 키 이름 {k} 대신 자리표시자를 써라 — {t}"
                );
            }
        };
        for b in &c().curriculum.belt {
            check(&b.motto, &b.id);
            for l in b.lesson.iter().chain(b.boss.iter()) {
                for t in texts_of(l) {
                    check(t, &l.id);
                }
            }
        }
        for t in c().sensei.all() {
            check(t, "sensei");
        }
    }

    #[test]
    fn 소환_과는_있는_세트와_있는_항목을_가리킨다() {
        let names: BTreeSet<_> = c().summon.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names.len(), c().summon.items.len(), "연습 목록 이름 중복");
        for i in &c().summon.items {
            assert!(
                ["app", "file", "dir"].contains(&i.kind.as_str()),
                "{}: kind",
                i.name
            );
            assert!(
                i.name.is_ascii(),
                "{}: 연습 목록 이름은 영문 (타수 공정성)",
                i.name
            );
        }
        for b in &c().curriculum.belt {
            for l in b
                .lesson
                .iter()
                .chain(b.boss.iter())
                .filter(|l| l.kind == "summon")
            {
                let set = l
                    .set
                    .as_deref()
                    .unwrap_or_else(|| panic!("{}: set 없음", l.id));
                let set = c()
                    .summon_set(set)
                    .unwrap_or_else(|| panic!("{}: 없는 세트", l.id));
                assert!(!set.requests.is_empty());
                for r in &set.requests {
                    match &r.target {
                        Some(t) => assert!(names.contains(t.as_str()), "{}: 없는 항목 {t}", l.id),
                        None => assert!(!r.accept.is_empty(), "{}: target 도 accept 도 없음", l.id),
                    }
                }
            }
        }
    }

    #[test]
    fn 흰띠는_전부_수련_가능하다() {
        let white = &c().curriculum.belt[0];
        assert_eq!(white.id, "white");
        assert!(white.lesson.iter().all(Lesson::is_ready));
        assert!(white.boss.as_ref().is_some_and(Lesson::is_ready));
    }
}
