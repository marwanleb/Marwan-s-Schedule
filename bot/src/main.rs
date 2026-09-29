//! Telegram capture for Marwan's Schedule.
//!
//! Deliberately a separate process, not something the app starts. The app has
//! no background services; this is one you opt into by running it. It talks to
//! the same SQLite store in WAL mode, so it works whether or not the window is
//! open, and messages sent while it is down are delivered by Telegram when it
//! comes back (they queue for 24h).
//!
//! Parsing is the same `interpret()` the app and the CLI use — there is no
//! second grammar and no LLM in the loop.

use chrono::{DateTime, Duration, Local, Utc};
use ms_core::{
    add_from_text, already_sent, due_soon, find_by_prefix, get_items, get_week, help_text,
    interpret, mark_sent, prune_sent, range12, set_done, set_setting, setting, stats, Command, Db, Filter,
    Kind, ListScope, Notice, time12,
};

const API: &str = "https://api.telegram.org/bot";
/// Long-poll timeout. Telegram holds the connection open until something
/// arrives, so this costs one idle socket rather than repeated requests.
const POLL_SECS: u64 = 50;
/// How far ahead a reminder goes out. A constant rather than a setting: ten
/// minutes was the ask, and a knob nobody turns is a knob that can be wrong.
const LEAD_MIN: i64 = 10;
/// Where to push. The bot has no other way to learn an address, so it records
/// whoever last spoke to it.
const CHAT_KEY: &str = "telegram_chat";

fn token() -> Option<String> {
    if let Ok(t) = std::env::var("TELEGRAM_TOKEN") {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    // bot.toml sits beside the store and is gitignored.
    let path = ms_core::paths::data_dir().join("bot.toml");
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        if let Some(v) = line.trim().strip_prefix("token") {
            let v = v.trim_start_matches([' ', '=']).trim().trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn local_zone() -> chrono_tz::Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(chrono_tz::UTC)
}

fn send(token: &str, chat_id: i64, text: &str) {
    // Telegram caps a message at 4096 characters.
    let body = if text.chars().count() > 4000 {
        text.chars().take(4000).collect::<String>()
    } else {
        text.to_string()
    };
    let _ = ureq::post(&format!("{API}{token}/sendMessage"))
        .send_json(ureq::json!({ "chat_id": chat_id, "text": body }));
}

/// One reminder, as a person would read it.
fn describe(n: &Notice, now: DateTime<Utc>, zone: chrono_tz::Tz) -> String {
    // Rounded up, so a notice fired at nine and a half minutes still says ten.
    let mins = ((n.at - now).num_seconds() as f64 / 60.0).ceil().max(0.0) as i64;
    let starts = n.at.with_timezone(&zone);
    let mut out = match n.kind {
        Kind::Block => format!("in {mins} min \u{2014} {}", n.title),
        Kind::Deadline => format!("due in {mins} min \u{2014} {}", n.title),
    };
    match n.ends {
        Some(e) => out.push_str(&format!("\n{}", range12(starts.time(), e.with_timezone(&zone).time()))),
        None => out.push_str(&format!("\nby {}", time12(starts.time()))),
    }
    if let Some(loc) = &n.location {
        out.push_str(&format!(" \u{b7} {loc}"));
    }
    out
}

/// Send anything newly due, once.
///
/// Called at the top of every poll, so the worst case is one long-poll late: a
/// notice lands 9 to 10 minutes ahead rather than exactly 10. Widening the
/// window instead would risk a gap between two ticks.
fn push_due(db: &Db, token: &str, zone: chrono_tz::Tz) {
    let Some(chat) = setting(db, CHAT_KEY).and_then(|s| s.parse::<i64>().ok()) else {
        return;
    };
    let now = Utc::now();
    for n in due_soon(db, now, Duration::minutes(LEAD_MIN), zone) {
        if already_sent(db, &n.key) {
            continue;
        }
        send(token, chat, &describe(&n, now, zone));
        // Recorded even when the send failed: a reminder that arrives twice is
        // worse than one that is missed, and the next occurrence is a new key.
        let _ = mark_sent(db, &n.key, now);
    }
    let _ = prune_sent(db, now - Duration::days(7));
}

/// "Fri 11 Sep 5pm": a deadline in the local zone, in 12-hour time.
fn when_due(d: DateTime<Utc>) -> String {
    let local = d.with_timezone(&Local);
    format!("{} {}", local.format("%a %d %b"), time12(local.time()))
}

fn fmt_mins(m: u32) -> String {
    if m % 60 == 0 {
        format!("{}h", m / 60)
    } else if m > 60 {
        format!("{}h{}m", m / 60, m % 60)
    } else {
        format!("{m}m")
    }
}

/// What the bot is waiting on from this chat, if anything.
///
/// A line with no date is easy to fire off and then forget about, so it is
/// held back and confirmed rather than filed silently. Lives in memory only:
/// if the bot restarts mid-question the pending line is lost, which is why it
/// is echoed back in the question itself.
/// What a half-finished capture is still missing, in the order it is asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Date,
    Time,
    Length,
}

impl Slot {
    fn question(self) -> &'static str {
        match self {
            Slot::Date => "when? (a date, or - to skip)",
            Slot::Time => "what time? (- to skip)",
            Slot::Length => "how long? (- to skip)",
        }
    }
}

enum Pending {
    /// A capture being completed one answer at a time.
    ///
    /// `text` is always a valid line of the grammar: each answer is appended
    /// and the whole thing re-read, so there is no second parser and an answer
    /// that supplies more than was asked ("friday 9am") simply fills both.
    Filling { text: String, asked: Slot, skipped: Vec<Slot> },
}

fn is_skip(t: &str) -> bool {
    matches!(t, "-" | "skip" | "none" | "no")
}

fn is_cancel(t: &str) -> bool {
    matches!(t, "cancel" | "stop" | "forget it")
}

/// The next thing worth asking about, or None when there is nothing left.
///
/// A repeat has no single date, and a time range already says both when it
/// starts and how long it runs, so neither is asked for again.
fn next_slot(p: &ms_core::Parsed, skipped: &[Slot]) -> Option<Slot> {
    let want = |s: Slot| !skipped.contains(&s);
    // A monthly repeat with no date would fall back to today's day of the
    // month. Asking is cheap, and that is usually not the day that was meant.
    let undated_month = p.monthday.is_some() && p.repeat_from.is_none();
    if ((p.due.is_none() && !p.repeats) || undated_month) && want(Slot::Date) {
        return Some(Slot::Date);
    }
    if p.at.is_none() && p.span.is_none() && want(Slot::Time) {
        return Some(Slot::Time);
    }
    // Not for a repeat: its length belongs to the rule, which the app edits,
    // and "trash every tue 20:00" is a finished thought already.
    if p.estimate_min.is_none() && p.span.is_none() && !p.repeats && want(Slot::Length) {
        return Some(Slot::Length);
    }
    None
}

/// Did the answer actually supply the thing that was asked for?
fn slot_filled(p: &ms_core::Parsed, slot: Slot) -> bool {
    match slot {
        // For a monthly repeat only a real date answers it: `repeats` is
        // already true before one is given.
        Slot::Date => {
            p.due.is_some() || p.repeat_from.is_some() || (p.repeats && p.monthday.is_none())
        }
        Slot::Time => p.at.is_some() || p.span.is_some(),
        Slot::Length => p.estimate_min.is_some() || p.span.is_some(),
    }
}

/// Ask the next question, or file it if there is nothing left to ask.
fn advance(db: &Db, text: String, skipped: Vec<Slot>, pending: &mut Option<Pending>) -> String {
    let p = ms_core::parse(&text, Local::now().date_naive());
    match next_slot(&p, &skipped) {
        Some(slot) => {
            let q = slot.question();
            *pending = Some(Pending::Filling { text, asked: slot, skipped });
            q.to_string()
        }
        None => commit(db, &text, "added"),
    }
}

/// Handle a message, carrying whatever this chat was asked last.
fn handle_with_state(db: &Db, text: &str, pending: &mut Option<Pending>) -> String {
    let trimmed = text.trim().to_ascii_lowercase();

    match pending.take() {
        Some(Pending::Filling { text: draft, asked, mut skipped }) => {
            if is_cancel(&trimmed) {
                return format!("dropped: {}", ms_core::parse(&draft, Local::now().date_naive()).title);
            }
            if is_skip(&trimmed) {
                skipped.push(asked);
                return advance(db, draft, skipped, pending);
            }

            // A length needs its marker; the other answers are grammar already.
            let answer = text.trim();
            let addition = if asked == Slot::Length && !answer.starts_with('~') {
                format!("~{answer}")
            } else {
                answer.to_string()
            };
            let combined = format!("{draft} {addition}");
            let reparsed = ms_core::parse(&combined, Local::now().date_naive());

            if slot_filled(&reparsed, asked) {
                skipped.push(asked);
                return advance(db, combined, skipped, pending);
            }

            // Not an answer to the question. Never lose either message: file
            // what was already there, and read this one as a fresh capture.
            let kept = commit(db, &draft, "filed as it was");
            return format!("{kept}

{}", handle_with_state(db, text, pending));
        }
        None => {}
    }

    let today = Local::now().date_naive();
    if let Command::Capture(p) = interpret(text, today) {
        // Ask for whatever the line did not say, one thing at a time. A
        // complete line asks nothing, so this is the price of being terse
        // rather than a tax on every capture.
        if !p.title.trim().is_empty() && next_slot(&p, &[]).is_some() {
            return advance(db, text.to_string(), Vec::new(), pending);
        }
    }

    handle(db, text)
}

/// Store a capture and describe what was made of it.
fn commit(db: &Db, text: &str, lead: &str) -> String {
    let today = Local::now().date_naive();
    let p = ms_core::parse(text, today);
    match add_from_text(db, text, today, Utc::now()) {
        Ok(item) => {
            let mut out = format!("{lead}: {}", item.title);
            if let Some(d) = item.due_at {
                out.push_str(&format!(
                    "
due {}",
                    when_due(d)
                ));
            }
            if item.recurs {
                out.push_str(&format!("
repeats {}", p.byday.join(", ")));
            }
            if let Some(e) = item.estimate_min {
                out.push_str(&format!("
estimate {}", fmt_mins(e)));
            }
            out
        }
        Err(e) => format!("could not add: {e}"),
    }
}

fn handle(db: &Db, text: &str) -> String {
    let today = Local::now().date_naive();
    let now = Utc::now();

    match interpret(text, today) {
        Command::Help => help_text(),

        Command::List(scope) => {
            let items = get_items(
                db,
                &Filter {
                    listed: Some(true),
                    done: Some(false),
                    on: Some(today),
                    ..Default::default()
                },
            );

            // Today means due by the end of today, which includes anything
            // already overdue. Week reaches seven days out. All is everything
            // still open, dated or not.
            let horizon = match scope {
                ListScope::Today => Some(today),
                ListScope::Week => Some(today + chrono::Duration::days(7)),
                ListScope::All => None,
            };
            let shown: Vec<_> = items
                .iter()
                .filter(|i| match horizon {
                    None => true,
                    Some(limit) => i
                        .due_at
                        .is_some_and(|d| d.with_timezone(&Local).date_naive() <= limit),
                })
                .collect();

            if shown.is_empty() {
                return match scope {
                    ListScope::Today => "nothing due today".into(),
                    ListScope::Week => "nothing due this week".into(),
                    ListScope::All => "nothing open".into(),
                };
            }

            let heading = match scope {
                ListScope::Today => "due today",
                ListScope::Week => "due this week",
                ListScope::All => "open",
            };
            let mut out = format!("{heading}:
");
            for i in &shown {
                let due = i
                    .due_at
                    .map(|d| d.with_timezone(&Local).format("  (%a %d %b)").to_string())
                    .unwrap_or_default();
                let est = i
                    .estimate_min
                    .map(|m| format!("  ~{}", fmt_mins(m)))
                    .unwrap_or_default();
                out.push_str(&format!("· {}{}{}
", i.title, due, est));
            }
            // Undated items are invisible to the narrower views, so say how
            // many are waiting rather than let them be forgotten.
            if scope != ListScope::All {
                let undated = items.iter().filter(|i| i.due_at.is_none()).count();
                if undated > 0 {
                    out.push_str(&format!("
({undated} with no date — send `list` for everything)"));
                }
            }
            out
        }

        Command::Week => {
            let week = get_week(db, today, local_zone());
            let mut out = format!("week of {}\n", week.anchor);
            for day in &week.days {
                out.push_str(&format!("\n{}\n", day.date.format("%a %d %b")));
                if day.placements.is_empty() {
                    out.push_str("  —\n");
                }
                for p in &day.placements {
                    let title = ms_core::store::fetch(db, &p.item_id)
                        .map(|i| i.title)
                        .unwrap_or_else(|| p.item_id.clone());
                    out.push_str(&format!(
                        "  {}  {}\n",
                        range12(p.starts_at.time(), p.ends_at.time()),
                        title
                    ));
                }
            }
            for d in &week.diagnostics {
                out.push_str(&format!("\nnote: {}\n", d.message));
            }
            out
        }

        Command::Done(prefix) => {
            let matches = find_by_prefix(db, &prefix);
            match matches.len() {
                0 => format!("nothing matches {prefix:?}"),
                1 => {
                    let item = &matches[0];
                    // A repeat is ticked off for today's occurrence only.
                    let on = item.recurs.then_some(today);
                    match set_done(db, &item.id, true, on, now) {
                        Ok(_) => format!("done: {}", item.title),
                        Err(e) => format!("could not update: {e}"),
                    }
                }
                _ => {
                    let mut out = format!("{prefix:?} matches {}:\n", matches.len());
                    for m in matches.iter().take(10) {
                        out.push_str(&format!("· {}\n", m.title));
                    }
                    out.push_str("\nbe more specific");
                    out
                }
            }
        }

        Command::Capture(p) => {
            if p.title.trim().is_empty() {
                return "nothing to add".into();
            }
            match add_from_text(db, text, today, now) {
                Ok(item) => {
                    let mut out = format!("added: {}", item.title);
                    if let Some(d) = item.due_at {
                        out.push_str(&format!("\ndue {}", when_due(d)));
                    }
                    if item.recurs {
                        out.push_str(&format!("\nrepeats {}", p.byday.join(", ")));
                    }
                    if let Some(e) = item.estimate_min {
                        out.push_str(&format!("\nestimate {}", fmt_mins(e)));
                    }
                    if p.consumed.is_empty() {
                        out.push_str("\n\n(no syntax recognised — saved as plain text. send `help` for the syntax)");
                    }
                    let s = stats(db, None);
                    if let Some(phrase) = s.phrase {
                        out.push_str(&format!("\n\nfyi: things take {phrase} than you estimate"));
                    }
                    out
                }
                Err(e) => format!("could not add: {e}"),
            }
        }
    }
}

fn main() {
    let Some(token) = token() else {
        eprintln!(
            "msbot: no token.\n\
             Set TELEGRAM_TOKEN, or put `token = \"...\"` in {}",
            ms_core::paths::data_dir().join("bot.toml").display()
        );
        std::process::exit(1);
    };

    let path = ms_core::paths::db_path();
    let db = match Db::open(&path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("msbot: could not open {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    eprintln!("msbot: polling, store at {}", path.display());

    let mut offset: i64 = 0;
    // What each chat was last asked. In memory only, so a restart forgets any
    // outstanding question — the question repeats the line back, so nothing
    // typed is lost either way.
    let mut pending: std::collections::HashMap<i64, Pending> = std::collections::HashMap::new();
    let zone = local_zone();
    loop {
        // Top of the loop, not the bottom: several arms below `continue`, and a
        // reminder must not depend on the poll having succeeded.
        push_due(&db, &token, zone);

        let url = format!("{API}{token}/getUpdates?timeout={POLL_SECS}&offset={offset}");
        let resp = ureq::get(&url)
            .timeout(std::time::Duration::from_secs(POLL_SECS + 15))
            .call();

        let body: serde_json::Value = match resp {
            Ok(r) => match r.into_json() {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("msbot: bad response: {e}");
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    continue;
                }
            },
            Err(e) => {
                // A dropped long-poll is normal; back off briefly and retry
                // rather than exiting, so the bot survives a flaky connection.
                eprintln!("msbot: poll failed ({e}); retrying");
                std::thread::sleep(std::time::Duration::from_secs(5));
                continue;
            }
        };

        let Some(updates) = body.get("result").and_then(|r| r.as_array()) else {
            continue;
        };

        for u in updates {
            if let Some(id) = u.get("update_id").and_then(|v| v.as_i64()) {
                offset = offset.max(id + 1);
            }
            let Some(msg) = u.get("message") else { continue };
            let Some(chat_id) = msg.pointer("/chat/id").and_then(|v| v.as_i64()) else {
                continue;
            };
            let Some(text) = msg.get("text").and_then(|v| v.as_str()) else {
                send(&token, chat_id, "I only understand text.");
                continue;
            };

            // Learn where to push. Cheap, and it means reminders start
            // working the first time you say anything at all.
            let _ = set_setting(&db, CHAT_KEY, &chat_id.to_string());

            let mut chat_state = pending.remove(&chat_id);
            let reply = handle_with_state(&db, text, &mut chat_state);
            if let Some(state) = chat_state {
                pending.insert(chat_id, state);
            }
            send(&token, chat_id, &reply);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{handle_with_state, Pending};
    use ms_core::{get_items, Db, Filter};

    /// Drive a whole exchange, returning what the bot said each time. The state
    /// is threaded exactly as `main` threads it, so these are real conversations.
    fn talk(db: &Db, lines: &[&str]) -> Vec<String> {
        let mut pending: Option<Pending> = None;
        lines.iter().map(|l| handle_with_state(db, l, &mut pending)).collect()
    }

    fn titles(db: &Db) -> Vec<String> {
        get_items(db, &Filter::default()).into_iter().map(|i| i.title).collect()
    }

    #[test]
    fn a_complete_line_is_never_questioned() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["gym fri 6am ~1h"]);
        assert!(said[0].starts_with("added"), "asked something it did not need to: {said:?}");
        assert_eq!(titles(&db), vec!["gym"]);
    }

    #[test]
    fn a_bare_title_is_asked_for_each_missing_thing_in_turn() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["dentist", "friday", "9am", "30m"]);
        assert!(said[0].contains("when"), "first question: {:?}", said[0]);
        assert!(said[1].contains("time"), "second question: {:?}", said[1]);
        assert!(said[2].contains("how long"), "third question: {:?}", said[2]);
        assert!(said[3].starts_with("added"), "then files it: {:?}", said[3]);

        let items = get_items(&db, &Filter::default());
        assert_eq!(items.len(), 1, "one item, not one per answer");
        assert_eq!(items[0].title, "dentist");
        assert_eq!(items[0].estimate_min, Some(30));
        assert!(items[0].due_at.is_some(), "the date answer stuck");
    }

    #[test]
    fn an_answer_that_says_more_than_was_asked_skips_ahead() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["dentist", "friday 9am"]);
        assert!(said[0].contains("when"));
        // Time came with the date, so the next question is length, not time.
        assert!(said[1].contains("how long"), "should have skipped the time: {:?}", said[1]);
    }

    #[test]
    fn skipping_moves_on_and_never_asks_twice() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["renew parking", "-", "-", "-"]);
        assert!(said[0].contains("when"));
        assert!(said[1].contains("time"));
        assert!(said[2].contains("how long"));
        assert!(said[3].starts_with("added"), "skipping everything still files it: {:?}", said[3]);
        assert_eq!(titles(&db), vec!["renew parking"]);
    }

    #[test]
    fn cancel_drops_the_draft_and_writes_nothing() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["dentist", "cancel"]);
        assert!(said[1].starts_with("dropped"), "{:?}", said[1]);
        assert!(titles(&db).is_empty(), "nothing should have been written");
    }

    /// The promise that matters: an answer the bot cannot read must not lose
    /// the thing being captured.
    #[test]
    fn an_unreadable_answer_files_the_draft_rather_than_losing_it() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["dentist", "sometime soonish"]);
        assert!(said[1].contains("filed as it was"), "{:?}", said[1]);
        assert!(titles(&db).contains(&"dentist".to_string()), "the draft survived");
    }

    /// An answer that happens to read as a date is taken as one, even if it
    /// looks like a fresh thought. Pinned deliberately: at a "when?" prompt
    /// the reading is reasonable, and the alternative is guessing at intent.
    #[test]
    fn an_answer_containing_a_date_is_treated_as_the_answer() {
        let db = Db::open_in_memory().unwrap();
        talk(&db, &["dentist", "buy milk tomorrow", "-", "-"]);
        let all = titles(&db);
        assert_eq!(all.len(), 1, "it merges rather than splitting: {all:?}");
        assert!(all[0].starts_with("dentist"), "{all:?}");
    }

    #[test]
    fn a_repeat_is_never_asked_for_a_date() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["trash every tue 20:00"]);
        assert!(said[0].starts_with("added"), "a repeat needs nothing else: {:?}", said[0]);
    }

    #[test]
    fn a_time_range_answers_both_time_and_length() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["seminar", "friday", "14:00-15:30"]);
        assert!(said[2].starts_with("added"), "a range needs no length question: {:?}", said[2]);
    }

    #[test]
    fn commands_still_work_and_do_not_start_a_ladder() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["help", "list"]);
        assert!(said[0].contains("Type what you want"), "{:?}", said[0]);
        assert!(!said[1].contains("when?"), "list is not a capture: {:?}", said[1]);
    }

    #[test]
    fn a_monthly_repeat_is_asked_which_day() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["pay rent monthly", "1 oct", "-"]);
        assert!(said[0].contains("when"), "{:?}", said[0]);
        assert!(said[1].contains("time"), "{:?}", said[1]);
        assert!(said[2].starts_with("added"), "{:?}", said[2]);
        let items = get_items(&db, &Filter::default());
        assert_eq!(items.len(), 1);
        assert!(items[0].recurs, "it repeats");
    }

    #[test]
    fn skipping_the_day_of_a_monthly_repeat_still_files_it() {
        let db = Db::open_in_memory().unwrap();
        let said = talk(&db, &["pay rent monthly", "-", "-"]);
        assert!(said[2].starts_with("added"), "{:?}", said[2]);
        assert!(get_items(&db, &Filter::default())[0].recurs);
    }

    #[test]
    fn a_reminder_reads_in_12_hour_time() {
        use chrono::TimeZone;
        // 15:30 UTC is 10:30am in Chicago in September; 17:00 UTC is noon.
        let at = chrono::Utc.with_ymd_and_hms(2026, 9, 9, 15, 30, 0).unwrap();
        let n = ms_core::Notice {
            key: "k".into(),
            kind: ms_core::Kind::Block,
            title: "PHYS201".into(),
            location: None,
            at,
            ends: Some(chrono::Utc.with_ymd_and_hms(2026, 9, 9, 17, 0, 0).unwrap()),
        };
        let text = super::describe(&n, at - chrono::Duration::minutes(10), chrono_tz::America::Chicago);
        assert!(text.ends_with("10:30am\u{2013}12pm"), "{text:?}");
        let deadline = ms_core::Notice { kind: ms_core::Kind::Deadline, ends: None, ..n };
        let text = super::describe(&deadline, at - chrono::Duration::minutes(10), chrono_tz::America::Chicago);
        assert!(text.ends_with("by 10:30am"), "{text:?}");
    }
}
