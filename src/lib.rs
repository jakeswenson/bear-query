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
//! ## Normalized Schema
//!
//! This library automatically normalizes Bear's Core Data schema through Common Table Expressions (CTEs).
//! All queries (both typed methods and the generic `query()` API) have access to these normalized views:
//!
//! ### `notes` Table
//!
//! The normalized view of all notes in Bear.
//!
//! | Column | Type | Description |
//! |--------|------|-------------|
//! | `id` | INTEGER | Note's primary key |
//! | `unique_id` | TEXT | Bear's UUID for the note |
//! | `title` | TEXT | Note title |
//! | `content` | TEXT | Full note content (Markdown) |
//! | `modified` | DATETIME | Last modification timestamp (converted from Core Data epoch) |
//! | `created` | DATETIME | Creation timestamp (converted from Core Data epoch) |
//! | `is_pinned` | INTEGER | 1 if pinned, 0 otherwise |
//! | `is_trashed` | INTEGER | 1 if in trash, 0 otherwise |
//! | `is_archived` | INTEGER | 1 if archived, 0 otherwise |
//!
//! ### `tags` Table
//!
//! The normalized view of all tags.
//!
//! | Column | Type | Description |
//! |--------|------|-------------|
//! | `id` | INTEGER | Tag's primary key |
//! | `name` | TEXT | Tag name (e.g., "work/projects") |
//! | `modified` | DATETIME | Last modification timestamp |
//!
//! ### `note_tags` Table
//!
//! Junction table linking notes to their tags (many-to-many relationship).
//!
//! | Column | Type | Description |
//! |--------|------|-------------|
//! | `note_id` | INTEGER | Foreign key to notes.id |
//! | `tag_id` | INTEGER | Foreign key to tags.id |
//!
//! ### `note_links` Table
//!
//! Links between notes (bidirectional wiki-style links).
//!
//! | Column | Type | Description |
//! |--------|------|-------------|
//! | `from_note_id` | INTEGER | Source note ID |
//! | `to_note_id` | INTEGER | Target note ID |
//!
//! ### Core Data Epoch Conversion
//!
//! Bear uses Apple's Core Data timestamp format (seconds since 2001-01-01 00:00:00 UTC).
//! This library automatically converts all timestamps to standard SQLite datetime format.
//!
//! The conversion is done via a CTE: `unixepoch('2001-01-01')`
//!
//! ### Schema Discovery
//!
//! The library discovers variable schema elements at initialization:
//! - Junction table column names (e.g., `Z_5NOTES`, `Z_13TAGS`)
//! - These numbers may vary across Bear versions
//!
//! For full schema details, see the `SCHEMA.md` documentation file.
//!
//! ## Example
//!
//! ```no_run
//! use bear_query::{BearDb, NotesQuery};
//!
//! # fn main() -> Result<(), bear_query::BearError> {
//! // Create a handle (no connection opened yet)
//! let db = BearDb::new()?;
//!
//! // Each method opens a connection, queries, and closes
//! let all_tags = db.tags()?;
//! let recent_notes = db.notes(NotesQuery::default())?;
//!
//! for note in recent_notes {
//!     println!("{}", note.title());
//! }
//! # Ok(())
//! # }
//! ```

mod dataframe;
mod schema;

pub use polars::prelude as polars_prelude;

use polars::prelude::*;
use rusqlite::types::{FromSql, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Connection, OpenFlags, Row, ToSql};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;
use time::OffsetDateTime;

use dataframe::query_to_dataframe;

/// Specifies the database location for BearDb.
///
/// For production code, use RealPath to connect to Bear's database.
/// For tests, use InMemory to create an isolated test database.
#[derive(Debug, Clone)]
pub enum DatabasePath {
  /// Path to Bear's actual database file
  RealPath(PathBuf),
  /// In-memory database for testing (only available with cfg(test))
  #[cfg(test)]
  InMemory,
}

impl DatabasePath {
  /// Opens a connection based on the database path type.
  /// For RealPath: opens with read-only flags and safety pragmas
  /// For InMemory: creates an in-memory database with test schema
  fn open_connection(&self) -> Result<Connection, BearError> {
    match self {
      DatabasePath::RealPath(path) => {
        // Open with maximum read-only protection:
        // - SQLITE_OPEN_READ_ONLY: Opens in read-only mode
        // - SQLITE_OPEN_NO_MUTEX: Disables internal mutexes (safe for single-threaded read-only)
        let conn = Connection::open_with_flags(
          path,
          OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        // Set busy timeout to handle database contention
        conn.busy_timeout(Duration::from_millis(5000))?;

        // Enable query_only mode as additional safety
        conn.pragma_update(None, "query_only", "ON")?;

        Ok(conn)
      }
      #[cfg(test)]
      DatabasePath::InMemory => {
        let conn = Connection::open_in_memory()?;
        schema::setup_test_schema(&conn)?;
        Ok(conn)
      }
    }
  }
}

#[derive(Debug, thiserror::Error)]
pub enum BearError {
  #[error("Unable to load users home directory")]
  NoHomeDirectory,
  #[error("SQL Error: {source}")]
  SqlError {
    #[from]
    source: rusqlite::Error,
  },
  #[error("Polars Error: {source}")]
  PolarsError {
    #[from]
    source: PolarsError,
  },
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
  pub fn limit(
    mut self,
    limit: u32,
  ) -> Self {
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
  db_path: DatabasePath,
  _metadata: schema::BearDbMetadata,
  normalizing_cte: String,
}

impl BearDb {
  /// Create a new BearDb handle. Opens a temporary connection to discover schema metadata,
  /// generates normalizing CTEs, then closes the connection.
  pub fn new() -> Result<Self, BearError> {
    let home_dir = dirs::home_dir().ok_or(BearError::NoHomeDirectory)?;

    let db_path = home_dir.join(
      "Library/Group Containers/9K33E3U3T4.net.shinyfrog.bear/Application Data/database.sqlite",
    );

    Self::new_with_path(DatabasePath::RealPath(db_path))
  }

  /// Create a new BearDb handle with a specific database path.
  /// This is primarily for testing with in-memory databases.
  pub(crate) fn new_with_path(db_path: DatabasePath) -> Result<Self, BearError> {
    // Open temporary connection to discover metadata
    let connection = db_path.open_connection()?;

    // Discover schema metadata
    let metadata = schema::discover_metadata(&connection)?;

    // Generate normalizing CTE based on discovered metadata
    let normalizing_cte = schema::generate_normalizing_cte(&metadata);

    // Connection is dropped here, closing it
    drop(connection);

    Ok(BearDb {
      db_path,
      _metadata: metadata,
      normalizing_cte,
    })
  }

  /// Opens a short-lived connection, wraps it in a Queryable with normalizing CTEs,
  /// executes the closure, and closes the connection.
  fn with_connection<F, R>(
    &self,
    f: F,
  ) -> Result<R, BearError>
  where
    F: FnOnce(&Queryable) -> Result<R, BearError>,
  {
    // Open connection using DatabasePath's connection handler
    let connection = self.db_path.open_connection()?;

    // Create Queryable wrapper with normalizing CTE
    let queryable = Queryable::new(&connection, &self.normalizing_cte);

    // Execute the closure with the queryable
    // Connection will be automatically closed when it goes out of scope
    f(&queryable)
  }

  /// Retrieves all tags from Bear
  pub fn tags(&self) -> Result<BearTags, BearError> {
    self.with_connection(|queryable| {
      let mut statement = queryable.prepare(
        r"
      SELECT
        id,
        name,
        modified
      FROM tags
      ORDER BY name ASC",
      )?;

      let results: rusqlite::Result<Vec<BearTag>> = statement
        .query_map([], |row| {
          Ok(BearTag {
            id: row.get("id")?,
            name: row.get("name")?,
            modified: row.get("modified")?,
          })
        })?
        .collect();

      let tags = results?.into_iter().map(|tag| (tag.id, tag)).collect();

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
  pub fn notes(
    &self,
    query: NotesQuery,
  ) -> Result<Vec<BearNote>, BearError> {
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

      let limit_clause = query
        .limit
        .map(|l| format!("LIMIT {}", l))
        .unwrap_or_default();

      let query = format!(
        r"
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
      {}",
        where_clause, limit_clause
      );

      let mut statement = queryable.prepare(&query)?;

      let results: rusqlite::Result<Vec<BearNote>> =
        statement.query_map([], note_from_row)?.collect();

      Ok(results?)
    })
  }

  /// Retrieves all notes linked from the specified note
  pub fn note_links(
    &self,
    from: BearNoteId,
  ) -> Result<Vec<BearNote>, BearError> {
    self.with_connection(|queryable| {
      let mut statement = queryable.prepare(
        r"
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
      ORDER BY n.modified DESC",
      )?;

      let results: rusqlite::Result<Vec<BearNote>> =
        statement.query_map([from], note_from_row)?.collect();

      Ok(results?)
    })
  }

  /// Retrieves all tag IDs associated with the specified note
  pub fn note_tags(
    &self,
    from: BearNoteId,
  ) -> Result<HashSet<BearTagId>, BearError> {
    self.with_connection(|queryable| {
      let mut statement = queryable.prepare(
        r"
      SELECT
        tag_id
      FROM note_tags
      WHERE note_id = ?",
      )?;

      let results: rusqlite::Result<HashSet<BearTagId>> = statement
        .query_map([from], |row| row.get("tag_id"))?
        .collect();

      Ok(results?)
    })
  }

  /// Execute a generic SQL SELECT query and return results as a Polars DataFrame.
  ///
  /// The query automatically has the normalizing CTEs prepended, so you can query
  /// against clean table names: `notes`, `tags`, `note_tags`, `note_links`.
  ///
  /// # Safety
  /// This method trusts the read-only connection flags to prevent writes. Only SELECT
  /// queries should be used, though this is not enforced by the library.
  ///
  /// # Examples
  /// ```no_run
  /// # use bear_query::BearDb;
  /// # fn main() -> Result<(), bear_query::BearError> {
  /// let db = BearDb::new()?;
  ///
  /// // Query normalized tables
  /// let df = db.query("SELECT title, modified FROM notes LIMIT 5")?;
  ///
  /// // Join tables
  /// let df = db.query(r"
  ///   SELECT n.title, t.name as tag_name
  ///   FROM notes n
  ///   JOIN note_tags nt ON n.id = nt.note_id
  ///   JOIN tags t ON nt.tag_id = t.id
  ///   WHERE n.is_trashed = 0
  ///   LIMIT 10
  /// ")?;
  ///
  /// println!("{}", df);
  /// # Ok(())
  /// # }
  /// ```
  pub fn query(
    &self,
    sql: &str,
  ) -> Result<DataFrame, BearError> {
    self.with_connection(|queryable| query_to_dataframe(queryable, sql))
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
  fn new(
    conn: &'a Connection,
    normalizing_cte: &'a str,
  ) -> Self {
    Self {
      conn,
      normalizing_cte,
    }
  }

  /// Test-only constructor for creating Queryable in tests
  ///
  /// This is pub(crate) so tests in other modules can create Queryables
  #[cfg(test)]
  pub(crate) fn new_for_test(
    conn: &'a Connection,
    normalizing_cte: &'a str,
  ) -> Self {
    Self::new(conn, normalizing_cte)
  }

  /// Prepares a statement with the normalizing CTE automatically prepended.
  /// The user's SQL should query against normalized table names (notes, tags, note_tags, note_links).
  pub fn prepare(
    &self,
    user_sql: &str,
  ) -> rusqlite::Result<rusqlite::Statement<'a>> {
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
  pub fn get(
    &self,
    tag_id: &BearTagId,
  ) -> Option<&BearTag> {
    self.tags.get(tag_id)
  }

  pub fn count(&self) -> usize {
    self.tags.len()
  }

  pub fn iter(&self) -> impl Iterator<Item = &BearTag> {
    self.tags.values()
  }

  pub fn names(
    &self,
    tag_ids: &HashSet<BearTagId>,
  ) -> HashSet<String> {
    tag_ids
      .into_iter()
      .filter_map(|id| self.get(id).map(|t| t.name.clone()))
      .collect()
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

#[cfg(test)]
mod tests {
  use super::*;

  /// Integration test demonstrating BearDb with in-memory database
  #[test]
  fn test_beardb_with_inmemory() {
    // Create a BearDb with an in-memory database (automatically sets up schema)
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Test the typed API
    let tags = db.tags().unwrap();
    assert_eq!(tags.count(), 2); // Should have 2 tags from test data

    let notes = db.notes(NotesQuery::default()).unwrap();
    assert_eq!(notes.len(), 2); // default() excludes trashed, so 2 notes

    // Test filtering - include all notes
    let all_notes = db
      .notes(NotesQuery::new().include_all().no_limit())
      .unwrap();
    assert_eq!(all_notes.len(), 3); // 3 notes total including trashed

    // Test the generic SQL query API
    let df = db
      .query("SELECT id, title FROM notes WHERE is_trashed = 0")
      .unwrap();
    assert_eq!(df.height(), 2); // 2 non-trashed notes
    assert_eq!(df.width(), 2); // 2 columns (id, title)

    // Test aggregation
    let df = db.query("SELECT COUNT(*) as count FROM notes").unwrap();
    assert_eq!(df.height(), 1);
    assert_eq!(df.width(), 1);

    // Verify the count column is an integer (not string)
    let series = df.column("count").unwrap();
    let value = series.get(0).unwrap();
    match value {
      AnyValue::Int64(n) => assert_eq!(n, 3),
      _ => panic!("Expected Int64, got: {:?}", value),
    }

    // Test join query
    let df = db
      .query(
        r"
      SELECT n.title, t.name as tag_name
      FROM notes n
      JOIN note_tags nt ON n.id = nt.note_id
      JOIN tags t ON nt.tag_id = t.id
    ",
      )
      .unwrap();
    assert_eq!(df.height(), 2); // 2 note-tag relationships
  }
}
