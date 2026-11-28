# Moth Tree-sitter grammars

Status: design only. The parser and queries have not been implemented.

This is the shared syntax owner for the official editor integrations. Read the [design](../design.md) and [roadmap](../roadmap.md) before adding grammar rules.

## Planned layout

- Shared JavaScript grammar rules for lexical forms, records, collections, maps and template structure.
- Root grammar definitions for `moth`, `moth_template` and `mon`. A separate Moth Markdown grammar may be needed after the injection prototype.
- Generated C parser sources and node metadata, with a small external scanner only if required by the tested grammar.
- Shared query sources for highlighting, language injections and structural features.
- Tree-sitter corpus and highlight tests, plus a reproducible parser-generation and validation entry point.

Settle the exact multi-grammar directory layout and build tooling in M1. Do not create three copied grammars or commit generated files from an unpinned toolchain. Compile the same sources for the native and Wasm consumers.

## Contracts

The editor parses authored syntax, not inferred types or resolved symbols. It remains useful on incomplete or invalid input. The compiler and MON codec decide validity. Template body text, literal code listings and nested Moth contributions must retain distinct ranges.

Use the delivered canonical language syntax and compiler lexical inventory at activation. No pre-migration compatibility grammar. Prove recovery, Unicode handling, incremental edit equivalence and capture boundaries through [shared tests](../tests/README.md).

Where editor query dialects differ, keep the shared structural pattern in this project and adapt it at the consuming boundary. Pin the generated node schema, parser ABI and dependent queries together.
