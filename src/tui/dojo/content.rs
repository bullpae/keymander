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
const GOLF: &str = include_str!("../../../assets/dojo/golf.toml");
const MAZE: &str = include_str!("../../../assets/dojo/maze.toml");
const IME: &str = include_str!("../../../assets/dojo/ime.toml");

/// 구현된 수련 종류. 나머지는 지도에 "공사 중"으로 보인다.
pub const READY_KINDS: &[&str] = &[
    "summon", "spar", "maze", "golf", "mouse", "ime", "mixed", "quiz", "daily", "seal",
];

/// 이어 붙이기(mixed)의 단계로 쓸 수 있는 종류 — 화면 하나로 끝나는 라운드
pub const STAGE_KINDS: &[&str] = &["summon", "spar", "maze", "golf", "mouse", "ime"];

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
#[derive(Debug, Clone, Default, Deserialize)]
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
    /// 대련 꼴 — `card`(기본) · `rain`
    pub style: Option<String>,
    /// 문항 수 (대련 카드·화살·과녁)
    pub count: Option<u32>,
    /// 화살비 박자 간격
    pub speed_ms: Option<u32>,
    /// 화살이 떨어지는 시간
    pub fall_ms: Option<u32>,
    /// 마우스 과의 기준 시간
    pub par_time_s: Option<u32>,
    /// 세트에서 앞의 몇 문항을 건너뛰고(`skip`) 몇 문항만(`take`) 쓴다 — 단계·오늘의 수련용
    pub skip: Option<usize>,
    pub take: Option<usize>,
    /// 대련 카드의 수 묶음 (없으면 상하좌우)
    #[serde(default)]
    pub moves: Vec<String>,
    /// 이어 붙이기(mixed)의 단계
    #[serde(default)]
    pub stage: Vec<Stage>,
    /// 연속 수련일 조건 (검은띠)
    pub days: Option<u32>,
    /// 인장 수 조건 (단)
    pub seals: Option<u32>,
}

/// 이어 붙이기의 한 단계 — 다른 과의 라운드를 줄여서 빌려 쓴다
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Stage {
    /// 빌려 올 과 id
    pub from: String,
    /// 단계 이름 (없으면 빌려 온 과 제목)
    pub label: Option<String>,
    pub skip: Option<usize>,
    pub take: Option<usize>,
    pub count: Option<u32>,
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

    /// 세트에서 이 과가 쓰는 구간 — `skip`·`take`
    pub fn window<T: Clone>(&self, items: &[T]) -> Vec<T> {
        let skip = self.skip.unwrap_or(0).min(items.len().saturating_sub(1));
        items
            .iter()
            .skip(skip)
            .take(self.take.unwrap_or(usize::MAX).max(1))
            .cloned()
            .collect()
    }

    /// 단계로 빌려 쓸 과를 만든다 — 제한 시간은 떼고 구간만 남긴다. id 는 원래 과
    /// 그대로라 화면이 제목·띠를 찾을 수 있다. 관문은 빌려 쓰지 않는다(콘텐츠 테스트).
    pub fn staged(&self, st: &Stage) -> Lesson {
        let mut l = self.clone();
        l.time_limit_s = None;
        l.skip = st.skip.or(self.skip);
        l.take = st.take.or(self.take);
        l.count = st.count.or(self.count);
        if let Some(label) = &st.label {
            l.title = label.clone();
        }
        l
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
    /// 비급 시험 (S16)
    pub quiz: Quiz,
}

#[derive(Debug, Clone, Deserialize)]
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
    /// 콤보 10 단위
    #[serde(default)]
    pub combo: Vec<String>,
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
            &self.combo,
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

#[derive(Debug, Deserialize)]
pub struct Golf {
    pub course: Vec<GolfCourse>,
}

/// 키 골프 코스 — 홀 여러 개
#[derive(Debug, Deserialize)]
pub struct GolfCourse {
    pub id: String,
    pub name: String,
    /// 홀 기본 허용 수
    pub moves: Vec<String>,
    /// 페이지 이동 줄 수
    pub page: Option<usize>,
    pub hole: Vec<GolfHole>,
}

/// 홀 — `text` 표기는 [`super::golf::Hole::parse`]
#[derive(Debug, Clone, Deserialize)]
pub struct GolfHole {
    pub title: String,
    pub text: String,
    pub moves: Option<Vec<String>>,
    pub par: Option<u32>,
    pub page: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct Mazes {
    pub set: Vec<MazeSet>,
}

#[derive(Debug, Deserialize)]
pub struct MazeSet {
    pub id: String,
    pub maze: Vec<MazeDef>,
}

/// 미로 — `#` 벽, `S` 시작, `E` 문, `$` 엽전
#[derive(Debug, Clone, Deserialize)]
pub struct MazeDef {
    pub title: String,
    pub map: String,
    #[serde(default)]
    pub fog: bool,
}

#[derive(Debug, Deserialize)]
pub struct Ime {
    pub set: Vec<ImeSet>,
}

/// 한영 리듬 문장 묶음 (docs/10 §4-3 S11)
#[derive(Debug, Deserialize)]
pub struct ImeSet {
    pub id: String,
    /// 줄 사이에 마우스 과녁을 둔다 — 쥠(레이어)이 한/영을 건드리지 않는지 본다
    #[serde(default)]
    pub click: bool,
    pub lines: Vec<String>,
}

pub struct Content {
    pub curriculum: Curriculum,
    pub scrolls: Scrolls,
    pub sensei: SenseiLines,
    pub summon: Summon,
    pub golf: Golf,
    pub maze: Mazes,
    pub ime: Ime,
}

impl Content {
    fn parse() -> Result<Self, String> {
        let mut c = Self::parse_raw()?;
        // 이어 붙이기는 단계들이 요구하는 수를 모두 요구한다
        let mut fill = Vec::new();
        for (bi, b) in c.curriculum.belt.iter().enumerate() {
            for (li, l) in b.lesson.iter().chain(b.boss.iter()).enumerate() {
                if l.stage.is_empty() || !l.needs.is_empty() {
                    continue;
                }
                let mut needs: Vec<String> = Vec::new();
                for st in &l.stage {
                    if let Some((_, _, src)) = c.lesson(&st.from) {
                        for n in &src.needs {
                            if !needs.contains(n) {
                                needs.push(n.clone());
                            }
                        }
                    }
                }
                fill.push((bi, li, needs));
            }
        }
        for (bi, li, needs) in fill {
            let b = &mut c.curriculum.belt[bi];
            let n = b.lesson.len();
            let l = if li < n {
                &mut b.lesson[li]
            } else {
                b.boss.as_mut().expect("보스")
            };
            l.needs = needs;
        }
        Ok(c)
    }

    fn parse_raw() -> Result<Self, String> {
        Ok(Self {
            curriculum: toml::from_str(CURRICULUM).map_err(|e| format!("curriculum.toml: {e}"))?,
            scrolls: toml::from_str(SCROLLS).map_err(|e| format!("scrolls.toml: {e}"))?,
            sensei: toml::from_str(SENSEI).map_err(|e| format!("sensei.toml: {e}"))?,
            summon: toml::from_str(SUMMON).map_err(|e| format!("summon.toml: {e}"))?,
            golf: toml::from_str(GOLF).map_err(|e| format!("golf.toml: {e}"))?,
            maze: toml::from_str(MAZE).map_err(|e| format!("maze.toml: {e}"))?,
            ime: toml::from_str(IME).map_err(|e| format!("ime.toml: {e}"))?,
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

    pub fn golf_course(&self, id: &str) -> Option<&GolfCourse> {
        self.golf.course.iter().find(|c| c.id == id)
    }

    pub fn maze_set(&self, id: &str) -> Option<&MazeSet> {
        self.maze.set.iter().find(|s| s.id == id)
    }

    pub fn ime_set(&self, id: &str) -> Option<&ImeSet> {
        self.ime.set.iter().find(|s| s.id == id)
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
    fn 구현된_과는_있는_코스와_세트를_가리킨다() {
        for b in &c().curriculum.belt {
            for l in b
                .lesson
                .iter()
                .chain(b.boss.iter())
                .filter(|l| l.is_ready())
            {
                let set = l.set.as_deref();
                match l.kind.as_str() {
                    "golf" => assert!(
                        set.and_then(|s| c().golf_course(s)).is_some(),
                        "{}: 없는 코스",
                        l.id
                    ),
                    "maze" => assert!(
                        set.and_then(|s| c().maze_set(s)).is_some(),
                        "{}: 없는 미로 세트",
                        l.id
                    ),
                    "mouse" => assert!(
                        ["move", "aim", "wheel", "drag", "leak", "rats"]
                            .contains(&set.unwrap_or("")),
                        "{}: 모르는 마우스 수련",
                        l.id
                    ),
                    "spar" => assert!(
                        matches!(l.style.as_deref(), None | Some("card") | Some("rain")),
                        "{}: 모르는 대련 꼴",
                        l.id
                    ),
                    "ime" => assert!(
                        set.and_then(|s| c().ime_set(s)).is_some(),
                        "{}: 없는 문장 묶음",
                        l.id
                    ),
                    _ => {}
                }
            }
        }
    }

    /// 한영 리듬 문장은 한/영이 실제로 바뀌어야 수련이 된다
    #[test]
    fn 한영_문장은_전환을_품는다() {
        use super::super::hangul::is_hangul;
        for set in &c().ime.set {
            for line in &set.lines {
                let switches = line
                    .chars()
                    .filter(|ch| ch.is_alphabetic())
                    .map(is_hangul)
                    .collect::<Vec<_>>()
                    .windows(2)
                    .filter(|w| w[0] != w[1])
                    .count();
                // 쥠 묶음은 일부러 전환 없는 줄을 둔다 — 쥠이 한/영을 건드렸는지 보려고
                assert!(
                    switches >= 1 || set.click,
                    "{}: 전환이 없다 — {line}",
                    set.id
                );
                assert!(
                    line.chars().count() <= 40,
                    "{}: 한 줄이 길다 — {line}",
                    set.id
                );
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

    /// 이어 붙이기의 단계는 화면 하나로 끝나는 과만, 관문은 빌리지 않는다
    #[test]
    fn 이어_붙이기의_단계는_있는_과를_빌린다() {
        let mut n = 0;
        for b in &c().curriculum.belt {
            for l in b.lesson.iter().chain(b.boss.iter()) {
                if l.kind != "mixed" {
                    assert!(l.stage.is_empty(), "{}: mixed 가 아닌데 단계가 있다", l.id);
                    continue;
                }
                assert!(l.stage.len() >= 2, "{}: 단계가 하나뿐", l.id);
                assert!(!l.needs.is_empty(), "{}: 단계의 수를 물려받는다", l.id);
                for st in &l.stage {
                    let (_, _, src) = c()
                        .lesson(&st.from)
                        .unwrap_or_else(|| panic!("{}: 없는 과 {}", l.id, st.from));
                    assert!(
                        !src.is_boss(),
                        "{}: 관문 {} 은 빌리지 않는다",
                        l.id,
                        st.from
                    );
                    assert!(
                        STAGE_KINDS.contains(&src.kind.as_str()),
                        "{}: {} 는 단계가 될 수 없다",
                        l.id,
                        st.from
                    );
                    n += 1;
                }
            }
        }
        assert!(n > 0);
    }

    #[test]
    fn 구간은_세트를_넘지_않는다() {
        let l = Lesson {
            skip: Some(10),
            take: Some(2),
            ..Default::default()
        };
        assert_eq!(l.window(&[1, 2, 3]), vec![3], "넘치는 skip 은 마지막 문항");
        let l = Lesson {
            skip: Some(1),
            take: Some(1),
            ..Default::default()
        };
        assert_eq!(l.window(&[1, 2, 3]), vec![2]);
        assert_eq!(Lesson::default().window(&[1, 2]), vec![1, 2]);
    }
}
