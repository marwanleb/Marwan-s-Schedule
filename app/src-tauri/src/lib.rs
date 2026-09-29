//! Tauri command layer. Every command is a thin wrapper over `ms-core`; no
//! logic lives here, so the CLI, the bot and the window cannot disagree.

use chrono::{Local, NaiveDate, Utc};
use ms_core::{
    active_session, add_from_text, add_placement, delete_item, delete_placement, get_items,
    get_days, help_text, move_occurrence, move_placement, restore_item, restore_occurrence, set_done,
    set_recurrence_tz, stats, store, set_estimate, set_listed, sweep_stale_sessions, timer_start,
    timer_stop, Db, Filter,
};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

pub struct AppDb(pub Mutex<Db>);

/// Read fresh on every call, never stored: travel is a render parameter, so
/// landing somewhere new needs no migration. Spec 5.3.
fn viewing_zone() -> chrono_tz::Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(chrono_tz::UTC)
}

#[derive(Serialize)]
pub struct ItemDto {
    id: String,
    title: String,
    tags: Vec<String>,
    category: Option<String>,
    location: Option<String>,
    listed: bool,
    due_at: Option<String>,
    estimate_min: Option<u32>,
    recurs: bool,
    done: bool,
    done_on: Vec<String>,
    completed_at: Option<String>,
    source: String,
    external_id: Option<String>,
    /// Banked seconds across finished, non-discarded sessions.
    spent_sec: i64,
}

fn to_dto(db: &Db, i: &ms_core::Item, on: Option<NaiveDate>) -> ItemDto {
    let spent_sec: i64 = db
        .conn
        .query_row(
            "SELECT COALESCE(SUM((julianday(ended_at) - julianday(started_at)) * 86400), 0)
             FROM sessions WHERE item_id = ?1 AND ended_at IS NOT NULL AND discarded = 0",
            rusqlite::params![i.id],
            |r| r.get::<_, f64>(0),
        )
        .unwrap_or(0.0) as i64;

    // One definition of "done" lives on Item; re-deriving it here is exactly
    // how the checkbox came to never tick.
    let done = i.is_done(on);

    ItemDto {
        id: i.id.clone(),
        title: i.title.clone(),
        tags: i.tags.clone(),
        category: i.category.clone(),
        location: i.location.clone(),
        listed: i.listed,
        due_at: i.due_at.map(|d| d.to_rfc3339()),
        estimate_min: i.estimate_min,
        recurs: i.recurs,
        done,
        done_on: i.done_on.iter().map(|d| d.to_string()).collect(),
        completed_at: i.completed_at.map(|d| d.to_rfc3339()),
        source: i.source.clone(),
        external_id: i.external_id.clone(),
        spent_sec,
    }
}

#[derive(Serialize)]
pub struct PlacementDto {
    id: String,
    item_id: String,
    title: String,
    category: Option<String>,
    location: Option<String>,
    starts_at: String,
    ends_at: String,
    origin: String,
    pinned_tz: Option<String>,
    foreign: bool,
    recurs: bool,
    done: bool,
}

#[derive(Serialize)]
pub struct DayDto {
    date: String,
    placements: Vec<PlacementDto>,
}

#[derive(Serialize)]
pub struct WeekDto {
    anchor: String,
    viewing_tz: String,
    days: Vec<DayDto>,
    diagnostics: Vec<DiagDto>,
}

#[derive(Serialize)]
pub struct DiagDto {
    item_id: Option<String>,
    message: String,
}

#[tauri::command]
fn cmd_get_week(state: State<'_, AppDb>, anchor: Option<String>) -> Result<WeekDto, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let today = Local::now().date_naive();
    // No anchor means the rolling window: yesterday, today and the five after.
    let anchor = anchor
        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok())
        .unwrap_or_else(|| today.pred_opt().unwrap_or(today));

    // Exactly where it was asked to start. The window is not a calendar week.
    let week = get_days(&db, anchor, viewing_zone());
    Ok(WeekDto {
        anchor: week.anchor.to_string(),
        viewing_tz: week.viewing_tz.clone(),
        days: week
            .days
            .iter()
            .map(|d| DayDto {
                date: d.date.to_string(),
                placements: d
                    .placements
                    .iter()
                    .map(|p| {
                        let item = store::fetch(&db, &p.item_id);
                        let done = item
                            .as_ref()
                            .map(|i| i.done_on.contains(&d.date))
                            .unwrap_or(false);
                        PlacementDto {
                            id: p.id.clone(),
                            item_id: p.item_id.clone(),
                            title: item
                                .as_ref()
                                .map(|i| i.title.clone())
                                .unwrap_or_else(|| p.item_id.clone()),
                            category: item.as_ref().and_then(|i| i.category.clone()),
                            location: item.as_ref().and_then(|i| i.location.clone()),
                            starts_at: p.starts_at.to_rfc3339(),
                            ends_at: p.ends_at.to_rfc3339(),
                            origin: format!("{:?}", p.origin).to_lowercase(),
                            pinned_tz: p.pinned_tz.clone(),
                            foreign: p.foreign,
                            recurs: item.as_ref().map(|i| i.recurs).unwrap_or(false),
                            done,
                        }
                    })
                    .collect(),
            })
            .collect(),
        diagnostics: week
            .diagnostics
            .iter()
            .map(|x| DiagDto {
                item_id: x.item_id.clone(),
                message: x.message.clone(),
            })
            .collect(),
    })
}

#[tauri::command]
fn cmd_get_items(state: State<'_, AppDb>, open_only: Option<bool>) -> Result<Vec<ItemDto>, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let today = Local::now().date_naive();
    // Everything, listed or not: the grid needs the items behind its blocks so
    // clicking one can open it. The To Do pane does its own filtering.
    let filter = Filter {
        done: open_only.unwrap_or(false).then_some(false),
        on: Some(today),
        ..Default::default()
    };
    Ok(get_items(&db, &filter)
        .iter()
        .map(|i| to_dto(&db, i, Some(today)))
        .collect())
}

#[derive(Serialize)]
pub struct AddResult {
    item: ItemDto,
    /// Tokens the parser recognised. Empty means the line was saved as plain
    /// text, which is worth telling the user rather than letting it drop
    /// quietly into Someday.
    consumed: Vec<String>,
    byday: Vec<String>,
    span: Option<(String, String)>,
    location: Option<String>,
}

#[tauri::command]
fn cmd_add(state: State<'_, AppDb>, text: String) -> Result<AddResult, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let today = Local::now().date_naive();
    let parsed = ms_core::parse(&text, today);
    let item = add_from_text(&db, &text, today, Utc::now()).map_err(|e| e.to_string())?;
    Ok(AddResult {
        item: to_dto(&db, &item, Some(today)),
        consumed: parsed.consumed,
        byday: parsed.byday,
        span: parsed
            .span
            .map(|(a, b)| (a.format("%H:%M").to_string(), b.format("%H:%M").to_string())),
        location: parsed.location,
    })
}

#[tauri::command]
fn cmd_set_done(
    state: State<'_, AppDb>,
    id: String,
    done: bool,
    on: Option<String>,
) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let today = Local::now().date_naive();
    let item = store::fetch(&db, &id).ok_or("no such item")?;
    // A repeat is completed for one occurrence, never forever. Spec 3.2.
    let date = match on.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()) {
        Some(d) => Some(d),
        None if item.recurs => Some(today),
        None => None,
    };
    set_done(&db, &id, done, date, Utc::now()).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn cmd_delete(state: State<'_, AppDb>, id: String) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    delete_item(&db, &id, Utc::now()).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn cmd_restore(state: State<'_, AppDb>, id: String) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    restore_item(&db, &id).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn cmd_except(state: State<'_, AppDb>, id: String, date: String) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let on = NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    ms_core::except(&db, &id, on)
}

#[derive(Serialize)]
pub struct SessionDto {
    id: String,
    item_id: String,
    title: String,
    started_at: String,
    estimate_min: Option<u32>,
    spent_sec: i64,
}

fn session_dto(db: &Db, s: &ms_core::Session) -> SessionDto {
    let item = store::fetch(db, &s.item_id);
    let spent_sec: i64 = db
        .conn
        .query_row(
            "SELECT COALESCE(SUM((julianday(ended_at) - julianday(started_at)) * 86400), 0)
             FROM sessions WHERE item_id = ?1 AND ended_at IS NOT NULL AND discarded = 0",
            rusqlite::params![s.item_id],
            |r| r.get::<_, f64>(0),
        )
        .unwrap_or(0.0) as i64;
    SessionDto {
        id: s.id.clone(),
        item_id: s.item_id.clone(),
        title: item.as_ref().map(|i| i.title.clone()).unwrap_or_default(),
        started_at: s.started_at.to_rfc3339(),
        estimate_min: item.as_ref().and_then(|i| i.estimate_min),
        spent_sec,
    }
}

#[tauri::command]
fn cmd_timer_start(
    state: State<'_, AppDb>,
    item_id: String,
    placement_id: Option<String>,
) -> Result<SessionDto, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let started = timer_start(&db, &item_id, placement_id.as_deref(), Utc::now())
        .map_err(|e| e.to_string())?;
    Ok(session_dto(&db, &started.started))
}

#[tauri::command]
fn cmd_timer_stop(state: State<'_, AppDb>, session_id: String) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    timer_stop(&db, &session_id, Utc::now()).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn cmd_active_session(state: State<'_, AppDb>) -> Result<Option<SessionDto>, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    Ok(active_session(&db).map(|s| session_dto(&db, &s)))
}

#[derive(Serialize)]
pub struct StatsDto {
    tag: Option<String>,
    n: usize,
    pct_over: Option<i32>,
    phrase: Option<String>,
    confident: bool,
}

#[tauri::command]
fn cmd_stats(state: State<'_, AppDb>, tag: Option<String>) -> Result<StatsDto, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let s = stats(&db, tag.as_deref());
    Ok(StatsDto {
        tag: s.tag,
        n: s.n,
        pct_over: s.pct_over,
        phrase: s.phrase,
        confident: s.confident,
    })
}

/// SQLite bumps `data_version` when *another* connection writes. Cheap enough
/// to ask repeatedly, and the only way this window learns that the bot or the
/// CLI changed something — there is no watcher and no socket by design.
#[tauri::command]
fn cmd_data_version(state: State<'_, AppDb>) -> Result<i64, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    db.conn
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_help() -> String {
    help_text()
}

#[tauri::command]
fn cmd_hide(window: tauri::Window) -> Result<(), String> {
    // The canvas keeps drawing unless something tells it to stop, and a hidden
    // window does not reliably fire `visibilitychange`. Say so explicitly.
    let _ = window.emit("window:hidden", ());
    window.hide().map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_add_placement(
    state: State<'_, AppDb>,
    item_id: String,
    starts_at: String,
    ends_at: String,
) -> Result<String, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    add_placement(&db, &item_id, &starts_at, &ends_at)
}

#[tauri::command]
fn cmd_move_placement(
    state: State<'_, AppDb>,
    id: String,
    starts_at: String,
    ends_at: String,
) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    move_placement(&db, &id, &starts_at, &ends_at)
}

#[tauri::command]
fn cmd_delete_placement(state: State<'_, AppDb>, id: String) -> Result<bool, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    delete_placement(&db, &id)
}

#[derive(Serialize)]
pub struct RecurrenceDto {
    byday: Vec<String>,
    start_time: String,
    end_time: String,
    tz: Option<String>,
    except_on: Vec<String>,
    /// Day of the month for a monthly rule; None for a weekly one.
    monthday: Option<u32>,
}

#[tauri::command]
fn cmd_get_recurrence(
    state: State<'_, AppDb>,
    item_id: String,
) -> Result<Option<RecurrenceDto>, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let row = db.conn.query_row(
        "SELECT byday, start_time, end_time, tz, except_on, monthday FROM recurrence WHERE item_id = ?1",
        rusqlite::params![item_id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<u32>>(5)?,
            ))
        },
    );
    Ok(row.ok().map(|(byday, start_time, end_time, tz, except, monthday)| RecurrenceDto {
        byday: byday.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect(),
        start_time,
        end_time,
        tz,
        except_on: except.split(',').filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()).collect(),
        monthday,
    }))
}

#[tauri::command]
fn cmd_set_recurrence(
    state: State<'_, AppDb>,
    item_id: String,
    byday: Vec<String>,
    start_time: String,
    end_time: String,
    monthday: Option<u32>,
) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let today = Local::now().date_naive();
    if byday.is_empty() && monthday.is_none() {
        db.conn
            .execute("DELETE FROM recurrence WHERE item_id = ?1", rusqlite::params![item_id])
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    // Fixed to the current zone when first created, matching typed capture.
    let tz = store::local_zone().name().to_string();
    db.conn
        .execute(
            "INSERT INTO recurrence (item_id, byday, start_time, end_time, tz, from_date, monthday)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(item_id) DO UPDATE SET byday = ?2, start_time = ?3, end_time = ?4, monthday = ?7",
            rusqlite::params![
                item_id,
                byday.join(","),
                start_time,
                end_time,
                tz,
                today.to_string(),
                monthday
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn cmd_set_recurrence_tz(
    state: State<'_, AppDb>,
    item_id: String,
    tz: Option<String>,
) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    set_recurrence_tz(&db, &item_id, tz.as_deref())
}

#[tauri::command]
fn cmd_remove_except(state: State<'_, AppDb>, item_id: String, date: String) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let on = NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    // Also drops anything moved out of that date, or the class comes back
    // while the copy you dragged elsewhere is still sitting there.
    restore_occurrence(&db, &item_id, on)
}

/// Move one occurrence of a repeat. The series is untouched; every other week
/// still comes from the rule.
#[tauri::command]
fn cmd_move_occurrence(
    state: State<'_, AppDb>,
    item_id: String,
    date: String,
    starts_at: String,
    ends_at: String,
) -> Result<String, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let from = NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    move_occurrence(&db, &item_id, from, &starts_at, &ends_at)
}

/// Render fields as a line of the capture grammar, so the form can show what
/// it is about to submit. The same function the Telegram ladder uses.
#[tauri::command]
fn cmd_compose(fields: ms_core::Fields) -> String {
    ms_core::line(&fields)
}

/// Zones offered in the picker: where you are now, plus the ones already in
/// use, plus the places you actually travel between.
#[tauri::command]
fn cmd_set_estimate(
    state: State<'_, AppDb>,
    id: String,
    minutes: Option<u32>,
) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    set_estimate(&db, &id, minutes).map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_set_listed(state: State<'_, AppDb>, id: String, listed: bool) -> Result<(), String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    set_listed(&db, &id, listed).map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_zones(state: State<'_, AppDb>) -> Result<Vec<String>, String> {
    let db = state.0.lock().map_err(|e| e.to_string())?;
    let mut zones: Vec<String> = vec![viewing_zone().name().to_string()];
    for z in ["America/Chicago", "Europe/Paris", "Asia/Beirut"] {
        if !zones.contains(&z.to_string()) {
            zones.push(z.to_string());
        }
    }
    if let Ok(mut stmt) = db.conn.prepare("SELECT DISTINCT tz FROM recurrence WHERE tz IS NOT NULL")
    {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            for z in rows.filter_map(|r| r.ok()) {
                if !zones.contains(&z) {
                    zones.push(z);
                }
            }
        }
    }
    Ok(zones)
}

/// Bring the window up, or put it away if it is already in front.
fn toggle_window(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("main") else { return };
    let visible = win.is_visible().unwrap_or(false);
    let focused = win.is_focused().unwrap_or(false);

    if visible && focused {
        let _ = win.emit("window:hidden", ());
        let _ = win.hide();
    } else {
        let _ = win.show();
        let _ = win.set_focus();
        // The canvas stops while hidden; tell it to start again.
        let _ = win.emit("window:shown", ());
    }
}

pub fn run() {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

    // Avoids Alt+Space (the Windows window menu, PowerToys Run) and is
    // unlikely to collide with a game binding.
    let summon = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyS);

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            // Starts hidden: it is a tray app, and appearing over whatever you
            // were doing at login would be the opposite of the point.
            Some(vec!["--hidden"]),
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, sc, event| {
                    if sc == &summon && event.state() == ShortcutState::Pressed {
                        toggle_window(app);
                    }
                })
                .build(),
        )
        // One setup only: Builder::setup replaces rather than appends, so a
        // second call would silently discard the first.
        .setup(move |app| {
            if let Err(e) = app.global_shortcut().register(summon) {
                // Another app may already own the combination. Not fatal — the
                // tray icon and the window still work.
                eprintln!("could not register Ctrl+Alt+S: {e}");
            }
            let dir = app
                .path()
                .app_data_dir()
                .expect("no app data dir");
            let db = Db::open(&dir.join("schedule.db")).expect("could not open store");

            // A timer left running past midnight is a crash, not work: quarantine
            // it before it can reach the statistics. Spec 5.5.
            let swept = sweep_stale_sessions(&db, Utc::now()).unwrap_or_default();
            if !swept.is_empty() {
                eprintln!("quarantined {} stale timer session(s)", swept.len());
            }

            app.manage(AppDb(Mutex::new(db)));

            let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Marwan's Schedule - Ctrl+Alt+S")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => toggle_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // Keep launch-at-login pointing at *this* binary.
            //
            // Enabling only when disabled is not enough: the entry records an
            // absolute path, so moving or reinstalling the app leaves a stale
            // one that silently launches the old copy — or nothing. Remember
            // the path we registered and re-register when it changes.
            {
                use tauri_plugin_autostart::ManagerExt;
                let al = app.autolaunch();
                let stamp = dir.join("autostart-path");
                let current = std::env::current_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let recorded = std::fs::read_to_string(&stamp).unwrap_or_default();
                let enabled = al.is_enabled().unwrap_or(false);

                if !enabled || recorded != current {
                    let _ = al.disable();
                    match al.enable() {
                        Ok(()) => {
                            let _ = std::fs::write(&stamp, &current);
                        }
                        Err(e) => eprintln!("could not enable launch at login: {e}"),
                    }
                }
            }

            // Launched at login: sit in the tray until summoned.
            if std::env::args().any(|a| a == "--hidden") {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }

            let handle = app.handle().clone();
            if let Some(win) = app.get_webview_window("main") {
                // Closing puts it away rather than quitting: it is a tray app,
                // and the point is that it stays one key away.
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Some(w) = handle.get_webview_window("main") {
                            let _ = w.emit("window:hidden", ());
                            let _ = w.hide();
                        }
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            cmd_get_week,
            cmd_get_items,
            cmd_add,
            cmd_set_done,
            cmd_delete,
            cmd_restore,
            cmd_except,
            cmd_timer_start,
            cmd_timer_stop,
            cmd_active_session,
            cmd_stats,
            cmd_help,
            cmd_hide,
            cmd_add_placement,
            cmd_move_placement,
            cmd_delete_placement,
            cmd_get_recurrence,
            cmd_set_recurrence,
            cmd_set_recurrence_tz,
            cmd_remove_except,
            cmd_move_occurrence,
            cmd_compose,
            cmd_zones,
            cmd_set_estimate,
            cmd_set_listed,
            cmd_data_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running application");
}
