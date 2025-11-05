# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`bear-query` is a read-only Rust library for querying the Bear note-taking app's SQLite database. The library is designed to minimize interference with Bear's operations while it's running.

## Core Architecture

### Short-Lived Connection Pattern

**Critical Design Principle**: This library uses short-lived database connections to avoid blocking Bear's write operations. Bear does NOT use SQLite's WAL (Write-Ahead Logging) mode, so connection management is critical.

The architecture is built around a single private method:

- `BearDb::with_connection<F, R>(&self, f: F)` - The ONLY place where database connections are opened
  - Opens connection with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX`
  - Sets busy timeout to 5000ms
  - Enables `PRAGMA query_only = ON`
  - Executes closure with connection reference
  - Automatically closes connection when closure returns

All public methods (`tags()`, `notes()`, `note_links()`, `note_tags()`) internally call `with_connection()` and should never hold connections longer than necessary.

### Database Schema & Normalization

Bear uses Core Data with SQLite persistence. The underlying schema uses Core Data conventions:
- Table names prefixed with `Z` (e.g., `ZSFNOTE`, `ZSFNOTETAG`)
- Column names prefixed with `Z` (e.g., `Z_PK`, `ZTITLE`)
- Junction table columns with numbers (e.g., `Z_5NOTES`, `Z_13TAGS`)
- Timestamps as seconds since 2001-01-01 (Core Data epoch)

#### Normalized Schema Layer

The library provides a normalization layer through automatically-generated Common Table Expressions (CTEs). This abstracts Bear's quirks and provides clean table names:

**Normalized Tables:**
- `notes` - Normalized view of `ZSFNOTE` (all note fields with clean names)
- `tags` - Normalized view of `ZSFNOTETAG` (tag IDs and names)
- `note_tags` - Normalized view of `Z_5TAGS` junction table (note_id, tag_id)
- `note_links` - Normalized view of `ZSFNOTEBACKLINK` (from_note_id, to_note_id)

**Key Transformations:**
1. **Timestamps**: Core Data epoch (2001-01-01) → SQLite datetime format
2. **Column Names**: `Z_PK` → `id`, `ZTITLE` → `title`, `ZTEXT` → `content`, etc.
3. **Boolean Fields**: `ZTRASHED` → `is_trashed`, `ZARCHIVED` → `is_archived`, `ZPINNED` → `is_pinned`
4. **Junction Columns**: Dynamically discovered numbered columns → `note_id`, `tag_id`

#### Schema Discovery

At initialization (`BearDb::new()`):
1. Opens temporary read-only connection
2. Queries `PRAGMA table_info(Z_5TAGS)` to discover junction column names
3. Generates normalizing CTE SQL based on discovered schema
4. Caches CTE string in `BearDb` for all subsequent queries
5. Closes connection

#### Queryable Abstraction

All database operations go through the `Queryable<'a>` wrapper:
- Wraps the raw SQLite `Connection`
- Automatically prepends normalizing CTEs to all queries
- Used by both typed methods (`tags()`, `notes()`, etc.) and generic `query()` API

For complete schema documentation, see `SCHEMA.md` in the repository root.

## Development Commands

### Building
```bash
cargo build          # Debug build
cargo build --release # Release build
```

### Running
```bash
cargo run            # Runs src/main.rs example
```

### Documentation
```bash
cargo doc --no-deps  # Generate library docs
```

### Testing
Currently no tests are implemented. When adding tests, use:
```bash
cargo test           # Run all tests
cargo test test_name # Run specific test
```

## Code Conventions

### Module System
- Uses the modern flat module system (NOT mod.rs files)

### No Standard Output in Library Code
- NEVER use `eprintln!`, `println!`, or similar in lib.rs
- Library code should be silent; output is only acceptable in main.rs or examples

### Connection Safety Rules
1. Never add a persistent connection field to `BearDb`
2. All database operations must go through `with_connection()`
3. The 5000ms busy timeout should not be changed without careful consideration
4. Never remove the read-only flags or query_only pragma
