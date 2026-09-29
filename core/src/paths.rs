//! Where the store lives on disk.
//!
//! The app, `sched` and the bot must open the same file, and the app's is
//! decided by Tauri: `app_data_dir()`, which is `dirs::data_dir()` joined with
//! the bundle identifier. This resolves it the same way without Tauri:
//!
//! ```text
//! macOS    ~/Library/Application Support/com.marwan.schedule
//! Linux    $XDG_DATA_HOME/com.marwan.schedule, else ~/.local/share/...
//! Windows  %APPDATA%\com.marwan.schedule
//! ```
//!
//! Reading `APPDATA` directly, as this used to, only works on Windows. On a Mac
//! it fell through to `$HOME`, so `sched` wrote to ~/com.marwan.schedule while
//! the app read from Application Support.

use std::path::PathBuf;

/// The bundle identifier in `app/src-tauri/tauri.conf.json`. A test holds the
/// two together.
pub const APP_ID: &str = "com.marwan.schedule";

/// The app's data directory: the store and `bot.toml` live here.
pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_ID)
}

/// The store every front door opens, unless `SCHEDULE_DB` names another.
pub fn db_path() -> PathBuf {
    std::env::var_os("SCHEDULE_DB")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir().join("schedule.db"))
}
