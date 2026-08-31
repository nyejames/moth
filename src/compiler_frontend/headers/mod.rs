//! Header parsing stage modules.
//!
//! WHAT: extracts file-level declarations, dependency clauses and start-function boundaries before AST build.
//! Header parsing also owns top-level symbol collection (`module_symbols`), so dependency sorting
//! and AST construction receive a pre-built symbol package without a separate manifest stage.

pub(crate) mod binding_environment;
mod const_fragments;
mod constant_dependencies;
mod dependency_canonicalization;
pub(crate) mod dependency_clause_syntax;
mod dependency_paths;
pub(crate) mod dependency_target;
mod file_dependency_clauses;
mod file_parser;
mod file_state;
mod hash_items;
mod header_dispatch;
pub(crate) mod module_symbols;
pub(crate) mod moth_template_prepare;
mod ordering_hints;
pub(crate) mod parse_file_headers;
pub(crate) mod plain_markdown_prepare;
mod public_exports;
mod start_capture;
mod symbol_collection;
pub(crate) mod synthetic_content_header;
mod top_level_classifier;
mod trait_headers;
mod types;
