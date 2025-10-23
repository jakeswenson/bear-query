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

### Database Schema

Bear uses Core Data with SQLite persistence. Key tables:
- `ZSFNOTE` - Contains notes (title, content, timestamps, flags)
- `ZSFNOTETAG` - Contains tags
- `Z_5TAGS` - Junction table linking notes to tags
- `ZSFNOTEBACKLINK` - Junction table for note-to-note links

Core Data uses a custom timestamp format (seconds since 2001-01-01), which this library converts to standard Unix timestamps.

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
