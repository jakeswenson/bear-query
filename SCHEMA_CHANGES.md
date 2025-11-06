# Schema Change Checklist

Use this checklist when making changes to the normalized schema (column names, table structure, ID types, etc.).

## 1. Schema Generation (`src/schema.rs`)
- [ ] Update `generate_normalizing_cte()` function with new column mappings
- [ ] Update test assertions in `test_generate_normalizing_cte()`
- [ ] Verify CTE syntax is valid SQL
- [ ] Update `setup_test_schema()` if test data structure needs to change

## 2. Query Code (`src/lib.rs`)
- [ ] Update all `SELECT` statements in typed methods:
  - [ ] `tags()`
  - [ ] `notes()`
  - [ ] `search()`
  - [ ] `note_links()`
  - [ ] `note_tags()`
  - [ ] `get_note_by_id()`
- [ ] Update all `WHERE` clauses that reference changed columns
- [ ] Update all `JOIN` conditions that reference changed columns
- [ ] Verify generic `query()` method examples in doc comments

## 3. Model Construction (`src/models.rs`)
- [ ] Update `note_from_row()` to read from correct column names
- [ ] Update `tag_from_row()` if tag schema changed
- [ ] Update struct field mappings if needed

## 4. Documentation
- [ ] Update inline doc comments in `src/lib.rs`:
  - [ ] Normalized schema tables section (near top of file)
  - [ ] Method examples that show SQL queries
- [ ] Update `SCHEMA.md`:
  - [ ] Table column descriptions for all affected tables
  - [ ] Example queries in each table section
  - [ ] Complete CTE example at the end of the file
- [ ] Update `README.md` if user-facing behavior changed

## 5. Examples
- [ ] Check `examples/query_demo.rs` for hardcoded column names in SQL queries
- [ ] Check `examples/null_analysis.rs` for hardcoded column names in SQL queries
- [ ] Update any other example files in `examples/`

## 6. Tests
- [ ] Run `cargo nextest run` - all tests must pass
- [ ] Run `cargo test --doc` - all doc tests must pass
- [ ] Run `cargo build --examples` - all examples must compile

## 7. Build Verification
- [ ] Run `cargo build --release` - must succeed without errors
- [ ] Run `cargo doc --no-deps` - documentation must build without warnings
- [ ] Check for any deprecation warnings in output

## Common Pitfalls

### Junction Tables
When note or tag ID formats change, remember to update:
- `note_tags` table (links notes to tags)
- `note_links` table (links notes to notes)

These often use subqueries to resolve internal IDs to public IDs.

### Example Files
Easy to miss since they're not part of the main library code. Always check:
- `examples/*.rs` files
- Search for old column names like `unique_id`, `Z_PK`, etc.

### Documentation SQL
SQL queries in documentation comments must be updated:
- Inline `///` doc comments with SQL examples
- README.md examples
- SCHEMA.md examples

### Test Assertions
Tests may pass but still assert on outdated values:
- Check test output for old column names
- Update assertions to match new schema structure
