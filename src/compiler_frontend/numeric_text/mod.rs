//! Compiler-owned numeric token storage and `StringTable` adapters.
//!
//! Numeric spelling, normalized fixed-width parsing, neutral scalar facts and float formatting
//! live in `moth_lexical`; this module keeps the token payload and compiler-specific adapters.

pub mod parse;
pub mod store;
pub mod token;
