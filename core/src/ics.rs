//! Reading a published calendar (an `.ics` feed from Outlook, Google, iCloud)
//! into import records.
//!
//! Every occurrence inside a window becomes its own dated block, rather than a
//! repeat rule. Calendars repeat in ways the week's rules cannot say (every
//! other Tuesday, the second Monday of the month, one instance moved to
//! Thursday), and a block per occurrence is correct for all of them.

use crate::import::ImportRecord;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use std::collections::{HashMap, HashSet};

/// The `external_id` prefix every record from a calendar feed carries. `sync`
/// owns everything under it, so a meeting deleted at the source is deleted here.
pub const PREFIX: &str = "ics:";

/// Where the feed's address is kept, in the store's `settings` table.
pub const URL_SETTING: &str = "ics_url";

/// How far back and forward a sync reads.
pub const DAYS_BACK: i64 = 7;
pub const DAYS_AHEAD: i64 = 120;

/// What a feed turned into, and what it could not.
#[derive(Debug, Default)]
pub struct Parsed {
    pub records: Vec<ImportRecord>,
    /// All-day events are not placed on the week: a holiday or a birthday
    /// drawn as a 24-hour block would bury the day it sits on.
    pub skipped_all_day: usize,
    pub warnings: Vec<String>,
}

/// A feed's `webcal://` address is plain HTTPS underneath.
pub fn fetchable(url: &str) -> String {
    match url.strip_prefix("webcal://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    }
}

/// One property line: `NAME;PARAM=x;PARAM="y:z":value`.
struct Prop {
    name: String,
    params: HashMap<String, String>,
    value: String,
}

fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match raw.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => lines.last_mut().unwrap().push_str(rest),
            _ => lines.push(raw.to_string()),
        }
    }
    lines
}

fn prop(line: &str) -> Option<Prop> {
    // The value starts at the first colon outside quotes; a TZID can be quoted
    // and contain one.
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| match c {
        '"' => {
            quoted = !quoted;
            None
        }
        ':' if !quoted => Some(i),
        _ => None,
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().trim_matches('"').to_string()))
        .collect();
    Some(Prop { name, params, value: value.to_string() })
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push(' '),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Outlook names zones the Windows way. These are the CLDR mappings for the
/// ones a person is likely to meet.
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("Dateline Standard Time", "Etc/GMT+12"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Mountain Standard Time", "America/Denver"),
    ("Central Standard Time", "America/Chicago"),
    ("Canada Central Standard Time", "America/Regina"),
    ("Central America Standard Time", "America/Guatemala"),
    ("Central Standard Time (Mexico)", "America/Mexico_City"),
    ("Eastern Standard Time", "America/New_York"),
    ("US Eastern Standard Time", "America/Indianapolis"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("Newfoundland Standard Time", "America/St_Johns"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("Argentina Standard Time", "America/Buenos_Aires"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("UTC", "UTC"),
    ("Coordinated Universal Time", "UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("W. Central Africa Standard Time", "Africa/Lagos"),
    ("Morocco Standard Time", "Africa/Casablanca"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("Middle East Standard Time", "Asia/Beirut"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("Jordan Standard Time", "Asia/Amman"),
    ("Syria Standard Time", "Asia/Damascus"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Arabic Standard Time", "Asia/Baghdad"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Iran Standard Time", "Asia/Tehran"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Pakistan Standard Time", "Asia/Karachi"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Bangladesh Standard Time", "Asia/Dhaka"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("E. Australia Standard Time", "Australia/Brisbane"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
];

/// An IANA name, a Windows name, or nothing we know — in which case the
/// machine's zone is the least wrong guess, and the caller is told.
fn zone(tzid: &str, fallback: Tz, warnings: &mut Vec<String>) -> Tz {
    if let Ok(tz) = tzid.parse::<Tz>() {
        return tz;
    }
    if let Some((_, iana)) = WINDOWS_ZONES.iter().find(|(w, _)| w.eq_ignore_ascii_case(tzid)) {
        return iana.parse().unwrap_or(fallback);
    }
    let note = format!("unknown time zone {tzid:?}; read as {}", fallback.name());
    if !warnings.contains(&note) {
        warnings.push(note);
    }
    fallback
}

enum When {
    At(DateTime<Tz>),
    AllDay,
}

fn when(p: &Prop, fallback: Tz, warnings: &mut Vec<String>) -> Option<When> {
    let v = p.value.trim();
    if p.params.get("VALUE").is_some_and(|t| t.eq_ignore_ascii_case("DATE")) || v.len() == 8 {
        return NaiveDate::parse_from_str(v, "%Y%m%d").ok().map(|_| When::AllDay);
    }
    if let Some(utc) = v.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some(When::At(Utc.from_utc_datetime(&naive).with_timezone(&chrono_tz::UTC)));
    }
    let naive = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    let tz = match p.params.get("TZID") {
        Some(id) => zone(id, fallback, warnings),
        // Floating time: whatever clock the reader is on.
        None => fallback,
    };
    tz.from_local_datetime(&naive).earliest().map(When::At)
}

/// `PT1H30M`, `P1D`, `-PT15M` (negative is nonsense for an event and refused).
fn duration(v: &str) -> Option<Duration> {
    let v = v.trim().strip_prefix('+').unwrap_or(v.trim());
    let v = v.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut in_time = false;
    for c in v.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match (unit, in_time) {
                    ('W', _) => Duration::weeks(n),
                    ('D', _) => Duration::days(n),
                    ('H', true) => Duration::hours(n),
                    ('M', true) => Duration::minutes(n),
                    ('S', true) => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    Some(total)
}

#[derive(Default)]
struct Event {
    uid: Option<String>,
    summary: Option<String>,
    location: Option<String>,
    start: Option<Prop>,
    end: Option<Prop>,
    duration: Option<String>,
    rrule: Option<String>,
    exdates: Vec<Prop>,
    rdates: Vec<Prop>,
    recurrence_id: Option<Prop>,
    cancelled: bool,
}

fn events(text: &str) -> Vec<Event> {
    let mut out = Vec::new();
    let mut current: Option<Event> = None;
    // Alarms nest inside events and carry their own DESCRIPTION, TRIGGER and
    // so on; none of it belongs to the event.
    let mut nested = 0usize;
    for line in unfold(text) {
        let Some(p) = prop(&line) else { continue };
        match (p.name.as_str(), p.value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") => {
                current = Some(Event::default());
                nested = 0;
                continue;
            }
            ("END", "VEVENT") => {
                out.extend(current.take());
                continue;
            }
            ("BEGIN", _) if current.is_some() => {
                nested += 1;
                continue;
            }
            ("END", _) if current.is_some() => {
                nested = nested.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        let (Some(ev), 0) = (current.as_mut(), nested) else { continue };
        match p.name.as_str() {
            "UID" => ev.uid = Some(p.value.trim().to_string()),
            "SUMMARY" => ev.summary = Some(unescape(&p.value)),
            "LOCATION" => ev.location = Some(unescape(&p.value)).filter(|l| !l.is_empty()),
            "DTSTART" => ev.start = Some(p),
            "DTEND" => ev.end = Some(p),
            "DURATION" => ev.duration = Some(p.value),
            "RRULE" => ev.rrule = Some(p.value.trim().to_string()),
            "EXDATE" => ev.exdates.push(p),
            "RDATE" => ev.rdates.push(p),
            "RECURRENCE-ID" => ev.recurrence_id = Some(p),
            "STATUS" => ev.cancelled = p.value.trim().eq_ignore_ascii_case("CANCELLED"),
            _ => {}
        }
    }
    out
}

/// A list-valued date property (`EXDATE`, `RDATE`) split into its dates.
fn instants(props: &[Prop], fallback: Tz, warnings: &mut Vec<String>) -> Vec<DateTime<Tz>> {
    props
        .iter()
        .flat_map(|p| {
            p.value.split(',').map(|v| Prop {
                name: p.name.clone(),
                params: p.params.clone(),
                value: v.to_string(),
            })
        })
        .filter_map(|p| match when(&p, fallback, warnings) {
            Some(When::At(t)) => Some(t),
            _ => None,
        })
        .collect()
}

fn stamp(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Read a feed into one record per occurrence overlapping `[from, to)`.
///
/// Never fails: an event it cannot read is skipped and named in `warnings`,
/// because one malformed invitation should not stop the rest of the week
/// arriving.
pub fn parse(text: &str, from: DateTime<Utc>, to: DateTime<Utc>, fallback: Tz) -> Parsed {
    let mut parsed = Parsed::default();
    let events = events(text);
    let warnings = &mut parsed.warnings;

    // Occurrences that were edited or cancelled one at a time. They arrive as
    // their own VEVENT carrying the master's UID and the original start.
    let mut overridden: HashSet<(String, DateTime<Utc>)> = HashSet::new();
    for ev in &events {
        if let (Some(uid), Some(rid)) = (&ev.uid, &ev.recurrence_id) {
            if let Some(When::At(t)) = when(rid, fallback, warnings) {
                overridden.insert((uid.clone(), t.with_timezone(&Utc)));
            }
        }
    }

    let mut seen: HashSet<String> = HashSet::new();
    for ev in &events {
        let title = ev.summary.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "(no title)".into());
        let Some(uid) = ev.uid.clone() else {
            warnings.push(format!("{title:?} has no UID; skipped"));
            continue;
        };
        if ev.cancelled {
            continue;
        }
        let start = match ev.start.as_ref().and_then(|p| when(p, fallback, warnings)) {
            Some(When::At(t)) => t,
            Some(When::AllDay) => {
                parsed.skipped_all_day += 1;
                continue;
            }
            None => {
                warnings.push(format!("{title:?} has no readable start; skipped"));
                continue;
            }
        };
        let length = ev
            .end
            .as_ref()
            .and_then(|p| match when(p, fallback, warnings) {
                Some(When::At(e)) => Some(e.with_timezone(&Utc) - start.with_timezone(&Utc)),
                _ => None,
            })
            .or_else(|| ev.duration.as_deref().and_then(duration))
            // A zero-length event is legal in a calendar and invisible on a
            // grid; give it a sliver the eye can find.
            .filter(|d| *d > Duration::zero())
            .unwrap_or(Duration::minutes(15));

        let push = |parsed_records: &mut Vec<ImportRecord>, seen: &mut HashSet<String>, key: String, at: DateTime<Utc>| {
            let end = at + length;
            if end <= from || at >= to || !seen.insert(key.clone()) {
                return;
            }
            parsed_records.push(ImportRecord {
                external_id: key,
                title: title.clone(),
                tags: vec!["calendar".into()],
                location: ev.location.clone(),
                starts_at: Some(at.to_rfc3339()),
                ends_at: Some(end.to_rfc3339()),
                ..Default::default()
            });
        };

        // An edited occurrence: keyed by where it originally sat, so moving
        // it again updates this record rather than adding another.
        if let Some(rid) = &ev.recurrence_id {
            let Some(When::At(orig)) = when(rid, fallback, warnings) else { continue };
            let key = format!("{PREFIX}{uid}@{}", stamp(orig.with_timezone(&Utc)));
            push(&mut parsed.records, &mut seen, key, start.with_timezone(&Utc));
            continue;
        }

        let Some(rule) = &ev.rrule else {
            // Keyed by UID alone, so a rescheduled meeting moves rather than
            // leaving its old slot behind.
            push(&mut parsed.records, &mut seen, format!("{PREFIX}{uid}"), start.with_timezone(&Utc));
            continue;
        };

        let occurrences = expand(rule, start, &ev.exdates, &ev.rdates, from - length, to, fallback, warnings)
            .unwrap_or_else(|e| {
                warnings.push(format!("{title:?} repeats in a way that could not be read ({e}); showing its first date only"));
                vec![start.with_timezone(&Utc)]
            });
        for at in occurrences {
            if overridden.contains(&(uid.clone(), at)) {
                continue;
            }
            let key = format!("{PREFIX}{uid}@{}", stamp(at));
            push(&mut parsed.records, &mut seen, key, at);
        }
    }
    parsed
}

#[allow(clippy::too_many_arguments)]
fn expand(
    rule: &str,
    start: DateTime<Tz>,
    exdates: &[Prop],
    rdates: &[Prop],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    fallback: Tz,
    warnings: &mut Vec<String>,
) -> Result<Vec<DateTime<Utc>>, String> {
    let rtz = rrule::Tz::Tz(start.timezone());
    let dt_start = start.with_timezone(&rtz);
    let unvalidated: rrule::RRule<rrule::Unvalidated> = rule.parse().map_err(|e| format!("{e}"))?;
    let mut set = unvalidated.build(dt_start).map_err(|e| format!("{e}"))?;
    for x in instants(exdates, fallback, warnings) {
        set = set.exdate(x.with_timezone(&rtz));
    }
    for r in instants(rdates, fallback, warnings) {
        set = set.rdate(r.with_timezone(&rtz));
    }
    let set = set
        .after(from.with_timezone(&rtz))
        .before(to.with_timezone(&rtz));
    // A daily meeting over the window is ~130; the cap only stops a runaway
    // rule (FREQ=MINUTELY) from filling the week.
    let result = set.all(1000);
    Ok(result.dates.into_iter().map(|d| d.with_timezone(&Utc)).collect())
}

/// What a feed published as "can view when I'm busy" calls every event. Some
/// organisations allow nothing more outside their walls.
const PLACEHOLDERS: &[&str] = &["busy", "tentative", "free", "out of office", "working elsewhere", "away"];

fn placeholder(title: &str) -> bool {
    PLACEHOLDERS.iter().any(|p| p.eq_ignore_ascii_case(title.trim()))
}

/// The event a key belongs to: `ics:<uid>` or `ics:<uid>@<original start>`.
/// Split from the right, and only at a stamp, because Google's UIDs contain an
/// `@` of their own.
fn series(external_id: &str) -> &str {
    let id = external_id.strip_prefix(PREFIX).unwrap_or(external_id);
    match id.rsplit_once('@') {
        Some((uid, at)) if at.len() == 16 && at.ends_with('Z') && at.as_bytes()[8] == b'T' => uid,
        _ => id,
    }
}

/// Names already given to each series here, for a feed that will not say.
fn known_names(db: &crate::Db) -> HashMap<String, (String, Option<String>)> {
    let mut out = HashMap::new();
    let Ok(mut stmt) = db.conn.prepare(
        "SELECT external_id, title, location FROM items
          WHERE external_id IS NOT NULL AND substr(external_id, 1, ?2) = ?1",
    ) else {
        return out;
    };
    let Ok(rows) = stmt.query_map(rusqlite::params![PREFIX, PREFIX.len() as i64], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))
    }) else {
        return out;
    };
    for (ext, title, location) in rows.filter_map(|r| r.ok()) {
        if !placeholder(&title) {
            out.insert(series(&ext).to_string(), (title, location));
        }
    }
    out
}

/// Read the feed text into the store, owning everything under `PREFIX`.
///
/// Where the feed only says "Busy", a name given to any occurrence of that
/// event here is kept, and carried to the occurrences still to arrive. Without
/// that, the next sync would put "Busy" back fifteen minutes after you named
/// your class.
pub fn sync_text(
    db: &crate::Db,
    text: &str,
    now: DateTime<Utc>,
    fallback: Tz,
) -> Result<(crate::ImportOutcome, Parsed), String> {
    if !text.contains("BEGIN:VCALENDAR") {
        // An expired or private link returns an HTML sign-in page, which would
        // otherwise parse as an empty calendar and delete every meeting.
        return Err("that is not a calendar feed (no BEGIN:VCALENDAR); nothing was changed".into());
    }
    let from = now - Duration::days(DAYS_BACK);
    let to = now + Duration::days(DAYS_AHEAD);
    let mut parsed = parse(text, from, to, fallback);
    let names = known_names(db);
    for r in parsed.records.iter_mut().filter(|r| placeholder(&r.title)) {
        if let Some((title, location)) = names.get(series(&r.external_id)) {
            r.title = title.clone();
            if r.location.is_none() {
                r.location = location.clone();
            }
        }
    }
    let outcome = crate::import::sync(db, PREFIX, &parsed.records, from, now).map_err(|e| e.to_string())?;
    Ok((outcome, parsed))
}
