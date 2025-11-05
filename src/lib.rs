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

/// Metadata discovered from Bear's database schema at initialization time.
/// This captures the variable parts of Bear's Core Data schema that may change across versions.
#[derive(Debug, Clone)]
struct BearDbMetadata {
  /// Column name in junction table that references notes (e.g., "Z_5NOTES")
  junction_notes_column: String,
  /// Column name in junction table that references tags (e.g., "Z_13TAGS")
  junction_tags_column: String,
}

/// Query options for filtering notes
#[derive(Debug, Clone)]
pub struct NotesQuery {
  limit: Option<u32>,
  include_trashed: bool,
  include_archived: bool,
}

impl Default for NotesQuery {
  fn default() -> Self {
    Self {
      limit: Some(10),
      include_trashed: false,
      include_archived: false,
    }
  }
}

impl NotesQuery {
  /// Create a new NotesQuery with default settings (limit: 10, exclude trashed and archived)
  pub fn new() -> Self {
    Self::default()
  }

  /// Set a limit on the number of notes to return
  pub fn limit(mut self, limit: u32) -> Self {
    self.limit = Some(limit);
    self
  }

  /// Remove the limit and return all matching notes
  pub fn no_limit(mut self) -> Self {
    self.limit = None;
    self
  }

  /// Include trashed notes in results
  pub fn include_trashed(mut self) -> Self {
    self.include_trashed = true;
    self
  }

  /// Include archived notes in results
  pub fn include_archived(mut self) -> Self {
    self.include_archived = true;
    self
  }

  /// Include both trashed and archived notes in results
  pub fn include_all(mut self) -> Self {
    self.include_trashed = true;
    self.include_archived = true;
    self
  }
}

/// Handle to Bear's database. All operations use short-lived connections internally.
pub struct BearDb {
  db_path: PathBuf,
  #[allow(dead_code)]
  metadata: BearDbMetadata,
  normalizing_cte: String,
}

impl BearDb {
  /// Create a new BearDb handle. Opens a temporary connection to discover schema metadata,
  /// generates normalizing CTEs, then closes the connection.
  pub fn new() -> Result<Self, BearError> {
    let home_dir = dirs::home_dir()
      .ok_or(BearError::NoHomeDirectory)?;

    let db_path = home_dir.join(
      "Library/Group Containers/9K33E3U3T4.net.shinyfrog.bear/Application Data/database.sqlite"
    );

    // Open temporary connection to discover metadata
    let connection = Connection::open_with_flags(
      &db_path,
      OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
    )?;
    connection.busy_timeout(Duration::from_millis(5000))?;
    connection.pragma_update(None, "query_only", "ON")?;

    // Discover schema metadata
    let metadata = Self::discover_metadata(&connection)?;

    // Generate normalizing CTE based on discovered metadata
    let normalizing_cte = Self::generate_normalizing_cte(&metadata);

    // Connection is dropped here, closing it
    drop(connection);

    Ok(BearDb {
      db_path,
      metadata,
      normalizing_cte,
    })
  }

  /// Discovers variable schema information from Bear's database
  fn discover_metadata(conn: &Connection) -> Result<BearDbMetadata, BearError> {
    // Query the junction table to find its column names
    let mut stmt = conn.prepare("PRAGMA table_info(Z_5TAGS)")?;
    let columns: Vec<String> = stmt.query_map([], |row| {
      row.get::<_, String>("name")
    })?.collect::<Result<Vec<_>, _>>()?;

    // Find the columns that reference notes and tags
    // They follow the pattern Z_<number>NOTES and Z_<number>TAGS
    let junction_notes_column = columns.iter()
      .find(|name| name.ends_with("NOTES"))
      .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?
      .clone();

    let junction_tags_column = columns.iter()
      .find(|name| name.ends_with("TAGS"))
      .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?
      .clone();

    Ok(BearDbMetadata {
      junction_notes_column,
      junction_tags_column,
    })
  }

  /// Generates the normalizing CTE SQL that abstracts Bear's Core Data schema
  fn generate_normalizing_cte(metadata: &BearDbMetadata) -> String {
    format!(r#"
WITH
  core_data AS (
    SELECT unixepoch('2001-01-01') as epoch
  ),
  notes AS (
    SELECT
      n.Z_PK as id,
      n.ZUNIQUEIDENTIFIER as unique_id,
      n.ZTITLE as title,
      n.ZTEXT as content,
      datetime(n.ZMODIFICATIONDATE + cd.epoch, 'unixepoch') as modified,
      datetime(n.ZCREATIONDATE + cd.epoch, 'unixepoch') as created,
      n.ZPINNED as is_pinned,
      n.ZTRASHED as is_trashed,
      n.ZARCHIVED as is_archived
    FROM ZSFNOTE as n, core_data as cd
  ),
  tags AS (
    SELECT
      t.Z_PK as id,
      t.ZTITLE as name,
      datetime(t.ZMODIFICATIONDATE + cd.epoch, 'unixepoch') as modified
    FROM ZSFNOTETAG as t, core_data as cd
  ),
  note_tags AS (
    SELECT
      nt.{} as note_id,
      nt.{} as tag_id
    FROM Z_5TAGS as nt
  ),
  note_links AS (
    SELECT
      nl.ZLINKEDBY as from_note_id,
      nl.ZLINKINGTO as to_note_id
    FROM ZSFNOTEBACKLINK as nl
  )
"#, metadata.junction_notes_column, metadata.junction_tags_column)
  }

  /// Opens a short-lived connection, wraps it in a Queryable with normalizing CTEs,
  /// executes the closure, and closes the connection.
  fn with_connection<F, R>(&self, f: F) -> Result<R, BearError>
  where
    F: FnOnce(&Queryable) -> Result<R, BearError>
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

    // Create Queryable wrapper with normalizing CTE
    let queryable = Queryable::new(&connection, &self.normalizing_cte);

    // Execute the closure with the queryable
    // Connection will be automatically closed when it goes out of scope
    f(&queryable)
  }

  /// Retrieves all tags from Bear
  pub fn tags(&self) -> Result<BearTags, BearError> {
    self.with_connection(|queryable| {
      let mut statement = queryable.prepare(r"
      SELECT
        id,
        name,
        modified
      FROM tags
      ORDER BY name ASC")?;

      let results: rusqlite::Result<Vec<BearTag>> = statement.query_map([], |row| {
        Ok(BearTag {
          id: row.get("id")?,
          name: row.get("name")?,
          modified: row.get("modified")?,
        })
      })?.collect();

      let tags = results?.into_iter()
        .map(|tag| (tag.id, tag))
        .collect();

      Ok(BearTags { tags })
    })
  }

  /// Retrieves notes from Bear, ordered by most recently modified.
  ///
  /// # Examples
  /// ```no_run
  /// # use bear_query::{BearDb, NotesQuery};
  /// # fn main() -> Result<(), bear_query::BearError> {
  /// let db = BearDb::new()?;
  ///
  /// // Get 10 most recent notes (default)
  /// let notes = db.notes(NotesQuery::default())?;
  ///
  /// // Get 20 most recent notes
  /// let notes = db.notes(NotesQuery::new().limit(20))?;
  ///
  /// // Get all notes including trashed and archived
  /// let notes = db.notes(NotesQuery::new().no_limit().include_all())?;
  /// # Ok(())
  /// # }
  /// ```
  pub fn notes(&self, query: NotesQuery) -> Result<Vec<BearNote>, BearError> {
    self.with_connection(|queryable| {
      // Build WHERE clause based on query options
      let mut where_clauses = Vec::new();
      if !query.include_trashed {
        where_clauses.push("is_trashed <> 1");
      }
      if !query.include_archived {
        where_clauses.push("is_archived <> 1");
      }

      let where_clause = if where_clauses.is_empty() {
        String::new()
      } else {
        format!("WHERE {}", where_clauses.join(" AND "))
      };

      let limit_clause = query.limit.map(|l| format!("LIMIT {}", l)).unwrap_or_default();

      let query = format!(r"
      SELECT
        id,
        unique_id,
        title,
        content,
        modified,
        created,
        is_pinned
      FROM notes
      {}
      ORDER BY modified DESC
      {}", where_clause, limit_clause);

      let mut statement = queryable.prepare(&query)?;

      let results: rusqlite::Result<Vec<BearNote>> = statement
        .query_map([], note_from_row)?
        .collect();

      Ok(results?)
    })
  }

  /// Retrieves all notes linked from the specified note
  pub fn note_links(&self, from: BearNoteId) -> Result<Vec<BearNote>, BearError> {
    self.with_connection(|queryable| {
      let mut statement = queryable.prepare(r"
      SELECT
        n.id,
        n.unique_id,
        n.title,
        n.content,
        n.modified,
        n.created,
        n.is_pinned
      FROM notes as n
      INNER JOIN note_links as nl ON nl.to_note_id = n.id
      WHERE n.is_trashed <> 1 AND n.is_archived <> 1 AND nl.from_note_id = ?
      ORDER BY n.modified DESC")?;

      let results: rusqlite::Result<Vec<BearNote>> = statement
        .query_map([from], note_from_row)?
        .collect();

      Ok(results?)
    })
  }

  /// Retrieves all tag IDs associated with the specified note
  pub fn note_tags(&self, from: BearNoteId) -> Result<HashSet<BearTagId>, BearError> {
    self.with_connection(|queryable| {
      let mut statement = queryable.prepare(r"
      SELECT
        tag_id
      FROM note_tags
      WHERE note_id = ?")?;

      let results: rusqlite::Result<HashSet<BearTagId>> = statement.query_map([from], |row| {
        row.get("tag_id")
      })?
        .collect();

      Ok(results?)
    })
  }
}

/// A wrapper around a database connection that automatically applies normalizing CTEs to queries.
/// This abstracts away Bear's Core Data quirks (Z_ prefixes, numbered columns, epoch timestamps).
pub struct Queryable<'a> {
  conn: &'a Connection,
  normalizing_cte: &'a str,
}

impl<'a> Queryable<'a> {
  /// Creates a new Queryable from a connection and pre-generated CTE string
  fn new(conn: &'a Connection, normalizing_cte: &'a str) -> Self {
    Self {
      conn,
      normalizing_cte,
    }
  }

  /// Prepares a statement with the normalizing CTE automatically prepended.
  /// The user's SQL should query against normalized table names (notes, tags, note_tags, note_links).
  pub fn prepare(&self, user_sql: &str) -> rusqlite::Result<rusqlite::Statement<'a>> {
    let full_sql = format!("{}\n{}", self.normalizing_cte, user_sql);
    self.conn.prepare(&full_sql)
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
  modified: Option<OffsetDateTime>,
}

impl BearTag {
  pub fn id(&self) -> BearTagId {
    self.id
  }

  pub fn name(&self) -> &str {
    &self.name
  }

  pub fn modified(&self) -> Option<OffsetDateTime> {
    self.modified
  }
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
  modified: OffsetDateTime,
  created: OffsetDateTime,
  is_pinned: bool,
}

impl BearNote {
  pub fn id(&self) -> BearNoteId {
    self.id
  }

  pub fn unique_id(&self) -> &str {
    &self.unique_id
  }

  pub fn title(&self) -> &str {
    &self.title
  }

  pub fn content(&self) -> &str {
    &self.content
  }

  pub fn modified(&self) -> OffsetDateTime {
    self.modified
  }

  pub fn created(&self) -> OffsetDateTime {
    self.created
  }

  pub fn is_pinned(&self) -> bool {
    self.is_pinned
  }
}

fn note_from_row(row: &Row) -> rusqlite::Result<BearNote> {
  Ok(BearNote {
    id: row.get("id")?,
    unique_id: row.get("unique_id")?,
    title: row.get("title")?,
    content: row.get("content")?,
    created: row.get("created")?,
    modified: row.get("modified")?,
    is_pinned: row.get("is_pinned")?,
  })
}



