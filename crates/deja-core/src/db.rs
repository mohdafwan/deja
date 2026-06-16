//! SQLite storage for command runs.
//!
//! Phase 0.1: sirf `runs` table use ho raha hai (command + exit + timing).
//! `snapshots` table abhi schema me hai par snapshot_id nullable — wo Phase 0.2 me bharega.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Ek command run ka record (DB me likhne ke liye).
pub struct Run {
    pub command: String,
    pub cwd: String,
    pub exit_code: i64,
    pub duration_ms: i64,
    pub started_at: i64,
}

/// DB file ka path: ~/.local/share/deja/history.db (Linux/Mac), AppData (Windows).
pub fn db_path() -> Result<PathBuf> {
    let mut dir = dirs::data_dir().context("data directory nahi mili (HOME set hai?)")?;
    dir.push("deja");
    std::fs::create_dir_all(&dir).context("deja data dir banane me dikkat")?;
    dir.push("history.db");
    Ok(dir)
}

/// Connection kholo aur schema ensure karo (idempotent).
pub fn open() -> Result<Connection> {
    let path = db_path()?;
    let conn = Connection::open(&path)
        .with_context(|| format!("DB open nahi hui: {}", path.display()))?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS snapshots (
            id            INTEGER PRIMARY KEY,
            os            TEXT,
            env_json      TEXT,
            path_json     TEXT,
            tool_versions TEXT,
            git_branch    TEXT,
            git_head      TEXT,
            git_dirty     INTEGER,
            key_files     TEXT,
            content_hash  TEXT
        );

        CREATE TABLE IF NOT EXISTS runs (
            id            INTEGER PRIMARY KEY,
            command       TEXT NOT NULL,
            command_hash  TEXT NOT NULL,
            cwd           TEXT NOT NULL,
            exit_code     INTEGER NOT NULL,
            duration_ms   INTEGER,
            started_at    INTEGER NOT NULL,
            snapshot_id   INTEGER REFERENCES snapshots(id)
        );

        CREATE INDEX IF NOT EXISTS idx_runs_cmd_cwd
            ON runs(command_hash, cwd, exit_code);
        "#,
    )
    .context("schema init fail")?;

    // Migration: purane DBs me content_hash column add karo (fresh DB pe ye
    // error dega kyunki column already hai — isliye ignore).
    let _ = conn.execute("ALTER TABLE snapshots ADD COLUMN content_hash TEXT", []);
    // Dedup ke liye unique index (NULL values distinct mante hain SQLite me).
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_snap_chash ON snapshots(content_hash);",
    )
    .context("snapshot dedup index fail")?;
    Ok(())
}

/// Command ko normalize karke ek stable hash do (dedup + diff lookup ke liye).
/// Abhi simple: trim + whitespace collapse. Baad me arguments-aware kar sakte hain.
pub fn command_hash(command: &str) -> String {
    let normalized: String = command.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Snapshot ke content ka stable hash (dedup ke liye).
fn snapshot_hash(snap: &crate::snapshot::Snapshot) -> String {
    let mut h = DefaultHasher::new();
    snap.os.hash(&mut h);
    snap.env_json.hash(&mut h);
    snap.path_json.hash(&mut h);
    snap.tool_versions.hash(&mut h);
    snap.git_branch.hash(&mut h);
    snap.git_head.hash(&mut h);
    snap.git_dirty.hash(&mut h);
    snap.key_files.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Ek snapshot DB me daalo (dedup ke saath — identical context dobara store nahi
/// hota). Lautata hai snapshot id (naya ya existing).
pub fn insert_snapshot(conn: &Connection, snap: &crate::snapshot::Snapshot) -> Result<i64> {
    let chash = snapshot_hash(snap);
    conn.execute(
        "INSERT OR IGNORE INTO snapshots
           (os, env_json, path_json, tool_versions, git_branch, git_head, git_dirty,
            key_files, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            snap.os,
            snap.env_json,
            snap.path_json,
            snap.tool_versions,
            snap.git_branch,
            snap.git_head,
            snap.git_dirty.map(|d| d as i64),
            snap.key_files,
            chash,
        ],
    )
    .context("snapshot insert fail")?;
    // Naya insert hua → last_insert_rowid; warna existing row dhundo.
    let id: i64 = conn
        .query_row(
            "SELECT id FROM snapshots WHERE content_hash = ?1",
            [&chash],
            |r| r.get(0),
        )
        .context("snapshot id lookup fail")?;
    Ok(id)
}

/// Ek run DB me daalo (optionally ek snapshot se link). Lautata hai naya row id.
pub fn insert_run(conn: &Connection, run: &Run, snapshot_id: Option<i64>) -> Result<i64> {
    let hash = command_hash(&run.command);
    conn.execute(
        "INSERT INTO runs
           (command, command_hash, cwd, exit_code, duration_ms, started_at, snapshot_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            run.command,
            hash,
            run.cwd,
            run.exit_code,
            run.duration_ms,
            run.started_at,
            snapshot_id,
        ],
    )
    .context("run insert fail")?;
    Ok(conn.last_insert_rowid())
}

/// Verify ke liye: latest N runs.
pub struct RunRow {
    pub id: i64,
    pub command: String,
    pub cwd: String,
    pub exit_code: i64,
    pub duration_ms: i64,
    /// Phase 0.3 (diff engine) me "last good run" dhundhne ke liye use hoga.
    #[allow(dead_code)]
    pub started_at: i64,
}

pub fn recent_runs(conn: &Connection, limit: i64) -> Result<Vec<RunRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, command, cwd, exit_code, duration_ms, started_at
         FROM runs ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit], |r| {
            Ok(RunRow {
                id: r.get(0)?,
                command: r.get(1)?,
                cwd: r.get(2)?,
                exit_code: r.get(3)?,
                duration_ms: r.get(4)?,
                started_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Ek command + cwd ka sabse recent run (koi bhi exit). `deja why <cmd>` ke liye.
pub fn most_recent_run_id(conn: &Connection, command: &str, cwd: &str) -> Result<Option<i64>> {
    let hash = command_hash(command);
    let id = conn
        .query_row(
            "SELECT id FROM runs WHERE command_hash = ?1 AND cwd = ?2
             ORDER BY started_at DESC, id DESC LIMIT 1",
            rusqlite::params![hash, cwd],
            |r| r.get(0),
        )
        .optional()?;
    Ok(id)
}

/// Is cwd ka sabse recent FAILED run. `deja why` (bina arg) ke liye.
pub fn most_recent_failed_run_id(conn: &Connection, cwd: &str) -> Result<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT id FROM runs WHERE cwd = ?1 AND exit_code != 0
             ORDER BY started_at DESC, id DESC LIMIT 1",
            [cwd],
            |r| r.get(0),
        )
        .optional()?;
    Ok(id)
}

/// Same command + same cwd ka latest SUCCESSFUL run + uska snapshot.
/// Phase 0.3: failure pe isse compare karke "kya badla" nikaalte hain.
pub fn last_good_snapshot(
    conn: &Connection,
    command: &str,
    cwd: &str,
) -> Result<Option<(RunRow, crate::snapshot::Snapshot)>> {
    let hash = command_hash(command);
    let row = conn
        .query_row(
            "SELECT r.id, r.command, r.cwd, r.exit_code, r.duration_ms, r.started_at,
                    s.os, s.env_json, s.path_json, s.tool_versions,
                    s.git_branch, s.git_head, s.git_dirty, s.key_files
             FROM runs r JOIN snapshots s ON r.snapshot_id = s.id
             WHERE r.command_hash = ?1 AND r.cwd = ?2 AND r.exit_code = 0
             ORDER BY r.started_at DESC, r.id DESC
             LIMIT 1",
            rusqlite::params![hash, cwd],
            |r| {
                let run = RunRow {
                    id: r.get(0)?,
                    command: r.get(1)?,
                    cwd: r.get(2)?,
                    exit_code: r.get(3)?,
                    duration_ms: r.get(4)?,
                    started_at: r.get(5)?,
                };
                let snap = crate::snapshot::Snapshot {
                    os: r.get(6)?,
                    env_json: r.get(7)?,
                    path_json: r.get(8)?,
                    tool_versions: r.get(9)?,
                    git_branch: r.get(10)?,
                    git_head: r.get(11)?,
                    git_dirty: r.get::<_, Option<i64>>(12)?.map(|d| d != 0),
                    key_files: r.get(13)?,
                };
                Ok((run, snap))
            },
        )
        .optional()?;
    Ok(row)
}

/// Ek run + uska linked snapshot (verify/`deja show` ke liye).
pub struct RunDetail {
    pub run: RunRow,
    pub snapshot: Option<crate::snapshot::Snapshot>,
}

pub fn get_run_detail(conn: &Connection, id: i64) -> Result<Option<RunDetail>> {
    let run = conn
        .query_row(
            "SELECT id, command, cwd, exit_code, duration_ms, started_at, snapshot_id
             FROM runs WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    RunRow {
                        id: r.get(0)?,
                        command: r.get(1)?,
                        cwd: r.get(2)?,
                        exit_code: r.get(3)?,
                        duration_ms: r.get(4)?,
                        started_at: r.get(5)?,
                    },
                    r.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .ok();

    let Some((run, snapshot_id)) = run else {
        return Ok(None);
    };

    let snapshot = match snapshot_id {
        Some(sid) => conn
            .query_row(
                "SELECT os, env_json, path_json, tool_versions, git_branch, git_head, git_dirty, key_files
                 FROM snapshots WHERE id = ?1",
                [sid],
                |r| {
                    Ok(crate::snapshot::Snapshot {
                        os: r.get(0)?,
                        env_json: r.get(1)?,
                        path_json: r.get(2)?,
                        tool_versions: r.get(3)?,
                        git_branch: r.get(4)?,
                        git_head: r.get(5)?,
                        git_dirty: r.get::<_, Option<i64>>(6)?.map(|d| d != 0),
                        key_files: r.get(7)?,
                    })
                },
            )
            .ok(),
        None => None,
    };

    Ok(Some(RunDetail { run, snapshot }))
}
