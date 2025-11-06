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
//! | `id` | TEXT | Bear's UUID for the note (primary identifier) |
//! | `core_db_id` | INTEGER | Internal Core Data primary key (for joins) |
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
//! | `note_id` | TEXT | Note UUID (references notes.id) |
//! | `tag_id` | INTEGER | Tag ID (references tags.id) |
//!
//! ### `note_links` Table
//!
//! Links between notes (bidirectional wiki-style links).
//!
//! | Column | Type | Description |
//! |--------|------|-------------|
//! | `from_note_id` | TEXT | Source note UUID (references notes.id) |
//! | `to_note_id` | TEXT | Target note UUID (references notes.id) |
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
//! use bear_query::{BearDb, NotesQuery, SearchQuery, SortOn};
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
//!     let title = note.title();
//!     if title.is_empty() {
//!         println!("[Untitled]");
//!     } else {
//!         println!("{}", title);
//!     }
//! }
//!
//! // Search notes by title and/or content
//! let search_results = db.search(SearchQuery::new("rust"))?;
//!
//! // Advanced search with filters
//! let filtered_results = db.search(
//!     SearchQuery::new("project")
//!         .title_only()
//!         .sort_by(SortOn::Title.asc())
//!         .limit(20)
//! )?;
//! # Ok(())
//! # }
//! ```

mod dataframe;
mod models;
mod schema;

pub use models::{Note, NoteId, Tag, TagId, TagsMap};
pub use polars::prelude as polars_prelude;

use models::{note_from_row, tag_from_row};
use polars::prelude::*;
use rusqlite::{Connection, OpenFlags};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use dataframe::query_to_dataframe;

/// Specifies the database location for BearDb.
///
/// For production code, use RealPath to connect to Bear's database.
/// For tests, use InMemory to create an isolated test database.
#[derive(Debug, Clone)]
enum DatabasePath {
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

/// Query options for filtering notes.
///
/// Use this builder to configure how notes are retrieved from Bear.
/// By default, returns the 10 most recently modified notes, excluding
/// trashed and archived notes.
///
/// # Examples
///
/// ```no_run
/// # use bear_query::{BearDb, NotesQuery};
/// # fn main() -> Result<(), bear_query::BearError> {
/// let db = BearDb::new()?;
///
/// // Default: 10 most recent notes, exclude trashed/archived
/// let notes = db.notes(NotesQuery::default())?;
///
/// // Get 20 notes
/// let notes = db.notes(NotesQuery::new().limit(20))?;
///
/// // Get all notes including trashed and archived
/// let notes = db.notes(NotesQuery::new().no_limit().include_all())?;
///
/// // Get all non-trashed notes (including archived)
/// let notes = db.notes(NotesQuery::new().no_limit().include_archived())?;
/// # Ok(())
/// # }
/// ```
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

/// What field to sort by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOn {
  /// Sort by modification timestamp
  Modified,
  /// Sort by creation timestamp
  Created,
  /// Sort by note title
  Title,
}

impl SortOn {
  /// Sort in ascending order (oldest/A-Z first)
  pub fn asc(self) -> SortOrder {
    SortOrder::Asc(self)
  }

  /// Sort in descending order (newest/Z-A first)
  pub fn desc(self) -> SortOrder {
    SortOrder::Desc(self)
  }
}

/// Sort order for search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
  /// Ascending order (oldest/A-Z first)
  Asc(SortOn),
  /// Descending order (newest/Z-A first)
  Desc(SortOn),
}

impl Default for SortOrder {
  fn default() -> Self {
    SortOrder::Desc(SortOn::Modified)
  }
}

impl SortOrder {
  fn to_sql(&self) -> &'static str {
    match self {
      SortOrder::Desc(SortOn::Modified) => "modified DESC",
      SortOrder::Asc(SortOn::Modified) => "modified ASC",
      SortOrder::Desc(SortOn::Created) => "created DESC",
      SortOrder::Asc(SortOn::Created) => "created ASC",
      SortOrder::Asc(SortOn::Title) => "title ASC",
      SortOrder::Desc(SortOn::Title) => "title DESC",
    }
  }
}

/// Query builder for searching notes.
///
/// Use this builder to configure note search with flexible filtering, sorting, and limits.
/// By default, searches both title and content, returns up to 50 results sorted by most
/// recently modified, and excludes trashed and archived notes.
///
/// # Examples
///
/// ```no_run
/// # use bear_query::{BearDb, SearchQuery, SortOn};
/// # fn main() -> Result<(), bear_query::BearError> {
/// let db = BearDb::new()?;
///
/// // Search in both title and content (default)
/// let notes = db.search(SearchQuery::new("rust"))?;
///
/// // Search only in titles
/// let notes = db.search(
///     SearchQuery::new("rust")
///         .title_only()
/// )?;
///
/// // Search only in content
/// let notes = db.search(
///     SearchQuery::new("rust")
///         .content_only()
/// )?;
///
/// // Complex search with custom options
/// let notes = db.search(
///     SearchQuery::new("programming")
///         .title_only()
///         .limit(20)
///         .sort_by(SortOn::Title.asc())
///         .include_archived()
/// )?;
///
/// // Case-sensitive search
/// let notes = db.search(
///     SearchQuery::new("Rust")
///         .case_sensitive()
/// )?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct SearchQuery {
  query: String,
  search_title: bool,
  search_content: bool,
  case_sensitive: bool,
  limit: Option<u32>,
  sort_by: SortOrder,
  include_trashed: bool,
  include_archived: bool,
}

impl SearchQuery {
  /// Create a new search with the given query string.
  ///
  /// By default:
  /// - Searches both title and content
  /// - Case-insensitive search
  /// - Limit: 50 results
  /// - Sort: Most recently modified first
  /// - Excludes trashed and archived notes
  pub fn new(query: impl Into<String>) -> Self {
    Self {
      query: query.into(),
      search_title: true,
      search_content: true,
      case_sensitive: false,
      limit: Some(50),
      sort_by: SortOrder::default(),
      include_trashed: false,
      include_archived: false,
    }
  }

  /// Search only in note titles (excludes content)
  pub fn title_only(mut self) -> Self {
    self.search_title = true;
    self.search_content = false;
    self
  }

  /// Search only in note content (excludes titles)
  pub fn content_only(mut self) -> Self {
    self.search_title = false;
    self.search_content = true;
    self
  }

  /// Search in both title and content (default)
  pub fn title_and_content(mut self) -> Self {
    self.search_title = true;
    self.search_content = true;
    self
  }

  /// Enable case-sensitive search (default is case-insensitive)
  pub fn case_sensitive(mut self) -> Self {
    self.case_sensitive = true;
    self
  }

  /// Set the maximum number of results to return
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

  /// Set the sort order for results
  pub fn sort_by(
    mut self,
    sort: SortOrder,
  ) -> Self {
    self.sort_by = sort;
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
  pub fn tags(&self) -> Result<TagsMap, BearError> {
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

      let results: rusqlite::Result<Vec<Tag>> = statement.query_map([], tag_from_row)?.collect();

      let tags = results?.into_iter().map(|tag| (tag.id(), tag)).collect();

      Ok(TagsMap { tags })
    })
  }

  /// Retrieves a specific note by its ID.
  ///
  /// Returns `None` if no note with the given ID exists.
  ///
  /// # Examples
  ///
  /// ```no_run
  /// # use bear_query::{BearDb, NoteId};
  /// # fn main() -> Result<(), bear_query::BearError> {
  /// let db = BearDb::new()?;
  ///
  /// // Look up a note by its UUID
  /// let note_id = NoteId::new("ABC123-DEF456-...".to_string());
  /// if let Some(note) = db.get_note_by_id(&note_id)? {
  ///     println!("Found note: {}", note.title());
  /// } else {
  ///     println!("Note not found");
  /// }
  /// # Ok(())
  /// # }
  /// ```
  pub fn get_note_by_id(
    &self,
    id: &NoteId,
  ) -> Result<Option<Note>, BearError> {
    self.with_connection(|queryable| {
      let mut statement = queryable.prepare(
        r"
      SELECT
        id,
        core_db_id,
        title,
        content,
        modified,
        created,
        is_pinned
      FROM notes
      WHERE id = ?",
      )?;

      let result = statement.query_row([id.as_str()], note_from_row);

      match result {
        Ok(note) => Ok(Some(note)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(BearError::SqlError { source: e }),
      }
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
  ) -> Result<Vec<Note>, BearError> {
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
        core_db_id,
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

      let results: rusqlite::Result<Vec<Note>> = statement.query_map([], note_from_row)?.collect();

      Ok(results?)
    })
  }

  /// Searches notes by title and/or content.
  ///
  /// Use `SearchQuery` to configure search options including which fields to search,
  /// sort order, limits, and inclusion of trashed/archived notes.
  ///
  /// # Examples
  /// ```no_run
  /// # use bear_query::{BearDb, SearchQuery, SortOn};
  /// # fn main() -> Result<(), bear_query::BearError> {
  /// let db = BearDb::new()?;
  ///
  /// // Simple search in both title and content
  /// let notes = db.search(SearchQuery::new("rust"))?;
  ///
  /// // Search only in titles, sorted alphabetically
  /// let notes = db.search(
  ///     SearchQuery::new("project")
  ///         .title_only()
  ///         .sort_by(SortOn::Title.asc())
  /// )?;
  ///
  /// // Case-sensitive search in content with custom limit
  /// let notes = db.search(
  ///     SearchQuery::new("TODO")
  ///         .content_only()
  ///         .case_sensitive()
  ///         .limit(100)
  /// )?;
  /// # Ok(())
  /// # }
  /// ```
  pub fn search(
    &self,
    search: SearchQuery,
  ) -> Result<Vec<Note>, BearError> {
    self.with_connection(|queryable| {
      // Build search conditions
      let mut search_conditions = Vec::new();

      let like_operator = if search.case_sensitive {
        "GLOB"
      } else {
        "LIKE"
      };
      let pattern = if search.case_sensitive {
        format!("*{}*", search.query)
      } else {
        format!("%{}%", search.query)
      };

      if search.search_title {
        search_conditions.push(format!("title {} ?", like_operator));
      }
      if search.search_content {
        search_conditions.push(format!("content {} ?", like_operator));
      }

      // If neither title nor content is selected, search nothing (return empty)
      if search_conditions.is_empty() {
        return Ok(Vec::new());
      }

      let search_clause = format!("({})", search_conditions.join(" OR "));

      // Build WHERE clause with filters
      let mut where_clauses = vec![search_clause];

      if !search.include_trashed {
        where_clauses.push("is_trashed <> 1".to_string());
      }
      if !search.include_archived {
        where_clauses.push("is_archived <> 1".to_string());
      }

      let where_clause = format!("WHERE {}", where_clauses.join(" AND "));

      let limit_clause = search
        .limit
        .map(|l| format!("LIMIT {}", l))
        .unwrap_or_default();

      let query_sql = format!(
        r"
      SELECT
        id,
        core_db_id,
        title,
        content,
        modified,
        created,
        is_pinned
      FROM notes
      {}
      ORDER BY {}
      {}",
        where_clause,
        search.sort_by.to_sql(),
        limit_clause
      );

      let mut statement = queryable.prepare(&query_sql)?;

      // Bind the pattern for each search condition
      let results: rusqlite::Result<Vec<Note>> = if search.search_title && search.search_content {
        // Both title and content: bind pattern twice
        statement
          .query_map([pattern.as_str(), pattern.as_str()], note_from_row)?
          .collect()
      } else {
        // Only one field: bind pattern once
        statement
          .query_map([pattern.as_str()], note_from_row)?
          .collect()
      };

      Ok(results?)
    })
  }

  /// Retrieves all notes linked from the specified note
  pub fn note_links(
    &self,
    from: &NoteId,
  ) -> Result<Vec<Note>, BearError> {
    self.with_connection(|queryable| {
      let mut statement = queryable.prepare(
        r"
      SELECT
        n.id,
        n.core_db_id,
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

      let results: rusqlite::Result<Vec<Note>> = statement
        .query_map([from.as_str()], note_from_row)?
        .collect();

      Ok(results?)
    })
  }

  /// Retrieves all tag IDs associated with the specified note
  pub fn note_tags(
    &self,
    from: &NoteId,
  ) -> Result<HashSet<TagId>, BearError> {
    self.with_connection(|queryable| {
      let mut statement = queryable.prepare(
        r"
      SELECT
        nt.tag_id
      FROM note_tags nt
      WHERE nt.note_id = ?",
      )?;

      let results: rusqlite::Result<HashSet<TagId>> = statement
        .query_map([from.as_str()], |row| row.get("tag_id"))?
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
    assert_eq!(tags.count(), 3); // Should have 3 tags from test data (including unmodified-tag)

    let notes = db.notes(NotesQuery::default()).unwrap();
    assert_eq!(notes.len(), 4); // default() excludes trashed, so 4 notes (not 5 - one is trashed)

    // Test filtering - include all notes
    let all_notes = db
      .notes(NotesQuery::new().include_all().no_limit())
      .unwrap();
    assert_eq!(all_notes.len(), 5); // 5 notes total including trashed

    // Test the generic SQL query API
    let df = db
      .query("SELECT id, title FROM notes WHERE is_trashed = 0")
      .unwrap();
    assert_eq!(df.height(), 4); // 4 non-trashed notes
    assert_eq!(df.width(), 2); // 2 columns (id, title)

    // Test aggregation
    let df = db.query("SELECT COUNT(*) as count FROM notes").unwrap();
    assert_eq!(df.height(), 1);
    assert_eq!(df.width(), 1);

    // Verify the count column is an integer (not string)
    let series = df.column("count").unwrap();
    let value = series.get(0).unwrap();
    match value {
      AnyValue::Int64(n) => assert_eq!(n, 5),
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

  /// Test handling of notes with empty title (not NULL, but empty string)
  #[test]
  fn test_note_with_empty_title() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Get all notes including the one with empty title (id=4)
    let notes = db
      .notes(NotesQuery::new().no_limit().include_all())
      .unwrap();

    // Find the note with empty title
    let note_with_empty_title = notes
      .iter()
      .find(|n| n.title().is_empty())
      .expect("Should have a note with empty title");

    // Verify the note exists and has empty title
    assert_eq!(note_with_empty_title.title(), "");

    // Verify other fields are still accessible
    assert!(note_with_empty_title.content().is_some());
    assert_eq!(
      note_with_empty_title.content().unwrap(),
      "Content with empty title"
    );
  }

  /// Test handling of notes with NULL content
  #[test]
  fn test_note_with_null_content() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    let notes = db
      .notes(NotesQuery::new().no_limit().include_all())
      .unwrap();

    // Find the note with NULL content (id=5)
    let note_with_null_content = notes
      .iter()
      .find(|n| n.content().is_none())
      .expect("Should have a note with NULL content");

    // Verify the note has a title but no content
    assert_eq!(note_with_null_content.title(), "Empty Note");
    assert!(note_with_null_content.content().is_none());
  }

  /// Test that all notes have unique_id
  #[test]
  fn test_all_notes_have_unique_id() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    let notes = db
      .notes(NotesQuery::new().no_limit().include_all())
      .unwrap();

    // All notes should have unique_id (never NULL)
    for note in notes {
      let uuid = note.id();
      assert!(!uuid.as_str().is_empty(), "unique_id should never be empty");
    }
  }

  /// Test handling of tags with NULL modified date
  #[test]
  fn test_tag_with_null_modified() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    let tags = db.tags().unwrap();

    // Find tag with NULL modified date (id=3, name="unmodified-tag")
    let unmodified_tag = tags
      .iter()
      .find(|t| t.modified().is_none())
      .expect("Should have a tag with NULL modified date");

    // Verify the tag has a name but no modified date
    assert_eq!(unmodified_tag.name(), Some("unmodified-tag"));
    assert!(unmodified_tag.modified().is_none());
  }

  /// Test that tag count includes tags with NULL modified dates
  #[test]
  fn test_tags_count_includes_null_modified() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    let tags = db.tags().unwrap();

    // Should have 3 tags total (work, personal, unmodified-tag)
    assert_eq!(tags.count(), 3);
  }

  /// Test querying for notes with empty title using generic query API
  #[test]
  fn test_query_with_empty_title() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Query for notes with empty title using generic query API
    let df = db
      .query("SELECT id, title, content FROM notes WHERE title = ''")
      .unwrap();

    assert_eq!(df.height(), 1); // Should find 1 note with empty title
  }

  /// Test querying for notes with NULL content
  #[test]
  fn test_query_with_null_content() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Query for notes with NULL content
    let df = db
      .query("SELECT id, title, content FROM notes WHERE content IS NULL")
      .unwrap();

    assert_eq!(df.height(), 1); // Should find 1 note with NULL content
  }

  /// Test that Tags::names handles NULL tag names gracefully
  #[test]
  fn test_note_tags_names_handles_null() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    let tags = db.tags().unwrap();

    // Get all tag IDs
    let all_tag_ids: HashSet<_> = tags.iter().map(|t| t.id()).collect();

    // Get names - should handle tags with NULL names gracefully
    let names = tags.names(&all_tag_ids);

    // Should have 3 valid names (all our test tags have names)
    assert_eq!(names.len(), 3);
    assert!(names.contains("work"));
    assert!(names.contains("personal"));
    assert!(names.contains("unmodified-tag"));
  }

  /// Test that all notes have valid IDs and required fields are always present
  #[test]
  fn test_all_notes_have_valid_id() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    let notes = db
      .notes(NotesQuery::new().no_limit().include_all())
      .unwrap();

    // Every note should have a valid id (primary key is never NULL)
    for note in notes {
      let _id = note.id(); // This should never panic

      // Timestamps should always be present
      let _created = note.created();
      let _modified = note.modified();

      // Boolean should always be present
      let _is_pinned = note.is_pinned();
    }
  }

  /// Test that all tags have valid IDs
  #[test]
  fn test_all_tags_have_valid_id() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    let tags = db.tags().unwrap();

    // Every tag should have a valid id (primary key is never NULL)
    for tag in tags.iter() {
      let _id = tag.id(); // This should never panic
    }
  }

  /// Test that note_links handles notes with NULL fields gracefully
  #[test]
  fn test_note_links_with_null_safe_notes() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Get the first note's ID
    let notes = db.notes(NotesQuery::new().limit(1)).unwrap();
    let first_note = &notes[0];

    // Query note links - should handle notes with NULL fields gracefully
    let linked_notes = db.note_links(first_note.id()).unwrap();

    // All linked notes should be queryable even if they have NULL fields
    for linked_note in linked_notes {
      let _id = linked_note.id();
      let _title = linked_note.title();
      let _content = linked_note.content(); // May be None, but shouldn't error
    }
  }

  /// Test get_note_by_id with existing note
  #[test]
  fn test_get_note_by_id_existing() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Get a note using the notes() API
    let notes = db.notes(NotesQuery::new().limit(1)).unwrap();
    let expected_note = &notes[0];
    let note_id = expected_note.id();

    // Look it up by ID
    let found_note = db.get_note_by_id(note_id).unwrap();

    assert!(found_note.is_some());
    let found_note = found_note.unwrap();

    // Verify it's the same note
    assert_eq!(found_note.id(), expected_note.id());
    assert_eq!(found_note.title(), expected_note.title());
    assert_eq!(found_note.content(), expected_note.content());
  }

  /// Test get_note_by_id with non-existent note
  #[test]
  fn test_get_note_by_id_not_found() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Try to get a note with an ID that doesn't exist
    let note_id = NoteId::new("nonexistent-uuid".to_string());
    let result = db.get_note_by_id(&note_id).unwrap();

    assert!(result.is_none());
  }

  /// Test get_note_by_id with note that has NULL content
  #[test]
  fn test_get_note_by_id_with_null_content() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Find a note with NULL content
    let notes = db
      .notes(NotesQuery::new().no_limit().include_all())
      .unwrap();
    let null_content_note = notes.iter().find(|n| n.content().is_none()).unwrap();
    let note_id = null_content_note.id();

    // Look it up by ID
    let found_note = db.get_note_by_id(note_id).unwrap();

    assert!(found_note.is_some());
    let found_note = found_note.unwrap();

    // Verify it has a title but no content
    assert_eq!(found_note.title(), "Empty Note");
    assert!(found_note.content().is_none());
  }

  /// Test basic search in both title and content
  #[test]
  fn test_search_basic() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Search for "first" which appears in title
    let results = db.search(SearchQuery::new("first")).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title(), "First Note");

    // Search for "second" which appears in title and content
    let results = db.search(SearchQuery::new("second")).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title(), "Second Note");

    // Search for "Content" which appears in multiple notes
    let results = db.search(SearchQuery::new("Content")).unwrap();
    assert!(results.len() >= 2);
  }

  /// Test search with title_only filter
  #[test]
  fn test_search_title_only() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Search for "Note" in titles only
    let results = db.search(SearchQuery::new("Note").title_only()).unwrap();

    // Should find "First Note", "Second Note", and "Empty Note" (not "Trashed Note" as it's excluded by default)
    assert_eq!(results.len(), 3);

    for note in &results {
      assert!(note.title().contains("Note"));
    }
  }

  /// Test search with content_only filter
  #[test]
  fn test_search_content_only() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Search for "Content" in content only
    let results = db
      .search(SearchQuery::new("Content").content_only())
      .unwrap();

    // Should find notes with "Content" in their content field
    assert!(results.len() >= 2);

    for note in &results {
      if let Some(content) = note.content() {
        assert!(content.contains("Content"));
      }
    }
  }

  /// Test search with case sensitivity
  #[test]
  fn test_search_case_sensitive() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Case-insensitive search (default) for "FIRST"
    let results = db.search(SearchQuery::new("FIRST")).unwrap();
    assert_eq!(results.len(), 1);

    // Case-sensitive search for "FIRST" should find nothing
    let results = db
      .search(SearchQuery::new("FIRST").case_sensitive())
      .unwrap();
    assert_eq!(results.len(), 0);

    // Case-sensitive search for "First" should find the note
    let results = db
      .search(SearchQuery::new("First").case_sensitive())
      .unwrap();
    assert_eq!(results.len(), 1);
  }

  /// Test search with limits
  #[test]
  fn test_search_with_limit() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Search for common term with limit
    let results = db.search(SearchQuery::new("note").limit(2)).unwrap();
    assert!(results.len() <= 2);

    // Search with no limit
    let results = db.search(SearchQuery::new("note").no_limit()).unwrap();
    assert!(results.len() >= 2);
  }

  /// Test search with sorting
  #[test]
  fn test_search_with_sorting() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Search sorted by title ascending
    let results = db
      .search(
        SearchQuery::new("Note")
          .title_only()
          .sort_by(SortOn::Title.asc()),
      )
      .unwrap();

    assert!(results.len() >= 2);

    // Verify alphabetical order
    for i in 0..results.len() - 1 {
      assert!(results[i].title() <= results[i + 1].title());
    }

    // Search sorted by title descending
    let results = db
      .search(
        SearchQuery::new("Note")
          .title_only()
          .sort_by(SortOn::Title.desc()),
      )
      .unwrap();

    assert!(results.len() >= 2);

    // Verify reverse alphabetical order
    for i in 0..results.len() - 1 {
      assert!(results[i].title() >= results[i + 1].title());
    }
  }

  /// Test search excluding trashed notes (default behavior)
  #[test]
  fn test_search_excludes_trashed_by_default() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Default search should exclude trashed
    let results = db.search(SearchQuery::new("Trashed")).unwrap();
    assert_eq!(results.len(), 0);

    // Including trashed should find it
    let results = db
      .search(SearchQuery::new("Trashed").include_trashed())
      .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title(), "Trashed Note");
  }

  /// Test search with include_all
  #[test]
  fn test_search_include_all() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Search with all notes included
    let results = db.search(SearchQuery::new("Note").include_all()).unwrap();

    // Should include trashed notes too
    let has_trashed = results.iter().any(|n| n.title() == "Trashed Note");
    assert!(has_trashed);
  }

  /// Test search with empty results
  #[test]
  fn test_search_no_results() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    let results = db.search(SearchQuery::new("nonexistent_term_xyz")).unwrap();
    assert_eq!(results.len(), 0);
  }

  /// Test search handles notes with NULL content gracefully
  #[test]
  fn test_search_with_null_content() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Search for "Empty Note" in title only to find the note with NULL content
    let results = db
      .search(SearchQuery::new("Empty Note").title_only())
      .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title(), "Empty Note");
    assert!(results[0].content().is_none());
  }

  /// Test search handles notes with empty title gracefully
  #[test]
  fn test_search_with_empty_title() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Search for content in a note with empty title
    let results = db
      .search(SearchQuery::new("empty title").content_only())
      .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title(), "");
  }

  /// Test complex search query with multiple filters
  #[test]
  fn test_search_complex_query() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Complex search: title only, sorted alphabetically, limited, include all
    let results = db
      .search(
        SearchQuery::new("Note")
          .title_only()
          .sort_by(SortOn::Title.asc())
          .limit(2)
          .include_all(),
      )
      .unwrap();

    assert!(results.len() <= 2);

    // Verify results are sorted
    if results.len() == 2 {
      assert!(results[0].title() <= results[1].title());
    }
  }

  /// Test that SearchQuery builder methods are chainable
  #[test]
  fn test_search_query_builder_chaining() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Build a complex query with method chaining
    let query = SearchQuery::new("note")
      .title_only()
      .case_sensitive()
      .limit(10)
      .sort_by(SortOn::Modified.desc())
      .include_archived();

    // Just verify it compiles and runs
    let _results = db.search(query).unwrap();
  }

  /// Test search with different SortOrder variants
  #[test]
  fn test_search_all_sort_orders() {
    let db = BearDb::new_with_path(DatabasePath::InMemory).unwrap();

    // Test all sort order variants compile and run
    let orders = vec![
      SortOn::Modified.desc(),
      SortOn::Modified.asc(),
      SortOn::Created.desc(),
      SortOn::Created.asc(),
      SortOn::Title.asc(),
      SortOn::Title.desc(),
    ];

    for order in orders {
      let _results = db.search(SearchQuery::new("Note").sort_by(order)).unwrap();
    }
  }
}
