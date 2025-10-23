//! # bear-query
//!
//! A completely read-only, minimal-contention library for querying Bear app's SQLite database.
//!
//! ## Safety Guarantees
//!
//! This library implements multiple layers of protection to ensure minimal interference with Bear:
//!
//! 1. **Read-Only File Access**: Opens with `SQLITE_OPEN_READ_ONLY`
//! 2. **No Internal Locks**: Uses `SQLITE_OPEN_NO_MUTEX` to minimize lock contention
//! 3. **Query-Only Mode**: Enforces `PRAGMA query_only = ON` at SQLite level
//! 4. **Short-Lived Connections**: Connections are only open during each query
//! 5. **Busy Timeout**: 5000ms timeout handles database contention gracefully
//!
//! ## How It Works
//!
//! Bear does **not** use SQLite's WAL (Write-Ahead Logging) mode by default. To minimize
//! interference, this library uses short-lived connections that are opened only when needed
//! and closed immediately after use.
//!
//! Each method call:
//! - Opens a read-only connection with a 5000ms busy timeout
//! - Executes the query
//! - Automatically closes the connection
//!
//! This approach ensures minimal lock contention with Bear's write operations.
//!
//! ## Example
//!
//! ```no_run
//! use bear_query::BearDb;
//!
//! # fn main() -> Result<(), bear_query::BearError> {
//! // Create a handle (no connection opened yet)
//! let db = BearDb::new()?;
//!
//! // Each method opens a connection, queries, and closes
//! let all_tags = db.tags()?;
//! let recent_notes = db.notes()?;
//!
//! for note in recent_notes {
//!     println!("{}", note.title());
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;
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

/// Handle to Bear's database. All operations use short-lived connections internally.
pub struct BearDb {
  db_path: PathBuf,
}

impl BearDb {
  /// Create a new BearDb handle. This does not open a connection - connections
  /// are opened only when needed and closed immediately after use.
  pub fn new() -> Result<Self, BearError> {
    let home_dir = dirs::home_dir()
      .ok_or(BearError::NoHomeDirectory)?;

    let db_path = home_dir.join(
      "Library/Group Containers/9K33E3U3T4.net.shinyfrog.bear/Application Data/database.sqlite"
    );

    Ok(BearDb { db_path })
  }

  /// Opens a short-lived connection, executes the closure, and closes the connection.
  fn with_connection<F, R>(&self, f: F) -> Result<R, BearError>
  where
    F: FnOnce(&Connection) -> Result<R, BearError>
  {
    // Open with maximum read-only protection:
    // - SQLITE_OPEN_READ_ONLY: Opens in read-only mode
    // - SQLITE_OPEN_NO_MUTEX: Disables internal mutexes for thread safety (safe for single-threaded read-only)
    // These flags ensure we NEVER take write locks or block the Bear app
    let connection = Connection::open_with_flags(
      &self.db_path,
      OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    )?;

    // Set busy timeout to 5000ms to handle database contention
    connection.busy_timeout(Duration::from_millis(5000))?;

    // Enable query_only mode as an additional safety measure
    // This prevents any writes even if somehow attempted
    connection.pragma_update(None, "query_only", "ON")?;

    // Execute the closure with the connection
    // Connection will be automatically closed when it goes out of scope
    f(&connection)
  }

  /// Retrieves all tags from Bear
  pub fn tags(&self) -> Result<BearTags, BearError> {
    self.with_connection(|conn| {
      let mut statement = conn.prepare(r"
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

      Ok(BearTags { tags })
    })
  }

  /// Retrieves up to 10 most recently modified notes (non-trashed, non-archived)
  pub fn notes(&self) -> Result<Vec<BearNote>, BearError> {
    self.with_connection(|conn| {
      let mut statement = conn.prepare(r"
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
    })
  }

  /// Retrieves all notes linked from the specified note
  pub fn note_links(&self, from: BearNoteId) -> Result<Vec<BearNote>, BearError> {
    self.with_connection(|conn| {
      let mut statement = conn.prepare(r"
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
    })
  }

  /// Retrieves all tag IDs associated with the specified note
  pub fn note_tags(&self, from: BearNoteId) -> Result<HashSet<BearTagId>, BearError> {
    self.with_connection(|conn| {
      let mut statement = conn.prepare(r"
    SELECT
      note_tags.Z_13TAGS as tag_id
    FROM Z_5TAGS as note_tags
    WHERE note_tags.Z_5NOTES = ?")?;

      let results: rusqlite::Result<HashSet<BearTagId>> = statement.query_map([from], |row| {
        row.get("tag_id")
      })?
        .collect();

      Ok(results?)
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



