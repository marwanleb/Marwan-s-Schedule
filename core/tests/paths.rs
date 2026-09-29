//! The app, `sched` and the bot must open one store. The app's comes from
//! Tauri's app_data_dir, so the shared resolver has to land on the same folder.

use ms_core::paths::{data_dir, db_path, APP_ID};
use std::path::PathBuf;

/// Tauri joins its data dir with the identifier from tauri.conf.json; if that
/// is ever renamed, the CLI and bot must follow.
#[test]
fn the_identifier_is_the_apps() {
    let conf = concat!(env!("CARGO_MANIFEST_DIR"), "/../app/src-tauri/tauri.conf.json");
    let conf: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(conf).unwrap()).unwrap();
    assert_eq!(conf["identifier"], APP_ID);
}

/// Where Tauri's app_data_dir resolves on each platform. On macOS this used to
/// be ~/com.marwan.schedule, a second store the app never read.
///
/// Every environment change lives in this one test: the file is its own test
/// binary, and nothing else in it reads these variables.
#[test]
fn the_store_is_in_the_apps_data_dir() {
    let home = PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap());

    #[cfg(target_os = "macos")]
    let expected = home.join("Library/Application Support").join(APP_ID);
    #[cfg(all(unix, not(target_os = "macos")))]
    let expected = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"))
        .join(APP_ID);
    #[cfg(windows)]
    let expected = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join(APP_ID);
    let _ = &home;

    assert_eq!(data_dir(), expected);

    // APPDATA means nothing off Windows, even when something sets it.
    #[cfg(not(windows))]
    {
        std::env::set_var("APPDATA", "/nowhere");
        assert_eq!(data_dir(), expected);
        std::env::remove_var("APPDATA");
    }

    std::env::remove_var("SCHEDULE_DB");
    assert_eq!(db_path(), expected.join("schedule.db"));

    std::env::set_var("SCHEDULE_DB", "/tmp/elsewhere.db");
    assert_eq!(db_path(), PathBuf::from("/tmp/elsewhere.db"));

    // An empty override is a mistake, not a path.
    std::env::set_var("SCHEDULE_DB", "");
    assert_eq!(db_path(), expected.join("schedule.db"));
    std::env::remove_var("SCHEDULE_DB");
}
