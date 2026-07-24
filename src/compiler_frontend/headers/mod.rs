//! Header parsing stage modules.
//!
//! WHAT: extracts file-level declarations/imports and start-function boundaries before AST build.
//! Header parsing also owns top-level symbol collection (`module_symbols`), so dependency sorting
//! and AST construction receive a pre-built symbol package without a separate manifest stage.

mod const_fragments;
mod constant_dependencies;
mod dependency_canonicalization;
mod file_imports;
mod file_parser;
mod file_state;
mod hash_items;
mod header_dispatch;
pub(crate) mod import_environment;
mod imports;
pub(crate) mod module_symbols;
pub(crate) mod moth_template_prepare;
mod ordering_hints;
pub(crate) mod parse_file_headers;
pub(crate) mod plain_markdown_prepare;
mod public_exports;
mod start_capture;
mod symbol_collection;
mod synthetic_content_header;
mod top_level_classifier;
mod trait_headers;
mod types;
