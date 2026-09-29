use chrono::{DateTime, Duration, TimeZone, Utc};
use ms_core::ics::{parse, sync_text, PREFIX};
use ms_core::{get_items, get_week, import, Db, Filter, ImportRecord};

fn now() -> DateTime<Utc> { Utc.with_ymd_and_hms(2026, 9, 29, 12, 0, 0).unwrap() }

fn feed(events: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:Microsoft Exchange Server 2010\r\n{events}END:VCALENDAR\r\n")
}

/// The shape Outlook publishes: Windows zone names, a weekly rule with an
/// EXDATE, one occurrence moved, one cancelled, and an alarm nested inside.
const CLASS: &str = "BEGIN:VEVENT\r\n\
UID:class-1\r\n\
SUMMARY:CPSC 215\\, lecture\r\n\
LOCATION:MECC\r\n\x20 175\r\n\
DTSTART;TZID=Eastern Standard Time:20260907T133000\r\n\
DTEND;TZID=Eastern Standard Time:20260907T144500\r\n\
RRULE:FREQ=WEEKLY;UNTIL=20261223T133000Z;INTERVAL=1;BYDAY=MO,WE;WKST=MO\r\n\
EXDATE;TZID=Eastern Standard Time:20260930T133000\r\n\
BEGIN:VALARM\r\n\
DESCRIPTION:REMINDER\r\n\
TRIGGER;RELATED=START:-PT15M\r\n\
END:VALARM\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:class-1\r\n\
RECURRENCE-ID;TZID=Eastern Standard Time:20261005T133000\r\n\
SUMMARY:CPSC 215\\, lecture (moved)\r\n\
DTSTART;TZID=Eastern Standard Time:20261006T100000\r\n\
DTEND;TZID=Eastern Standard Time:20261006T111500\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:class-1\r\n\
RECURRENCE-ID;TZID=Eastern Standard Time:20261007T133000\r\n\
STATUS:CANCELLED\r\n\
DTSTART;TZID=Eastern Standard Time:20261007T133000\r\n\
DTEND;TZID=Eastern Standard Time:20261007T144500\r\n\
END:VEVENT\r\n";

const HOLIDAY: &str = "BEGIN:VEVENT\r\nUID:hol\r\nSUMMARY:Fall break\r\n\
DTSTART;VALUE=DATE:20261012\r\nDTEND;VALUE=DATE:20261014\r\nEND:VEVENT\r\n";

fn meeting(uid: &str, start: &str) -> String {
    format!("BEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:{uid}\r\nDTSTART:{start}\r\nDURATION:PT30M\r\nEND:VEVENT\r\n")
}

fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }

fn window() -> (DateTime<Utc>, DateTime<Utc>) {
    (now() - Duration::days(7), now() + Duration::days(14))
}

#[test]
fn an_outlook_weekly_class_expands_with_its_exceptions() {
    let (from, to) = window();
    let p = parse(&feed(CLASS), from, to, chrono_tz::UTC);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);

    let mut starts: Vec<_> = p.records.iter().map(|r| at(r.starts_at.as_ref().unwrap())).collect();
    starts.sort();
    // Mon/Wed 1:30pm Eastern = 17:30 UTC in September. Sep 30 is an EXDATE,
    // Oct 5 was moved to Tue Oct 6 10:00, Oct 7 was cancelled.
    let want: Vec<DateTime<Utc>> = [
        "2026-09-23T17:30:00Z", "2026-09-28T17:30:00Z",
        "2026-10-06T14:00:00Z", "2026-10-12T17:30:00Z",
    ].iter().map(|s| at(s)).collect();
    assert_eq!(starts, want);

    let first = p.records.iter().find(|r| r.starts_at.as_deref().map(at) == Some(want[0])).unwrap();
    assert_eq!(first.title, "CPSC 215, lecture", "escapes undone");
    assert_eq!(first.location.as_deref(), Some("MECC 175"), "folded line joined");
    assert_eq!(at(first.ends_at.as_ref().unwrap()) - want[0], Duration::minutes(75));

    let moved = p.records.iter().find(|r| r.title.ends_with("(moved)")).unwrap();
    assert_eq!(moved.external_id, format!("{PREFIX}class-1@20261005T173000Z"),
        "keyed by where it was, so moving it again updates rather than adds");
}

#[test]
fn all_day_events_are_counted_not_placed() {
    let (from, to) = window();
    let p = parse(&feed(HOLIDAY), from, to, chrono_tz::UTC);
    assert!(p.records.is_empty());
    assert_eq!(p.skipped_all_day, 1);
}

#[test]
fn synced_events_sit_on_the_week_and_stay_out_of_the_list() {
    let db = Db::open_in_memory().unwrap();
    let (o, _) = sync_text(&db, &feed(&meeting("standup", "20260930T150000Z")), now(), chrono_tz::UTC).unwrap();
    assert_eq!(o.added, 1);

    let items = get_items(&db, &Filter::default());
    assert!(!items[0].listed, "nobody ticks off a meeting");
    let week = get_week(&db, now().date_naive(), chrono_tz::UTC);
    let placed: Vec<_> = week.days.iter().flat_map(|d| &d.placements).collect();
    assert_eq!(placed.len(), 1);
    assert_eq!(placed[0].starts_at.with_timezone(&Utc), at("2026-09-30T15:00:00Z"));
}

#[test]
fn re_syncing_the_same_feed_adds_nothing() {
    let db = Db::open_in_memory().unwrap();
    let text = feed(CLASS);
    sync_text(&db, &text, now(), chrono_tz::UTC).unwrap();
    let before = get_items(&db, &Filter::default()).len();
    let (o, _) = sync_text(&db, &text, now(), chrono_tz::UTC).unwrap();
    assert_eq!((o.added, o.removed), (0, 0));
    assert_eq!(get_items(&db, &Filter::default()).len(), before);
    let blocks: i64 = db.conn.query_row("SELECT count(*) FROM placements", [], |r| r.get(0)).unwrap();
    assert_eq!(blocks as usize, before, "one block per event, not one per run");
}

/// Cancelled at the source means gone here, but only going forward: last
/// month's meetings fall out of the window because they are old, and the
/// week already lived should keep them.
#[test]
fn a_cancelled_meeting_leaves_and_an_old_one_stays() {
    let db = Db::open_in_memory().unwrap();
    let old = now() - Duration::days(30);
    import(&db, &[ImportRecord {
        external_id: format!("{PREFIX}last-month"),
        title: "last month".into(),
        starts_at: Some(old.to_rfc3339()),
        ends_at: Some((old + Duration::hours(1)).to_rfc3339()),
        ..Default::default()
    }], now()).unwrap();

    let both = feed(&(meeting("keep", "20261001T150000Z") + &meeting("drop", "20261002T150000Z")));
    sync_text(&db, &both, now(), chrono_tz::UTC).unwrap();
    let (o, _) = sync_text(&db, &feed(&meeting("keep", "20261001T150000Z")), now(), chrono_tz::UTC).unwrap();
    assert_eq!(o.removed, 1);

    let mut titles: Vec<_> = get_items(&db, &Filter::default()).into_iter().map(|i| i.title).collect();
    titles.sort();
    assert_eq!(titles, ["keep", "last month"]);
}

#[test]
fn a_sync_never_touches_what_you_typed_or_other_imports() {
    let db = Db::open_in_memory().unwrap();
    ms_core::add_from_text(&db, "dentist on thu 3pm", now().date_naive(), now()).unwrap();
    import(&db, &[ImportRecord {
        external_id: "canvas:1".into(), title: "pset".into(),
        due_at: Some("2026-10-02T23:59:00Z".into()), ..Default::default()
    }], now()).unwrap();

    sync_text(&db, &feed(""), now(), chrono_tz::UTC).unwrap();
    assert_eq!(get_items(&db, &Filter::default()).len(), 2);
}

/// An expired or private link answers with a sign-in page. Read as a calendar
/// it is an empty one, and an empty calendar deletes every meeting.
#[test]
fn a_page_that_is_not_a_calendar_changes_nothing() {
    let db = Db::open_in_memory().unwrap();
    sync_text(&db, &feed(&meeting("keep", "20261001T150000Z")), now(), chrono_tz::UTC).unwrap();

    let err = sync_text(&db, "<html><body>Sign in</body></html>", now(), chrono_tz::UTC).unwrap_err();
    assert!(err.contains("not a calendar"), "{err}");
    assert_eq!(get_items(&db, &Filter::default()).len(), 1);
}

#[test]
fn an_unknown_zone_is_read_as_the_fallback_and_said_so() {
    let (from, to) = window();
    let ev = "BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:x\r\nDTSTART;TZID=Somewhere Standard Time:20260930T090000\r\n\
DTEND;TZID=Somewhere Standard Time:20260930T100000\r\nEND:VEVENT\r\n";
    let p = parse(&feed(ev), from, to, chrono_tz::Asia::Beirut);
    assert_eq!(p.records.len(), 1);
    assert_eq!(at(p.records[0].starts_at.as_ref().unwrap()), at("2026-09-30T06:00:00Z"));
    assert_eq!(p.warnings.len(), 1);
}

#[test]
fn a_block_needs_both_ends() {
    let db = Db::open_in_memory().unwrap();
    let err = import(&db, &[ImportRecord {
        external_id: "x:1".into(), title: "half".into(),
        starts_at: Some("2026-10-01T15:00:00Z".into()), ..Default::default()
    }], now()).unwrap_err();
    assert!(format!("{err}").contains("both"), "{err}");
    assert!(get_items(&db, &Filter::default()).is_empty());
}

/// A feed published as free/busy calls everything "Busy". A name given here to
/// one occurrence survives the next sync and reaches the rest of the series,
/// including weeks that had not arrived yet.
#[test]
fn a_name_given_to_a_busy_series_sticks_and_spreads() {
    let db = Db::open_in_memory().unwrap();
    let busy = |until: &str| feed(&format!("BEGIN:VEVENT\r\nUID:abc@google.com\r\nSUMMARY:Busy\r\n\
DTSTART:20260928T123000Z\r\nDTEND:20260928T134500Z\r\nRRULE:FREQ=WEEKLY;UNTIL={until}\r\nEND:VEVENT\r\n"));
    sync_text(&db, &busy("20261006T000000Z"), now(), chrono_tz::UTC).unwrap();

    let first = &get_items(&db, &Filter::default())[0];
    db.conn.execute("UPDATE items SET title = 'CPSC 352', location = 'MECC 124' WHERE id = ?1",
        rusqlite::params![first.id]).unwrap();

    // The series grows by a week, as the window rolls forward.
    sync_text(&db, &busy("20261013T000000Z"), now(), chrono_tz::UTC).unwrap();
    let items = get_items(&db, &Filter::default());
    assert_eq!(items.len(), 3);
    for i in &items {
        assert_eq!((i.title.as_str(), i.location.as_deref()), ("CPSC 352", Some("MECC 124")));
    }
}
