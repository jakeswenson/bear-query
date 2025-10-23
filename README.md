# bear-query

A **completely read-only**, **minimal-contention** Rust library for querying the [Bear](https://bear.app) note-taking app's SQLite database.

## Overview

This library provides safe, read-only access to Bear's internal SQLite database with minimal interference. It uses **short-lived connections** that are opened only when needed and closed immediately after use.

## Safety Guarantees

This library implements multiple layers of protection to ensure minimal interference with Bear:

### 1. Read-Only File Access
- Opens the database with `SQLITE_OPEN_READ_ONLY` flag
- The OS prevents any write operations at the file descriptor level

### 2. No Internal Locks
- Uses `SQLITE_OPEN_NO_MUTEX` flag to disable SQLite's internal mutexes
- Minimizes lock contention with Bear's write operations

### 3. Query-Only Mode
- Enforces `PRAGMA query_only = ON` at the SQLite level
- Additional safety layer that prevents writes even if attempted programmatically

### 4. Short-Lived Connections
- Connections are only open for the duration of each query
- 5000ms busy timeout handles any database contention gracefully
- Automatic connection cleanup after each operation

### 5. No WAL Mode Requirement
- Bear does **not** use WAL (Write-Ahead Logging) mode by default
- Short-lived connections ensure we don't hold locks during Bear's writes
- Busy timeout allows Bear to complete write operations without blocking

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
bear-query = { path = "." }  # or git/version once published
```

## Usage

### Basic Example

```rust
use bear_query::{BearDb, BearError};

fn main() -> Result<(), BearError> {
    // Create a BearDb handle (doesn't open a connection yet)
    let db = BearDb::new()?;

    // Each method call opens a connection, runs the query, and closes it
    let all_tags = db.tags()?;
    println!("Tags: {:?}", all_tags);

    // Retrieve recent notes (limited to 10)
    let recent_notes = db.notes()?;

    for note in recent_notes {
        println!("Title: {}", note.title());

        // Get links from this note (opens and closes a connection)
        let links = db.note_links(note.id())?;
        for link in links {
            println!("  -> Linked to: {}", link.title());
        }

        // Get tags for this note (opens and closes a connection)
        let note_tag_ids = db.note_tags(note.id())?;
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

#### Methods

- **`BearDb::new() -> Result<BearDb, BearError>`**
  Creates a handle to Bear's database (no connection is opened)

- **`BearDb::tags(&self) -> Result<BearTags, BearError>`**
  Retrieves all tags from Bear (opens and closes a connection)

- **`BearDb::notes(&self) -> Result<Vec<BearNote>, BearError>`**
  Retrieves up to 10 most recently modified notes (non-trashed, non-archived)

- **`BearDb::note_links(&self, from: BearNoteId) -> Result<Vec<BearNote>, BearError>`**
  Retrieves all notes linked from the specified note

- **`BearDb::note_tags(&self, from: BearNoteId) -> Result<HashSet<BearTagId>, BearError>`**
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

The `notes()` method limits results to 10 notes to prevent excessive memory usage. To retrieve all notes, modify the SQL query in the `BearDb::notes()` method in `src/lib.rs`:

```rust
// Remove or increase the LIMIT
LIMIT 10  // Change to your desired limit or remove
```

## Safety Notes

### Why This Is Safe

1. **No Write Operations**: Multiple read-only flags prevent any writes
2. **Short-Lived Connections**: Connections are only open during queries, minimizing lock contention
3. **Busy Timeout**: 5000ms timeout allows Bear to complete writes without permanent blocking
4. **Crash Isolation**: If this library crashes, Bear is unaffected since connections are short-lived

### Important Note on WAL Mode

Bear does **not** use WAL (Write-Ahead Logging) mode by default. This library is designed to work safely without WAL by:
- Using very short-lived connections
- Setting a reasonable busy timeout (5000ms)
- Opening connections only when absolutely necessary

This approach ensures minimal interference with Bear's normal operations.

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

- **rusqlite** (0.37.0): SQLite interface with bundled SQLite for portability
- **dirs** (6.0.0): Cross-platform user directory detection
- **time** (0.3.44): Date/time handling for Core Data timestamps
- **serde** (1.0+): Serialization framework (used by time)
- **thiserror** (2.0+): Error handling macros

All dependencies are pinned to their latest stable releases as of January 2025.

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
