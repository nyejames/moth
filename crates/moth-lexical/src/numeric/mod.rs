//! Shared numeric spelling and neutral scalar facts.
//!
//! `grammar` and `parse` own numeric spelling and destination materialisation; `binary16` and
//! `format` own finite binary-float conversion and formatting. The remaining leaves hold the small
//! scalar, profile, precision and decimal facts those operations share. Compiler token stores,
//! type lookup and arbitrary-precision decimal values stay above this module.

pub mod binary16;
pub mod decimal;
pub mod fixed_scalar;
pub mod format;
pub mod grammar;
pub mod parse;
pub mod precision;
pub mod profile;
