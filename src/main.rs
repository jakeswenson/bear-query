use bear_query::{BearDb, BearError, NotesQuery};

fn main() -> Result<(), BearError> {
  let db = BearDb::new()?;

  // Example 1: Using typed API
  println!("=== Typed API Examples ===\n");

  let tags = db.tags()?;
  println!("Total tags: {}\n", tags.count());

  // Using the new NotesQuery API - much more readable!
  db.notes(NotesQuery::default())?
    .into_iter()
    .for_each(|note| {
      println!("Note: {:?}", note.title());
      db.note_links(note.id())
        .unwrap()
        .into_iter()
        .for_each(|link| println!("  Linked: {:?}", link.title()));

      let note_tags = db.note_tags(note.id()).unwrap();
      println!("  Tags: {:?}\n", tags.names(&note_tags));
    });

  // Example 2: Using generic SQL query API with DataFrames
  println!("\n=== Generic SQL Query API Examples ===\n");

  // Simple query
  println!("Top 5 most recent notes:");
  let df = db
    .query("SELECT title, created FROM notes WHERE is_trashed = 0 ORDER BY created DESC LIMIT 5")?;
  println!("{}\n", df);

  // Join query
  println!("Notes with their tags:");
  let df = db.query(
    r"
    SELECT n.title, t.name as tag_name
    FROM notes n
    JOIN note_tags nt ON n.id = nt.note_id
    JOIN tags t ON nt.tag_id = t.id
    WHERE n.is_trashed = 0
    ORDER BY n.modified DESC
    LIMIT 10
  ",
  )?;
  println!("{}\n", df);

  // Aggregation query
  println!("Tag usage statistics:");
  let df = db.query(
    r"
    SELECT t.name, COUNT(*) as note_count
    FROM tags t
    JOIN note_tags nt ON t.id = nt.tag_id
    GROUP BY t.id, t.name
    ORDER BY note_count DESC
    LIMIT 10
  ",
  )?;
  println!("{}\n", df);

  Ok(())
}
