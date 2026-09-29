//! What the bot would push, without a token and without waiting ten minutes.
//!
//!     cargo run -p ms-core --example notify_preview
//!     cargo run -p ms-core --example notify_preview -- 60
//!     cargo run -p ms-core --example notify_preview -- 15 2026-09-09T15:22:00Z
//!
//! Reads `SCHEDULE_DB`, or the same default path the app and the bot use.

use chrono::{DateTime, Duration, Utc};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let lead: i64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);
    let now: DateTime<Utc> = args
        .get(1)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let path = ms_core::paths::db_path();

    let db = match ms_core::Db::open(&path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("cannot open {}: {e}", path.display());
            std::process::exit(1);
        }
    };

    let zone = ms_core::local_zone();
    let here = now.with_timezone(&zone);
    println!(
        "{}\n{} \u{2014} next {lead} min from {} {}\n",
        path.display(),
        zone.name(),
        here.format("%a %d %b"),
        ms_core::time12(here.time())
    );

    let out = ms_core::due_soon(&db, now, Duration::minutes(lead), zone);
    if out.is_empty() {
        println!("nothing");
    }
    for n in &out {
        let mins = ((n.at - now).num_seconds() as f64 / 60.0).ceil() as i64;
        let starts = n.at.with_timezone(&zone).time();
        let when = match n.ends {
            Some(e) => ms_core::range12(starts, e.with_timezone(&zone).time()),
            None => ms_core::time12(starts),
        };
        println!(
            "{:>3} min  {:<15} {}{}   [{}]",
            mins,
            when,
            n.title,
            n.location.as_deref().map(|l| format!(" @{l}")).unwrap_or_default(),
            n.key
        );
    }
}
