//! What gets pushed, and — more importantly — what does not.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::America::Chicago;
use ms_core::{
    add_from_text_in, already_sent, due_soon, get_days, mark_sent, move_occurrence,
    move_placement, prune_sent, set_done, set_setting, setting, Db, Kind,
};

const LEAD: i64 = 10;

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

/// Capture a line the way every front door does, at a fixed instant.
fn add(db: &Db, text: &str, today: &str, now: &str) -> ms_core::Item {
    add_from_text_in(db, text, d(today), utc(now), Chicago).unwrap()
}

fn soon(db: &Db, now: &str) -> Vec<ms_core::Notice> {
    due_soon(db, utc(now), Duration::minutes(LEAD), Chicago)
}

// Austin is UTC-5 in September, so 09:00 local is 14:00Z.

#[test]
fn a_block_inside_the_window_is_announced() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "standup on 09/09/2026 09:00", "2026-09-08", "2026-09-08T12:00:00Z");

    let out = soon(&db, "2026-09-09T13:52:00Z");
    assert_eq!(out.len(), 1, "expected one notice, got {out:?}");
    assert_eq!(out[0].title, "standup");
    assert_eq!(out[0].kind, Kind::Block);
    assert_eq!(out[0].at, utc("2026-09-09T14:00:00Z"));
}

#[test]
fn a_block_beyond_the_window_is_not() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "standup on 09/09/2026 09:00", "2026-09-08", "2026-09-08T12:00:00Z");

    assert!(soon(&db, "2026-09-09T13:30:00Z").is_empty(), "38 minutes out is not soon");
}

#[test]
fn a_block_already_begun_is_not_announced() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "standup on 09/09/2026 09:00", "2026-09-08", "2026-09-08T12:00:00Z");

    assert!(soon(&db, "2026-09-09T14:00:00Z").is_empty(), "it has started; that is not news");
    assert!(soon(&db, "2026-09-09T14:05:00Z").is_empty());
}

#[test]
fn a_recurring_class_is_announced_on_each_occurrence() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "PHYS201 every wed 10:30-12:00 @Hall 2.106", "2026-09-08", "2026-09-08T12:00:00Z");

    // 10:30 Austin is 15:30Z.
    let first = soon(&db, "2026-09-09T15:22:00Z");
    assert_eq!(first.len(), 1, "got {first:?}");
    assert_eq!(first[0].title, "PHYS201");
    assert_eq!(first[0].location.as_deref(), Some("Hall 2.106"));

    let next = soon(&db, "2026-09-16T15:22:00Z");
    assert_eq!(next.len(), 1);
    assert_ne!(first[0].key, next[0].key, "each occurrence needs its own key");
}

#[test]
fn a_ticked_occurrence_is_not_announced() {
    let db = Db::open_in_memory().unwrap();
    let item = add(&db, "PHYS201 every wed 10:30-12:00", "2026-09-08", "2026-09-08T12:00:00Z");
    set_done(&db, &item.id, true, Some(d("2026-09-09")), utc("2026-09-09T12:00:00Z")).unwrap();

    assert!(soon(&db, "2026-09-09T15:22:00Z").is_empty(), "already done this week");
    assert_eq!(soon(&db, "2026-09-16T15:22:00Z").len(), 1, "next week is still open");
}

#[test]
fn a_deadline_in_the_list_is_announced() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "lab report due 09/09/2026 17:00", "2026-09-08", "2026-09-08T12:00:00Z");

    let out = soon(&db, "2026-09-09T21:55:00Z");
    assert_eq!(out.len(), 1, "got {out:?}");
    assert_eq!(out[0].kind, Kind::Deadline);
    assert_eq!(out[0].ends, None);
}

#[test]
fn a_ticked_deadline_is_not() {
    let db = Db::open_in_memory().unwrap();
    let item = add(&db, "lab report due 09/09/2026 17:00", "2026-09-08", "2026-09-08T12:00:00Z");
    set_done(&db, &item.id, true, None, utc("2026-09-09T12:00:00Z")).unwrap();

    assert!(soon(&db, "2026-09-09T21:55:00Z").is_empty());
}

#[test]
fn the_window_reaches_across_the_week_boundary() {
    let db = Db::open_in_memory().unwrap();
    // Monday 00:05 Austin = 05:05Z Monday.
    add(&db, "early flight on 14/09/2026 00:05", "2026-09-08", "2026-09-08T12:00:00Z");

    // Sunday 23:57 Austin = Monday 04:57Z, still inside the previous week.
    let out = soon(&db, "2026-09-14T04:57:00Z");
    assert_eq!(out.len(), 1, "expanding only the current week would miss this: {out:?}");
}

#[test]
fn a_sent_notice_is_remembered_and_pruned() {
    let db = Db::open_in_memory().unwrap();
    let now = utc("2026-09-09T13:52:00Z");
    assert!(!already_sent(&db, "k"));

    mark_sent(&db, "k", now).unwrap();
    assert!(already_sent(&db, "k"));
    mark_sent(&db, "k", now).unwrap();
    assert!(already_sent(&db, "k"), "marking twice is not an error");

    assert_eq!(prune_sent(&db, now - Duration::days(1)).unwrap(), 0, "not old enough");
    assert!(already_sent(&db, "k"));
    assert_eq!(prune_sent(&db, now + Duration::days(1)).unwrap(), 1);
    assert!(!already_sent(&db, "k"));
}

#[test]
fn a_setting_round_trips() {
    let db = Db::open_in_memory().unwrap();
    assert_eq!(setting(&db, "telegram_chat"), None);
    set_setting(&db, "telegram_chat", "12345").unwrap();
    set_setting(&db, "telegram_chat", "67890").unwrap();
    assert_eq!(setting(&db, "telegram_chat").as_deref(), Some("67890"));
}

#[test]
fn nothing_scheduled_means_nothing_to_say() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "renew parking", "2026-09-08", "2026-09-08T12:00:00Z");
    assert!(soon(&db, "2026-09-09T13:52:00Z").is_empty(), "an undated task has no moment");
}

#[test]
fn an_on_item_is_announced_once_not_twice() {
    let db = Db::open_in_memory().unwrap();
    // "on" makes it a block AND leaves it in the list; both would otherwise
    // fire at the same instant.
    add(&db, "pick up a parcel on 09/09/2026 20:00", "2026-09-08", "2026-09-08T12:00:00Z");

    let out = soon(&db, "2026-09-10T00:52:00Z");
    assert_eq!(out.len(), 1, "one thing happening once: {out:?}");
    assert_eq!(out[0].kind, Kind::Block, "the block says more than the deadline");
}

#[test]
fn a_deadline_at_a_different_hour_still_gets_its_own_notice() {
    let db = Db::open_in_memory().unwrap();
    let item = add(&db, "essay due 09/09/2026 23:00", "2026-09-08", "2026-09-08T12:00:00Z");
    // A block earlier the same day must not suppress the evening deadline.
    ms_core::add_placement(
        &db,
        &item.id,
        "2026-09-09T14:00:00-05:00",
        "2026-09-09T15:00:00-05:00",
    )
    .unwrap();

    assert_eq!(soon(&db, "2026-09-09T18:52:00Z").len(), 1, "the block");
    assert_eq!(soon(&db, "2026-09-10T03:52:00Z").len(), 1, "the deadline, hours later");
}

/// The block the grid shows for an item on `date`, as the drag handler sees it.
fn block_on(db: &Db, date: &str) -> ms_core::Placement {
    let week = get_days(db, d(date), Chicago);
    week.days[0].placements[0].clone()
}

/// "standup on 09/09 9am" is one thing said once: a block, and a deadline at
/// the same instant. Dragging the block has to take the deadline with it, or
/// the bot goes on reminding you of a nine o'clock nothing is at any more.
#[test]
fn moving_an_on_block_moves_its_reminder_too() {
    let db = Db::open_in_memory().unwrap();
    let item = add(&db, "standup on 09/09/2026 09:00", "2026-09-08", "2026-09-08T12:00:00Z");
    let p = block_on(&db, "2026-09-09");
    // Dragged from 9:00 to 11:00 Austin, as the grid writes it.
    move_placement(&db, &p.id, "2026-09-09T11:00:00-05:00", "2026-09-09T12:00:00-05:00").unwrap();

    let stale = soon(&db, "2026-09-09T13:52:00Z");
    assert!(stale.is_empty(), "nothing is at nine any more: {stale:?}");

    let out = soon(&db, "2026-09-09T15:52:00Z");
    assert_eq!(out.len(), 1, "announced once, at the new time: {out:?}");
    assert_eq!(out[0].kind, Kind::Block);
    assert_eq!(out[0].at, utc("2026-09-09T16:00:00Z"));

    let due = ms_core::store::fetch(&db, &item.id).unwrap().due_at;
    assert_eq!(due, Some(utc("2026-09-09T16:00:00Z")), "the deadline followed the block");
}

/// "on" with a range is due when it starts, like "on" with a single time. It
/// used to fall back to 11:59pm and get a second reminder that night, one
/// that moving the block could not take with it.
#[test]
fn an_on_range_is_announced_once_and_moves_with_its_block() {
    let db = Db::open_in_memory().unwrap();
    let item = add(&db, "standup on 09/09/2026 09:00-10:00", "2026-09-08", "2026-09-08T12:00:00Z");
    let due = ms_core::store::fetch(&db, &item.id).unwrap().due_at;
    assert_eq!(due, Some(utc("2026-09-09T14:00:00Z")), "due when the block starts");

    let out = soon(&db, "2026-09-09T13:52:00Z");
    assert_eq!(out.len(), 1, "one notice at nine: {out:?}");
    assert_eq!(out[0].kind, Kind::Block);
    assert!(soon(&db, "2026-09-10T04:52:00Z").is_empty(), "nothing at 11:59pm");

    let p = block_on(&db, "2026-09-09");
    move_placement(&db, &p.id, "2026-09-09T11:00:00-05:00", "2026-09-09T12:00:00-05:00").unwrap();
    assert!(soon(&db, "2026-09-09T13:52:00Z").is_empty(), "nothing is at nine any more");
    assert_eq!(soon(&db, "2026-09-09T15:52:00Z").len(), 1, "announced once at eleven");
}

/// A deadline that was never the block's start is its own fact, and stays.
#[test]
fn moving_a_block_leaves_a_separate_deadline_alone() {
    let db = Db::open_in_memory().unwrap();
    let item = add(&db, "essay due 09/09/2026 23:00", "2026-09-08", "2026-09-08T12:00:00Z");
    ms_core::add_placement(&db, &item.id, "2026-09-09T14:00:00-05:00", "2026-09-09T15:00:00-05:00")
        .unwrap();
    let p = block_on(&db, "2026-09-09");
    move_placement(&db, &p.id, "2026-09-09T16:00:00-05:00", "2026-09-09T17:00:00-05:00").unwrap();

    let due = ms_core::store::fetch(&db, &item.id).unwrap().due_at;
    assert_eq!(due, Some(utc("2026-09-10T04:00:00Z")), "11pm is still 11pm");
    assert_eq!(soon(&db, "2026-09-10T03:52:00Z").len(), 1, "and still gets its reminder");
}

/// Pulling the bottom edge changes when it ends, not when it starts; the one
/// notice stays where it was.
#[test]
fn resizing_an_on_block_announces_it_once() {
    let db = Db::open_in_memory().unwrap();
    add(&db, "standup on 09/09/2026 09:00", "2026-09-08", "2026-09-08T12:00:00Z");
    let p = block_on(&db, "2026-09-09");
    move_placement(&db, &p.id, "2026-09-09T09:00:00-05:00", "2026-09-09T10:30:00-05:00").unwrap();

    let out = soon(&db, "2026-09-09T13:52:00Z");
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].ends, Some(utc("2026-09-09T15:30:00Z")));
}

/// A moved occurrence is a real row. Dragging it a second time, to another
/// day, is a plain move of that row: the grid must not cancel the date it now
/// sits on and place a fresh copy beside the old one, which is how the bot came
/// to announce a class twice.
///
/// This pins the store contract the grid relies on. Choosing that path lives in
/// `app/src/main.js` (drag dispatch on `origin`) and is not exercised here.
#[test]
fn a_moved_occurrence_dragged_again_is_announced_once() {
    let db = Db::open_in_memory().unwrap();
    let item = add(&db, "PHYS201 every wed 10:30-12:00", "2026-09-08", "2026-09-08T12:00:00Z");
    // Wednesday's class goes to Thursday 2pm...
    let copy = move_occurrence(
        &db,
        &item.id,
        d("2026-09-09"),
        "2026-09-10T14:00:00-05:00",
        "2026-09-10T15:30:00-05:00",
    )
    .unwrap();
    // ...and from there to Thursday 4pm, by its own id.
    move_placement(&db, &copy, "2026-09-10T16:00:00-05:00", "2026-09-10T17:30:00-05:00").unwrap();

    assert!(soon(&db, "2026-09-09T15:22:00Z").is_empty(), "Wednesday is cancelled");
    assert!(soon(&db, "2026-09-10T18:52:00Z").is_empty(), "2pm is not where it is");
    let out = soon(&db, "2026-09-10T20:52:00Z");
    assert_eq!(out.len(), 1, "once, at 4pm: {out:?}");
}
