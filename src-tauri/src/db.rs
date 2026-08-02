use rusqlite::Connection;
use std::path::PathBuf;

fn database_path() -> PathBuf {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("StorageDoctor");
    std::fs::create_dir_all(&dir).ok();
    dir.join("storage-doctor.db")
}

pub fn open() -> rusqlite::Result<Connection> {
    let conn = Connection::open(database_path())?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS scans (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            started_at TEXT NOT NULL,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'running',
            recoverable_bytes INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS scan_drives (
            scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
            letter TEXT NOT NULL,
            label TEXT NOT NULL,
            total_bytes INTEGER NOT NULL,
            used_bytes INTEGER NOT NULL,
            free_bytes INTEGER NOT NULL,
            is_removable INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS folders (
            scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
            path TEXT NOT NULL,
            name TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            file_count INTEGER NOT NULL,
            recoverable_bytes INTEGER NOT NULL DEFAULT 0,
            recoverable_measured INTEGER NOT NULL DEFAULT 0,
            depth INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_folders_scan_size
            ON folders(scan_id, size_bytes DESC);

        CREATE TABLE IF NOT EXISTS large_files (
            scan_id INTEGER NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
            path TEXT NOT NULL,
            name TEXT NOT NULL,
            extension TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            modified_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_large_files_scan_size
            ON large_files(scan_id, size_bytes DESC);

        CREATE TABLE IF NOT EXISTS recommendations (
            id TEXT PRIMARY KEY,
            scan_id INTEGER REFERENCES scans(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            recoverable_bytes INTEGER NOT NULL,
            risk TEXT NOT NULL CHECK (risk IN ('low', 'medium', 'high')),
            recommended INTEGER NOT NULL DEFAULT 0,
            ignored INTEGER NOT NULL DEFAULT 0,
            paths TEXT
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS operations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            performed_at TEXT NOT NULL,
            source TEXT NOT NULL,
            item_count INTEGER NOT NULL,
            freed_bytes INTEGER NOT NULL,
            method TEXT NOT NULL
        );
        ",
    )?;

    // Best-effort column additions for databases created by earlier builds.
    let _ = conn.execute("ALTER TABLE folders ADD COLUMN depth INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute(
        "ALTER TABLE folders ADD COLUMN recoverable_bytes INTEGER NOT NULL DEFAULT 0",
        [],
    );
    // Adding the column above backfills every existing row with 0, which is
    // indistinguishable from "nothing to clear". Tracked per row rather than
    // per scan because folders are also measured on demand, one at a time.
    let _ = conn.execute(
        "ALTER TABLE folders ADD COLUMN recoverable_measured INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute("ALTER TABLE recommendations ADD COLUMN paths TEXT", []);
    Ok(())
}
