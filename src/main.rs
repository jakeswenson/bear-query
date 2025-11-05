use bear_query::{BearDb, BearError, NotesQuery};

fn main() -> Result<(), BearError> {
  let db = BearDb::new()?;

  let tags = db.tags()?;

  println!("{:?}", tags);

  // Using the new NotesQuery API - much more readable!
  db.notes(NotesQuery::default())?.into_iter().for_each(|note| {
    println!("{:?}", note);
    db.note_links(note.id()).unwrap().into_iter().for_each(|link| {
      println!("Linked: {:?}", link.title())
    });

    let note_tags = db.note_tags(note.id()).unwrap();
    println!("Tags: {:?}", tags.names(&note_tags));
  });

  Ok(())
}
