use bear_query::{BearDb, BearError, note_links, notes, tags, note_tags};

fn main() -> Result<(), BearError> {
  let db = BearDb::open()?;

  dbg!(db.connection.query_row("PRAGMA journal_mode", [], |row| row.get::<usize, String>(0))?);

  let tags = tags(&db)?;

  println!("{:?}", tags);

  notes(&db)?.into_iter().for_each(|note| {
    println!("{:?}", note);
    note_links(&db, note.id()).unwrap().into_iter().for_each(|link| {
      println!("Linked: {:?}", link.title())
    });

    let note_tags = note_tags(&db, note.id()).unwrap();
    println!("Tags: {:?}", tags.names(&note_tags));
  });

  Ok(())
}
