//! Data models for Bear database entities.
//!
//! This module contains all the types representing Bear's database entities:
//! notes, tags, and their identifiers.

use rusqlite::types::{FromSql, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Row, ToSql};
use std::collections::{HashMap, HashSet};
use time::OffsetDateTime;

/// Internal database ID wrapper.
///
/// This wraps SQLite's INTEGER PRIMARY KEY values.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct DbId(pub(crate) i64);

/// Unique identifier for a Bear note.
///
/// This wraps the note's SQLite primary key (`Z_PK` in Bear's schema).
///
/// # Creating IDs
///
/// You can create a `BearNoteId` from an `i64` value:
///
/// ```
/// use bear_query::BearNoteId;
///
/// let note_id = BearNoteId::new(42);
/// ```
///
/// # Usage
///
/// Use this ID for:
/// - Looking up specific notes with `db.get_note_by_id()`
/// - Querying related data (tags, links)
/// - Storing references to notes
///
/// The ID is stable across the note's lifetime and is the recommended
/// way to reference notes programmatically.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct BearNoteId(pub(crate) DbId);

impl BearNoteId {
  /// Creates a new `BearNoteId` from an `i64` primary key value.
  ///
  /// # Example
  ///
  /// ```
  /// use bear_query::BearNoteId;
  ///
  /// let note_id = BearNoteId::new(42);
  /// ```
  pub fn new(id: i64) -> Self {
    BearNoteId(DbId(id))
  }

  /// Returns the underlying `i64` value of this ID.
  ///
  /// # Example
  ///
  /// ```
  /// use bear_query::BearNoteId;
  ///
  /// let note_id = BearNoteId::new(42);
  /// assert_eq!(note_id.as_i64(), 42);
  /// ```
  pub fn as_i64(self) -> i64 {
    self.0.0
  }
}

/// Unique identifier for a Bear tag.
///
/// This wraps the tag's SQLite primary key.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct BearTagId(pub(crate) DbId);

impl BearTagId {
  /// Creates a new `BearTagId` from an `i64` primary key value.
  pub fn new(id: i64) -> Self {
    BearTagId(DbId(id))
  }

  /// Returns the underlying `i64` value of this ID.
  pub fn as_i64(self) -> i64 {
    self.0.0
  }
}

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

/// A tag from Bear's database.
///
/// Tags in Bear organize notes hierarchically (e.g., "work/projects/bear-query").
///
/// # Nullable Fields
///
/// - **`name`**: Tag name. In theory should never be NULL, but we handle it defensively.
///   A tag without a name would be unusual but possible in a corrupted database.
/// - **`modified`**: Timestamp of last modification. May be `None` for tags that have
///   never been explicitly modified.
///
/// # Example
///
/// ```no_run
/// # use bear_query::BearDb;
/// # fn main() -> Result<(), bear_query::BearError> {
/// let db = BearDb::new()?;
/// let tags = db.tags()?;
///
/// for tag in tags.iter() {
///     let name = tag.name().unwrap_or("[unnamed]");
///     match tag.modified() {
///         Some(modified) => println!("Tag '{}' modified: {}", name, modified),
///         None => println!("Tag '{}' (never modified)", name),
///     }
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct BearTag {
  id: BearTagId,
  name: Option<String>,
  modified: Option<OffsetDateTime>,
}

impl BearTag {
  /// Returns the tag's primary key identifier.
  ///
  /// This is always present (never NULL) and stable across the tag's lifetime.
  pub fn id(&self) -> BearTagId {
    self.id
  }

  /// Returns the tag's name, if present.
  ///
  /// Returns `None` in the rare case of a tag without a name (corrupted data).
  ///
  /// Tag names can be hierarchical, using `/` as separator:
  /// - `"work"` - top-level tag
  /// - `"work/projects"` - nested tag
  /// - `"work/projects/bear-query"` - deeply nested tag
  pub fn name(&self) -> Option<&str> {
    self.name.as_deref()
  }

  /// Returns the timestamp of the tag's last modification, if available.
  ///
  /// Returns `None` for tags that have never been explicitly modified.
  pub fn modified(&self) -> Option<OffsetDateTime> {
    self.modified
  }
}

/// Helper to construct BearTag from a database row
pub(crate) fn tag_from_row(row: &Row) -> rusqlite::Result<BearTag> {
  Ok(BearTag {
    id: row.get("id")?,
    name: row.get("name")?,
    modified: row.get("modified")?,
  })
}

/// Collection of tags from Bear's database.
#[derive(Debug)]
pub struct BearTags {
  pub(crate) tags: HashMap<BearTagId, BearTag>,
}

impl BearTags {
  /// Gets a tag by its ID.
  pub fn get(
    &self,
    tag_id: &BearTagId,
  ) -> Option<&BearTag> {
    self.tags.get(tag_id)
  }

  /// Returns the number of tags.
  pub fn count(&self) -> usize {
    self.tags.len()
  }

  /// Returns an iterator over all tags.
  pub fn iter(&self) -> impl Iterator<Item = &BearTag> {
    self.tags.values()
  }

  /// Returns the names of the tags with the given IDs.
  ///
  /// Tags with NULL names are omitted from the result.
  pub fn names(
    &self,
    tag_ids: &HashSet<BearTagId>,
  ) -> HashSet<String> {
    tag_ids
      .iter()
      .filter_map(|id| self.get(id).and_then(|t| t.name.clone()))
      .collect()
  }
}

/// A note from Bear's database.
///
/// # Nullable Fields
///
/// Only one field in Bear notes can be NULL in the database:
///
/// - **`content`**: Empty notes may have `None` for content.
///
/// # Always-Present Fields
///
/// The following fields are **always present** (never NULL):
///
/// - **`title`**: All notes have titles (may be empty string, but never NULL)
/// - **`unique_id`**: Bear's UUID identifier (always present)
/// - **`id`**: Primary key (always present)
/// - **`modified`**, **`created`**: Timestamps (always present)
/// - **`is_pinned`**: Boolean flag (always present)
///
/// # Identifiers
///
/// Bear notes have two types of identifiers:
///
/// ## Primary Key (`id()`)
/// - Type: `BearNoteId` (wraps SQLite's integer primary key)
/// - **Always present** (never NULL)
/// - Stable across the lifetime of the note
/// - Use this for all programmatic references, joins, and lookups
/// - Maps to Bear's internal `Z_PK` column
///
/// ## UUID (`unique_id()`)
/// - Type: `&str` (Bear's UUID like 'ABC123-DEF456-...')
/// - **Always present** (never NULL)
/// - Bear uses this internally for syncing and x-callback-url schemes
/// - Used in Bear's x-callback-url API (e.g., `bear://x-callback-url/open-note?id=UUID`)
///
/// # Example
///
/// ```no_run
/// # use bear_query::{BearDb, NotesQuery};
/// # fn main() -> Result<(), bear_query::BearError> {
/// let db = BearDb::new()?;
/// let notes = db.notes(NotesQuery::default())?;
///
/// for note in notes {
///     let note_id = note.id();
///     let title = note.title();
///     let uuid = note.unique_id();
///
///     // Only content may be None
///     let content = note.content().unwrap_or("");
///
///     println!("Note {}: {} ({} bytes)", note_id.as_i64(), title, content.len());
///     println!("  UUID: {}", uuid);
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct BearNote {
  id: BearNoteId,
  unique_id: String,
  title: String,
  content: Option<String>,
  modified: OffsetDateTime,
  created: OffsetDateTime,
  is_pinned: bool,
}

impl BearNote {
  /// Returns the note's primary key identifier.
  ///
  /// This is the **recommended identifier** for all programmatic use:
  /// - Always present (never NULL)
  /// - Stable across the note's lifetime
  /// - Efficient for database queries and joins
  ///
  /// Use this for:
  /// - Looking up notes by ID
  /// - Querying related data (tags, links)
  /// - Storing references to notes
  pub fn id(&self) -> BearNoteId {
    self.id
  }

  /// Returns the note's Bear UUID identifier.
  ///
  /// This UUID is used by Bear for:
  /// - Syncing notes across devices
  /// - x-callback-url API (e.g., `bear://x-callback-url/open-note?id=UUID`)
  ///
  /// For programmatic queries and joins, prefer using `id()` (the primary key).
  ///
  /// # Example
  ///
  /// ```no_run
  /// # use bear_query::{BearDb, NotesQuery};
  /// # fn main() -> Result<(), bear_query::BearError> {
  /// # let db = BearDb::new()?;
  /// # let notes = db.notes(NotesQuery::default())?;
  /// # let note = &notes[0];
  /// let uuid = note.unique_id();
  /// println!("Open in Bear: bear://x-callback-url/open-note?id={}", uuid);
  /// # Ok(())
  /// # }
  /// ```
  pub fn unique_id(&self) -> &str {
    &self.unique_id
  }

  /// Returns the note's title.
  ///
  /// All notes have titles (this is never NULL), though the title may be an empty string.
  ///
  /// # Example
  ///
  /// ```no_run
  /// # use bear_query::{BearDb, NotesQuery};
  /// # fn main() -> Result<(), bear_query::BearError> {
  /// # let db = BearDb::new()?;
  /// # let notes = db.notes(NotesQuery::default())?;
  /// # let note = &notes[0];
  /// let title = note.title();
  /// if title.is_empty() {
  ///     println!("[Untitled]");
  /// } else {
  ///     println!("Title: {}", title);
  /// }
  /// # Ok(())
  /// # }
  /// ```
  pub fn title(&self) -> &str {
    &self.title
  }

  /// Returns the note's content (Markdown), if present.
  ///
  /// Returns `None` for empty notes or notes with NULL content.
  ///
  /// # Example
  ///
  /// ```no_run
  /// # use bear_query::{BearDb, NotesQuery};
  /// # fn main() -> Result<(), bear_query::BearError> {
  /// # let db = BearDb::new()?;
  /// # let notes = db.notes(NotesQuery::default())?;
  /// # let note = &notes[0];
  /// let content = note.content().unwrap_or("");
  /// println!("Content length: {} bytes", content.len());
  /// # Ok(())
  /// # }
  /// ```
  pub fn content(&self) -> Option<&str> {
    self.content.as_deref()
  }

  /// Returns the timestamp of the note's last modification.
  ///
  /// This is always present (never NULL).
  pub fn modified(&self) -> OffsetDateTime {
    self.modified
  }

  /// Returns the timestamp when the note was created.
  ///
  /// This is always present (never NULL).
  pub fn created(&self) -> OffsetDateTime {
    self.created
  }

  /// Returns whether the note is pinned.
  ///
  /// This is always present (never NULL).
  pub fn is_pinned(&self) -> bool {
    self.is_pinned
  }
}

/// Helper to construct BearNote from a database row
pub(crate) fn note_from_row(row: &Row) -> rusqlite::Result<BearNote> {
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

  /// Test BearNoteId::new and as_i64
  #[test]
  fn test_bear_note_id_construction() {
    let note_id = BearNoteId::new(42);
    assert_eq!(note_id.as_i64(), 42);

    // Test round-trip
    let id_value = 12345;
    let note_id = BearNoteId::new(id_value);
    assert_eq!(note_id.as_i64(), id_value);
  }

  /// Test BearTagId::new and as_i64
  #[test]
  fn test_bear_tag_id_construction() {
    let tag_id = BearTagId::new(42);
    assert_eq!(tag_id.as_i64(), 42);

    // Test round-trip
    let id_value = 12345;
    let tag_id = BearTagId::new(id_value);
    assert_eq!(tag_id.as_i64(), id_value);
  }

  /// Test BearNoteId equality and hashing
  #[test]
  fn test_bear_note_id_equality() {
    let id1 = BearNoteId::new(42);
    let id2 = BearNoteId::new(42);
    let id3 = BearNoteId::new(43);

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);

    // Test that they can be used in HashSet
    let mut set = std::collections::HashSet::new();
    set.insert(id1);
    assert!(set.contains(&id2));
    assert!(!set.contains(&id3));
  }

  /// Test BearTagId equality and hashing
  #[test]
  fn test_bear_tag_id_equality() {
    let id1 = BearTagId::new(42);
    let id2 = BearTagId::new(42);
    let id3 = BearTagId::new(43);

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);

    // Test that they can be used in HashSet
    let mut set = std::collections::HashSet::new();
    set.insert(id1);
    assert!(set.contains(&id2));
    assert!(!set.contains(&id3));
  }
}
