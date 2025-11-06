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
pub(crate) struct CoreDbId(pub(crate) i64);

/// Internal Core Data note identifier.
///
/// This wraps the note's SQLite primary key (`Z_PK` in Bear's schema).
/// This ID is internal to the database and should not be exposed in public APIs.
/// Use `NoteId` for the public API instead.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub(crate) struct CoreDbNoteId(pub(crate) CoreDbId);

impl CoreDbNoteId {
  // Internal construction is done via FromSql trait when reading from database
}

/// Bear note identifier (UUID-based).
///
/// This wraps Bear's UUID identifier for notes. This is the identifier that Bear
/// uses in its UI, x-callback-url API, and for syncing notes across devices.
///
/// # Creating IDs
///
/// You can create a `NoteId` from a UUID string:
///
/// ```
/// use bear_query::NoteId;
///
/// let note_id = NoteId::new("ABC123-DEF456-...".to_string());
/// ```
///
/// # Usage
///
/// Use this ID for:
/// - Opening notes in Bear via x-callback-url: `bear://x-callback-url/open-note?id={uuid}`
/// - Storing stable references to notes that work across devices
/// - Matching notes in sync operations
///
/// This is Bear's primary identifier for notes.
#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct NoteId(String);

impl NoteId {
  /// Creates a new `NoteId` from a UUID string.
  ///
  /// # Example
  ///
  /// ```
  /// use bear_query::NoteId;
  ///
  /// let note_id = NoteId::new("ABC123-DEF456-...".to_string());
  /// ```
  pub fn new(uuid: String) -> Self {
    NoteId(uuid)
  }

  /// Returns the UUID as a string slice.
  ///
  /// # Example
  ///
  /// ```
  /// use bear_query::NoteId;
  ///
  /// let note_id = NoteId::new("ABC123-DEF456-...".to_string());
  /// assert_eq!(note_id.as_str(), "ABC123-DEF456-...");
  /// ```
  pub fn as_str(&self) -> &str {
    &self.0
  }

  /// Consumes the NoteId and returns the inner UUID string.
  pub fn into_string(self) -> String {
    self.0
  }
}

/// Unique identifier for a Bear tag.
///
/// This wraps the tag's SQLite primary key.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct TagId(pub(crate) CoreDbId);

impl TagId {
  /// Creates a new `TagId` from an `i64` primary key value.
  pub fn new(id: i64) -> Self {
    TagId(CoreDbId(id))
  }

  /// Returns the underlying `i64` value of this ID.
  pub fn as_i64(self) -> i64 {
    self.0.0
  }
}

impl FromSql for CoreDbId {
  fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
    Ok(Self(value.as_i64()?))
  }
}

impl FromSql for CoreDbNoteId {
  fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
    Ok(Self(FromSql::column_result(value)?))
  }
}

impl FromSql for TagId {
  fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
    Ok(Self(FromSql::column_result(value)?))
  }
}

impl ToSql for CoreDbId {
  fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
    self.0.to_sql()
  }
}

impl ToSql for CoreDbNoteId {
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
pub struct Tag {
  id: TagId,
  name: Option<String>,
  modified: Option<OffsetDateTime>,
}

impl Tag {
  /// Returns the tag's primary key identifier.
  ///
  /// This is always present (never NULL) and stable across the tag's lifetime.
  pub fn id(&self) -> TagId {
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

/// Helper to construct Tag from a database row
pub(crate) fn tag_from_row(row: &Row) -> rusqlite::Result<Tag> {
  Ok(Tag {
    id: row.get("id")?,
    name: row.get("name")?,
    modified: row.get("modified")?,
  })
}

/// Collection of tags from Bear's database.
///
/// This is a map of tag IDs to tags, returned by `BearDb::tags()`.
/// It provides convenient methods for looking up tags and converting
/// tag ID sets into tag names.
///
/// # Example
///
/// ```no_run
/// # use bear_query::BearDb;
/// # fn main() -> Result<(), bear_query::BearError> {
/// let db = BearDb::new()?;
/// let tags = db.tags()?;
///
/// println!("Total tags: {}", tags.count());
///
/// for tag in tags.iter() {
///     if let Some(name) = tag.name() {
///         println!("Tag: {}", name);
///     }
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct TagsMap {
  pub(crate) tags: HashMap<TagId, Tag>,
}

impl TagsMap {
  /// Gets a tag by its ID.
  pub fn get(
    &self,
    tag_id: &TagId,
  ) -> Option<&Tag> {
    self.tags.get(tag_id)
  }

  /// Returns the number of tags.
  pub fn count(&self) -> usize {
    self.tags.len()
  }

  /// Returns an iterator over all tags.
  pub fn iter(&self) -> impl Iterator<Item = &Tag> {
    self.tags.values()
  }

  /// Returns the names of the tags with the given IDs.
  ///
  /// Tags with NULL names are omitted from the result.
  pub fn names(
    &self,
    tag_ids: &HashSet<TagId>,
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
/// Bear notes use UUIDs as their primary identifier:
///
/// ## UUID (`id()`)
/// - Type: `&NoteId` (Bear's UUID like 'ABC123-DEF456-...')
/// - **Always present** (never NULL)
/// - Stable across the lifetime of the note and across devices
/// - Use this for all programmatic references and API calls
/// - Bear uses this for syncing and x-callback-url schemes
/// - Used in Bear's x-callback-url API (e.g., `bear://x-callback-url/open-note?id=UUID`)
///
/// The internal Core Data primary key is not exposed in the public API.
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
///
///     // Only content may be None
///     let content = note.content().unwrap_or("");
///
///     println!("Note {}: {} ({} bytes)", note_id.as_str(), title, content.len());
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Note {
  _core_db_id: CoreDbNoteId,
  id: NoteId,
  title: String,
  content: Option<String>,
  modified: OffsetDateTime,
  created: OffsetDateTime,
  is_pinned: bool,
}

impl Note {
  /// Returns the note's internal Core Data ID (for internal use only).
  ///
  /// This is used internally for database queries and joins.
  /// External users should use `id()` which returns Bear's UUID.
  pub(crate) fn _core_db_id(&self) -> CoreDbNoteId {
    self._core_db_id
  }

  /// Returns the note's Bear identifier.
  ///
  /// This is Bear's UUID identifier, which is:
  /// - Always present (never NULL)
  /// - Stable across the note's lifetime
  /// - Works across devices (syncing)
  /// - Used in Bear's x-callback-url API
  ///
  /// Use this for:
  /// - Opening notes in Bear
  /// - Storing references to notes
  /// - Matching notes across devices
  ///
  /// # Example
  ///
  /// ```no_run
  /// # use bear_query::{BearDb, NotesQuery};
  /// # fn main() -> Result<(), bear_query::BearError> {
  /// # let db = BearDb::new()?;
  /// # let notes = db.notes(NotesQuery::default())?;
  /// # let note = &notes[0];
  /// let note_id = note.id();
  /// println!("Open in Bear: bear://x-callback-url/open-note?id={}", note_id.as_str());
  /// # Ok(())
  /// # }
  /// ```
  pub fn id(&self) -> &NoteId {
    &self.id
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

/// Helper to construct Note from a database row
pub(crate) fn note_from_row(row: &Row) -> rusqlite::Result<Note> {
  Ok(Note {
    _core_db_id: row.get("id")?,
    id: NoteId::new(row.get("unique_id")?),
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

  /// Test NoteId::new and as_str
  #[test]
  fn test_note_id_construction() {
    let uuid = "ABC123-DEF456-GHI789".to_string();
    let note_id = NoteId::new(uuid.clone());
    assert_eq!(note_id.as_str(), &uuid);

    // Test round-trip
    let uuid2 = "note-uuid-12345".to_string();
    let note_id2 = NoteId::new(uuid2.clone());
    assert_eq!(note_id2.into_string(), uuid2);
  }

  /// Test TagId::new and as_i64
  #[test]
  fn test_bear_tag_id_construction() {
    let tag_id = TagId::new(42);
    assert_eq!(tag_id.as_i64(), 42);

    // Test round-trip
    let id_value = 12345;
    let tag_id = TagId::new(id_value);
    assert_eq!(tag_id.as_i64(), id_value);
  }

  /// Test NoteId equality and hashing
  #[test]
  fn test_note_id_equality() {
    let id1 = NoteId::new("ABC123".to_string());
    let id2 = NoteId::new("ABC123".to_string());
    let id3 = NoteId::new("DEF456".to_string());

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);

    // Test that they can be used in HashSet
    let mut set = std::collections::HashSet::new();
    set.insert(id1);
    assert!(set.contains(&id2));
    assert!(!set.contains(&id3));
  }

  /// Test TagId equality and hashing
  #[test]
  fn test_bear_tag_id_equality() {
    let id1 = TagId::new(42);
    let id2 = TagId::new(42);
    let id3 = TagId::new(43);

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);

    // Test that they can be used in HashSet
    let mut set = std::collections::HashSet::new();
    set.insert(id1);
    assert!(set.contains(&id2));
    assert!(!set.contains(&id3));
  }
}
