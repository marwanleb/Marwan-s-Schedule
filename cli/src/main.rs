//! `sched` — the command-line front door to the same store and the same parser
//! the app uses. See spec §8.
//!
//! Exit codes: 0 success, 1 refused (bad input, nothing written), 2 not found.

use chrono::{Local, NaiveDate, Utc};
use clap::{Parser, Subcommand};
use ms_core::{
    add_from_text, delete_item, find_by_prefix, get_items, get_week, help_text, import, set_done,
    stats, Db, Filter, ImportRecord,
};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "sched",
    about = "Marwan's Schedule — capture, list and time your week",
    long_about = None,
    disable_help_subcommand = true
)]
struct Cli {
    /// Store to use. Defaults to the app's own database.
    #[arg(long, global = true)]
    db: Option<PathBuf>,

    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Add an item from a typed line, e.g. "math hw fri ~2h #math"
    Add { text: Vec<String> },
    /// Show open items
    List {
        /// Only what is due today or overdue
        #[arg(long)]
        today: bool,
    },
    /// Show this week's schedule
    Week {
        /// Any date inside the week; defaults to now
        #[arg(long)]
        on: Option<String>,
    },
    /// Tick off the first item whose title starts with the given text
    Done {
        text: Vec<String>,
        /// Which occurrence, for a repeating item (YYYY-MM-DD)
        #[arg(long)]
        on: Option<String>,
    },
    /// Cancel one occurrence of a repeating item
    Except { id: String, date: String },
    /// Delete an item outright
    Rm { id: String },
    /// Bulk upsert from a JSON file; safe to re-run
    Import { file: PathBuf },
    /// Mirror a published calendar (an .ics link) onto the week; safe to re-run
    SyncIcs {
        /// The feed's address. Remembered, so the app keeps syncing it; omit to
        /// reuse the saved one
        url: Option<String>,
        /// Read a downloaded .ics file instead of fetching
        #[arg(long, conflicts_with = "url")]
        file: Option<PathBuf>,
        /// Forget the saved address and take its upcoming events off the week
        #[arg(long, conflicts_with_all = ["url", "file"])]
        forget: bool,
    },
    /// Dump every item as JSON
    Export,
    /// How much longer things take than estimated
    Stats {
        #[arg(long)]
        tag: Option<String>,
    },
    /// Syntax and command reference
    Help,
}

fn default_db() -> PathBuf {
    // Must match the app's Tauri app_data_dir (APPDATA/<identifier>) and the
    // bot's, or the three front doors open different stores.
    if let Ok(explicit) = std::env::var("SCHEDULE_DB") {
        return PathBuf::from(explicit);
    }
    let base = PathBuf::from(
        std::env::var("APPDATA")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".into()),
    );
    // Tauri's app_data_dir on a Mac. Without this the CLI opened a different
    // store in the home folder and nothing it wrote reached the window.
    #[cfg(target_os = "macos")]
    let base = base.join("Library").join("Application Support");
    base.join("com.marwan.schedule").join("schedule.db")
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("sched: {msg}");
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if matches!(cli.command, Cmd::Help) {
        print!("{}", help_text());
        return ExitCode::SUCCESS;
    }

    let path = cli.db.clone().unwrap_or_else(default_db);
    let db = match Db::open(&path) {
        Ok(db) => db,
        Err(e) => return fail(&format!("could not open {}: {e}", path.display())),
    };

    let now = Utc::now();
    let today = Local::now().date_naive();

    match cli.command {
        Cmd::Help => unreachable!("handled above"),

        Cmd::Add { text } => {
            let line = text.join(" ");
            if line.trim().is_empty() {
                return fail("nothing to add");
            }
            match add_from_text(&db, &line, today, now) {
                Ok(item) => {
                    if cli.json {
                        println!("{}", item_json(&item));
                    } else {
                        println!("added  {}  [{}]", item.title, item.id);
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&format!("could not add: {e}")),
            }
        }

        Cmd::List { today: only_today } => {
            let mut items = get_items(
                &db,
                &Filter {
                    listed: Some(true),
                    done: Some(false),
                    // Done-ness is per occurrence: a weekly chore ticked off
                    // last week is open again today. Spec 3.2.
                    on: Some(today),
                    ..Default::default()
                },
            );
            if only_today {
                let end = today.and_hms_opt(23, 59, 59).unwrap().and_utc();
                items.retain(|i| i.due_at.is_some_and(|d| d <= end));
            }
            if cli.json {
                println!("[{}]", items.iter().map(item_json).collect::<Vec<_>>().join(","));
            } else if items.is_empty() {
                println!("nothing open");
            } else {
                for i in &items {
                    let due = i
                        .due_at
                        .map(|d| d.with_timezone(&Local).format("  due %a %d %b").to_string())
                        .unwrap_or_default();
                    let est = i
                        .estimate_min
                        .map(|m| format!("  ~{}", fmt_mins(m)))
                        .unwrap_or_default();
                    println!("  [ ] {}{}{}", i.title, due, est);
                }
            }
            ExitCode::SUCCESS
        }

        Cmd::Week { on } => {
            let anchor = match on {
                None => today,
                Some(s) => match NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
                    Ok(d) => d,
                    Err(_) => return fail(&format!("not a date: {s}")),
                },
            };
            let zone = local_zone();
            let week = get_week(&db, anchor, zone);

            if cli.json {
                println!("{}", week_json(&week));
            } else {
                println!("week of {}   ({})", week.anchor, week.viewing_tz);
                for day in &week.days {
                    println!("{}", day.date.format("%a %d %b"));
                    if day.placements.is_empty() {
                        println!("      —");
                    }
                    for p in &day.placements {
                        let title = ms_core::store::fetch(&db, &p.item_id)
                            .map(|i| i.title)
                            .unwrap_or_else(|| p.item_id.clone());
                        let zone = if p.foreign {
                            p.pinned_tz.clone().unwrap_or_default()
                        } else {
                            String::new()
                        };
                        // Padded so titles line up down the week.
                        println!(
                            "      {:<15} {} {}",
                            ms_core::range12(p.starts_at.time(), p.ends_at.time()),
                            title,
                            zone
                        );
                    }
                }
                for d in &week.diagnostics {
                    eprintln!("note: {}", d.message);
                }
            }
            ExitCode::SUCCESS
        }

        Cmd::Done { text, on } => {
            let prefix = text.join(" ");
            let matches = find_by_prefix(&db, &prefix);
            match matches.len() {
                0 => {
                    eprintln!("sched: nothing matches {prefix:?}");
                    ExitCode::from(2)
                }
                1 => {
                    let date = match on.as_deref().map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d")) {
                        Some(Ok(d)) => Some(d),
                        Some(Err(_)) => return fail("--on wants YYYY-MM-DD"),
                        // A repeating item is completed for one occurrence;
                        // default to today rather than forever. Spec 3.2.
                        None if matches[0].recurs => Some(today),
                        None => None,
                    };
                    match set_done(&db, &matches[0].id, true, date, now) {
                        Ok(_) => {
                            println!("done   {}", matches[0].title);
                            ExitCode::SUCCESS
                        }
                        Err(e) => fail(&format!("could not update: {e}")),
                    }
                }
                _ => {
                    eprintln!("sched: {prefix:?} matches {} items:", matches.len());
                    for m in &matches {
                        eprintln!("  {}  {}", m.id, m.title);
                    }
                    ExitCode::from(2)
                }
            }
        }

        Cmd::Except { id, date } => {
            if NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
                return fail("date wants YYYY-MM-DD");
            }
            let existing: Result<String, _> = db.conn.query_row(
                "SELECT except_on FROM recurrence WHERE item_id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            );
            let Ok(existing) = existing else {
                eprintln!("sched: {id} has no repeat rule");
                return ExitCode::from(2);
            };
            let mut dates: Vec<String> = existing
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if !dates.contains(&date) {
                dates.push(date.clone());
            }
            match db.conn.execute(
                "UPDATE recurrence SET except_on = ?2 WHERE item_id = ?1",
                rusqlite::params![id, dates.join(",")],
            ) {
                Ok(_) => {
                    println!("cancelled {date} for {id}");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&format!("could not update: {e}")),
            }
        }

        Cmd::Rm { id } => match delete_item(&db, &id) {
            Ok(true) => {
                println!("removed {id}");
                ExitCode::SUCCESS
            }
            Ok(false) => {
                eprintln!("sched: no item {id}");
                ExitCode::from(2)
            }
            Err(e) => fail(&format!("could not remove: {e}")),
        },

        Cmd::Import { file } => {
            let text = match std::fs::read_to_string(&file) {
                Ok(t) => t,
                Err(e) => return fail(&format!("could not read {}: {e}", file.display())),
            };
            let records: Vec<ImportRecord> = match serde_json::from_str(strip_bom(&text)) {
                Ok(r) => r,
                Err(e) => return fail(&format!("{} is not a valid import file: {e}", file.display())),
            };
            match import(&db, &records, now) {
                Ok(o) => {
                    println!("imported  {} added, {} updated", o.added, o.updated);
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&format!("{e}")),
            }
        }

        Cmd::SyncIcs { url, file, forget } => {
            use ms_core::ics;
            if forget {
                let _ = db.conn.execute(
                    "DELETE FROM settings WHERE key = ?1",
                    rusqlite::params![ics::URL_SETTING],
                );
                let from = now - chrono::Duration::days(ics::DAYS_BACK);
                return match ms_core::sync(&db, ics::PREFIX, &[], from, now) {
                    Ok(o) => {
                        println!("forgot the calendar  {} upcoming events removed", o.removed);
                        ExitCode::SUCCESS
                    }
                    Err(e) => fail(&format!("{e}")),
                };
            }
            let text = if let Some(file) = file {
                match std::fs::read_to_string(&file) {
                    Ok(t) => t,
                    Err(e) => return fail(&format!("could not read {}: {e}", file.display())),
                }
            } else {
                let Some(url) = url.or_else(|| ms_core::setting(&db, ics::URL_SETTING)) else {
                    return fail("no calendar yet: sched sync-ics <link to the .ics feed>");
                };
                let body = match ureq::get(&ics::fetchable(&url))
                    .timeout(std::time::Duration::from_secs(30))
                    .call()
                {
                    Ok(r) => r.into_string(),
                    Err(e) => return fail(&format!("could not fetch the calendar: {e}")),
                };
                match body {
                    Ok(t) => {
                        // Saved only once it has answered, so a mistyped link
                        // is not what the app goes on polling.
                        if t.contains("BEGIN:VCALENDAR") {
                            let _ = ms_core::set_setting(&db, ics::URL_SETTING, &url);
                        }
                        t
                    }
                    Err(e) => return fail(&format!("could not read the calendar: {e}")),
                }
            };
            match ics::sync_text(&db, strip_bom(&text), now, ms_core::local_zone()) {
                Ok((o, parsed)) => {
                    for w in &parsed.warnings {
                        eprintln!("sched: {w}");
                    }
                    let skipped = match parsed.skipped_all_day {
                        0 => String::new(),
                        n => format!(", {n} all-day skipped"),
                    };
                    println!(
                        "synced  {} added, {} updated, {} removed{skipped}",
                        o.added, o.updated, o.removed
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }

        Cmd::Export => {
            let items = get_items(&db, &Filter::default());
            println!("[{}]", items.iter().map(item_json).collect::<Vec<_>>().join(","));
            ExitCode::SUCCESS
        }

        Cmd::Stats { tag } => {
            let s = stats(&db, tag.as_deref());
            if cli.json {
                println!(
                    r#"{{"tag":{},"n":{},"pct_over":{},"phrase":{},"confident":{}}}"#,
                    json_opt_str(s.tag.as_deref()),
                    s.n,
                    s.pct_over.map(|v| v.to_string()).unwrap_or("null".into()),
                    json_opt_str(s.phrase.as_deref()),
                    s.confident
                );
            } else {
                match (&s.phrase, s.n) {
                    (Some(p), n) => println!("{p} than you estimate  (from {n} items)"),
                    (None, 0) => println!("nothing timed against an estimate yet"),
                    (None, n) => println!(
                        "not enough data yet — {n} item(s) timed, need 5 before this means anything"
                    ),
                }
            }
            ExitCode::SUCCESS
        }
    }
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

fn local_zone() -> chrono_tz::Tz {
    // The zone is read fresh, never stored: travel is a render parameter. §5.3
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(chrono_tz::UTC)
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn json_opt_str(v: Option<&str>) -> String {
    v.map(|s| format!("\"{}\"", esc(s))).unwrap_or("null".into())
}

fn item_json(i: &ms_core::Item) -> String {
    format!(
        r#"{{"id":"{}","title":"{}","tags":[{}],"category":{},"location":{},"listed":{},"due_at":{},"estimate_min":{},"recurs":{},"done":{},"source":"{}","external_id":{}}}"#,
        esc(&i.id),
        esc(&i.title),
        i.tags.iter().map(|t| format!("\"{}\"", esc(t))).collect::<Vec<_>>().join(","),
        json_opt_str(i.category.as_deref()),
        json_opt_str(i.location.as_deref()),
        i.listed,
        json_opt_str(i.due_at.map(|d| d.to_rfc3339()).as_deref()),
        i.estimate_min.map(|v| v.to_string()).unwrap_or("null".into()),
        i.recurs,
        i.completed,
        esc(&i.source),
        json_opt_str(i.external_id.as_deref()),
    )
}

fn week_json(w: &ms_core::Week) -> String {
    let days = w
        .days
        .iter()
        .map(|d| {
            let ps = d
                .placements
                .iter()
                .map(|p| {
                    format!(
                        r#"{{"id":"{}","item_id":"{}","starts_at":"{}","ends_at":"{}","foreign":{},"pinned_tz":{}}}"#,
                        esc(&p.id),
                        esc(&p.item_id),
                        p.starts_at.to_rfc3339(),
                        p.ends_at.to_rfc3339(),
                        p.foreign,
                        json_opt_str(p.pinned_tz.as_deref())
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!(r#"{{"date":"{}","placements":[{}]}}"#, d.date, ps)
        })
        .collect::<Vec<_>>()
        .join(",");

    let diags = w
        .diagnostics
        .iter()
        .map(|d| {
            format!(
                r#"{{"item_id":{},"message":"{}"}}"#,
                json_opt_str(d.item_id.as_deref()),
                esc(&d.message)
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    format!(
        r#"{{"anchor":"{}","viewing_tz":"{}","days":[{}],"diagnostics":[{}]}}"#,
        w.anchor, w.viewing_tz, days, diags
    )
}

/// Drop a leading UTF-8 byte-order mark.
///
/// Windows PowerShell 5.1 writes one from both `Set-Content -Encoding utf8`
/// and `Out-File -Encoding utf8`, which is how an agent on Windows naturally
/// writes an import file. serde then refuses the document with "expected value
/// at line 1 column 1" — an error that points at the first character without
/// saying that the first character is invisible. The mark carries no meaning
/// in a file we are about to parse as UTF-8, so it is dropped rather than
/// reported.
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::strip_bom;
    use ms_core::ImportRecord;

    const DOC: &str = r#"[{"external_id":"a","title":"t"}]"#;

    /// The premise for stripping at all. If serde ever accepts a leading mark
    /// on its own, this fails and the strip becomes dead code.
    #[test]
    fn serde_alone_refuses_a_byte_order_mark() {
        let with_bom = format!("\u{feff}{DOC}");
        assert!(serde_json::from_str::<Vec<ImportRecord>>(&with_bom).is_err());
    }

    #[test]
    fn an_import_written_with_a_byte_order_mark_still_parses() {
        let with_bom = format!("\u{feff}{DOC}");
        let recs: Vec<ImportRecord> =
            serde_json::from_str(strip_bom(&with_bom)).expect("a BOM must not refuse the file");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].title, "t");
    }

    #[test]
    fn a_document_without_one_is_returned_unchanged() {
        assert_eq!(strip_bom(DOC), DOC);
    }

    /// Only a leading mark is a BOM; anywhere else it is content.
    #[test]
    fn a_mark_that_is_not_leading_is_left_alone() {
        let inner = "[\u{feff}]";
        assert_eq!(strip_bom(inner), inner);
    }
}
