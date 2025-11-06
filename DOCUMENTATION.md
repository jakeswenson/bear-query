# Documentation Update Checklist

Use this checklist when making any significant changes to the library to ensure all documentation stays in sync.

## Core Documentation Files

### `src/lib.rs` (Main Library Documentation)
- [ ] Top-level module documentation (`//!` comments at file start)
- [ ] Normalized schema tables section (if schema-related changes)
- [ ] Example code in module docs
- [ ] Method documentation with examples
- [ ] Safety guarantees section

### `README.md` (User-Facing Documentation)
- [ ] Basic example in "Usage" section
- [ ] API Reference section (list of types and methods)
- [ ] Example queries if schema changed
- [ ] Installation instructions if dependencies changed
- [ ] Safety notes if guarantees changed

### `SCHEMA.md` (Schema Reference)
- [ ] Table column descriptions
- [ ] Example queries for each table
- [ ] Core Data transformation notes
- [ ] Complete CTE example at end
- [ ] Schema discovery process if changed

## Type-Specific Documentation

### When Adding New Types
- [ ] Add type to "Core Types" section in README.md
- [ ] Add comprehensive doc comments with examples
- [ ] Export type in `src/lib.rs` (make it public)
- [ ] Add usage examples in module docs

### When Adding New Methods
- [ ] Add method to "Methods" section in README.md
- [ ] Add doc comments with parameter descriptions
- [ ] Add usage examples in doc comments
- [ ] Consider adding example to `examples/` directory

### When Changing Behavior
- [ ] Update affected method documentation
- [ ] Update examples that demonstrate the behavior
- [ ] Update README.md if user-facing
- [ ] Add migration notes if breaking change

## Quick Checks

After making changes, verify:
- [ ] `cargo doc --no-deps` builds without warnings
- [ ] `cargo test --doc` passes (tests code in doc comments)
- [ ] Examples in README.md are copy-paste ready
- [ ] All code examples use correct syntax and types

## Documentation Style Guide

### Code Examples
- Use `no_run` for examples that need real Bear database
- Use proper error handling (`Result<(), BearError>`)
- Keep examples concise but complete
- Show realistic use cases

### Descriptions
- Start with a brief one-line summary
- Explain what the function does, not how it works internally
- Document edge cases and NULL handling
- Include "See also" references to related functions

### SQL Examples
- Use normalized table names (`notes`, not `ZSFNOTE`)
- Show practical queries users would actually write
- Include comments explaining non-obvious parts
- Keep queries formatted and readable
