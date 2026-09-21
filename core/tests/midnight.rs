//! Blocks late in the evening.
//!
//! A time with no end gets a default length, and 11:30pm plus an hour used to
//! wrap round to 12:30am on the same date -- before the start -- so the block
//! was dropped: a repeat with a note nobody saw, a one-off with nothing at all.
//! Defaults now stop at midnight, and an end of 12am means the midnight after.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::America::Chicago;
use ms_core::{add_from_text_in, get_days, Db, Placement};

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn now() -> DateTime<Utc> {
    "2026-08-31T12:00:00Z".parse().unwrap()
}

fn add(db: &Db, line: &str) {
    add_from_text_in(db, line, d("2026-08-31"), now(), Chicago).unwrap();
}

/// Every block in the seven days from `start`, with the day it was listed on.
fn blocks(db: &Db, start: &str) -> Vec<(NaiveDate, Placement)> {
    get_days(db, d(start), Chicago)
        .days
        .into_iter()
        .flat_map(|day| day.placements.into_iter().map(move |p| (day.date, p)))
        .collect()
}

fn minutes(p: &Placement) -> i64 {
    (p.ends_at - p.starts_at).num_minutes()
}

#[test]
fn a_late_repeat_with_no_end_runs_to_midnight() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "trash every tue 11:30pm");
    let got = blocks(&db, "2026-08-31");
    assert_eq!(got.len(), 1, "one Tuesday in the window: {got:?}");
    let (day, p) = &got[0];
    assert_eq!(*day, d("2026-09-01"));
    assert_eq!(p.starts_at.format("%H:%M").to_string(), "23:30");
    assert_eq!(minutes(p), 30, "up to midnight, not wrapped round to the morning");
    assert_eq!(p.ends_at.date_naive(), d("2026-09-02"), "that midnight is the next date");
}

#[test]
fn a_late_one_off_gets_its_block() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "call home on 02/09/2026 11:30pm");
    let got = blocks(&db, "2026-08-31");
    assert_eq!(got.len(), 1, "it used to be dropped without a word: {got:?}");
    assert_eq!(minutes(&got[0].1), 30);
}

#[test]
fn an_estimate_is_cut_at_midnight() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "study on 02/09/2026 10:30pm ~3h");
    let got = blocks(&db, "2026-08-31");
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(minutes(&got[0].1), 90, "10:30pm to midnight, not three hours");
}

#[test]
fn a_range_written_up_to_midnight_is_kept() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "gaming every fri 11pm-12am");
    let got = blocks(&db, "2026-08-31");
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(minutes(&got[0].1), 60);
}

#[test]
fn a_minute_before_midnight_still_shows() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "x every wed 11:59pm");
    let got = blocks(&db, "2026-08-31");
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(minutes(&got[0].1), 1);
}

/// Midnight as a *start* is the start of the day, not the end of it.
#[test]
fn a_block_starting_at_midnight_is_an_hour_long() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "x every thu 12am");
    let got = blocks(&db, "2026-08-31");
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1.starts_at.format("%H:%M").to_string(), "00:00");
    assert_eq!(minutes(&got[0].1), 60);
}

/// Running past midnight into the next day is not supported yet. It must be
/// reported, not silently dropped.
#[test]
fn a_range_past_midnight_is_reported_not_lost() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "shift every fri 10pm-2am");
    let week = get_days(&db, d("2026-08-31"), Chicago);
    assert!(week.days.iter().all(|day| day.placements.is_empty()));
    assert!(
        week.diagnostics.iter().any(|x| x.message.contains("not after start")),
        "{:?}",
        week.diagnostics
    );
}
