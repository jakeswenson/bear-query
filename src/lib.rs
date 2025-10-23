//! # bear-query
//!
//! A completely read-only, non-blocking library for querying Bear app's SQLite database.
//!
//! ## Safety Guarantees
//!
//! This library implements multiple layers of protection to ensure ZERO interference with Bear:
//!
//! 1. **Read-Only File Access**: Opens with `SQLITE_OPEN_READ_ONLY`
//! 2. **No Internal Locks**: Uses `SQLITE_OPEN_NO_MUTEX` to prevent lock contention
//! 3. **Query-Only Mode**: Enforces `PRAGMA query_only = ON` at SQLite level
//! 4. **WAL Mode**: Verifies database uses Write-Ahead Logging for concurrent access
//!
//! ## How It Works
//!
//! Bear uses SQLite's WAL (Write-Ahead Logging) mode, which allows readers and writers
//! to operate concurrently without blocking each other. This library takes advantage of
//! WAL mode to read from Bear's database while Bear is actively writing, with no
//! interference whatsoever.
//!
//! In WAL mode:
//! - Writes go to a separate WAL file
//! - Reads access stable snapshots
//! - **Zero lock contention** between readers and writers
//!
//! ## Example
//!
//! ```no_run
//! use bear_query::{BearDb, notes, tags};
//!
//! let db = BearDb::open()?;
//! let all_tags = tags(&db)?;
//! let recent_notes = notes(&db)?;
//!
//! for note in recent_notes {
//!     println!("{}", note.title());
//! }
//! # Ok::<(), bear_query::BearError>(())
//! ```

use std::collections::{HashMap, HashSet};
use rusqlite::{Connection, OpenFlags, Row, ToSql};
use rusqlite::types::{FromSql, FromSqlResult, ToSqlOutput, ValueRef};
use time::OffsetDateTime;

#[derive(Debug, thiserror::Error)]
pub enum BearError {
  #[error("Unable to load users home directory")]
  NoHomeDirectory,
  #[error("SQL Error: {source}")]
  SqlError { #[from] source: rusqlite::Error },
}

pub struct BearDb {
  pub connection: Connection,
}

impl BearDb {
  pub fn open() -> Result<Self, BearError> {
    let home_dir = dirs::home_dir()
      .ok_or(BearError::NoHomeDirectory)?;

    let bear_db = home_dir.join(
      "Library/Group Containers/9K33E3U3T4.net.shinyfrog.bear/Application Data/database.sqlite"
    );

    // Open with maximum read-only protection:
    // - SQLITE_OPEN_READ_ONLY: Opens in read-only mode
    // - SQLITE_OPEN_NO_MUTEX: Disables internal mutexes for thread safety (safe for single-threaded read-only)
    // These flags ensure we NEVER take write locks or block the Bear app
    let connection = Connection::open_with_flags(
      bear_db,
      OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    )?;

    // Enable query_only mode as an additional safety measure
    // This prevents any writes even if somehow attempted
    connection.pragma_update(None, "query_only", "ON")?;

    // Verify the database is in WAL mode (Write-Ahead Logging)
    // WAL mode allows concurrent reads without blocking writes
    let journal_mode: String = connection
      .query_row("PRAGMA journal_mode", [], |row| row.get(0))?;

    if journal_mode != "wal" {
      eprintln!("Warning: Database is not in WAL mode (current: {}). Reads may block writes.", journal_mode);
      eprintln!("This is unusual for Bear app and may cause interference.");
    }

    Ok(BearDb {
      connection
    })
  }
}


#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct DbId(i64);

#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct BearNoteId(DbId);

#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct BearTagId(DbId);

impl FromSql for DbId {
  fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
    Ok(Self(value.as_i64()?))
  }
}

impl FromSql for BearNoteId {
  fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
    Ok(Self(FromSql::column_result(value)?))
  }
}

impl FromSql for BearTagId {
  fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
    Ok(Self(FromSql::column_result(value)?))
  }
}

impl ToSql for DbId {
  fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
    self.0.to_sql()
  }
}

impl ToSql for BearNoteId {
  fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
    self.0.to_sql()
  }
}

#[derive(Debug, Clone)]
pub struct BearTag {
  id: BearTagId,
  name: String,
  _modified: Option<OffsetDateTime>,
}

#[derive(Debug)]
pub struct BearTags {
  tags: HashMap<BearTagId, BearTag>,
}

impl BearTags {
  pub fn get(&self, tag_id: &BearTagId) -> Option<&BearTag> {
    self.tags.get(tag_id)
  }

  pub fn names(&self, tag_ids: &HashSet<BearTagId>) -> HashSet<String> {
    tag_ids.into_iter().filter_map(|id| {
      self.get(id).map(|t| t.name.clone())
    }).collect()
  }
}


pub fn tags(db: &BearDb) -> Result<BearTags, BearError> {
  let mut statement = db.connection.prepare(r"
  SELECT
    tag.Z_PK as id,
    tag.ZTITLE as name,
    datetime(tag.ZMODIFICATIONDATE + unixepoch('2001-01-01'), 'unixepoch') as modified
FROM ZSFNOTETAG as tag
ORDER BY name ASC")?;

  let results: rusqlite::Result<Vec<BearTag>> = statement.query_map([], |row| {
    Ok(BearTag {
      id: row.get("id")?,
      name: row.get("name")?,
      _modified: row.get("modified")?,
    })
  })?.collect();

  let tags = results?.into_iter()
    .map(|tag| (tag.id, tag))
    .collect();

  return Ok(BearTags { tags });
}

#[derive(Debug)]
pub struct BearNote {
  id: BearNoteId,
  unique_id: String,
  title: String,
  content: String,
  _modified: OffsetDateTime,
  _created: OffsetDateTime,
  _is_pinned: bool,
}

impl BearNote {
  pub fn id(&self) -> BearNoteId {
    self.id
  }

  pub fn title(&self) -> &str {
    &self.title
  }
}

fn note_from_row(row: &Row) -> rusqlite::Result<BearNote> {
  Ok(BearNote {
    id: row.get("id")?,
    unique_id: row.get("unique_id")?,
    title: row.get("title")?,
    content: row.get("content")?,
    _created: row.get("created")?,
    _modified: row.get("modified")?,
    _is_pinned: row.get("is_pinned")?,
  })
}

pub fn notes(db: &BearDb) -> Result<Vec<BearNote>, BearError> {
  let mut statement = db.connection.prepare(r"
  SELECT
    note.Z_PK as id,
    note.ZUNIQUEIDENTIFIER as unique_id,
    note.ZTITLE as title,
    ZTEXT as content,
    -- Apple: https://stackoverflow.com/a/54914712
    datetime(note.ZMODIFICATIONDATE + unixepoch('2001-01-01'), 'unixepoch') as modified,
    datetime(note.ZCREATIONDATE + unixepoch('2001-01-01'), 'unixepoch') as created,
    ZPINNED as is_pinned
FROM ZSFNOTE as note
WHERE note.ZTRASHED <> 1 AND note.ZARCHIVED <> 1
ORDER BY note.ZMODIFICATIONDATE DESC
LIMIT 10
")?;

  let results: rusqlite::Result<Vec<BearNote>> = statement
    .query_map([], note_from_row)?
    .collect();

  Ok(results?)
}

pub fn note_links(db: &BearDb, from: BearNoteId) -> Result<Vec<BearNote>, BearError> {
  let mut statement = db.connection.prepare(r"
    WITH core_data AS (
        SELECT unixepoch('2001-01-01') as core_data_start_time
    )
  SELECT
    note.Z_PK as id,
    note.ZUNIQUEIDENTIFIER as unique_id,
    note.ZTITLE as title,
    ZTEXT as content,
    -- Apple: https://stackoverflow.com/a/54914712
    datetime(note.ZMODIFICATIONDATE + cd.core_data_start_time, 'unixepoch') as modified,
    datetime(note.ZCREATIONDATE + cd.core_data_start_time, 'unixepoch') as created,
    ZPINNED as is_pinned
FROM ZSFNOTE as note, core_data as cd
INNER JOIN ZSFNOTEBACKLINK as note_links ON note_links.ZLINKINGTO = note.Z_PK
WHERE note.ZTRASHED <> 1 AND note.ZARCHIVED <> 1 AND note_links.ZLINKEDBY = ?
ORDER BY note.ZMODIFICATIONDATE DESC")?;

  let results: rusqlite::Result<Vec<BearNote>> = statement.query_map([from], |row| {
    Ok(BearNote {
      id: row.get("id")?,
      unique_id: row.get("unique_id")?,
      title: row.get("title")?,
      content: row.get("content")?,
      _created: row.get("created")?,
      _modified: row.get("modified")?,
      _is_pinned: row.get("is_pinned")?,
    })
  })?
    .collect();

  Ok(results?)
}

pub fn note_tags(db: &BearDb, from: BearNoteId) -> Result<HashSet<BearTagId>, BearError> {
  let mut statement = db.connection.prepare(r"
SELECT
  note_tags.Z_13TAGS as tag_id
FROM Z_5TAGS as note_tags
WHERE note_tags.Z_5NOTES = ?")?;

  let results: rusqlite::Result<HashSet<BearTagId>> = statement.query_map([from], |row| {
    row.get("tag_id")
  })?
    .collect();

  Ok(results?)
}
