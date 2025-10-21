# bear-query

A **completely read-only**, **non-blocking** Rust library for querying the [Bear](https://bear.app) note-taking app's SQLite database.

## Overview

This library provides safe, read-only access to Bear's internal SQLite database without interfering with the Bear app's operations. It is designed to be **completely undetectable** and **never blocks writes**.

## Safety Guarantees

This library implements multiple layers of protection to ensure it **NEVER** interferes with Bear:

### 1. Read-Only File Access
- Opens the database with `SQLITE_OPEN_READ_ONLY` flag
- The OS prevents any write operations at the file descriptor level

### 2. No Internal Locks
- Uses `SQLITE_OPEN_NO_MUTEX` flag to disable SQLite's internal mutexes
- Prevents any lock contention with Bear's write operations

### 3. Query-Only Mode
- Enforces `PRAGMA query_only = ON` at the SQLite level
- Additional safety layer that prevents writes even if attempted programmatically

### 4. WAL Mode Compatibility
- Verifies the database is in WAL (Write-Ahead Logging) mode
- WAL mode allows **concurrent reads and writes without blocking**
- Bear uses WAL mode by default, enabling zero-interference reads

## How It Works

### SQLite WAL Mode

Bear's database uses SQLite's WAL (Write-Ahead Logging) mode, which provides key benefits:

- **Concurrent Access**: Readers and writers don't block each other
- **No Read Locks**: Read operations never block write operations
- **Crash Safety**: The database remains consistent even if a reader crashes

In WAL mode:
- Writes go to a separate WAL file (`database.sqlite-wal`)
- Reads access stable snapshots of the database
- **Zero lock contention** between readers and writers

This means `bear-query` can read from Bear's database while Bear is actively writing, with **no interference whatsoever**.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
bear-query = { path = "." }  # or git/version once published
```

## Usage

### Basic Example

```rust
use bear_query::{BearDb, BearError, notes, tags, note_tags, note_links};

fn main() -> Result<(), BearError> {
    // Open Bear's database (completely read-only, non-blocking)
    let db = BearDb::open()?;

    // Retrieve all tags
    let all_tags = tags(&db)?;
    println!("Tags: {:?}", all_tags);

    // Retrieve recent notes (limited to 10)
    let recent_notes = notes(&db)?;

    for note in recent_notes {
        println!("Title: {}", note.title());

        // Get links from this note
        let links = note_links(&db, note.id())?;
        for link in links {
            println!("  -> Linked to: {}", link.title());
        }

        // Get tags for this note
        let note_tag_ids = note_tags(&db, note.id())?;
        let tag_names = all_tags.names(&note_tag_ids);
        println!("  Tags: {:?}", tag_names);
    }

    Ok(())
}
```

### API Reference

#### Core Types

- **`BearDb`**: Connection to Bear's database
- **`BearNote`**: Represents a note with title, content, metadata
- **`BearTag`**: Represents a tag
- **`BearTags`**: Collection of tags with lookup methods

#### Functions

- **`BearDb::open() -> Result<BearDb, BearError>`**
  Opens Bear's database with read-only, non-blocking access

- **`tags(db: &BearDb) -> Result<BearTags, BearError>`**
  Retrieves all tags from Bear

- **`notes(db: &BearDb) -> Result<Vec<BearNote>, BearError>`**
  Retrieves up to 10 most recently modified notes (non-trashed, non-archived)

- **`note_links(db: &BearDb, from: BearNoteId) -> Result<Vec<BearNote>, BearError>`**
  Retrieves all notes linked from the specified note

- **`note_tags(db: &BearDb, from: BearNoteId) -> Result<HashSet<BearTagId>, BearError>`**
  Retrieves all tag IDs associated with the specified note

## Database Location

Bear stores its database at:
```
~/Library/Group Containers/9K33E3U3T4.net.shinyfrog.bear/Application Data/database.sqlite
```

This library automatically locates the database using the user's home directory.

## Technical Details

### Bear's Database Schema

Bear uses Core Data with SQLite persistence. Key tables:

- **`ZSFNOTE`**: Contains notes (title, content, timestamps, flags)
- **`ZSFNOTETAG`**: Contains tags
- **`Z_7TAGS`**: Junction table linking notes to tags
- **`Z_7LINKEDNOTES`**: Junction table for note-to-note links

### Core Data Timestamps

Bear uses Apple's Core Data timestamp format (seconds since 2001-01-01). This library automatically converts them to standard Unix timestamps.

### Query Limits

The `notes()` function limits results to 10 notes to prevent excessive memory usage. To retrieve all notes, modify the SQL query in `src/lib.rs`:

```rust
// Remove or increase the LIMIT
LIMIT 10  // Change to your desired limit or remove
```

## Safety Notes

### Why This Is Safe

1. **No Write Operations**: Multiple read-only flags prevent any writes
2. **WAL Mode**: Bear's use of WAL mode means reads never block writes
3. **No Shared Locks**: The combination of flags ensures no locks are taken
4. **Crash Isolation**: If this library crashes, Bear is unaffected

### Verification

The library automatically verifies WAL mode is enabled and warns if not:

```
Warning: Database is not in WAL mode (current: delete). Reads may block writes.
```

If you see this warning, **do not use this library** while Bear is running, as it could cause interference.

## Error Handling

The library uses `BearError` for all errors:

```rust
pub enum BearError {
    NoHomeDirectory,  // Cannot locate home directory
    SqlError { .. },  // SQLite operation failed
}
```

## Dependencies

This library uses minimal, well-maintained dependencies:

- **rusqlite** (0.32+): SQLite interface with bundled SQLite for portability
- **dirs** (5.0+): Cross-platform user directory detection
- **time** (0.3+): Date/time handling for Core Data timestamps
- **serde** (1.0+): Serialization framework (used by time)
- **thiserror** (1.0+): Error handling macros

All dependencies use semver-compatible version ranges and are updated to their latest stable releases.

## Building

```bash
cargo build --release
```

First build will download dependencies from crates.io.

## Running

```bash
cargo run
```

## License

This is an unofficial tool and is not affiliated with Bear or Shiny Frog.

## Contributing

Contributions are welcome! Please ensure all changes maintain the read-only, non-blocking guarantees.

---

**Important**: This library is designed for read-only access only. Never attempt to modify Bear's database directly, as this could corrupt your notes.
