//! The adapter's own tables, next to p2panda's in the same SQLite file.
//!
//! p2panda's store keeps the operations and the group and key state, but it
//! cannot list the devices it holds logs of, does not know which entries are
//! still waiting for a dependency, and does not keep decrypted payloads. These
//! three tables do.

use sqlx::{Row, SqlitePool};
use teamlog::{DeviceId, Entry, EntryId, Payload, TeamId};

pub(crate) const PENDING: i64 = 0;
pub(crate) const APPLIED: i64 = 1;
/// p2panda refused the entry. It is kept, later entries may build on it, and
/// it is tried again whenever something else was applied.
pub(crate) const FAILED: i64 = 2;

pub(crate) async fn create(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS teamlog_meta (k TEXT PRIMARY KEY, v BLOB NOT NULL)",
        "CREATE TABLE IF NOT EXISTS teamlog_entries (
            author BLOB NOT NULL, seq INTEGER NOT NULL, id BLOB NOT NULL UNIQUE,
            version INTEGER NOT NULL, bytes BLOB NOT NULL, state INTEGER NOT NULL,
            reason TEXT,
            PRIMARY KEY (author, seq))",
        "CREATE TABLE IF NOT EXISTS teamlog_history (
            team BLOB NOT NULL, author BLOB NOT NULL, n INTEGER NOT NULL, data BLOB NOT NULL,
            PRIMARY KEY (team, author, n))",
    ] {
        sqlx::query(sql).execute(pool).await?;
    }
    Ok(())
}

fn bytes32(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<[u8; 32], sqlx::Error> {
    let bytes: Vec<u8> = row.try_get(column)?;
    bytes
        .try_into()
        .map_err(|_| sqlx::Error::Decode(format!("{column} is not 32 bytes").into()))
}

pub(crate) async fn meta(pool: &SqlitePool, key: &str) -> Result<Option<Vec<u8>>, sqlx::Error> {
    let row = sqlx::query("SELECT v FROM teamlog_meta WHERE k = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    row.map(|row| row.try_get("v")).transpose()
}

pub(crate) async fn set_meta(
    pool: &SqlitePool,
    key: &str,
    value: &[u8],
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT OR REPLACE INTO teamlog_meta (k, v) VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

fn entry(row: &sqlx::sqlite::SqliteRow) -> Result<Entry, sqlx::Error> {
    Ok(Entry {
        version: row.try_get::<i64, _>("version")? as u8,
        author: DeviceId(bytes32(row, "author")?),
        seq: row.try_get::<i64, _>("seq")? as u64,
        id: EntryId(bytes32(row, "id")?),
        bytes: row.try_get("bytes")?,
    })
}

pub(crate) async fn insert_entry(
    pool: &SqlitePool,
    entry: &Entry,
    state: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO teamlog_entries (author, seq, id, version, bytes, state)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&entry.author.0[..])
    .bind(entry.seq as i64)
    .bind(&entry.id.0[..])
    .bind(i64::from(entry.version))
    .bind(&entry.bytes[..])
    .bind(state)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn set_state(
    pool: &SqlitePool,
    id: EntryId,
    state: i64,
    reason: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE teamlog_entries SET state = ?, reason = ? WHERE id = ?")
        .bind(state)
        .bind(reason)
        .bind(&id.0[..])
        .execute(pool)
        .await?;
    Ok(())
}

/// The state of an entry, `None` when it is not held.
pub(crate) async fn state(pool: &SqlitePool, id: &[u8; 32]) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query("SELECT state FROM teamlog_entries WHERE id = ?")
        .bind(&id[..])
        .fetch_optional(pool)
        .await?;
    row.map(|row| row.try_get("state")).transpose()
}

pub(crate) async fn entry_at(
    pool: &SqlitePool,
    author: DeviceId,
    seq: u64,
) -> Result<Option<Entry>, sqlx::Error> {
    let row = sqlx::query("SELECT * FROM teamlog_entries WHERE author = ? AND seq = ?")
        .bind(&author.0[..])
        .bind(seq as i64)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(entry).transpose()
}

pub(crate) async fn entries_after(
    pool: &SqlitePool,
    author: DeviceId,
    from: u64,
) -> Result<Vec<Entry>, sqlx::Error> {
    let rows =
        sqlx::query("SELECT * FROM teamlog_entries WHERE author = ? AND seq >= ? ORDER BY seq")
            .bind(&author.0[..])
            .bind(from as i64)
            .fetch_all(pool)
            .await?;
    rows.iter().map(entry).collect()
}

/// Every entry in `state`, with the reason p2panda gave when it failed.
pub(crate) async fn in_state(
    pool: &SqlitePool,
    state: i64,
) -> Result<Vec<(Entry, Option<String>)>, sqlx::Error> {
    let rows = sqlx::query("SELECT * FROM teamlog_entries WHERE state = ? ORDER BY author, seq")
        .bind(state)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|row| Ok((entry(row)?, row.try_get("reason")?)))
        .collect()
}

pub(crate) async fn heights(pool: &SqlitePool) -> Result<Vec<(DeviceId, u64)>, sqlx::Error> {
    let rows =
        sqlx::query("SELECT author, COUNT(*) AS height FROM teamlog_entries GROUP BY author")
            .fetch_all(pool)
            .await?;
    rows.iter()
        .map(|row| {
            Ok((
                DeviceId(bytes32(row, "author")?),
                row.try_get::<i64, _>("height")? as u64,
            ))
        })
        .collect()
}

pub(crate) async fn insert_payload(
    pool: &SqlitePool,
    team: TeamId,
    payload: &Payload,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT OR IGNORE INTO teamlog_history (team, author, n, data) VALUES (?, ?, ?, ?)",
    )
    .bind(&team.0[..])
    .bind(&payload.author.0[..])
    .bind(payload.n as i64)
    .bind(&payload.data[..])
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn history(pool: &SqlitePool, team: TeamId) -> Result<Vec<Payload>, sqlx::Error> {
    let rows = sqlx::query("SELECT * FROM teamlog_history WHERE team = ? ORDER BY author, n")
        .bind(&team.0[..])
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|row| {
            Ok(Payload {
                author: DeviceId(bytes32(row, "author")?),
                n: row.try_get::<i64, _>("n")? as u64,
                data: row.try_get("data")?,
            })
        })
        .collect()
}

pub(crate) async fn payloads_by(
    pool: &SqlitePool,
    team: TeamId,
    author: DeviceId,
) -> Result<u64, sqlx::Error> {
    let row =
        sqlx::query("SELECT COUNT(*) AS n FROM teamlog_history WHERE team = ? AND author = ?")
            .bind(&team.0[..])
            .bind(&author.0[..])
            .fetch_one(pool)
            .await?;
    Ok(row.try_get::<i64, _>("n")? as u64)
}
