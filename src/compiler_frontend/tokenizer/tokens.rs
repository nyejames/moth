//! Tokenizer source-token facade.
//!
//! WHAT: re-exports the schema, storage, cursor and lexer-stream owners under the established
//!       `tokenizer::tokens` API.
//! WHY: callers share one canonical source-token model without coupling to its owner-local modules.

#[path = "cursor.rs"]
mod cursor;
#[path = "schema.rs"]
mod schema;
#[path = "storage.rs"]
mod storage;
#[path = "stream.rs"]
mod stream;

pub use cursor::*;
pub use schema::*;
pub use storage::*;
pub use stream::*;

#[cfg(test)]
#[path = "tests/tokens_remap_tests.rs"]
mod tokens_remap_tests;

#[cfg(test)]
#[path = "tests/token_cursor_tests.rs"]
mod token_cursor_tests;
#[cfg(test)]
#[path = "tests/token_taxonomy_tests.rs"]
mod token_taxonomy_tests;
