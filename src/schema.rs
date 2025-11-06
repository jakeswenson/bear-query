//! Schema discovery and normalization for Bear's Core Data SQLite database.
//!
//! This module handles the variable parts of Bear's schema that may change across versions,
//! particularly the numbered junction tables (e.g., Z_5TAGS) and their column names.

use rusqlite::Connection;

use crate::BearError;

/// Sets up a Bear-like schema with sample test data in an in-memory database
#[cfg(test)]
pub fn setup_test_schema(conn: &Connection) -> Result<(), BearError> {
  conn
    .execute_batch(
      r"
      CREATE TABLE ZSFNOTE (
        Z_PK INTEGER PRIMARY KEY,
        ZUNIQUEIDENTIFIER TEXT,
        ZTITLE TEXT,
        ZTEXT TEXT,
        ZMODIFICATIONDATE REAL,
        ZCREATIONDATE REAL,
        ZPINNED INTEGER,
        ZTRASHED INTEGER,
        ZARCHIVED INTEGER
      );

      CREATE TABLE ZSFNOTETAG (
        Z_PK INTEGER PRIMARY KEY,
        ZTITLE TEXT,
        ZMODIFICATIONDATE REAL
      );

      CREATE TABLE Z_5TAGS (
        Z_5NOTES INTEGER,
        Z_13TAGS INTEGER
      );

      CREATE TABLE ZSFNOTEBACKLINK (
        ZLINKEDBY INTEGER,
        ZLINKINGTO INTEGER
      );

      -- Insert sample test data
      -- Core Data epoch: 2001-01-01, so timestamp 0 = 2001-01-01
      INSERT INTO ZSFNOTE (Z_PK, ZUNIQUEIDENTIFIER, ZTITLE, ZTEXT, ZMODIFICATIONDATE, ZCREATIONDATE, ZPINNED, ZTRASHED, ZARCHIVED)
      VALUES
        (1, 'note-uuid-1', 'First Note', 'Content of first note', 0, 0, 0, 0, 0),
        (2, 'note-uuid-2', 'Second Note', 'Content of second note', 31536000, 31536000, 1, 0, 0),
        (3, 'note-uuid-3', 'Trashed Note', 'This is trashed', 0, 0, 0, 1, 0);

      INSERT INTO ZSFNOTETAG (Z_PK, ZTITLE, ZMODIFICATIONDATE)
      VALUES
        (1, 'work', 0),
        (2, 'personal', 0);

      INSERT INTO Z_5TAGS (Z_5NOTES, Z_13TAGS)
      VALUES
        (1, 1),
        (2, 2);

      INSERT INTO ZSFNOTEBACKLINK (ZLINKEDBY, ZLINKINGTO)
      VALUES
        (1, 2);
    ",
    )
    .map_err(|e| BearError::SqlError { source: e })?;

  Ok(())
}

/// Metadata discovered from Bear's database schema at initialization time.
/// This captures the variable parts of Bear's Core Data schema that may change across versions.
#[derive(Debug, Clone)]
pub struct BearDbMetadata {
  /// Name of the junction table linking notes to tags (e.g., "Z_5TAGS")
  pub junction_table_name: String,
  /// Column name in junction table that references notes (e.g., "Z_5NOTES")
  pub junction_notes_column: String,
  /// Column name in junction table that references tags (e.g., "Z_13TAGS")
  pub junction_tags_column: String,
}

/// Discovers variable schema information from Bear's database
pub fn discover_metadata(conn: &Connection) -> Result<BearDbMetadata, BearError> {
  // First, find the junction table - it should match the pattern Z_<number>TAGS
  let junction_table_name = find_junction_table(conn)?;

  // Query the junction table to find its column names
  let query = format!("PRAGMA table_info({})", junction_table_name);
  let mut stmt = conn.prepare(&query)?;
  let columns: Vec<String> = stmt
    .query_map([], |row| row.get::<_, String>("name"))?
    .collect::<Result<Vec<_>, _>>()?;

  // Find the columns that reference notes and tags
  // They follow the pattern Z_<number>NOTES and Z_<number>TAGS
  let junction_notes_column = columns
    .iter()
    .find(|name| name.ends_with("NOTES"))
    .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?
    .clone();

  let junction_tags_column = columns
    .iter()
    .find(|name| name.ends_with("TAGS"))
    .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?
    .clone();

  Ok(BearDbMetadata {
    junction_table_name,
    junction_notes_column,
    junction_tags_column,
  })
}

/// Finds the junction table name by querying sqlite_master for tables matching Z_<number>TAGS
fn find_junction_table(conn: &Connection) -> Result<String, BearError> {
  let mut stmt = conn.prepare(
    r"
    SELECT name
    FROM sqlite_master
    WHERE type = 'table'
      AND name GLOB 'Z_[0-9]*TAGS'
    LIMIT 1
  ",
  )?;

  let table_name: String = stmt.query_row([], |row| row.get(0))?;

  Ok(table_name)
}

/// Generates the normalizing CTE SQL that abstracts Bear's Core Data schema
pub fn generate_normalizing_cte(metadata: &BearDbMetadata) -> String {
  format!(
    r#"
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
    FROM {} as nt
  ),
  note_links AS (
    SELECT
      nl.ZLINKEDBY as from_note_id,
      nl.ZLINKINGTO as to_note_id
    FROM ZSFNOTEBACKLINK as nl
  )
"#,
    metadata.junction_notes_column, metadata.junction_tags_column, metadata.junction_table_name
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_discover_metadata_with_test_schema() {
    let conn = Connection::open_in_memory().unwrap();

    // Set up a Bear-like schema with Z_5TAGS junction table
    conn
      .execute_batch(
        r"
      CREATE TABLE Z_5TAGS (
        Z_5NOTES INTEGER,
        Z_13TAGS INTEGER
      );
    ",
      )
      .unwrap();

    let metadata = discover_metadata(&conn).unwrap();

    assert_eq!(metadata.junction_table_name, "Z_5TAGS");
    assert_eq!(metadata.junction_notes_column, "Z_5NOTES");
    assert_eq!(metadata.junction_tags_column, "Z_13TAGS");
  }

  #[test]
  fn test_discover_metadata_with_different_numbers() {
    let conn = Connection::open_in_memory().unwrap();

    // Test with different numbers (simulating a different Bear version)
    conn
      .execute_batch(
        r"
      CREATE TABLE Z_7TAGS (
        Z_7NOTES INTEGER,
        Z_15TAGS INTEGER
      );
    ",
      )
      .unwrap();

    let metadata = discover_metadata(&conn).unwrap();

    assert_eq!(metadata.junction_table_name, "Z_7TAGS");
    assert_eq!(metadata.junction_notes_column, "Z_7NOTES");
    assert_eq!(metadata.junction_tags_column, "Z_15TAGS");
  }

  #[test]
  fn test_generate_normalizing_cte() {
    let metadata = BearDbMetadata {
      junction_table_name: "Z_5TAGS".to_string(),
      junction_notes_column: "Z_5NOTES".to_string(),
      junction_tags_column: "Z_13TAGS".to_string(),
    };

    let cte = generate_normalizing_cte(&metadata);

    // Verify the CTE contains the correct table and column names
    assert!(cte.contains("FROM Z_5TAGS as nt"));
    assert!(cte.contains("nt.Z_5NOTES as note_id"));
    assert!(cte.contains("nt.Z_13TAGS as tag_id"));
  }
}
