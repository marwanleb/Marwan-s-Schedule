use crate::db::Db;
use crate::parse::CATEGORIES;
use crate::store::new_id;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One record in a bulk import. `external_id` is required and caller-owned:
/// it is what makes a re-run a no-op instead of a duplicate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImportRecord {
    pub external_id: String,
    pub title: String,
    #[serde(default)]
    pub due_at: Option<String>,
    #[serde(default)]
    pub estimate_min: Option<u32>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub location: Option<String>,
    /// A weekly commitment — a class, a shift. Without this an agent importing
    /// a timetable would have to fall back to `add`, which duplicates on every
    /// re-run, defeating the point of importing.
    #[serde(default)]
    pub repeat: Option<ImportRepeat>,
    /// A single dated block — a meeting from someone else's calendar. Both
    /// RFC3339, both or neither. Like a repeat, it occupies the week and stays
    /// out of the to-do list: nobody ticks off a meeting.
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default)]
    pub ends_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportRepeat {
    /// `mon` … `sun`.
    pub byday: Vec<String>,
    /// Wall clock, `HH:MM`.
    pub start_time: String,
    pub end_time: String,
    /// IANA zone to fix it to; omit to follow the machine.
    #[serde(default)]
    pub tz: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportOutcome {
    pub added: usize,
    pub updated: usize,
    /// Only ever non-zero from `sync`; a plain import never deletes.
    pub removed: usize,
}

/// Upsert a batch keyed on `external_id`, in a single transaction.
///
/// Two properties matter more than anything else here, because an agent is the
/// usual caller and agents retry:
///
/// * Re-running the same batch changes nothing. The key is supplied by the
///   caller and enforced by a unique index, not by asking the agent to be
///   careful.
/// * A batch applies completely or not at all. A half-applied import is worse
///   than a failed one, because it looks like it worked.
///
/// Items you typed yourself have no `external_id` and are never matched.
pub fn import(
    db: &Db,
    records: &[ImportRecord],
    now: DateTime<Utc>,
) -> rusqlite::Result<ImportOutcome> {
    validate(records)?;
    let tx = db.conn.unchecked_transaction()?;
    let outcome = apply(&tx, records, now)?;
    tx.commit()?;
    Ok(outcome)
}

/// Make everything under `prefix` match `records` exactly: upsert what is
/// there, delete what is not.
///
/// This is what a calendar subscription needs and a plain import cannot give:
/// a meeting cancelled at the source has to leave the week, and nothing in an
/// upsert ever removes anything.
///
/// Only items that start at or after `window_from` are eligible for removal.
/// The source is read through a window, so a meeting from last month is absent
/// because it is old, not because it was cancelled, and keeping it keeps the
/// week you already lived intact.
///
/// Items you typed yourself have no `external_id` and are never touched; nor is
/// anything imported under a different prefix.
pub fn sync(
    db: &Db,
    prefix: &str,
    records: &[ImportRecord],
    window_from: DateTime<Utc>,
    now: DateTime<Utc>,
) -> rusqlite::Result<ImportOutcome> {
    if prefix.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName(
            "a sync needs a prefix, or it would own every imported item".into(),
        ));
    }
    if let Some(r) = records.iter().find(|r| !r.external_id.starts_with(prefix)) {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "{:?} is outside {prefix:?}; nothing was imported",
            r.external_id
        )));
    }
    validate(records)?;

    let tx = db.conn.unchecked_transaction()?;
    let mut outcome = apply(&tx, records, now)?;

    let keep: std::collections::HashSet<&str> =
        records.iter().map(|r| r.external_id.as_str()).collect();
    let owned: Vec<(String, String, Option<String>)> = {
        let mut stmt = tx.prepare(
            "SELECT i.id, i.external_id,
                    (SELECT min(p.starts_at) FROM placements p WHERE p.item_id = i.id)
               FROM items i
              WHERE i.external_id IS NOT NULL AND substr(i.external_id, 1, ?2) = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![prefix, prefix.len() as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (id, ext, starts) in owned {
        if keep.contains(ext.as_str()) {
            continue;
        }
        // Compared as instants: stored offsets differ, strings would not order.
        let past = starts
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|s| s < window_from);
        if past {
            continue;
        }
        tx.execute("DELETE FROM items WHERE id = ?1", rusqlite::params![id])?;
        outcome.removed += 1;
    }

    tx.commit()?;
    Ok(outcome)
}

/// Check the whole batch before anything is written.
fn validate(records: &[ImportRecord]) -> rusqlite::Result<()> {
    for (i, r) in records.iter().enumerate() {
        if r.external_id.trim().is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "record {i} ({:?}) has no external_id; nothing was imported",
                r.title
            )));
        }
        if r.title.trim().is_empty() {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "record {i} has an empty title; nothing was imported"
            )));
        }
        if let Some(rep) = &r.repeat {
            if rep.byday.is_empty() {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "record {i} ({:?}) repeats on no days; nothing was imported",
                    r.title
                )));
            }
            if let Some(tz) = &rep.tz {
                if tz.parse::<chrono_tz::Tz>().is_err() {
                    return Err(rusqlite::Error::InvalidParameterName(format!(
                        "record {i} ({:?}) has unknown time zone {tz:?}; nothing was imported",
                        r.title
                    )));
                }
            }
        }
        match (&r.starts_at, &r.ends_at) {
            (None, None) => {}
            (Some(s), Some(e)) => {
                if r.repeat.is_some() {
                    return Err(rusqlite::Error::InvalidParameterName(format!(
                        "record {i} ({:?}) has both a repeat and a block; nothing was imported",
                        r.title
                    )));
                }
                let (Ok(s), Ok(e)) = (
                    DateTime::parse_from_rfc3339(s),
                    DateTime::parse_from_rfc3339(e),
                ) else {
                    return Err(rusqlite::Error::InvalidParameterName(format!(
                        "record {i} ({:?}) has a block time that is not RFC3339; nothing was imported",
                        r.title
                    )));
                };
                if e <= s {
                    return Err(rusqlite::Error::InvalidParameterName(format!(
                        "record {i} ({:?}) ends before it starts; nothing was imported",
                        r.title
                    )));
                }
            }
            _ => {
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "record {i} ({:?}) needs both starts_at and ends_at; nothing was imported",
                    r.title
                )));
            }
        }
    }
    Ok(())
}

fn apply(
    tx: &rusqlite::Transaction<'_>,
    records: &[ImportRecord],
    now: DateTime<Utc>,
) -> rusqlite::Result<ImportOutcome> {
    let mut outcome = ImportOutcome::default();
    for r in records {
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM items WHERE external_id = ?1",
                rusqlite::params![r.external_id],
                |row| row.get(0),
            )
            .ok();

        let tags = r.tags.join(",");
        // A timetabled commitment occupies the week; it does not belong in the
        // to-do list. Spec 3.1.
        let listed = (r.repeat.is_none() && r.starts_at.is_none()) as i64;
        let category = r
            .tags
            .iter()
            .find(|t| CATEGORIES.contains(&t.to_ascii_lowercase().as_str()))
            .map(|t| t.to_ascii_lowercase());

        match existing {
            Some(id) => {
                tx.execute(
                    "UPDATE items SET title = ?2, tags = ?3, category = ?4, due_at = ?5,
                                      estimate_min = ?6, location = ?7, listed = ?8
                     WHERE id = ?1",
                    rusqlite::params![
                        id, r.title, tags, category, r.due_at, r.estimate_min,
                        r.location, listed
                    ],
                )?;
                write_repeat(tx, &id, r)?;
                write_block(tx, &id, r)?;
                outcome.updated += 1;
            }
            None => {
                let id = new_id();
                tx.execute(
                    "INSERT INTO items
                       (id, title, tags, category, location, listed, due_at, estimate_min,
                        created_at, source, external_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'import', ?10)",
                    rusqlite::params![
                        id,
                        r.title,
                        tags,
                        category,
                        r.location,
                        listed,
                        r.due_at,
                        r.estimate_min,
                        now.to_rfc3339(),
                        r.external_id
                    ],
                )?;
                write_repeat(tx, &id, r)?;
                write_block(tx, &id, r)?;
                outcome.added += 1;
            }
        }
    }

    Ok(outcome)
}

/// The source owns the block's time: a re-run puts it back where the source
/// says, and a record that lost its time loses its block. A block moved out of
/// a repeat (`moved`) is a different thing and is left alone.
fn write_block(
    tx: &rusqlite::Transaction<'_>,
    item_id: &str,
    r: &ImportRecord,
) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM placements WHERE item_id = ?1 AND origin = 'oneoff'",
        rusqlite::params![item_id],
    )?;
    if let (Some(s), Some(e)) = (&r.starts_at, &r.ends_at) {
        tx.execute(
            "INSERT INTO placements (id, item_id, starts_at, ends_at, origin)
             VALUES (?1, ?2, ?3, ?4, 'oneoff')",
            rusqlite::params![
                format!("plc_{}", uuid::Uuid::new_v4().simple()),
                item_id,
                s,
                e
            ],
        )?;
    }
    Ok(())
}

/// Replace the item's rule with whatever the record says, including removing it
/// when the record no longer repeats — a cancelled class must stop appearing.
fn write_repeat(
    tx: &rusqlite::Transaction<'_>,
    item_id: &str,
    r: &ImportRecord,
) -> rusqlite::Result<()> {
    let Some(rep) = &r.repeat else {
        tx.execute(
            "DELETE FROM recurrence WHERE item_id = ?1",
            rusqlite::params![item_id],
        )?;
        return Ok(());
    };
    tx.execute(
        "INSERT INTO recurrence (item_id, byday, start_time, end_time, tz, from_date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(item_id) DO UPDATE
           SET byday = ?2, start_time = ?3, end_time = ?4, tz = ?5",
        rusqlite::params![
            item_id,
            rep.byday.join(","),
            rep.start_time,
            rep.end_time,
            rep.tz,
            "1970-01-01",
        ],
    )?;
    Ok(())
}
