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
use bear_query::{BearDb, BearError, NotesQuery};

fn main() -> Result<(), BearError> {
    // Create a BearDb handle (doesn't open a connection yet)
    let db = BearDb::new()?;

    // Each method call opens a connection, runs the query, and closes it
    let all_tags = db.tags()?;
    println!("Found {} tags", all_tags.count());

    // Retrieve recent notes (default: limited to 10, exclude trashed/archived)
    let recent_notes = db.notes(NotesQuery::default())?;

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

    // Get all notes including trashed and archived
    let all_notes = db.notes(NotesQuery::new().no_limit().include_all())?;
    println!("Total notes: {}", all_notes.len());

    // Use the generic query API to get custom data as a DataFrame
    let df = db.query("SELECT title, created FROM notes LIMIT 5")?;
    println!("{}", df);

    Ok(())
}
```

### API Reference

#### Core Types

- **`BearDb`**: Connection to Bear's database
- **`BearNote`**: Represents a note with title, content, metadata
- **`BearTag`**: Represents a tag
- **`BearTags`**: Collection of tags with lookup methods
- **`NotesQuery`**: Builder for configuring note queries (filtering, limits)
- **`BearNoteId`**: Type-safe note identifier
- **`BearTagId`**: Type-safe tag identifier
- **`DataFrame`**: Polars DataFrame (from `polars::prelude::DataFrame`) returned by `query()` method

#### Methods

- **`BearDb::new() -> Result<BearDb, BearError>`**
  Creates a handle to Bear's database (no connection is opened)

- **`BearDb::tags(&self) -> Result<BearTags, BearError>`**
  Retrieves all tags from Bear (opens and closes a connection)

- **`BearDb::notes(&self, query: NotesQuery) -> Result<Vec<BearNote>, BearError>`**
  Retrieves notes from Bear, ordered by most recently modified. Use `NotesQuery` to configure filtering and limits.

- **`BearDb::note_links(&self, from: BearNoteId) -> Result<Vec<BearNote>, BearError>`**
  Retrieves all notes linked from the specified note

- **`BearDb::note_tags(&self, from: BearNoteId) -> Result<HashSet<BearTagId>, BearError>`**
  Retrieves all tag IDs associated with the specified note

- **`BearDb::query(&self, sql: &str) -> Result<DataFrame, BearError>`**
  Execute a generic SQL SELECT query and return results as a Polars DataFrame. Normalized tables (`notes`, `tags`, `note_tags`, `note_links`) are automatically available.

#### NotesQuery Builder Methods

- **`NotesQuery::new()` / `NotesQuery::default()`**
  Creates a new query with defaults (limit: 10, exclude trashed and archived)

- **`.limit(n: u32) -> NotesQuery`**
  Set a limit on the number of notes to return

- **`.no_limit() -> NotesQuery`**
  Remove the limit and return all matching notes

- **`.include_trashed() -> NotesQuery`**
  Include trashed notes in results

- **`.include_archived() -> NotesQuery`**
  Include archived notes in results

- **`.include_all() -> NotesQuery`**
  Include both trashed and archived notes in results

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
- **`Z_5TAGS`**: Junction table linking notes to tags (column names may vary by Bear version)
- **`ZSFNOTEBACKLINK`**: Junction table for note-to-note links

### Core Data Timestamps

Bear uses Apple's Core Data timestamp format (seconds since 2001-01-01). This library automatically converts them to standard Unix timestamps.

### Query Configuration

The `notes()` method uses `NotesQuery` to configure results. Examples:

```rust
// Default: 10 most recent notes, exclude trashed/archived
let notes = db.notes(NotesQuery::default())?;

// Get 20 notes
let notes = db.notes(NotesQuery::new().limit(20))?;

// Get all notes
let notes = db.notes(NotesQuery::new().no_limit())?;

// Get all notes including trashed and archived
let notes = db.notes(NotesQuery::new().no_limit().include_all())?;
```

### Using the Generic Query API

For custom queries beyond the typed API, use the `query()` method which returns Polars DataFrames:

```rust
// Simple select
let df = db.query("SELECT title, created FROM notes LIMIT 5")?;

// Join notes with tags
let df = db.query(r"
    SELECT n.title, t.name as tag_name
    FROM notes n
    JOIN note_tags nt ON n.id = nt.note_id
    JOIN tags t ON nt.tag_id = t.id
    WHERE n.is_trashed = 0
    ORDER BY n.modified DESC
    LIMIT 10
")?;

// Aggregation
let df = db.query("SELECT COUNT(*) as count FROM notes WHERE is_pinned = 1")?;

// The normalized tables available: notes, tags, note_tags, note_links
println!("{}", df);  // Polars DataFrame with nice formatting
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
    NoHomeDirectory,       // Cannot locate home directory
    SqlError { .. },       // SQLite operation failed
    PolarsError { .. },    // Polars DataFrame operation failed
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
