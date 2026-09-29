//! Deleting hides an item everywhere it could turn up. The row is kept, so an
//! undo brings it back whole and a re-import cannot bring it back at all.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::America::Chicago;
use ms_core::{
    active_session, add_from_text_in, delete_item, due_soon, get_days, get_items, import,
    restore_item, restore_last, set_done, stats, timer_start, timer_stop, Db, Filter,
    ImportRecord,
};

fn utc(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }
fn d(s: &str) -> NaiveDate { NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap() }

fn add(db: &Db, text: &str) -> ms_core::Item {
    add_from_text_in(db, text, d("2026-09-08"), utc("2026-09-08T12:00:00Z"), Chicago).unwrap()
}

fn titles(db: &Db) -> Vec<String> {
    get_items(db, &Filter::default()).into_iter().map(|i| i.title).collect()
}

fn blocks_on(db: &Db, day: &str) -> usize {
    get_days(db, d(day), Chicago).days[0].placements.len()
}

#[test]
fn a_deleted_item_leaves_the_list_the_week_and_the_reminders() {
    let db = Db::open_in_memory().unwrap();
    let item = add(&db, "standup on 09/09/2026 09:00");
    assert_eq!(blocks_on(&db, "2026-09-09"), 1);
    let before = due_soon(&db, utc("2026-09-09T13:52:00Z"), Duration::minutes(10), Chicago);
    assert!(!before.is_empty(), "the reminder exists before");

    assert!(delete_item(&db, &item.id, utc("2026-09-08T13:00:00Z")).unwrap());

    assert!(titles(&db).is_empty());
    assert_eq!(blocks_on(&db, "2026-09-09"), 0, "its block left the week");
    let after = due_soon(&db, utc("2026-09-09T13:52:00Z"), Duration::minutes(10), Chicago);
    assert!(after.is_empty(), "no reminder for a deleted item: {after:?}");
    assert!(ms_core::store::fetch(&db, &item.id).is_none());
}

#[test]
fn deleting_a_repeat_takes_every_week_of_it() {
    let db = Db::open_in_memory().unwrap();
    let gym = add(&db, "gym mon/wed/fri 6am");
    assert_eq!(blocks_on(&db, "2026-09-09"), 1);
    delete_item(&db, &gym.id, utc("2026-09-08T13:00:00Z")).unwrap();
    for day in ["2026-09-09", "2026-09-16", "2026-10-14"] {
        assert_eq!(blocks_on(&db, day), 0, "gym still on {day}");
    }
}

#[test]
fn restoring_brings_it_back_as_it_was() {
    let db = Db::open_in_memory().unwrap();
    let hw = add(&db, "math hw fri ~2h");
    set_done(&db, &hw.id, true, None, utc("2026-09-08T12:30:00Z")).unwrap();
    delete_item(&db, &hw.id, utc("2026-09-08T13:00:00Z")).unwrap();

    assert!(restore_item(&db, &hw.id).unwrap());
    let back = ms_core::store::fetch(&db, &hw.id).expect("visible again");
    assert_eq!(back.title, "math hw");
    assert_eq!(back.estimate_min, Some(120));
    assert!(back.completed, "its tick survived the round trip");
}

#[test]
fn delete_and_restore_say_when_there_was_nothing_to_do() {
    let db = Db::open_in_memory().unwrap();
    let hw = add(&db, "math hw");
    assert!(!restore_item(&db, &hw.id).unwrap(), "was never deleted");
    assert!(delete_item(&db, &hw.id, utc("2026-09-08T13:00:00Z")).unwrap());
    assert!(!delete_item(&db, &hw.id, utc("2026-09-08T13:01:00Z")).unwrap(), "already gone");
    assert!(!delete_item(&db, "no-such-id", utc("2026-09-08T13:01:00Z")).unwrap());
}

#[test]
fn undo_restores_the_most_recent_deletion() {
    let db = Db::open_in_memory().unwrap();
    let a = add(&db, "first");
    let b = add(&db, "second");
    delete_item(&db, &a.id, utc("2026-09-08T13:00:00Z")).unwrap();
    delete_item(&db, &b.id, utc("2026-09-08T13:05:00Z")).unwrap();

    assert_eq!(restore_last(&db).unwrap().map(|i| i.title), Some("second".into()));
    assert_eq!(restore_last(&db).unwrap().map(|i| i.title), Some("first".into()));
    assert!(restore_last(&db).unwrap().is_none());
}

#[test]
fn a_deleted_import_stays_deleted_when_imported_again() {
    let db = Db::open_in_memory().unwrap();
    // Read from JSON, the way an agent hands it over.
    let rec: ImportRecord = serde_json::from_str(
        r#"{"external_id": "canvas:assignment:88213", "title": "STAT240 problem set 4",
            "due_at": "2026-09-11T23:59:00-05:00"}"#,
    )
    .unwrap();
    import(&db, std::slice::from_ref(&rec), utc("2026-09-08T12:00:00Z")).unwrap();
    let id = get_items(&db, &Filter::default())[0].id.clone();
    delete_item(&db, &id, utc("2026-09-08T13:00:00Z")).unwrap();

    let again = import(&db, std::slice::from_ref(&rec), utc("2026-09-09T12:00:00Z")).unwrap();
    assert_eq!((again.added, again.updated), (0, 1), "matched the hidden row, no copy");
    assert!(titles(&db).is_empty(), "the next sync must not bring it back");

    // And the hidden row took the update, so a restore shows current data.
    let moved = ImportRecord { due_at: Some("2026-09-12T23:59:00-05:00".into()), ..rec };
    import(&db, &[moved], utc("2026-09-09T12:00:00Z")).unwrap();
    restore_item(&db, &id).unwrap();
    let back = ms_core::store::fetch(&db, &id).unwrap();
    assert_eq!(back.due_at, Some(utc("2026-09-13T04:59:00Z")));
}

#[test]
fn a_running_timer_stops_and_tracked_time_still_counts() {
    let db = Db::open_in_memory().unwrap();
    let hw = add(&db, "essay ~1h");
    let s = timer_start(&db, &hw.id, None, utc("2026-09-08T13:00:00Z")).unwrap().started;
    timer_stop(&db, &s.id, utc("2026-09-08T15:00:00Z")).unwrap();
    timer_start(&db, &hw.id, None, utc("2026-09-08T16:00:00Z")).unwrap();

    delete_item(&db, &hw.id, utc("2026-09-08T16:30:00Z")).unwrap();

    assert!(active_session(&db).is_none(), "the timer was left running on a deleted item");
    assert_eq!(stats(&db, None).n, 1, "two hours against a one-hour estimate still counts");
}
