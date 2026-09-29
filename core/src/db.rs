use rusqlite::Connection;
use std::path::Path;

pub struct Db {
    pub conn: Connection,
}

impl Db {
    pub fn open_in_memory() -> rusqlite::Result<Db> {
        let conn = Connection::open_in_memory()?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Open (creating if needed) a file-backed store.
    ///
    /// WAL is required, not a tuning choice: the `sched` CLI writes while the
    /// app has the same file open, and the default rollback journal would lock
    /// one of them out. Spec 8.
    pub fn open(path: &Path) -> rusqlite::Result<Db> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        // Explicit, not inherited: rusqlite currently defaults foreign_keys ON,
        // but ON DELETE CASCADE is load-bearing here (an orphaned recurrence
        // row would render forever) and must not depend on a default.
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        self.create_tables()?;
        self.add_missing_columns()
    }

    fn create_tables(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS items (
              id           TEXT PRIMARY KEY,
              title        TEXT NOT NULL,
              tags         TEXT NOT NULL DEFAULT '',
              category     TEXT,
              location     TEXT,
              listed       INTEGER NOT NULL DEFAULT 1,
              due_at       TEXT,
              estimate_min INTEGER,
              created_at   TEXT NOT NULL,
              source       TEXT NOT NULL DEFAULT 'self'
                           CHECK (source IN ('self','import')),
              external_id  TEXT,
              -- Set instead of removing the row. A deleted import has to stay
              -- here, or its next re-import would bring it straight back.
              deleted_at   TEXT
            );


            CREATE TABLE IF NOT EXISTS completions (
              item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
              -- '' rather than NULL: SQLite treats NULLs as distinct in a
              -- composite key, which would allow duplicate completion rows.
              on_date TEXT NOT NULL DEFAULT '',
              done_at TEXT NOT NULL,
              PRIMARY KEY (item_id, on_date)
            );

            CREATE TABLE IF NOT EXISTS recurrence (
              item_id    TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
              byday      TEXT NOT NULL,
              start_time TEXT NOT NULL,
              end_time   TEXT NOT NULL,
              tz         TEXT,
              from_date  TEXT NOT NULL,
              until_date TEXT,
              except_on  TEXT NOT NULL DEFAULT '',
              -- Day of the month for a monthly rule; NULL for a weekly one.
              monthday   INTEGER
            );

            CREATE TABLE IF NOT EXISTS placements (
              id         TEXT PRIMARY KEY,
              item_id    TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
              starts_at  TEXT NOT NULL,
              ends_at    TEXT NOT NULL,
              origin     TEXT NOT NULL CHECK (origin IN ('oneoff','moved')),
              moved_from TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_placements_item ON placements(item_id);

            CREATE TABLE IF NOT EXISTS sessions (
              id           TEXT PRIMARY KEY,
              item_id      TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
              -- Deliberately NOT a foreign key: editing a recurrence destroys
              -- the generated placement a session may point at, and the session
              -- must survive that. A dangling value is expected. Spec 3.
              placement_id TEXT,
              started_at   TEXT NOT NULL,
              ended_at     TEXT,
              discarded    INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_sessions_item ON sessions(item_id);

            -- Small facts that belong to the store rather than to any item:
            -- which Telegram chat to push to, for one. A table rather than a
            -- file so every process sees the same answer without a second
            -- config path to keep in step.
            CREATE TABLE IF NOT EXISTS settings (
              key   TEXT PRIMARY KEY,
              value TEXT NOT NULL
            );

            -- One row per notification already sent. Without it a restart --
            -- or a poll two seconds later -- would push the same reminder
            -- again, which is the fastest way to make someone mute a bot.
            CREATE TABLE IF NOT EXISTS notified (
              key     TEXT PRIMARY KEY,
              sent_at TEXT NOT NULL
            );
            ",
        )
    }

    /// Bring an existing store up to the current schema.
    ///
    /// `CREATE TABLE IF NOT EXISTS` does nothing to a table that already
    /// exists, so every column added after a store was created was simply
    /// absent from it — which shipped, and made the bot answer every message
    /// with "table items has no column named location".
    ///
    /// Adding whatever is missing, rather than tracking a version number, is
    /// idempotent by construction: it runs on every launch, converges from any
    /// older shape, and cannot get out of step with a counter.
    fn add_missing_columns(&self) -> rusqlite::Result<()> {
        // (table, column, definition). Defaults only — SQLite cannot add a
        // NOT NULL column without one, and none of these are required.
        const ADDED: &[(&str, &str, &str)] = &[
            ("items", "tags", "TEXT NOT NULL DEFAULT ''"),
            ("items", "category", "TEXT"),
            ("items", "location", "TEXT"),
            ("items", "listed", "INTEGER NOT NULL DEFAULT 1"),
            ("items", "due_at", "TEXT"),
            ("items", "estimate_min", "INTEGER"),
            ("items", "source", "TEXT NOT NULL DEFAULT 'self'"),
            ("items", "external_id", "TEXT"),
            ("items", "deleted_at", "TEXT"),
            ("recurrence", "tz", "TEXT"),
            ("recurrence", "until_date", "TEXT"),
            ("recurrence", "except_on", "TEXT NOT NULL DEFAULT ''"),
            ("recurrence", "monthday", "INTEGER"),
            ("placements", "moved_from", "TEXT"),
            ("sessions", "placement_id", "TEXT"),
            ("sessions", "discarded", "INTEGER NOT NULL DEFAULT 0"),
        ];

        for (table, column, def) in ADDED {
            if !self.has_column(table, column)? {
                self.conn.execute_batch(&format!(
                    "ALTER TABLE {table} ADD COLUMN {column} {def};"
                ))?;
            }
        }

        // The unique index on external_id cannot be created until the column
        // exists, so it lives here rather than with the tables.
        self.conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_items_external
               ON items(external_id) WHERE external_id IS NOT NULL;",
        )
    }

    fn has_column(&self, table: &str, column: &str) -> rusqlite::Result<bool> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            if row.get::<_, String>(1)? == column {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
