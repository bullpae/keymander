//! 시간 표현 읽기 — `10분`, `1시간 30분`, `내일 9시`, `오후 3시 30분`, `11/7`, `월요일` (docs/23)

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Weekday};

/// 저장 형식 (지역 시간). 사람이 파일을 열어 읽고 고칠 수 있게 시간대를 적지 않는다
const STORED: &str = "%Y-%m-%dT%H:%M";

pub fn format_stored(t: NaiveDateTime) -> String {
    t.format(STORED).to_string()
}

pub fn parse_stored(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.trim(), STORED).ok()
}

fn take_number(s: &str) -> Option<(i64, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 || end > 6 {
        return None;
    }
    Some((s[..end].parse().ok()?, &s[end..]))
}

/// `10분`, `1시간 30분`, `2시간반`, `90s`, `1h30m` → 초. 단위 없는 숫자는 읽지 않는다
pub fn parse_duration(s: &str) -> Option<i64> {
    const UNITS: &[(&str, i64)] = &[
        ("시간", 3600),
        ("hours", 3600),
        ("hour", 3600),
        ("hrs", 3600),
        ("hr", 3600),
        ("h", 3600),
        ("분", 60),
        ("minutes", 60),
        ("minute", 60),
        ("mins", 60),
        ("min", 60),
        ("m", 60),
        ("초", 1),
        ("seconds", 1),
        ("second", 1),
        ("secs", 1),
        ("sec", 1),
        ("s", 1),
    ];
    let mut rest = s.trim();
    let mut total = 0;
    let mut last_unit = 0;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('반').filter(|_| last_unit == 3600) {
            total += 1800;
            rest = r.trim_start();
            last_unit = 0;
            continue;
        }
        let (n, r) = take_number(rest)?;
        let (unit, secs) = UNITS.iter().find(|(u, _)| r.starts_with(u))?;
        total += n * secs;
        last_unit = *secs;
        rest = r[unit.len()..].trim_start();
    }
    (total > 0).then_some(total)
}

/// `:timer` 글 → (초, 이름). `10분 라면`, `라면 10분`, `1시간 30분 회의`, `10 라면`(숫자만이면 분)
pub fn parse_timer(text: &str) -> Option<(i64, String)> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let read = |part: &[&str]| {
        let joined = part.join(" ");
        parse_duration(&joined).or_else(|| {
            let (n, rest) = take_number(&joined)?;
            (rest.is_empty() && n > 0).then_some(n * 60)
        })
    };
    for n in (1..=tokens.len()).rev() {
        if let Some(secs) = read(&tokens[..n]) {
            return Some((secs, tokens[n..].join(" ")));
        }
        if let Some(secs) = read(&tokens[tokens.len() - n..]) {
            return Some((secs, tokens[..tokens.len() - n].join(" ")));
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq)]
enum Meridiem {
    Am,
    Pm,
}

fn weekday_of(s: &str) -> Option<(Weekday, &str)> {
    const NAMES: &[(&str, Weekday)] = &[
        ("월요일", Weekday::Mon),
        ("화요일", Weekday::Tue),
        ("수요일", Weekday::Wed),
        ("목요일", Weekday::Thu),
        ("금요일", Weekday::Fri),
        ("토요일", Weekday::Sat),
        ("일요일", Weekday::Sun),
    ];
    NAMES
        .iter()
        .find_map(|(name, day)| s.strip_prefix(name).map(|r| (*day, r)))
}

/// `2026-11-07`, `11/7`, `11월 7일` 중 하나로 시작하면 (날짜, 나머지)
fn take_date(s: &str, today: NaiveDate) -> Option<(NaiveDate, &str)> {
    let (first, rest) = take_number(s)?;
    if let Some(r) = rest.strip_prefix('-') {
        let (m, r) = take_number(r)?;
        let (d, r) = take_number(r.strip_prefix('-')?)?;
        return Some((
            NaiveDate::from_ymd_opt(first as i32, m as u32, d as u32)?,
            r,
        ));
    }
    let (day, rest) = if let Some(r) = rest.strip_prefix('/') {
        take_number(r)?
    } else {
        let r = rest.strip_prefix('월')?;
        let (d, r) = take_number(r.trim_start())?;
        (d, r.strip_prefix('일')?)
    };
    let this_year = NaiveDate::from_ymd_opt(today.year(), first as u32, day as u32)?;
    let date = if this_year < today {
        NaiveDate::from_ymd_opt(today.year() + 1, first as u32, day as u32)?
    } else {
        this_year
    };
    Some((date, rest))
}

/// `15:30`, `3시`, `3시 30분`, `3시반` 중 하나로 시작하면 (시, 분, 나머지)
fn take_time(s: &str) -> Option<(u32, u32, &str)> {
    let (h, rest) = take_number(s)?;
    if let Some(r) = rest.strip_prefix(':') {
        let (m, r) = take_number(r)?;
        return Some((h as u32, m as u32, r));
    }
    let r = rest.strip_prefix('시')?.trim_start();
    if let Some(r2) = r.strip_prefix('반') {
        return Some((h as u32, 30, r2));
    }
    if let Some((m, r2)) = take_number(r) {
        if let Some(r3) = r2.strip_prefix('분') {
            return Some((h as u32, m as u32, r3));
        }
    }
    Some((h as u32, 0, r))
}

/// `@` 뒤 글 → 시각. 날짜만 있으면 오전 9시.
///
/// 오전·오후를 안 적으면: 날짜가 있을 때 1~6시는 오후로 읽고(`내일 3시` = 15시), 날짜도 없으면
/// 앞으로 가장 먼저 오는 때로 읽는다(지금 14시면 `3시` = 15시, 16시면 내일 3시).
pub fn parse_due(text: &str, now: NaiveDateTime) -> Option<NaiveDateTime> {
    let text = text.trim();
    let relative = text
        .strip_suffix('뒤')
        .or_else(|| text.strip_suffix('후'))
        .unwrap_or(text);
    if let Some(secs) = parse_duration(relative) {
        return Some(now + Duration::seconds(secs));
    }

    let today = now.date();
    let mut rest = text;
    let mut date = None;
    let mut next_week = false;
    let mut weekday = None;
    let mut meridiem = None;
    let mut time = None;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("오늘") {
            date = Some(today);
            rest = r;
        } else if let Some(r) = rest.strip_prefix("내일") {
            date = Some(today + Duration::days(1));
            rest = r;
        } else if let Some(r) = rest.strip_prefix("모레") {
            date = Some(today + Duration::days(2));
            rest = r;
        } else if let Some(r) = rest.strip_prefix("다음주") {
            next_week = true;
            rest = r;
        } else if let Some((d, r)) = weekday_of(rest) {
            weekday = Some(d);
            rest = r;
        } else if let Some(r) = rest.strip_prefix("오전") {
            meridiem = Some(Meridiem::Am);
            rest = r;
        } else if let Some(r) = rest.strip_prefix("오후") {
            meridiem = Some(Meridiem::Pm);
            rest = r;
        } else if let Some((d, r)) = take_date(rest, today) {
            date = Some(d);
            rest = r;
        } else if let Some((h, m, r)) = take_time(rest) {
            time = Some((h, m));
            rest = r;
        } else {
            return None;
        }
        rest = rest.trim_start();
    }
    if date.is_none() && weekday.is_none() && time.is_none() {
        return None;
    }

    let at = |d: NaiveDate, h: u32, m: u32| d.and_hms_opt(h, m, 0);
    let (hour, minute) = match time {
        Some((h, m)) => {
            if h > 24 || m > 59 {
                return None;
            }
            (h, m)
        }
        None => (9, 0),
    };
    let fixed_hour = match meridiem {
        Some(Meridiem::Pm) if hour < 12 => Some(hour + 12),
        Some(Meridiem::Am) if hour == 12 => Some(0),
        Some(_) => Some(hour),
        None if hour == 0 || hour >= 13 || time.is_none() => Some(hour),
        None => None,
    };

    if let Some(day) = weekday {
        let from_today = (day.num_days_from_monday() as i64
            - today.weekday().num_days_from_monday() as i64)
            .rem_euclid(7);
        let hour = fixed_hour.unwrap_or(if hour <= 6 { hour + 12 } else { hour });
        let this = today + Duration::days(from_today);
        let target = if next_week {
            today - Duration::days(today.weekday().num_days_from_monday() as i64)
                + Duration::days(7 + day.num_days_from_monday() as i64)
        } else if at(this, hour, minute)? <= now {
            this + Duration::days(7)
        } else {
            this
        };
        return at(target, hour, minute);
    }
    if let Some(d) = date {
        let hour = fixed_hour.unwrap_or(if hour <= 6 { hour + 12 } else { hour });
        return at(d, hour, minute);
    }
    // 시각만 — 앞으로 가장 먼저 오는 때
    let tomorrow = today + Duration::days(1);
    if let Some(h) = fixed_hour {
        let t = at(today, h, minute)?;
        return if t > now {
            Some(t)
        } else {
            at(tomorrow, h, minute)
        };
    }
    let am = at(today, hour, minute)?;
    if am > now {
        return Some(am);
    }
    if hour < 12 {
        let pm = at(today, hour + 12, minute)?;
        if pm > now {
            return Some(pm);
        }
    }
    at(tomorrow, hour, minute)
}

/// `:td` 글을 나눈 결과
#[derive(Debug, PartialEq)]
pub struct DueText {
    pub title: String,
    pub due: Option<NaiveDateTime>,
    /// `@` 뒤를 시각으로 읽지 못했다 (글 그대로 둔다 — 메일 주소일 수도 있다)
    pub unreadable: bool,
}

/// `보고서 쓰기 @내일 9시 #업무` → 제목 `보고서 쓰기 #업무` + 시각
pub fn split_due(text: &str, now: NaiveDateTime) -> DueText {
    let text = text.trim();
    let literal = |unreadable| DueText {
        title: text.to_string(),
        due: None,
        unreadable,
    };
    let start = if text.starts_with('@') {
        0
    } else {
        match text.rfind(" @") {
            Some(i) => i + 1,
            None => return literal(false),
        }
    };
    let after = &text[start + 1..];
    let (when, tags) = match after.find(" #") {
        Some(i) => (&after[..i], after[i..].trim()),
        None => (after, ""),
    };
    let Some(due) = parse_due(when, now) else {
        return literal(!when.trim().is_empty());
    };
    let title = format!("{} {tags}", text[..start].trim());
    DueText {
        title: title.trim().to_string(),
        due: Some(due),
        unreadable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-06(금) 14:00
    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 6)
            .unwrap()
            .and_hms_opt(14, 0, 0)
            .unwrap()
    }

    fn due(s: &str) -> String {
        parse_due(s, now()).map(format_stored).unwrap_or_default()
    }

    #[test]
    fn 기간() {
        assert_eq!(parse_duration("10분"), Some(600));
        assert_eq!(parse_duration("1시간 30분"), Some(5400));
        assert_eq!(parse_duration("2시간반"), Some(9000));
        assert_eq!(parse_duration("1h30m"), Some(5400));
        assert_eq!(parse_duration("90s"), Some(90));
        assert_eq!(parse_duration("10"), None);
        assert_eq!(parse_duration("라면"), None);
        assert_eq!(parse_duration("0분"), None);
    }

    #[test]
    fn 타이머_글() {
        let t = |s: &str| parse_timer(s);
        assert_eq!(t("10분 라면"), Some((600, "라면".into())));
        assert_eq!(t("라면 10분"), Some((600, "라면".into())));
        assert_eq!(t("1시간 30분 긴 회의"), Some((5400, "긴 회의".into())));
        assert_eq!(t("5"), Some((300, String::new())));
        assert_eq!(t("5 차"), Some((300, "차".into())));
        assert_eq!(t("라면"), None);
        assert_eq!(t(""), None);
    }

    #[test]
    fn 날짜와_시각() {
        assert_eq!(due("내일 9시"), "2026-11-07T09:00");
        assert_eq!(
            due("내일 3시"),
            "2026-11-07T15:00",
            "날짜가 있으면 3시는 오후"
        );
        assert_eq!(due("내일 오전 3시"), "2026-11-07T03:00");
        assert_eq!(due("내일"), "2026-11-07T09:00");
        assert_eq!(due("모레 오후 2시 30분"), "2026-11-08T14:30");
        assert_eq!(due("오늘 15:30"), "2026-11-06T15:30");
        assert_eq!(due("내일9시반"), "2026-11-07T09:30");
        assert_eq!(due("11/7"), "2026-11-07T09:00");
        assert_eq!(due("11월 7일 18시"), "2026-11-07T18:00");
        assert_eq!(due("1/2"), "2027-01-02T09:00", "지난 날짜는 내년");
        assert_eq!(due("2026-11-07 09:00"), "2026-11-07T09:00");
    }

    #[test]
    fn 시각만_쓰면_앞으로_가장_먼저_오는_때() {
        assert_eq!(due("3시"), "2026-11-06T15:00");
        assert_eq!(due("2시"), "2026-11-07T02:00", "14시 정각은 지났다");
        assert_eq!(due("오전 9시"), "2026-11-07T09:00");
        assert_eq!(due("오후 3시"), "2026-11-06T15:00");
        assert_eq!(due("13시"), "2026-11-07T13:00");
        assert_eq!(due("12시"), "2026-11-07T12:00");
    }

    #[test]
    fn 요일() {
        assert_eq!(due("월요일"), "2026-11-09T09:00");
        assert_eq!(due("금요일 15시"), "2026-11-06T15:00", "오늘 아직 안 지남");
        assert_eq!(due("금요일 9시"), "2026-11-13T09:00", "오늘 지남");
        assert_eq!(due("다음주 월요일"), "2026-11-09T09:00");
        assert_eq!(due("다음주 금요일"), "2026-11-13T09:00");
    }

    #[test]
    fn 상대_시간() {
        assert_eq!(due("30분 뒤"), "2026-11-06T14:30");
        assert_eq!(due("2시간 후"), "2026-11-06T16:00");
        assert_eq!(due("1시간"), "2026-11-06T15:00");
    }

    #[test]
    fn 못_읽는_글() {
        for s in ["", "언젠가", "25시", "3시 99분", "13/40", "example.com"] {
            assert_eq!(due(s), "", "{s}");
        }
    }

    #[test]
    fn 할_일_글_나누기() {
        let d = split_due("보고서 쓰기 @내일 9시 #업무", now());
        assert_eq!(d.title, "보고서 쓰기 #업무");
        assert_eq!(format_stored(d.due.unwrap()), "2026-11-07T09:00");
        assert!(!d.unreadable);

        let d = split_due("문의 me@example.com", now());
        assert_eq!(d.title, "문의 me@example.com");
        assert_eq!(d.due, None);
        assert!(!d.unreadable, "공백 뒤 @ 가 아니면 시각이 아니다");

        let d = split_due("회의 @언젠가", now());
        assert_eq!(d.title, "회의 @언젠가");
        assert!(d.unreadable);

        assert_eq!(split_due("그냥 할 일", now()).due, None);
        assert_eq!(split_due("@내일", now()).title, "");
    }

    #[test]
    fn 저장_형식은_되읽힌다() {
        let t = parse_due("내일 9시", now()).unwrap();
        assert_eq!(parse_stored(&format_stored(t)), Some(t));
        assert_eq!(parse_stored("엉망"), None);
    }
}
