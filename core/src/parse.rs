use chrono::{Datelike, Duration, NaiveDate, NaiveTime};

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub title: String,
    pub consumed: Vec<String>,
    pub estimate_min: Option<u32>,
    pub at: Option<NaiveTime>,
    pub byday: Vec<String>,
    pub due: Option<NaiveDate>,
    /// Stated by the `every` keyword, never inferred from how many weekdays
    /// were typed. `math hw fri` is due Friday; `trash every tue` repeats.
    pub repeats: bool,
    /// Day of the month for a monthly repeat; None otherwise.
    pub monthday: Option<u32>,
    /// Where a monthly repeat starts, when a date came with it.
    pub repeat_from: Option<NaiveDate>,
    pub span: Option<(NaiveTime, NaiveTime)>,
    /// False when a time range was given: typing a span means scheduling, not
    /// listing. Spec 3.1.
    pub listed: bool,
    pub tags: Vec<String>,
    /// The one tag that is also a colour category, if any. Spec 3.3.
    pub category: Option<String>,
    pub location: Option<String>,
    pub priority: bool,
    /// Gets a block on the week for its one date. Either written with "on"
    /// and a time, or a bare range on a date ("appt 1 oct 11:20-12:20"). Only
    /// set when there is a time and a date to place it at — otherwise the
    /// item would be in neither the list nor the week and vanish.
    pub scheduled: bool,
    /// True = fixed to the zone it was created in; false = follows the machine.
    /// Defaults to true: a class that drifts to the wrong hour costs more than
    /// a gym slot that does.
    pub pinned: bool,
}

/// Closed set, because each needs a colour the UI can rely on. Spec 3.3.
pub const CATEGORIES: [&str; 4] = ["work", "life", "body", "social"];

/// What a single trailing token turned out to mean.
enum Token {
    Weekday(usize),
    Weekdays(Vec<usize>),
    /// A word that sits between a task and its details. "due" and "on" also
    /// say which kind of thing it is; the rest carry nothing.
    Filler,
    Due,
    On,
    Time(u32, u32),
    Estimate(u32),
    /// Days from today: 0 = today, 1 = tomorrow.
    Relative(i64),
    Date(NaiveDate),
    /// Day and month with the year left open, resolved forward from today.
    DayMonth(u32, u32),
    Every,
    Daily,
    Monthly,
    Span(NaiveTime, NaiveTime),
    Tag(String),
    Zone { pinned: bool },
}

const DAY_CODES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

const DAY_NAMES: [&str; 7] = [
    "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday",
];

/// One weekday, written any of the ways people write them: `mon`, `monday`,
/// `mondays`, `tues`, `thurs`.
fn weekday_index(raw: &str) -> Option<usize> {
    let t = raw.trim_end_matches('s');           // "mondays", "weds"
    DAY_CODES
        .iter()
        .position(|d| *d == t)
        .or_else(|| DAY_NAMES.iter().position(|d| *d == t))
        // "tues", "thur", "thurs" — a prefix of the full name, four or more
        // letters so nothing short and ambiguous slips through.
        .or_else(|| {
            (t.len() >= 4)
                .then(|| DAY_NAMES.iter().position(|d| d.starts_with(t)))
                .flatten()
        })
}

fn weekday(t: &str) -> Option<Token> {
    // "mon/wed/fri" is one word to the scanner but three days.
    if t.contains('/') {
        let days: Vec<usize> = t.split('/').filter_map(weekday_index).collect();
        if days.len() > 1 && days.len() == t.split('/').count() {
            return Some(Token::Weekdays(days));
        }
    }
    weekday_index(t).map(Token::Weekday)
}

/// The plural says it repeats: "mondays" means every Monday.
fn plural_weekday(t: &str) -> bool {
    t.ends_with('s') && weekday_index(t).is_some() && weekday_index(t.trim_end_matches('s')).is_some()
}

/// `5pm`, `5:30pm`, `17:00`.
fn time(t: &str) -> Option<Token> {
    // "12:00p" and "5p" are as common as the two-letter forms when people type
    // in a hurry, so both are accepted.
    // "8 p.m." and "20:00:00" both turn up; strip the punctuation and any
    // trailing seconds before looking at the rest.
    let t = &t.replace('.', "");
    // Drop seconds only when there really are seconds: stripping ":00" from
    // "9:00" would leave "9" and quietly destroy every time range.
    let t = if t.matches(':').count() == 2 {
        t.rsplit_once(':').map(|(head, _)| head).unwrap_or(t)
    } else {
        t.as_str()
    };
    let (body, shift) = if let Some(b) = t.strip_suffix("pm").or_else(|| t.strip_suffix('p')) {
        (b, 12)
    } else if let Some(b) = t.strip_suffix("am").or_else(|| t.strip_suffix('a')) {
        (b, 0)
    } else {
        (t, u32::MAX) // marker: 24h form, no meridiem
    };

    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok()?, 0),
    };
    if m > 59 {
        return None;
    }

    let hour = if shift == u32::MAX {
        // Bare digits are only a time when written as h:mm; "5" alone is not.
        if !body.contains(':') || h > 23 {
            return None;
        }
        h
    } else {
        if h == 0 || h > 12 {
            return None;
        }
        (h % 12) + shift
    };
    Some(Token::Time(hour, m))
}

/// `~2h`, `~90m`, `~1.5h`.
fn estimate(t: &str) -> Option<Token> {
    let body = t.strip_prefix('~')?;
    if let Some(h) = body.strip_suffix('h') {
        let hours: f64 = h.parse().ok()?;
        if hours <= 0.0 {
            return None;
        }
        return Some(Token::Estimate((hours * 60.0).round() as u32));
    }
    if let Some(m) = body.strip_suffix('m') {
        let mins: u32 = m.parse().ok()?;
        if mins == 0 {
            return None;
        }
        return Some(Token::Estimate(mins));
    }
    None
}

const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june",
    "july", "august", "september", "october", "november", "december",
];

/// Month index from a full or three-letter name. `sept` is accepted too,
/// because people write it.
fn month_of(t: &str) -> Option<u32> {
    let t = t.trim_end_matches('.');
    if t.len() < 3 {
        return None;
    }
    if t == "sept" {
        return Some(9);
    }
    MONTHS
        .iter()
        .position(|m| *m == t || m.starts_with(t) && t.len() == 3)
        .map(|i| i as u32 + 1)
}

/// A day number, with or without an ordinal suffix: `2`, `2nd`, `23rd`.
fn day_of(t: &str) -> Option<u32> {
    let t = t.trim_end_matches(',');
    // Only an ordinal may follow the number. Stripping any trailing letters at
    // all read "9am" as the 9th, so "1 oct 9am" became October 9 with no time.
    let core = ["st", "nd", "rd", "th"].iter().find_map(|s| t.strip_suffix(s)).unwrap_or(t);
    // Two digits at most: a four-digit number is a year, not a day.
    if core.is_empty() || core.len() > 2 || !core.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = core.parse().ok()?;
    (1..=31).contains(&n).then_some(n)
}

fn year_of(t: &str) -> Option<i32> {
    let core = t.trim_end_matches(',');
    (core.len() == 4).then(|| core.parse().ok())?
}

fn relative(t: &str) -> Option<Token> {
    match t {
        "today" => Some(Token::Relative(0)),
        "tomorrow" => Some(Token::Relative(1)),
        "noon" | "midday" => Some(Token::Time(12, 0)),
        "midnight" => Some(Token::Time(0, 0)),
        _ => None,
    }
}

/// `9:00-10:15`, `9am-10:15am`.
fn span(t: &str) -> Option<Token> {
    let (a, b) = t.split_once('-')?;
    match (time(a)?, time(b)?) {
        (Token::Time(h1, m1), Token::Time(h2, m2)) => Some(Token::Span(
            NaiveTime::from_hms_opt(h1, m1, 0)?,
            NaiveTime::from_hms_opt(h2, m2, 0)?,
        )),
        _ => None,
    }
}

fn tag(raw: &str) -> Option<Token> {
    let body = raw.strip_prefix('#')?;
    if body.is_empty() {
        return None;
    }
    let lower = body.to_ascii_lowercase();
    // Placement markers are consumed rather than kept: they say where the item
    // sits, not what it is about, and a "#floating" chip would be noise.
    match lower.as_str() {
        "fixed" | "pinned" => return Some(Token::Zone { pinned: true }),
        "floating" | "float" | "systime" => return Some(Token::Zone { pinned: false }),
        _ => {}
    }
    Some(Token::Tag(lower))
}

/// Pull a trailing `@place` off the line before anything else looks at it.
///
/// Locations contain spaces — "Hall 2.106", "the corner cafe" — so `@` covers more than
/// one word. It has to happen before the right-to-left scan, which would
/// otherwise halt on "1.314" and never reach the marker.
///
/// It stops at anything the scanner would recognise on its own — a tag, a time,
/// a weekday — so "@starbucks 3pm" keeps the time. A bare "@" is a marker too,
/// because people write "meet bob @ starbucks". An `@` mid-word
/// ("bob@example.com") is not.
fn split_location(input: &str) -> (String, Option<String>) {
    let words: Vec<&str> = input.split_whitespace().collect();
    let Some(at) = words.iter().position(|w| w.starts_with('@')) else {
        return (input.to_string(), None);
    };

    let first = words[at].trim_start_matches('@');
    let rest_from = at + 1;

    // The place runs until something that means something on its own.
    let stop = words[rest_from..]
        .iter()
        .position(|w| w.starts_with('#') || recognise(w).is_some())
        .map(|k| rest_from + k)
        .unwrap_or(words.len());

    let mut place = first.to_string();
    for w in &words[rest_from..stop] {
        if !place.is_empty() {
            place.push(' ');
        }
        place.push_str(w);
    }
    if place.is_empty() {
        return (input.to_string(), None);
    }

    let mut head: Vec<&str> = words[..at].to_vec();
    head.extend_from_slice(&words[stop..]);
    (head.join(" "), Some(place))
}

/// Words people put between a task and its time. They carry no information
/// themselves, but halting on one used to discard the date behind it too.
/// Only reached inside a run of tokens — a line merely ending in "to" is
/// untouched, because the scan stops before it.
fn filler(t: &str) -> Option<Token> {
    match t {
        "due" => Some(Token::Due),
        "on" => Some(Token::On),
        "at" | "by" | "from" => Some(Token::Filler),
        _ => None,
    }
}

fn every(t: &str) -> Option<Token> {
    (t == "every" || t == "weekly").then_some(Token::Every)
}

/// `daily` is a keyword; the bare word `day` deliberately is not, because it
/// ends ordinary sentences ("buy milk for the day") and would eat them.
fn daily(t: &str) -> Option<Token> {
    (t == "daily" || t == "everyday").then_some(Token::Daily)
}

/// `monthly` repeats on a date's day of the month. The one word only: `month`
/// on its own ends ordinary sentences ("the report for the month").
fn monthly(t: &str) -> Option<Token> {
    (t == "monthly").then_some(Token::Monthly)
}

/// Split a date on either separator, e.g. `9-02-2026` or `9/02/2026`.
fn date_parts(t: &str) -> Option<(Vec<&str>, char)> {
    let sep = ['-', '/', '.'].into_iter().find(|c| t.contains(*c))?;
    let parts: Vec<&str> = t.split(sep).collect();
    (parts.len() == 2 || parts.len() == 3).then_some((parts, sep))
}

/// `26` means 2026. Two-digit years are read into this century.
fn widen_year(y: u32) -> i32 {
    if y < 100 { 2000 + y as i32 } else { y as i32 }
}

/// A written date: `2026-09-15`, `9-02-2026`, `9/02/2026`, or `9/2` with the
/// year left open.
///
/// Three-part forms need a four-digit year, so a version number like `1-2-3`
/// is not mistaken for one — chrono will happily read that as the year 1.
///
/// Where one number is above 12 it can only be the day, which settles the
/// order by itself. Where both could be a month, day comes first — the order
/// the user asked for, and the one most of the world writes. The app echoes
/// the date it resolved, so a wrong reading is visible rather than silent.
fn date(t: &str) -> Option<Token> {
    let (parts, sep) = date_parts(t)?;
    let nums: Vec<u32> = parts.iter().filter_map(|p| p.parse::<u32>().ok()).collect();
    if nums.len() != parts.len() {
        return None;
    }

    match parts.len() {
        3 => {
            let (a, b, c) = (nums[0], nums[1], nums[2]);
            // ISO: the year comes first.
            if parts[0].len() == 4 {
                return NaiveDate::from_ymd_opt(a as i32, b, c).map(Token::Date);
            }
            // Two or four digits; anything else is not a year.
            if parts[2].len() != 4 && parts[2].len() != 2 {
                return None;
            }
            let (day, month) = if b > 12 { (b, a) } else { (a, b) };
            NaiveDate::from_ymd_opt(widen_year(c), month, day).map(Token::Date)
        }
        _ => {
            // A dash between two small numbers is far more often a range
            // ("read chapters 2-9") than a date, so two-part dates need a
            // slash or a dot.
            if sep == '-' {
                return None;
            }
            let (a, b) = (nums[0], nums[1]);
            let (day, month) = if b > 12 { (b, a) } else { (a, b) };
            if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
                return None;
            }
            Some(Token::DayMonth(day, month))
        }
    }
}

/// Recognise a run of one to three trailing words as a single token.
///
/// Some things are only a token when read together — "2 Sep 2026", "Sep 2",
/// "8:00 pm". Trying the longest window first means "Sep 2" is a date rather
/// than a month name followed by an unrecognised number.
fn recognise_window(win: &[&str]) -> Option<Token> {
    match win.len() {
        1 => recognise(win[0]),
        2 => two_words(win[0], win[1]),
        3 => three_words(win[0], win[1], win[2]),
        _ => None,
    }
}

fn two_words(a: &str, b: &str) -> Option<Token> {
    let (la, lb) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());

    // "8:00 pm" — the meridiem written separately.
    if matches!(lb.as_str(), "am" | "pm" | "a.m." | "p.m.") {
        if let Some(t @ Token::Time(..)) = time(&format!("{la}{}", lb.replace('.', ""))) {
            return Some(t);
        }
    }
    // "2 Sep" and "Sep 2".
    if let (Some(day), Some(month)) = (day_of(&la), month_of(&lb)) {
        return day_month(day, month);
    }
    if let (Some(month), Some(day)) = (month_of(&la), day_of(&lb)) {
        return day_month(day, month);
    }
    None
}

fn three_words(a: &str, b: &str, c: &str) -> Option<Token> {
    let (la, lb, lc) = (
        a.to_ascii_lowercase(),
        b.to_ascii_lowercase(),
        c.to_ascii_lowercase(),
    );
    // "2 Sep 2026" and "Sep 2, 2026".
    let year = year_of(&lc)?;
    let (day, month) = match (day_of(&la), month_of(&lb), month_of(&la), day_of(&lb)) {
        (Some(d), Some(m), _, _) => (d, m),
        (_, _, Some(m), Some(d)) => (d, m),
        _ => return None,
    };
    NaiveDate::from_ymd_opt(year, month, day).map(Token::Date)
}

/// A day and month with no year: the next time that date comes round.
fn day_month(day: u32, month: u32) -> Option<Token> {
    (1..=31).contains(&day).then_some(Token::DayMonth(day, month))
}

fn recognise(raw: &str) -> Option<Token> {
    // Tags keep the original casing of their body, so they are matched against
    // the raw word rather than the lowered one.
    if let Some(tok) = tag(raw) {
        return Some(tok);
    }
    let t = raw.to_ascii_lowercase();
    weekday(&t)
        .or_else(|| filler(&t))
        .or_else(|| every(&t))
        .or_else(|| daily(&t))
        .or_else(|| monthly(&t))
        .or_else(|| relative(&t))
        .or_else(|| span(&t))
        .or_else(|| time(&t))
        .or_else(|| date(&t))
        .or_else(|| estimate(&t))
}

/// First occurrence of `weekday` on or after `today`, so naming today's own
/// weekday means today rather than a week away.
fn next_weekday(today: NaiveDate, target: usize) -> NaiveDate {
    let now = today.weekday().num_days_from_monday() as i64;
    let ahead = (target as i64 - now).rem_euclid(7);
    today + Duration::days(ahead)
}

/// First date with this day and month on or after `today`.
fn next_day_month(today: NaiveDate, day: u32, month: u32) -> Option<NaiveDate> {
    for year in [today.year(), today.year() + 1] {
        if let Some(d) = NaiveDate::from_ymd_opt(year, month, day) {
            if d >= today {
                return Some(d);
            }
        }
    }
    None
}

/// Never fails: unparseable input becomes a title. See spec 6.
///
/// Scans from the right and stops at the first token it does not recognise, so
/// prose containing date-shaped words ("the March report") is left intact.
pub fn parse(input: &str, today: NaiveDate) -> Parsed {
    let (head, location_at) = split_location(input);
    let words: Vec<&str> = head.split_whitespace().collect();
    let mut consumed: Vec<String> = Vec::new();
    let mut estimate_min = None;
    let mut at = None;
    let mut byday: Vec<String> = Vec::new();
    let mut due: Option<NaiveDate> = None;
    let mut repeats = false;
    let mut monthly = false;
    let mut span_at: Option<(NaiveTime, NaiveTime)> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut pinned = true;
    let mut said_due = false;
    let mut said_on = false;
    let mut end = words.len();
    let mut seen_detail = false;

    while end > 0 {
        // Longest window first, so "Sep 2" beats a lone "2".
        let mut hit = None;
        for len in (1..=3.min(end)).rev() {
            if let Some(tok) = recognise_window(&words[end - len..end]) {
                hit = Some((len, tok));
                break;
            }
        }
        let Some((len, token)) = hit else { break };
        // A filler only means something between a title and its details. With
        // nothing recognised to its right it is just the last word of a
        // sentence -- "meet him by" -- and consuming it would lose the word.
        let is_filler = matches!(&token, Token::Filler | Token::Due | Token::On);
        if is_filler && !seen_detail {
            break;
        }
        seen_detail |= !is_filler;
        match token {
            // Scanning right-to-left, so a repeated token of the same kind
            // leaves the leftmost value in place.
            Token::Estimate(m) => estimate_min = Some(m),
            Token::Time(h, m) => at = NaiveTime::from_hms_opt(h, m, 0),
            Token::Weekday(i) => {
                if plural_weekday(&words[end - len].to_ascii_lowercase()) {
                    repeats = true;
                }
                if !byday.contains(&DAY_CODES[i].to_string()) {
                    byday.push(DAY_CODES[i].to_string());
                }
            }
            Token::Weekdays(days) => {
                for i in days {
                    if !byday.contains(&DAY_CODES[i].to_string()) {
                        byday.push(DAY_CODES[i].to_string());
                    }
                }
            }
            Token::Filler => {}
            Token::Due => said_due = true,
            Token::On => said_on = true,
            Token::Relative(days) => due = today.checked_add_signed(Duration::days(days)),
            Token::Date(d) => due = Some(d),
            Token::DayMonth(d, m) => due = next_day_month(today, d, m),
            Token::Every => repeats = true,
            Token::Monthly => {
                repeats = true;
                monthly = true;
            }
            Token::Daily => {
                repeats = true;
                for code in DAY_CODES {
                    if !byday.contains(&code.to_string()) {
                        byday.push(code.to_string());
                    }
                }
            }
            Token::Span(a, b) => span_at = Some((a, b)),
            Token::Tag(t) => {
                if !tags.contains(&t) {
                    tags.push(t);
                }
            }
            Token::Zone { pinned: p } => pinned = p,
        }
        for w in words[end - len..end].iter().rev() {
            consumed.push(w.to_string());
        }
        end -= len;
    }
    consumed.reverse();
    byday.reverse();
    byday.sort_by_key(|c| DAY_CODES.iter().position(|d| d == c).unwrap_or(usize::MAX));
    tags.reverse();

    // `!!` is written in front of the title rather than trailing, so it is
    // stripped from the left.
    let mut title = words[..end].join(" ");
    let stripped = title.trim_start_matches('!').trim_start().to_string();
    // Only treat `!!` as a marker when something survives it: "!!!" on its own
    // is a capture, and capture is never discarded. Spec 6.
    let priority = title.starts_with("!!") && !stripped.is_empty();
    if priority {
        title = stripped;
    }

    let category = tags
        .iter()
        .find(|t| CATEGORIES.contains(&t.as_str()))
        .cloned();

    // "on" puts it on the week, but only when there is a time to place it at;
    // otherwise it would be in neither the list nor the schedule. "due" wins,
    // because a deadline is the point of saying it.
    let said_on_at = said_on && !said_due && (at.is_some() || span_at.is_some());

    // A repeat has no single due date. Otherwise a lone weekday names the next
    // such day; several weekdays without `every` can only mean a repeat, so
    // they are treated as one.
    // A monthly repeat lands on its date's day of the month and starts there;
    // with no date, on today's. Weekdays alongside it contradict it, and the
    // explicit cadence wins rather than leaving a rule that means neither.
    let (monthday, repeat_from) = if monthly {
        byday.clear();
        (Some(due.unwrap_or(today).day()), due)
    } else {
        (None, None)
    };

    if repeats || byday.len() > 1 {
        repeats = true;
        due = None;
    } else if due.is_none() && byday.len() == 1 {
        if let Some(i) = DAY_CODES.iter().position(|d| *d == byday[0]) {
            due = Some(next_weekday(today, i));
        }
    }

    // A bare range on a date is a one-off block on that day. Without this it
    // was neither listed nor placed, and the range was thrown away for an
    // 11:59pm deadline nobody could see.
    let dated_block = !said_due && !repeats && due.is_some() && span_at.is_some();
    let scheduled = said_on_at || dated_block;

    Parsed {
        title,
        consumed,
        estimate_min,
        at,
        byday,
        due,
        repeats,
        monthday,
        repeat_from,
        span: span_at,
        // A bare time range is a block and nothing else -- a class does not
        // belong in a to-do list. "on" is different: it says when something
        // happens, not that it stops being a thing to finish, so it gets an
        // hour on the week AND stays tickable.
        listed: span_at.is_none() || said_on_at,
        scheduled,
        tags,
        category,
        location: location_at,
        priority,
        pinned,
    }
}
