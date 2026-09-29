//! Opening a database created by an older build must work.
//!
//! `CREATE TABLE IF NOT EXISTS` does nothing to a table that already exists, so
//! adding a column to the schema silently left every existing store without it.
//! That shipped, and the Telegram bot answered every message with
//! "table items has no column named location".

use chrono::{TimeZone, Utc};
use ms_core::{add_from_text_in, get_items, Db, Filter};
use chrono_tz::America::Chicago;

fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 31).unwrap()
}
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, 0).unwrap()
}

/// Build a store the way an early build would have, then open it normally.
fn legacy_db(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("legacy.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE items (
          id TEXT PRIMARY KEY,
          title TEXT NOT NULL,
          created_at TEXT NOT NULL
        );
        CREATE TABLE recurrence (
          item_id TEXT PRIMARY KEY,
          byday TEXT NOT NULL,
          start_time TEXT NOT NULL,
          end_time TEXT NOT NULL,
          from_date TEXT NOT NULL
        );
        INSERT INTO items (id, title, created_at)
        VALUES ('old1', 'something from before', '2026-08-01T00:00:00Z');
        ",
    )
    .unwrap();
    drop(conn);
    path
}

#[test]
fn an_older_store_opens_and_still_works() {
    let dir = std::env::temp_dir().join(format!("ms-mig-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = legacy_db(&dir);

    let db = Db::open(&path).expect("an older store must still open");

    // The row that was already there survives.
    let items = get_items(&db, &Filter::default());
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "something from before");

    // And everything added since works against it — this is what was broken.
    let added = add_from_text_in(
        &db, "PHYS201 every mon 10:30-12:00 @Hall 2.106 #work", today(), now(), Chicago,
    )
    .expect("adding must not fail on a migrated store");
    assert_eq!(added.location.as_deref(), Some("Hall 2.106"));
    assert_eq!(added.category.as_deref(), Some("work"));
    assert!(added.recurs);

    let tz: Option<String> = db.conn
        .query_row("SELECT tz FROM recurrence WHERE item_id = ?1",
                   rusqlite::params![added.id], |r| r.get(0)).unwrap();
    assert_eq!(tz.as_deref(), Some("America/Chicago"), "columns added to recurrence too");

    assert!(ms_core::delete_item(&db, &added.id, now()).unwrap(), "and deleting, which needs deleted_at");
    assert_eq!(get_items(&db, &Filter::default()).len(), 1);

    std::fs::remove_file(&path).ok();
}

/// Migrating twice must be harmless — it runs on every single launch.
#[test]
fn migrating_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("ms-mig2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = legacy_db(&dir);

    for _ in 0..3 {
        let db = Db::open(&path).expect("re-opening must be fine");
        assert_eq!(get_items(&db, &Filter::default()).len(), 1);
    }
    std::fs::remove_file(&path).ok();
}
