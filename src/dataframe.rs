/// DataFrame conversion logic for rusqlite to Polars
///
/// This module handles converting rusqlite query results into Polars DataFrames.
/// It uses column-wise construction for optimal performance.
use crate::{BearError, Queryable};
use polars::prelude::*;

use rusqlite::types::ValueRef;

/// Represents a value that can be stored in a column during DataFrame construction
#[derive(Debug, Clone)]
enum ColumnValue {
  Null,
  Integer(i64),
  Real(f64),
  Text(String),
  Blob(Vec<u8>),
}

impl From<ValueRef<'_>> for ColumnValue {
  fn from(value: ValueRef) -> Self {
    match value {
      ValueRef::Null => ColumnValue::Null,
      ValueRef::Integer(i) => ColumnValue::Integer(i),
      ValueRef::Real(f) => ColumnValue::Real(f),
      ValueRef::Text(s) => ColumnValue::Text(String::from_utf8_lossy(s).to_string()),
      ValueRef::Blob(b) => ColumnValue::Blob(b.to_vec()),
    }
  }
}

/// Converts rusqlite query results into a Polars DataFrame.
///
/// Uses column-wise construction for optimal performance. Preserves SQLite types:
/// - NULL -> Null
/// - INTEGER -> Int64
/// - REAL -> Float64
/// - TEXT -> String
/// - BLOB -> Binary
///
/// # Arguments
/// * `queryable` - The Queryable wrapper that prepends normalizing CTEs
/// * `sql` - The user's SQL query (will have CTEs prepended automatically)
///
/// # Returns
/// A Polars DataFrame containing the query results, or a BearError on failure
pub fn query_to_dataframe(
  queryable: &Queryable,
  sql: &str,
) -> Result<DataFrame, BearError> {
  let mut stmt = queryable.prepare(sql)?;

  // Get column names
  let column_names: Vec<String> = stmt
    .column_names()
    .into_iter()
    .map(|s| s.to_string())
    .collect();
  let column_count = column_names.len();

  // Prepare vectors for each column (column-wise construction is much faster)
  let mut columns: Vec<Vec<ColumnValue>> = vec![Vec::new(); column_count];

  // Collect data row by row, distributing into columns
  let rows = stmt
    .query_map([], |row| {
      let mut values = Vec::new();
      for i in 0..column_count {
        // Get the actual value with its type preserved
        let value = row
          .get_ref(i)
          .map(ColumnValue::from)
          .unwrap_or(ColumnValue::Null);
        values.push(value);
      }
      Ok(values)
    })
    .map_err(|e| BearError::SqlError { source: e })?;

  for row_result in rows {
    let row_values = row_result.map_err(|e| BearError::SqlError { source: e })?;
    for (col_idx, value) in row_values.into_iter().enumerate() {
      columns[col_idx].push(value);
    }
  }

  // Build Series from column vectors based on their types
  let series: Vec<_> = column_names
    .into_iter()
    .zip(columns)
    .map(|(name, data)| {
      // Infer the column type from the data
      build_series(name, data)
    })
    .collect();

  Ok(DataFrame::new(series)?)
}

/// Builds a Polars Series from a column of values, inferring the appropriate type
fn build_series(
  name: String,
  values: Vec<ColumnValue>,
) -> Column {
  // Determine the predominant type (ignoring nulls)
  let mut has_integer = false;
  let mut has_real = false;
  let mut has_text = false;
  let mut has_blob = false;

  for value in &values {
    match value {
      ColumnValue::Integer(_) => has_integer = true,
      ColumnValue::Real(_) => has_real = true,
      ColumnValue::Text(_) => has_text = true,
      ColumnValue::Blob(_) => has_blob = true,
      ColumnValue::Null => {}
    }
  }

  // Priority: Real > Integer > Text > Blob
  if has_real {
    // Float64 column
    let data: Vec<Option<f64>> = values
      .into_iter()
      .map(|v| match v {
        ColumnValue::Real(f) => Some(f),
        ColumnValue::Integer(i) => Some(i as f64), // Promote integer to float
        ColumnValue::Null => None,
        _ => None, // Mixed types, treat as null
      })
      .collect();
    Series::new(name.into(), data).into()
  } else if has_integer {
    // Int64 column
    let data: Vec<Option<i64>> = values
      .into_iter()
      .map(|v| match v {
        ColumnValue::Integer(i) => Some(i),
        ColumnValue::Null => None,
        _ => None, // Mixed types, treat as null
      })
      .collect();
    Series::new(name.into(), data).into()
  } else if has_text {
    // String column
    let data: Vec<Option<String>> = values
      .into_iter()
      .map(|v| match v {
        ColumnValue::Text(s) => Some(s),
        ColumnValue::Null => None,
        _ => None,
      })
      .collect();
    Series::new(name.into(), data).into()
  } else if has_blob {
    // Binary column
    let data: Vec<Option<Vec<u8>>> = values
      .into_iter()
      .map(|v| match v {
        ColumnValue::Blob(b) => Some(b),
        ColumnValue::Null => None,
        _ => None,
      })
      .collect();
    Series::new(name.into(), data).into()
  } else {
    // All nulls - create a null string column
    let data: Vec<Option<String>> = values.into_iter().map(|_| None).collect();
    Series::new(name.into(), data).into()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use rusqlite::Connection;

  /// Helper to create an in-memory database with Bear-like schema
  fn create_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();

    // Create Bear's Core Data tables with Z_ prefixes
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
    ",
      )
      .unwrap();

    // Insert test data
    // Core Data epoch: 2001-01-01 = Unix timestamp 978307200
    // So a timestamp of 0 in Core Data = 2001-01-01
    // A timestamp of 31536000 (1 year) = 2002-01-01

    conn.execute_batch(r"
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
    ").unwrap();

    conn
  }

  /// Helper to create a Queryable with normalizing CTEs for testing
  fn create_test_queryable(conn: &Connection) -> Queryable<'_> {
    let normalizing_cte = r"
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
      nt.Z_5NOTES as note_id,
      nt.Z_13TAGS as tag_id
    FROM Z_5TAGS as nt
  ),
  note_links AS (
    SELECT
      nl.ZLINKEDBY as from_note_id,
      nl.ZLINKINGTO as to_note_id
    FROM ZSFNOTEBACKLINK as nl
  )
";
    Queryable::new_for_test(conn, normalizing_cte)
  }

  #[test]
  fn test_simple_query() {
    let conn = create_test_db();
    let queryable = create_test_queryable(&conn);

    let df = query_to_dataframe(&queryable, "SELECT id, title FROM notes").unwrap();

    assert_eq!(df.height(), 3); // 3 notes total
    assert_eq!(df.width(), 2); // 2 columns (id, title)

    let column_names = df.get_column_names();
    assert_eq!(column_names.len(), 2);
    assert_eq!(column_names[0].as_str(), "id");
    assert_eq!(column_names[1].as_str(), "title");
  }

  #[test]
  fn test_filtered_query() {
    let conn = create_test_db();
    let queryable = create_test_queryable(&conn);

    let df =
      query_to_dataframe(&queryable, "SELECT title FROM notes WHERE is_trashed = 0").unwrap();

    assert_eq!(df.height(), 2); // Only 2 non-trashed notes
  }

  #[test]
  fn test_join_query() {
    let conn = create_test_db();
    let queryable = create_test_queryable(&conn);

    let df = query_to_dataframe(
      &queryable,
      r"
      SELECT n.title, t.name as tag_name
      FROM notes n
      JOIN note_tags nt ON n.id = nt.note_id
      JOIN tags t ON nt.tag_id = t.id
    ",
    )
    .unwrap();

    assert_eq!(df.height(), 2); // 2 note-tag relationships
    assert_eq!(df.width(), 2); // title and tag_name columns
  }

  #[test]
  fn test_empty_result() {
    let conn = create_test_db();
    let queryable = create_test_queryable(&conn);

    let df = query_to_dataframe(&queryable, "SELECT * FROM notes WHERE id = 999").unwrap();

    assert_eq!(df.height(), 0); // No results
    assert_eq!(df.width(), 9); // But still has all columns from notes
  }

  #[test]
  fn test_aggregation() {
    let conn = create_test_db();
    let queryable = create_test_queryable(&conn);

    let df = query_to_dataframe(&queryable, "SELECT COUNT(*) as count FROM notes").unwrap();

    assert_eq!(df.height(), 1);
    assert_eq!(df.width(), 1);
  }

  #[test]
  fn test_timestamp_conversion() {
    let conn = create_test_db();
    let queryable = create_test_queryable(&conn);

    let df = query_to_dataframe(&queryable, "SELECT modified FROM notes WHERE id = 1").unwrap();

    assert_eq!(df.height(), 1);

    // Check that timestamp was converted (should be "2001-01-01 00:00:00")
    let series = df.column("modified").unwrap();
    let value = series.get(0).unwrap();

    // The value should be a string starting with "2001-01-01"
    match value {
      AnyValue::String(s) => {
        assert!(
          s.starts_with("2001-01-01"),
          "Expected timestamp to start with 2001-01-01, got: {}",
          s
        );
      }
      AnyValue::StringOwned(s) => {
        assert!(
          s.as_str().starts_with("2001-01-01"),
          "Expected timestamp to start with 2001-01-01, got: {}",
          s
        );
      }
      _ => panic!("Expected string value, got: {:?}", value),
    }
  }

  #[test]
  fn test_null_values() {
    let conn = create_test_db();

    // Add a note with NULL title
    conn.execute(
      "INSERT INTO ZSFNOTE (Z_PK, ZUNIQUEIDENTIFIER, ZTITLE, ZTEXT, ZMODIFICATIONDATE, ZCREATIONDATE, ZPINNED, ZTRASHED, ZARCHIVED)
       VALUES (4, 'note-uuid-4', NULL, 'Content', 0, 0, 0, 0, 0)",
      []
    ).unwrap();

    let queryable = create_test_queryable(&conn);
    let df = query_to_dataframe(&queryable, "SELECT id, title FROM notes WHERE id = 4").unwrap();

    assert_eq!(df.height(), 1);

    let series = df.column("title").unwrap();
    let value = series.get(0).unwrap();

    assert!(
      matches!(value, AnyValue::Null),
      "Expected NULL value, got: {:?}",
      value
    );
  }
}
