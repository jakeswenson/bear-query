use bear_query::{BearDb, BearError};

fn main() -> Result<(), BearError> {
  let db = BearDb::new()?;

  let tags = db.tags()?;

  println!("{:?}", tags);

  db.notes()?.into_iter().for_each(|note| {
    println!("{:?}", note);
    db.note_links(note.id()).unwrap().into_iter().for_each(|link| {
      println!("Linked: {:?}", link.title())
    });

    let note_tags = db.note_tags(note.id()).unwrap();
    println!("Tags: {:?}", tags.names(&note_tags));
  });

  Ok(())
}
