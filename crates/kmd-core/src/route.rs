//! `:길` / `:route` — 카카오맵 길찾기로 넘기기 (docs/20).
//!
//! 경로 계산과 화면은 카카오맵 웹이 맡는다. 런처는 출발지·도착지 좌표를 찾아
//! `map.kakao.com/link/by/<수단>/이름,위도,경도/이름,위도,경도` 링크를 만들 뿐이다.
//! 카카오맵 웹은 이름만으로는 경로를 열지 않으므로 좌표가 꼭 필요하다:
//!
//! 1. config `[launcher.route.places]` 에 적어 둔 장소(집·회사)는 바로 링크가 된다.
//! 2. 처음 가는 곳은 카카오 REST 키가 있으면 장소 검색 API로 후보를 찾는다
//!    ([`search_places`]). 키는 NextMove 와 같은 `~/.config/nextmove/kakao.env`.
//! 3. 키가 없으면 카카오맵 장소 검색 화면을 연다.
//!
//! 카카오 약관은 응답을 저장·캐시하지 못하게 한다 — 검색 결과는 화면에만 두고,
//! 항목을 열어도 URL 열기라 실행 이력에 남지 않는다.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::index::{IndexItem, ItemKind, Source};
use crate::process::HideConsole;

pub const ROUTE_ALIASES: &[&str] = &[":route", ":길"];

/// 키를 찾는 환경 변수 이름 (NextMove 와 같다)
pub const KEY_ENV: &str = "KAKAO_REST_API_KEY";

/// 현재 위치를 뜻하는 낱말 — `:길 여기 → 판교역`, `:길 회사 → 여기`
const HERE_WORDS: &[&str] = &["여기", "현재위치", "현재 위치", "현위치", "here"];

/// 장소 검색 후보 수
const SEARCH_SIZE: u32 = 7;

/// `[launcher.route]` — 길찾기 설정.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct RouteConfig {
    /// 출발지를 안 적었을 때 현재 위치(OS 위치 서비스, 데몬 경유)에서 출발한다.
    /// 위치가 고정된 데스크톱은 꺼 두고 `origin` 만 쓴다.
    pub use_location: bool,
    /// 현재 위치를 못 얻었거나 `use_location = false` 일 때의 출발지 (`places` 의 키).
    /// 없으면 카카오맵에서 고른다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// 이름 → `"위도,경도"`. 예: `집 = "37.4979,127.0276"`
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub places: BTreeMap<String, String>,
    /// 카카오 REST 키 파일 (`KAKAO_REST_API_KEY=...` 줄). 기본 `~/.config/nextmove/kakao.env`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kakao_key_file: Option<PathBuf>,
}

impl Default for RouteConfig {
    fn default() -> Self {
        Self {
            use_location: true,
            origin: None,
            places: BTreeMap::new(),
            kakao_key_file: None,
        }
    }
}

/// 이동 수단 — 카카오맵 링크의 `by/<slug>`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Transit,
    Car,
    Walk,
    Bike,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Transit, Mode::Car, Mode::Walk, Mode::Bike];

    fn slug(self) -> &'static str {
        match self {
            Mode::Transit => "traffic",
            Mode::Car => "car",
            Mode::Walk => "walk",
            Mode::Bike => "bicycle",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Transit => "대중교통",
            Mode::Car => "자동차",
            Mode::Walk => "도보",
            Mode::Bike => "자전거",
        }
    }

    fn icon(self, use_emoji: bool) -> &'static str {
        match (self, use_emoji) {
            (Mode::Transit, true) => "\u{1F687}",
            (Mode::Car, true) => "\u{1F697}",
            (Mode::Walk, true) => "\u{1F6B6}",
            (Mode::Bike, true) => "\u{1F6B2}",
            (Mode::Transit, false) => "[BUS]",
            (Mode::Car, false) => "[CAR]",
            (Mode::Walk, false) => "[WLK]",
            (Mode::Bike, false) => "[BIK]",
        }
    }

    fn from_word(w: &str) -> Option<Mode> {
        match w.to_lowercase().as_str() {
            "대중교통" | "버스" | "지하철" | "transit" | "bus" => Some(Mode::Transit),
            "차" | "자동차" | "자가용" | "car" | "drive" => Some(Mode::Car),
            "도보" | "걸어" | "걷기" | "walk" => Some(Mode::Walk),
            "자전거" | "bike" | "bicycle" => Some(Mode::Bike),
            _ => None,
        }
    }
}

/// 좌표가 있는 장소
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lng: f64,
    /// 목록 둘째 줄 — 주소·거리
    pub detail: String,
}

/// 출발지·도착지 한쪽
#[derive(Debug, Clone, PartialEq)]
pub enum End {
    /// 안 적음
    None,
    /// config 에 있는 장소
    Known(Place),
    /// 좌표를 찾아야 하는 이름
    Text(String),
    /// 현재 위치 — 데몬에게 물어 [`Plan::with_here`] 로 바꾼다
    Here,
}

/// `:길` 입력을 푼 것
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// 적지 않았으면 None — 대중교통으로 연다
    pub mode: Option<Mode>,
    pub from: End,
    pub to: End,
}

impl Plan {
    /// 장소 검색이 필요한가
    pub fn needs_lookup(&self) -> bool {
        matches!(self.to, End::Text(_)) || matches!(self.from, End::Text(_))
    }

    /// 현재 위치가 필요한가
    pub fn needs_here(&self) -> bool {
        self.from == End::Here || self.to == End::Here
    }

    /// 현재 위치를 채운다. 못 얻었으면(`None`) 출발지는 `origin` 으로, 도착지는 비운다.
    pub fn with_here(mut self, cfg: &RouteConfig, here: Option<Place>) -> Plan {
        if self.from == End::Here {
            self.from = match &here {
                Some(p) => End::Known(p.clone()),
                None => cfg.origin(),
            };
        }
        if self.to == End::Here {
            self.to = here.map_or(End::None, End::Known);
        }
        self
    }

    fn mode(&self) -> Mode {
        self.mode.unwrap_or(Mode::Transit)
    }
}

/// `"37.49,127.02"` → (위도, 경도). 한국 밖 좌표도 받지만 순서가 뒤집힌 것은 고친다.
fn parse_coords(s: &str) -> Option<(f64, f64)> {
    let (a, b) = s.split_once(',')?;
    let a: f64 = a.trim().parse().ok()?;
    let b: f64 = b.trim().parse().ok()?;
    if !(a.is_finite() && b.is_finite()) {
        return None;
    }
    // 위도는 ±90 안 — 경도를 먼저 적었으면 바꾼다
    if a.abs() > 90.0 && b.abs() <= 90.0 {
        Some((b, a))
    } else {
        Some((a, b))
    }
}

impl RouteConfig {
    /// config 에 적힌 장소 (대소문자·앞뒤 공백 무시)
    pub fn place(&self, name: &str) -> Option<Place> {
        let name = name.trim();
        let (key, value) = self
            .places
            .iter()
            .find(|(k, _)| k.trim().eq_ignore_ascii_case(name))?;
        let (lat, lng) = parse_coords(value)?;
        Some(Place {
            name: key.trim().to_string(),
            lat,
            lng,
            detail: "내 장소".to_string(),
        })
    }

    fn end(&self, text: &str) -> End {
        let text = text.trim();
        if text.is_empty() {
            End::None
        } else if HERE_WORDS.contains(&text.to_lowercase().as_str()) {
            End::Here
        } else if let Some(p) = self.place(text) {
            End::Known(p)
        } else {
            End::Text(text.to_string())
        }
    }

    /// 기본 출발지
    fn origin(&self) -> End {
        match &self.origin {
            Some(name) => self.end(name),
            None => End::None,
        }
    }

    /// 카카오 REST 키 — 환경 변수가 먼저, 그다음 키 파일
    pub fn kakao_key(&self) -> Option<String> {
        if let Some(k) = std::env::var(KEY_ENV).ok().and_then(|k| valid_key(&k)) {
            return Some(k);
        }
        let path = self
            .kakao_key_file
            .clone()
            .or_else(|| dirs::home_dir().map(|h| h.join(".config/nextmove/kakao.env")))?;
        key_from_env_file(&std::fs::read_to_string(path).ok()?)
    }
}

/// 키는 영숫자만 — curl 설정 줄에 그대로 넣으므로 따옴표·줄바꿈을 막는다
fn valid_key(k: &str) -> Option<String> {
    let k = k.trim().trim_matches(['"', '\'']);
    (!k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric())).then(|| k.to_string())
}

/// `KAKAO_REST_API_KEY=...` 줄을 찾는다 (`export ` 와 따옴표 허용)
fn key_from_env_file(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (k, v) = line.split_once('=')?;
        (k.trim() == KEY_ENV).then(|| valid_key(v)).flatten()
    })
}

/// 입력에서 명령 별칭을 뗀 나머지
fn strip_alias(query: &str) -> &str {
    ROUTE_ALIASES
        .iter()
        .find_map(|a| query.strip_prefix(a))
        .unwrap_or("")
        .trim()
}

/// `:길 [수단] [출발 → ]도착` 을 푼다.
///
/// 출발과 도착 구분: `→`, `->`, `>`, 또는 "회사에서 판교역" 처럼 `에서 `.
pub fn parse(cfg: &RouteConfig, query: &str) -> Plan {
    let mut rest = strip_alias(query);
    let mut mode = None;
    if let Some((first, tail)) = rest.split_once(' ') {
        if let Some(m) = Mode::from_word(first) {
            mode = Some(m);
            rest = tail.trim();
        }
    } else if let Some(m) = Mode::from_word(rest) {
        mode = Some(m);
        rest = "";
    }

    let split = ["→", "->", ">"]
        .iter()
        .find_map(|sep| rest.split_once(sep))
        .or_else(|| rest.split_once("에서 "));
    let (from, to) = match split {
        Some((f, t)) => (cfg.end(f), cfg.end(t)),
        None if cfg.use_location => (End::Here, cfg.end(rest)),
        None => (cfg.origin(), cfg.end(rest)),
    };
    Plan { mode, from, to }
}

/// 링크 경로 조각 — 쉼표·슬래시는 카카오 링크의 구분자라 공백으로 바꾼다
fn segment(p: &Place) -> String {
    let name: String = p
        .name
        .chars()
        .map(|c| if matches!(c, ',' | '/') { ' ' } else { c })
        .collect();
    let mut enc = String::new();
    for b in name.trim().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                enc.push(b as char)
            }
            _ => enc.push_str(&format!("%{b:02X}")),
        }
    }
    format!("{enc},{:.6},{:.6}", p.lat, p.lng)
}

/// 카카오맵 길찾기 링크. 출발지가 없으면 도착지만 채운 길찾기 화면(자동차 탭)을 연다.
pub fn route_url(mode: Mode, from: Option<&Place>, to: &Place) -> String {
    match from {
        Some(f) => format!(
            "https://map.kakao.com/link/by/{}/{}/{}",
            mode.slug(),
            segment(f),
            segment(to)
        ),
        None => format!("https://map.kakao.com/link/to/{}", segment(to)),
    }
}

/// 카카오맵 장소 검색 링크
pub fn search_url(text: &str) -> String {
    format!(
        "https://map.kakao.com/link/search/{}",
        crate::web::url_encode(text.trim()).replace('+', "%20")
    )
}

fn item(name: String, detail: String, icon: &str, url: Option<String>) -> IndexItem {
    IndexItem {
        name,
        path: detail,
        icon: icon.to_string(),
        kind: if url.is_some() {
            ItemKind::WebSearch
        } else {
            ItemKind::SystemCommand
        },
        source: Source::Plugin,
        keywords: url.unwrap_or_else(|| "kmd:route:info".to_string()),
        icon_path: None,
    }
}

fn route_item(mode: Mode, from: Option<&Place>, to: &Place, use_emoji: bool) -> IndexItem {
    let title = match from {
        Some(f) => format!("{} → {}  ({})", f.name, to.name, mode.label()),
        None => format!("→ {}  (카카오맵에서 출발지 선택)", to.name),
    };
    item(
        title,
        to.detail.clone(),
        mode.icon(use_emoji),
        Some(route_url(mode, from, to)),
    )
}

/// 카카오맵 장소 검색 화면 항목
pub fn search_item(text: &str, use_emoji: bool) -> IndexItem {
    item(
        format!("카카오맵에서 '{}' 검색", text.trim()),
        "검색 결과에서 '도착'을 누르면 길찾기가 열립니다".to_string(),
        if use_emoji {
            "\u{1F5FA}\u{FE0F}"
        } else {
            "[MAP]"
        },
        Some(search_url(text)),
    )
}

/// 안내 항목 (열 것 없음)
pub fn notice(title: &str, detail: &str, use_emoji: bool) -> IndexItem {
    item(
        title.to_string(),
        detail.to_string(),
        if use_emoji { "\u{2139}\u{FE0F}" } else { "[i]" },
        None,
    )
}

/// 데몬이 준 현재 위치 → 출발지 장소
pub fn here_place(lat: f64, lng: f64, accuracy_m: f64, age_secs: u64) -> Place {
    let mut detail = String::from("현재 위치");
    if accuracy_m > 0.0 {
        detail.push_str(&format!(" ±{accuracy_m:.0}m"));
    }
    if age_secs >= 120 {
        detail.push_str(&format!(" ({}분 전 측정)", age_secs / 60));
    }
    Place {
        name: "현재 위치".to_string(),
        lat,
        lng,
        detail,
    }
}

fn known(end: &End) -> Option<&Place> {
    match end {
        End::Known(p) => Some(p),
        _ => None,
    }
}

/// 네트워크 없이 낼 수 있는 항목.
///
/// 도착지까지 좌표가 있으면 바로 길찾기 항목이 나오고, 찾아야 하면 장소 검색
/// 링크가 나온다 (키가 있으면 호출부가 [`lookup_items`] 를 이어 부른다).
/// 현재 위치 없이(TUI·데스크톱의 첫 화면) — 출발지는 `origin` 으로 대신한다.
pub fn items(cfg: &RouteConfig, query: &str, use_emoji: bool) -> Vec<IndexItem> {
    items_for(cfg, &parse(cfg, query).with_here(cfg, None), use_emoji)
}

/// 현재 위치까지 채운 [`Plan`] 의 항목.
pub fn items_for(cfg: &RouteConfig, plan: &Plan, use_emoji: bool) -> Vec<IndexItem> {
    let from = known(&plan.from);
    match &plan.to {
        End::None | End::Here => {
            let mut out = vec![notice(
                ":길 [대중교통|차|도보|자전거] [출발 →] 도착",
                "예: :길 판교역   :길 차 회사 → 강남역   (카카오맵 길찾기를 엽니다)",
                use_emoji,
            )];
            out.extend(
                cfg.places
                    .keys()
                    .filter_map(|k| cfg.place(k))
                    .filter(|p| from.is_none_or(|f| f.name != p.name))
                    .map(|p| route_item(plan.mode(), from, &p, use_emoji)),
            );
            if cfg.places.is_empty() {
                out.push(notice(
                    "자주 가는 곳을 config 에 적어 두세요",
                    "[launcher.route] origin = \"집\"   [launcher.route.places] 집 = \"37.4979,127.0276\"",
                    use_emoji,
                ));
            }
            out
        }
        End::Known(to) if !matches!(plan.from, End::Text(_)) => match plan.mode {
            Some(m) => vec![route_item(m, from, to, use_emoji)],
            None if from.is_some() => Mode::ALL
                .iter()
                .map(|&m| route_item(m, from, to, use_emoji))
                .collect(),
            None => vec![route_item(Mode::Transit, None, to, use_emoji)],
        },
        End::Known(_) | End::Text(_) => {
            let text = match (&plan.from, &plan.to) {
                (_, End::Text(t)) => t.clone(),
                (End::Text(t), _) => t.clone(),
                _ => unreachable!(),
            };
            vec![search_item(&text, use_emoji)]
        }
    }
}

/// 장소 검색 API 를 거친 항목 — 도착지 후보마다 길찾기 한 줄.
/// 블로킹(curl)이므로 GUI 스레드에서 부르지 말 것.
pub fn lookup_items(key: &str, plan: &Plan, use_emoji: bool) -> Result<Vec<IndexItem>, String> {
    let mode = plan.mode();
    let from = match &plan.from {
        End::Known(p) => Some(p.clone()),
        End::None | End::Here => None,
        End::Text(t) => Some(
            search_places(key, t, None)?
                .into_iter()
                .next()
                .ok_or_else(|| format!("출발지 '{t}' 를 찾지 못했습니다"))?,
        ),
    };
    let mut out = match &plan.to {
        End::Known(to) => vec![route_item(mode, from.as_ref(), to, use_emoji)],
        End::Text(t) => {
            let found = search_places(key, t, from.as_ref())?;
            if found.is_empty() {
                vec![notice(
                    &format!("'{t}' 와 맞는 장소가 없습니다"),
                    "이름을 바꿔 보거나 아래 카카오맵 검색을 쓰세요",
                    use_emoji,
                )]
            } else {
                found
                    .iter()
                    .map(|to| route_item(mode, from.as_ref(), to, use_emoji))
                    .collect()
            }
        }
        End::None | End::Here => Vec::new(),
    };
    if let End::Text(t) = &plan.to {
        out.push(search_item(t, use_emoji));
    }
    Ok(out)
}

#[derive(Deserialize)]
struct KeywordResponse {
    #[serde(default)]
    documents: Vec<KeywordDoc>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct KeywordDoc {
    place_name: String,
    #[serde(default)]
    road_address_name: String,
    #[serde(default)]
    address_name: String,
    #[serde(default)]
    category_group_name: String,
    #[serde(default)]
    distance: String,
    x: String,
    y: String,
}

/// 응답 JSON → 장소 목록
fn parse_keyword_response(body: &str) -> Result<Vec<Place>, String> {
    let resp: KeywordResponse =
        serde_json::from_str(body).map_err(|e| format!("카카오 응답을 읽지 못했습니다: {e}"))?;
    if resp.documents.is_empty() {
        if let Some(msg) = resp.message {
            return Err(format!("카카오 장소 검색 실패: {msg}"));
        }
    }
    Ok(resp
        .documents
        .into_iter()
        .filter_map(|d| {
            let lng: f64 = d.x.parse().ok()?;
            let lat: f64 = d.y.parse().ok()?;
            let addr = if d.road_address_name.is_empty() {
                d.address_name
            } else {
                d.road_address_name
            };
            let mut detail: Vec<String> = Vec::new();
            if !d.category_group_name.is_empty() {
                detail.push(d.category_group_name);
            }
            detail.push(addr);
            if let Ok(m) = d.distance.parse::<f64>() {
                detail.push(if m >= 1000.0 {
                    format!("{:.1}km", m / 1000.0)
                } else {
                    format!("{m:.0}m")
                });
            }
            Some(Place {
                name: d.place_name,
                lat,
                lng,
                detail: detail.join(" · "),
            })
        })
        .collect())
}

/// 카카오 로컬 키워드 검색. `near` 가 있으면 거리도 받는다.
///
/// HTTP 라이브러리 대신 OS 기본 `curl` 을 쓴다 (macOS·Windows 10+ 기본 탑재).
/// 키는 명령줄(프로세스 목록에 보임) 대신 표준 입력의 curl 설정으로 넘긴다.
pub fn search_places(key: &str, text: &str, near: Option<&Place>) -> Result<Vec<Place>, String> {
    let key = valid_key(key).ok_or("카카오 키 형식이 잘못됐습니다")?;
    let mut cmd = Command::new("curl");
    cmd.hide_console();
    cmd.args([
        "-sS",
        "--max-time",
        "4",
        "-G",
        "https://dapi.kakao.com/v2/local/search/keyword.json",
        "--data-urlencode",
        &format!("query={}", text.trim()),
        "-d",
        &format!("size={SEARCH_SIZE}"),
        "-K",
        "-",
    ]);
    if let Some(p) = near {
        cmd.args([
            "-d",
            &format!("x={:.6}", p.lng),
            "-d",
            &format!("y={:.6}", p.lat),
        ]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl 을 실행하지 못했습니다: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "header = \"Authorization: KakaoAK {key}\"");
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("curl 실패: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("카카오에 연결하지 못했습니다: {}", err.trim()));
    }
    parse_keyword_response(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RouteConfig {
        RouteConfig {
            use_location: false,
            origin: Some("집".into()),
            places: BTreeMap::from([
                ("집".into(), "37.4979, 127.0276".into()),
                ("회사".into(), "127.1112,37.3948".into()),
            ]),
            kakao_key_file: None,
        }
    }

    #[test]
    fn 입력을_수단_출발_도착으로_푼다() {
        let c = cfg();
        let p = parse(&c, ":길 판교역");
        assert_eq!(p.mode, None);
        assert!(matches!(p.from, End::Known(ref h) if h.name == "집"));
        assert_eq!(p.to, End::Text("판교역".into()));

        let p = parse(&c, ":길 차 회사 → 강남역");
        assert_eq!(p.mode, Some(Mode::Car));
        assert!(matches!(p.from, End::Known(ref h) if h.name == "회사"));
        assert_eq!(p.to, End::Text("강남역".into()));

        let p = parse(&c, ":route 회사에서 집");
        assert!(matches!(p.to, End::Known(ref h) if h.name == "집"));
        assert!(!p.needs_lookup());

        let p = parse(&c, ":길 서울역 -> 회사");
        assert_eq!(p.from, End::Text("서울역".into()));
        assert!(p.needs_lookup());

        assert_eq!(parse(&c, ":길 도보").mode, Some(Mode::Walk));
        assert_eq!(parse(&c, ":길").to, End::None);
    }

    #[test]
    fn 기본_출발지는_현재_위치이고_못_얻으면_origin() {
        let c = RouteConfig {
            use_location: true,
            ..cfg()
        };
        let p = parse(&c, ":길 회사");
        assert_eq!(p.from, End::Here);
        assert!(p.needs_here());

        let here = here_place(37.5, 127.0, 42.0, 0);
        let got = p.clone().with_here(&c, Some(here.clone()));
        assert_eq!(got.from, End::Known(here));
        let i = items_for(&c, &got, true);
        assert!(i[0].name.starts_with("현재 위치 → 회사"));
        assert!(i[0].keywords.contains("37.500000,127.000000"));

        // 위치를 못 얻으면 origin(집)
        let fallback = p.with_here(&c, None);
        assert!(matches!(fallback.from, End::Known(ref h) if h.name == "집"));

        // 낱말로도 부른다
        assert_eq!(parse(&cfg(), ":길 회사 → 여기").to, End::Here);
        assert_eq!(
            here_place(1.0, 2.0, 0.0, 300).detail,
            "현재 위치 (5분 전 측정)"
        );
    }

    #[test]
    fn 경도를_먼저_적어도_고친다() {
        let p = cfg().place("회사").unwrap();
        assert!((p.lat - 37.3948).abs() < 1e-9 && (p.lng - 127.1112).abs() < 1e-9);
    }

    #[test]
    fn 링크는_좌표와_인코딩된_이름을_쓴다() {
        let c = cfg();
        let (h, w) = (c.place("집").unwrap(), c.place("회사").unwrap());
        assert_eq!(
            route_url(Mode::Transit, Some(&h), &w),
            "https://map.kakao.com/link/by/traffic/%EC%A7%91,37.497900,127.027600/%ED%9A%8C%EC%82%AC,37.394800,127.111200"
        );
        let odd = Place {
            name: "A,B/C".into(),
            ..w.clone()
        };
        assert!(route_url(Mode::Car, None, &odd)
            .starts_with("https://map.kakao.com/link/to/A%20B%20C,37.394800"));
        assert_eq!(
            search_url("판교 역"),
            "https://map.kakao.com/link/search/%ED%8C%90%EA%B5%90%20%EC%97%AD"
        );
    }

    #[test]
    fn 아는_장소끼리는_수단별로_바로_열린다() {
        let c = cfg();
        let all = items(&c, ":길 회사", true);
        assert_eq!(all.len(), 4);
        assert!(all.iter().all(|i| i.kind == ItemKind::WebSearch));
        assert!(all[0].keywords.contains("/by/traffic/"));
        assert!(all[1].keywords.contains("/by/car/"));

        let one = items(&c, ":길 자전거 회사", true);
        assert_eq!(one.len(), 1);
        assert!(one[0].keywords.contains("/by/bicycle/"));

        // 처음 가는 곳은 (키 없이) 장소 검색 링크
        let s = items(&c, ":길 판교역", true);
        assert_eq!(s.len(), 1);
        assert!(s[0].keywords.contains("/link/search/"));

        // 빈 입력은 사용법 + 내 장소 (출발지 자신은 뺀다)
        let help = items(&c, ":길", true);
        assert_eq!(help.len(), 2);
        assert!(help[1].name.contains("회사"));
    }

    #[test]
    fn 출발지가_없으면_도착만_채운다() {
        let c = RouteConfig {
            origin: None,
            ..cfg()
        };
        let i = items(&c, ":길 회사", true);
        assert_eq!(i.len(), 1);
        assert!(i[0].keywords.contains("/link/to/"));
    }

    #[test]
    fn 키_파일을_읽는다() {
        assert_eq!(
            key_from_env_file("# c\nexport KAKAO_REST_API_KEY=\"abc123\"\n"),
            Some("abc123".into())
        );
        assert_eq!(key_from_env_file("KAKAO_REST_API_KEY=a\"b"), None);
        assert_eq!(key_from_env_file("OTHER=1"), None);
    }

    #[test]
    fn 카카오_응답을_장소로_바꾼다() {
        let body = r#"{"documents":[{"place_name":"판교역 신분당선","road_address_name":"경기 성남시 분당구 판교역로 160","address_name":"","category_group_name":"지하철역","distance":"12345","x":"127.1112","y":"37.3948"},{"place_name":"bad","x":"x","y":"1"}],"meta":{}}"#;
        let p = parse_keyword_response(body).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "판교역 신분당선");
        assert_eq!(
            p[0].detail,
            "지하철역 · 경기 성남시 분당구 판교역로 160 · 12.3km"
        );
        assert!(
            parse_keyword_response(r#"{"errorType":"x","message":"wrong appKey"}"#)
                .unwrap_err()
                .contains("wrong appKey")
        );
    }
}

/// 실제 카카오 호출 — 키가 있는 기기에서 `cargo test -p kmd-core route_live -- --ignored --nocapture`
#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore]
    fn route_live() {
        let cfg = RouteConfig::default();
        let key = cfg.kakao_key().expect("kakao.env 키");
        let near = Place {
            name: "강남역".into(),
            lat: 37.4979,
            lng: 127.0276,
            detail: String::new(),
        };
        let found = search_places(&key, "판교역", Some(&near)).unwrap();
        assert!(!found.is_empty());
        for p in &found {
            println!(
                "{} | {} | {}",
                p.name,
                p.detail,
                route_url(Mode::Transit, Some(&near), p)
            );
        }
    }
}
