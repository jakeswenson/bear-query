# Bear-Query Normalized Schema

This document describes the normalized database schema provided by the `bear-query` library.

## Overview

Bear uses Apple's Core Data framework with SQLite persistence. The underlying schema uses Core Data's conventions:
- Table names prefixed with `Z` (e.g., `ZSFNOTE`, `ZSFNOTETAG`)
- Column names prefixed with `Z` (e.g., `Z_PK`, `ZTITLE`)
- Numbered junction table columns (e.g., `Z_5NOTES`, `Z_13TAGS`)
- Timestamps as seconds since 2001-01-01 (Core Data epoch)

`bear-query` abstracts these quirks through automatically-generated Common Table Expressions (CTEs), providing a clean, normalized schema.

## Normalized Tables

All queries (both typed API methods and generic SQL queries via `query()`) automatically have access to these normalized views:

### `notes` Table

The primary table containing all notes in Bear.

| Column | Type | SQLite Type | Description |
|--------|------|-------------|-------------|
| `id` | Integer | `INTEGER` | Note's primary key (maps to `Z_PK`) |
| `unique_id` | String | `TEXT` | Bear's UUID identifier for the note |
| `title` | String | `TEXT` | Note title |
| `content` | String | `TEXT` | Full note content in Markdown format |
| `modified` | DateTime | `TEXT` | Last modification timestamp (ISO 8601 format) |
| `created` | DateTime | `TEXT` | Creation timestamp (ISO 8601 format) |
| `is_pinned` | Boolean | `INTEGER` | 1 if note is pinned, 0 otherwise |
| `is_trashed` | Boolean | `INTEGER` | 1 if note is in trash, 0 otherwise |
| `is_archived` | Boolean | `INTEGER` | 1 if note is archived, 0 otherwise |

**Source Table:** `ZSFNOTE`

**Example Queries:**

```sql
-- Get 10 most recent non-trashed notes
SELECT title, modified FROM notes
WHERE is_trashed = 0
ORDER BY modified DESC
LIMIT 10;

-- Find notes by keyword
SELECT id, title FROM notes
WHERE content LIKE '%keyword%'
AND is_trashed = 0;

-- Get all pinned notes
SELECT title, created FROM notes
WHERE is_pinned = 1
AND is_archived = 0;
```

### `tags` Table

All tags in Bear's database.

| Column | Type | SQLite Type | Description |
|--------|------|-------------|-------------|
| `id` | Integer | `INTEGER` | Tag's primary key (maps to `Z_PK`) |
| `name` | String | `TEXT` | Tag name (e.g., "work/projects/bear-query") |
| `modified` | DateTime | `TEXT` | Last modification timestamp (ISO 8601 format) |

**Source Table:** `ZSFNOTETAG`

**Example Queries:**

```sql
-- Get all tags
SELECT id, name FROM tags ORDER BY name;

-- Find tags by prefix
SELECT name FROM tags
WHERE name LIKE 'work/%';

-- Count tags
SELECT COUNT(*) as tag_count FROM tags;
```

### `note_tags` Table

Junction table representing the many-to-many relationship between notes and tags.

| Column | Type | SQLite Type | Description |
|--------|------|-------------|-------------|
| `note_id` | Integer | `INTEGER` | Foreign key to `notes.id` |
| `tag_id` | Integer | `INTEGER` | Foreign key to `tags.id` |

**Source Table:** `Z_5TAGS` (numbers may vary across Bear versions)

**Example Queries:**

```sql
-- Get all tags for a specific note
SELECT t.name
FROM tags t
JOIN note_tags nt ON t.id = nt.tag_id
WHERE nt.note_id = ?;

-- Get all notes with a specific tag
SELECT n.title
FROM notes n
JOIN note_tags nt ON n.id = nt.note_id
JOIN tags t ON nt.tag_id = t.id
WHERE t.name = 'work/projects';

-- Tag usage statistics
SELECT t.name, COUNT(*) as note_count
FROM tags t
JOIN note_tags nt ON t.id = nt.tag_id
GROUP BY t.id, t.name
ORDER BY note_count DESC;
```

### `note_links` Table

Represents wiki-style links between notes.

| Column | Type | SQLite Type | Description |
|--------|------|-------------|-------------|
| `from_note_id` | Integer | `INTEGER` | Source note ID (the note containing the link) |
| `to_note_id` | Integer | `INTEGER` | Target note ID (the linked note) |

**Source Table:** `ZSFNOTEBACKLINK`

**Example Queries:**

```sql
-- Get all notes linked FROM a specific note
SELECT n.title, n.modified
FROM notes n
JOIN note_links nl ON n.id = nl.to_note_id
WHERE nl.from_note_id = ?;

-- Get all notes linking TO a specific note (backlinks)
SELECT n.title
FROM notes n
JOIN note_links nl ON n.id = nl.from_note_id
WHERE nl.to_note_id = ?;

-- Find notes with the most outbound links
SELECT n.title, COUNT(*) as link_count
FROM notes n
JOIN note_links nl ON n.id = nl.from_note_id
GROUP BY n.id, n.title
ORDER BY link_count DESC
LIMIT 10;
```

## Core Data Transformations

### Timestamp Conversion

Bear stores timestamps as seconds since **2001-01-01 00:00:00 UTC** (Core Data epoch), not the standard Unix epoch (1970-01-01).

The library automatically converts all timestamps using:

```sql
datetime(ZMODIFICATIONDATE + unixepoch('2001-01-01'), 'unixepoch')
```

This results in standard ISO 8601 formatted datetime strings (e.g., `2024-11-19 23:50:00`).

### Junction Table Column Discovery

The `Z_5TAGS` junction table uses numbered column names that may vary across Bear versions:
- `Z_5NOTES` - References the notes table
- `Z_13TAGS` - References the tags table

These numbers (5, 13) are Core Data's internal entity type IDs and may change. The library discovers the actual column names at initialization using `PRAGMA table_info()`.

### Boolean Fields

SQLite doesn't have native boolean types. Bear uses integers:
- `1` = true
- `0` = false

All boolean fields (`is_pinned`, `is_trashed`, `is_archived`) follow this convention.

## Schema Discovery Process

At initialization (`BearDb::new()`), the library:

1. Opens a temporary read-only connection
2. Queries `PRAGMA table_info(Z_5TAGS)` to discover column names
3. Generates normalizing CTEs based on discovered schema
4. Caches the CTE SQL for all subsequent queries
5. Closes the connection

This ensures the library adapts to schema variations across Bear versions.

## Complete CTE Example

Here's what the generated CTE looks like (simplified):

```sql
WITH
  core_data AS (
    SELECT unixepoch('2001-01-01') as epoch
  ),
  notes AS (
    SELECT
      n.Z_PK as id,
      n.ZUNIQUEIDENTIFIER as unique_id,
      n.ZTITLE as title,
      n.ZTEXT as content,
      datetime(n.ZMODIFICATIONDATE + cd.epoch, 'unixepoch') as modified,
      datetime(n.ZCREATIONDATE + cd.epoch, 'unixepoch') as created,
      n.ZPINNED as is_pinned,
      n.ZTRASHED as is_trashed,
      n.ZARCHIVED as is_archived
    FROM ZSFNOTE as n, core_data as cd
  ),
  tags AS (
    SELECT
      t.Z_PK as id,
      t.ZTITLE as name,
      datetime(t.ZMODIFICATIONDATE + cd.epoch, 'unixepoch') as modified
    FROM ZSFNOTETAG as t, core_data as cd
  ),
  note_tags AS (
    SELECT
      nt.Z_5NOTES as note_id,
      nt.Z_13TAGS as tag_id
    FROM Z_5TAGS as nt
  ),
  note_links AS (
    SELECT
      nl.ZLINKEDBY as from_note_id,
      nl.ZLINKINGTO as to_note_id
    FROM ZSFNOTEBACKLINK as nl
  )
-- Your query goes here
SELECT * FROM notes LIMIT 10;
```

## Usage with the Generic Query API

```rust
use bear_query::BearDb;

let db = BearDb::new()?;

// All queries automatically use normalized schema
let df = db.query("
  SELECT n.title, t.name as tag_name, n.modified
  FROM notes n
  JOIN note_tags nt ON n.id = nt.note_id
  JOIN tags t ON nt.tag_id = t.id
  WHERE n.is_trashed = 0
  AND t.name LIKE 'work/%'
  ORDER BY n.modified DESC
  LIMIT 20
")?;

println!("{}", df);
```

## Important Notes

1. **Read-Only**: All queries are strictly read-only. The library uses multiple safety layers (file flags, pragma, connection mode) to prevent writes.

2. **Short-Lived Connections**: Each query opens and closes a connection. Don't assume connections persist across calls.

3. **No Transactions**: Since connections are short-lived, transaction management isn't supported. Each query is independent.

4. **Bear Compatibility**: This schema is specific to Bear's current Core Data implementation. Future Bear updates may change the underlying schema.

5. **Null Values**: Some fields may be NULL:
   - `tags.modified` can be NULL for tags that haven't been modified
   - `notes.title` can be NULL for untitled notes

## References

- [Bear App](https://bear.app)
- [Core Data Documentation](https://developer.apple.com/documentation/coredata)
- [SQLite Documentation](https://www.sqlite.org/docs.html)
