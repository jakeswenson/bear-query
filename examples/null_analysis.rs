//! Example demonstrating how to use the query interface to analyze NULL values in Bear's database.
//!
//! This example shows how many notes have NULL titles, content, and IDs.

use bear_query::BearDb;

fn main() -> Result<(), bear_query::BearError> {
  // Connect to Bear's database
  let db = BearDb::new()?;

  println!("=== Bear Database NULL Value Analysis ===\n");

  // Query for notes with NULL titles
  let null_titles_df = db.query("SELECT COUNT(*) as count FROM notes WHERE title IS NULL")?;
  let null_titles_count = null_titles_df.column("count")?.i64()?.get(0).unwrap();

  println!("Notes with NULL titles: {}", null_titles_count);

  // Query for notes with NULL content
  let null_content_df = db.query("SELECT COUNT(*) as count FROM notes WHERE content IS NULL")?;
  let null_content_count = null_content_df.column("count")?.i64()?.get(0).unwrap();

  println!("Notes with NULL content: {}", null_content_count);

  // Query for notes with NULL id (UUID)
  let null_id_df = db.query("SELECT COUNT(*) as count FROM notes WHERE id IS NULL")?;
  let null_id_count = null_id_df.column("count")?.i64()?.get(0).unwrap();

  println!("Notes with NULL id: {}", null_id_count);

  // Get total note count for comparison
  let total_df = db.query("SELECT COUNT(*) as count FROM notes")?;
  let total_count = total_df.column("count")?.i64()?.get(0).unwrap();

  println!("\nTotal notes in database: {}", total_count);

  // Show percentages
  if total_count > 0 {
    println!("\n=== Percentages ===");
    println!(
      "NULL titles: {:.2}%",
      (null_titles_count as f64 / total_count as f64) * 100.0
    );
    println!(
      "NULL content: {:.2}%",
      (null_content_count as f64 / total_count as f64) * 100.0
    );
    println!(
      "NULL id: {:.2}%",
      (null_id_count as f64 / total_count as f64) * 100.0
    );
  }

  // Show some examples of notes with NULL titles (if any exist)
  if null_titles_count > 0 {
    println!("\n=== Sample Notes with NULL Titles ===");
    let sample_df = db.query("SELECT id, content FROM notes WHERE title IS NULL LIMIT 5")?;

    println!("{}", sample_df);
  }

  // Show some examples of notes with NULL content (if any exist)
  if null_content_count > 0 {
    println!("\n=== Sample Notes with NULL Content ===");
    let sample_df = db.query("SELECT id, title FROM notes WHERE content IS NULL LIMIT 5")?;

    println!("{}", sample_df);
  }

  // More detailed analysis: notes with multiple NULL fields
  println!("\n=== Notes with Multiple NULL Fields ===");
  let multiple_nulls_df = db.query(
    r"
        SELECT
            COUNT(*) as count,
            CASE
                WHEN title IS NULL AND content IS NULL THEN 'Both title and content'
                WHEN title IS NULL AND id IS NULL THEN 'Both title and id'
                WHEN content IS NULL AND id IS NULL THEN 'Both content and id'
                ELSE 'Other combination'
            END as null_combination
        FROM notes
        WHERE (title IS NULL OR content IS NULL OR id IS NULL)
        GROUP BY null_combination
        ",
  )?;

  if multiple_nulls_df.height() > 0 {
    println!("{}", multiple_nulls_df);
  } else {
    println!("No notes with multiple NULL fields found.");
  }

  // Analysis by note status (trashed, archived, etc.)
  println!("\n=== NULL Values by Note Status ===");
  let by_status_df = db.query(
    r"
        SELECT
            CASE
                WHEN is_trashed = 1 THEN 'Trashed'
                WHEN is_archived = 1 THEN 'Archived'
                ELSE 'Active'
            END as status,
            COUNT(*) as total_notes,
            SUM(CASE WHEN title IS NULL THEN 1 ELSE 0 END) as null_titles,
            SUM(CASE WHEN content IS NULL THEN 1 ELSE 0 END) as null_content,
            SUM(CASE WHEN id IS NULL THEN 1 ELSE 0 END) as null_id
        FROM notes
        GROUP BY status
        ORDER BY total_notes DESC
        ",
  )?;

  println!("{}", by_status_df);

  Ok(())
}
