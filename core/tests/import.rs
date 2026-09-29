use chrono::{DateTime, TimeZone, Utc};
use ms_core::{get_items, import, Db, Filter, ImportRecord};

fn now() -> DateTime<Utc> { Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, 0).unwrap() }

fn rec(ext: &str, title: &str) -> ImportRecord {
    ImportRecord {
        external_id: ext.into(),
        title: title.into(),
        due_at: Some("2026-09-04T23:59:00-05:00".into()),
        estimate_min: Some(120),
        tags: vec!["math".into()],
        location: None,
        repeat: None,
        ..Default::default()
    }
}

/// The failure that produced the predecessor's duplicate tasks was a fresh
/// uuid on every agent call. Agents retry; the same import must be a no-op.
#[test]
fn importing_the_same_records_twice_changes_nothing() {
    let db = Db::open_in_memory().unwrap();
    let batch = vec![rec("canvas:1", "STAT240 problem set"), rec("canvas:2", "MATH210 lab")];

    let first = import(&db, &batch, now()).unwrap();
    assert_eq!((first.added, first.updated), (2, 0));

    let second = import(&db, &batch, now()).unwrap();
    assert_eq!((second.added, second.updated), (0, 2), "recognised, not re-added");

    assert_eq!(get_items(&db, &Filter::default()).len(), 2, "still two items");
}

#[test]
fn a_corrected_record_updates_in_place() {
    let db = Db::open_in_memory().unwrap();
    import(&db, &[rec("canvas:1", "STAT240 problem set")], now()).unwrap();

    let mut fixed = rec("canvas:1", "STAT240 problem set (revised)");
    fixed.due_at = Some("2026-09-06T23:59:00-05:00".into());
    import(&db, &[fixed], now()).unwrap();

    let items = get_items(&db, &Filter::default());
    assert_eq!(items.len(), 1, "no rival row");
    assert_eq!(items[0].title, "STAT240 problem set (revised)");
    assert_eq!(items[0].source, "import");
}

/// Without a caller-owned key there is nothing to recognise a repeat by, so
/// the record is refused rather than silently duplicated later.
#[test]
fn a_record_without_an_external_id_is_refused_and_nothing_is_written() {
    let db = Db::open_in_memory().unwrap();
    let mut bad = rec("", "no key");
    bad.external_id = String::new();

    let batch = vec![rec("canvas:1", "fine"), bad, rec("canvas:3", "also fine")];
    let err = import(&db, &batch, now()).unwrap_err();

    assert!(format!("{err}").contains("external_id"), "error should name the problem: {err}");
    assert!(
        get_items(&db, &Filter::default()).is_empty(),
        "a partial import is worse than none: the whole batch must roll back"
    );
}

#[test]
fn imports_never_touch_what_you_typed_yourself() {
    let db = Db::open_in_memory().unwrap();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
    ms_core::add_from_text(&db, "STAT240 problem set", today, now()).unwrap();

    import(&db, &[rec("canvas:1", "STAT240 problem set")], now()).unwrap();

    let items = get_items(&db, &Filter::default());
    assert_eq!(items.len(), 2, "same title, different provenance, both kept");
    assert_eq!(items.iter().filter(|i| i.source == "self").count(), 1);
    assert_eq!(items.iter().filter(|i| i.external_id.is_none()).count(), 1);
}

/// A class timetable is the main thing an agent imports, and it is recurring.
/// Without this the agent has to fall back to `add`, which duplicates on every
/// re-run — the exact failure `import` exists to prevent.
#[test]
fn a_recurring_class_can_be_imported_and_re_imported() {
    let db = Db::open_in_memory().unwrap();
    let class = ImportRecord {
        external_id: "gcal:phys201".into(),
        title: "PHYS201".into(),
        due_at: None,
        estimate_min: None,
        tags: vec!["class".into()],
        location: Some("Hall 2.106".into()),
        repeat: Some(ms_core::ImportRepeat {
            byday: vec!["mon".into(), "wed".into()],
            start_time: "10:30".into(),
            end_time: "12:00".into(),
            tz: Some("America/Chicago".into()),
        }),
        ..Default::default()
    };

    let first = import(&db, std::slice::from_ref(&class), now()).unwrap();
    assert_eq!((first.added, first.updated), (1, 0));

    let items = get_items(&db, &Filter::default());
    assert_eq!(items.len(), 1);
    assert!(items[0].recurs, "imported as a repeat, not a one-off");
    assert!(!items[0].listed, "a timetabled class does not belong in the to-do list");
    assert_eq!(items[0].location.as_deref(), Some("Hall 2.106"));

    let (byday, start, tz): (String, String, Option<String>) = db.conn.query_row(
        "SELECT byday, start_time, tz FROM recurrence WHERE item_id = ?1",
        rusqlite::params![items[0].id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!(byday, "mon,wed");
    assert_eq!(start, "10:30");
    assert_eq!(tz.as_deref(), Some("America/Chicago"));

    // Re-running must not add a second class, nor a second rule.
    let second = import(&db, &[class], now()).unwrap();
    assert_eq!((second.added, second.updated), (0, 1));
    assert_eq!(get_items(&db, &Filter::default()).len(), 1);
    let rules: i64 = db.conn
        .query_row("SELECT count(*) FROM recurrence", [], |r| r.get(0)).unwrap();
    assert_eq!(rules, 1, "one rule, not two");
}

/// Dropping the repeat on a later import turns it back into a plain item —
/// a class that was cancelled should not keep appearing on the week.
#[test]
fn removing_the_repeat_on_re_import_clears_the_rule() {
    let db = Db::open_in_memory().unwrap();
    let mut r = ImportRecord {
        external_id: "gcal:x".into(), title: "seminar".into(), due_at: None,
        estimate_min: None, tags: vec![], location: None,
        repeat: Some(ms_core::ImportRepeat {
            byday: vec!["fri".into()], start_time: "15:00".into(),
            end_time: "16:00".into(), tz: None,
        }),
        ..Default::default()
    };
    import(&db, std::slice::from_ref(&r), now()).unwrap();
    assert!(get_items(&db, &Filter::default())[0].recurs);

    r.repeat = None;
    import(&db, &[r], now()).unwrap();
    assert!(!get_items(&db, &Filter::default())[0].recurs, "rule removed");
}

/// AGENT.md is what an agent is pointed at, so its example must really work.
/// Documentation that has drifted from the code is worse than none: the agent
/// follows it confidently and writes nothing, or the wrong thing.
#[test]
fn the_example_in_agent_md_actually_imports() {
    let doc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../AGENT.md"),
    )
    .expect("AGENT.md should sit beside the crates");

    let block = doc
        .split("```json")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("AGENT.md should carry a json import example");

    let records: Vec<ImportRecord> =
        serde_json::from_str(block).expect("the documented shape must deserialize");
    assert!(records.len() >= 2, "the example should show both a deadline and a repeat");
    assert!(
        records.iter().any(|r| r.repeat.is_some()),
        "the example must cover the recurring case — it is the main thing an agent imports"
    );

    let db = Db::open_in_memory().unwrap();
    let out = import(&db, &records, now()).unwrap();
    assert_eq!(out.added, records.len());

    // And re-running it is still a no-op, which is the document's headline claim.
    let again = import(&db, &records, now()).unwrap();
    assert_eq!((again.added, again.updated), (0, records.len()));
}
