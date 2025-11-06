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
use bear_query::{BearDb, BearError, NotesQuery, SearchQuery, SortOn};

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

    // Search notes by title and/or content
    let search_results = db.search(SearchQuery::new("rust"))?;
    println!("Found {} notes matching 'rust'", search_results.len());

    // Advanced search with filters
    let project_notes = db.search(
        SearchQuery::new("project")
            .title_only()
            .sort_by(SortOn::Title.asc())
            .limit(20)
    )?;

    // Use the generic query API to get custom data as a DataFrame
    let df = db.query("SELECT title, created FROM notes LIMIT 5")?;
    println!("{}", df);

    Ok(())
}
```

### API Reference

#### Core Types

- **`BearDb`**: Connection to Bear's database
- **`Note`**: Represents a note with title, content, metadata
- **`Tag`**: Represents a tag
- **`TagsMap`**: Collection of tags with lookup methods
- **`NotesQuery`**: Builder for configuring note queries (filtering, limits)
- **`SearchQuery`**: Builder for configuring note searches (search scope, sorting, filtering)
- **`SortOn`**: What field to sort by (Modified, Created, Title) with `.asc()` and `.desc()` methods
- **`SortOrder`**: Sort direction (Asc/Desc) wrapping a SortOn field
- **`NoteId`**: Type-safe note identifier (Bear's UUID)
- **`TagId`**: Type-safe tag identifier
- **`DataFrame`**: Polars DataFrame (from `polars::prelude::DataFrame`) returned by `query()` method

#### Methods

- **`BearDb::new() -> Result<BearDb, BearError>`**
  Creates a handle to Bear's database (no connection is opened)

- **`BearDb::tags(&self) -> Result<TagsMap, BearError>`**
  Retrieves all tags from Bear (opens and closes a connection)

- **`BearDb::note(&self, id: &NoteId) -> Result<Option<Note>, BearError>`**
  Retrieves a specific note by its ID. Returns `None` if no note with the given ID exists.

- **`BearDb::notes(&self, query: NotesQuery) -> Result<Vec<Note>, BearError>`**
  Retrieves notes from Bear, ordered by most recently modified. Use `NotesQuery` to configure filtering and limits.

- **`BearDb::search(&self, query: SearchQuery) -> Result<Vec<Note>, BearError>`**
  Searches notes by title and/or content. Use `SearchQuery` to configure search scope, sorting, and filtering.

- **`BearDb::note_links(&self, from: &NoteId) -> Result<Vec<Note>, BearError>`**
  Retrieves all notes linked from the specified note

- **`BearDb::note_tags(&self, from: &NoteId) -> Result<HashSet<TagId>, BearError>`**
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

#### SearchQuery Builder Methods

- **`SearchQuery::new(query: impl Into<String>) -> SearchQuery`**
  Creates a new search with the given query string. Defaults: searches both title and content, case-insensitive, limit 50, sorted by most recently modified, excludes trashed and archived.

- **`.title_only() -> SearchQuery`**
  Search only in note titles (excludes content)

- **`.content_only() -> SearchQuery`**
  Search only in note content (excludes titles)

- **`.title_and_content() -> SearchQuery`**
  Search in both title and content (default)

- **`.case_sensitive() -> SearchQuery`**
  Enable case-sensitive search (default is case-insensitive)

- **`.limit(n: u32) -> SearchQuery`**
  Set the maximum number of results to return

- **`.no_limit() -> SearchQuery`**
  Remove the limit and return all matching notes

- **`.sort_by(order: SortOrder) -> SearchQuery`**
  Set the sort order for results

- **`.include_trashed() -> SearchQuery`**
  Include trashed notes in search results

- **`.include_archived() -> SearchQuery`**
  Include archived notes in search results

- **`.include_all() -> SearchQuery`**
  Include both trashed and archived notes in search results

#### SortOn Fields and Methods

Use `SortOn` to specify what field to sort by, then call `.asc()` or `.desc()`:

- **`SortOn::Modified`** - Sort by modification timestamp
  - `.desc()` - Most recently modified first (default)
  - `.asc()` - Least recently modified first
- **`SortOn::Created`** - Sort by creation timestamp
  - `.desc()` - Most recently created first
  - `.asc()` - Least recently created first
- **`SortOn::Title`** - Sort by note title
  - `.asc()` - Alphabetical (A-Z)
  - `.desc()` - Reverse alphabetical (Z-A)

## Searching Notes

The search API provides flexible full-text search across note titles and content:

```rust
use bear_query::{BearDb, SearchQuery, SortOn};

let db = BearDb::new()?;

// Simple search in both title and content
let results = db.search(SearchQuery::new("rust"))?;

// Search only in titles
let results = db.search(SearchQuery::new("project").title_only())?;

// Search only in content
let results = db.search(SearchQuery::new("TODO").content_only())?;

// Case-sensitive search
let results = db.search(SearchQuery::new("Rust").case_sensitive())?;

// Complex search with multiple options
let results = db.search(
    SearchQuery::new("programming")
        .title_only()
        .sort_by(SortOn::Title.asc())
        .limit(20)
        .include_archived()
)?;
```

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
