//! Rust-only MON literal-data codec for standalone Rust consumers.
//!
//! `moth-mon` owns literal reading, schema preparation, defaults, validation, encoding, owned
//! values, resource limits and public failures. Callers supply complete text or values and a
//! prepared schema. This crate performs no file I/O, project discovery, module compilation or
//! expression evaluation. It depends on `moth-lexical`, not the compiler.
//!
//! The reader and writer operate on one prepared schema:
//!
//! - [`encode_document`] writes a record value as one complete MON document.
//! - [`encode_value`] writes an eligible nested value as complete literal text.
//! - [`decode_document`] validates a complete document, applies defaults, and returns an owned
//!   schema-checked record.
//! - [`decode_document_bytes`] also accepts raw bytes and reports invalid UTF-8 with a byte span.
//! - [`Schema::with_profile`] selects any of the four combinations of 32/64-bit `Int` width and
//!   32/64-bit `Float` precision. `Uint` follows the profile's `Int` width, so the default is
//!   Uint32. Schemas prepared without an explicit profile use
//!   [`NumericProfile::STANDARD`] (`Int32`/`Float64`). Preparation captures the profile and validated
//!   defaults in an immutable [`PreparedSchema`] reusable across reading and writing calls.
//!   Fixed widths and `Byte` remain profile-independent and distinct from `Int`, `Uint` and `Float`.
//! - [`SchemaType::Integer`] preserves arbitrary-precision whole-number text. [`SchemaType::Decimal`]
//!   carries an exact scale in `0..=256`; a scale-two target accepts `1.2` and rejects `1.239`.
//!   These Rust data-family names do not introduce source types or numeric constructors.
//! - Integer schemas and `Byte` require whole-number spelling. Binary floats round to nearest with
//!   ties to even at their own precision and reject non-finite results.
//! - Map keys follow one declared schema type: `String`, `Bool`, `Char`, `Int`, `Uint`, a
//!   fixed-width integer, or `Byte`. Duplicate decoded keys and duplicate field or variant
//!   names fail rather than keeping a first or last value.
//!
//! Encoding a [`Value::String`] always encodes string data. It never guesses that text resembles
//! MON and should be inserted raw or decoded. Decoded values own their trees and outlive input text.
//! Failures never publish partial results. [`MonError`] implements [`std::fmt::Display`] and
//! [`std::error::Error`] while retaining its structured code, path, original-input byte span and detail
//! without retaining the input. Receiver [`Limits`] bound text, nesting, nodes and materialisation.
//! Shared identifier rules reject reserved labels, qualifiers and schema names, including
//! leading-underscore and ASCII-case variants. Invalid supplied names report
//! [`MonErrorCode::InvalidIdentifier`]. Schema names report [`MonErrorCode::InvalidSchema`]
//! and invalid default names or values report [`MonErrorCode::InvalidDefault`] during preparation.
//! Resource-budget failures retain their budget-specific codes, including during default validation.
//!
//! Compiler clients can use the same API and Rust types through the deliberate `moth::mon`
//! convenience re-export. Moth-native operations, automatic schemas derived from Moth types,
//! the `$mon` directive and a static MON builder remain outside this API.
//!
//! # Prepare once, reuse with typed data
//!
//! ```
//! use moth_mon::{
//!     Field, FloatPrecision, IntWidth, MonErrorCode, NumericProfile, Schema, SchemaType,
//!     Value, decode_document, decode_document_bytes, encode_document,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let profile = NumericProfile {
//!     int_width: IntWidth::Bits64,
//!     float_precision: FloatPrecision::Bits32,
//! };
//! let schema = Schema::record(vec![
//!     Field::with_default("title", SchemaType::String, Value::String("Strategy".into())),
//!     Field::required("count", SchemaType::Int),
//!     Field::required("ratio", SchemaType::Float),
//!     Field::required("serial", SchemaType::U64),
//!     Field::required("channel", SchemaType::Byte),
//!     Field::required("half", SchemaType::F16),
//!     Field::required("single", SchemaType::F32),
//!     Field::required("price", SchemaType::Decimal { scale: 2 }),
//! ])
//! .with_profile(profile)
//! .prepare()?;
//!
//! let input = String::from(
//!     "count = 3_000_000_000, ratio = 0.5, serial = 18446744073709551615, \
//!      channel = 255, half = 0.5, single = 0.25, price = 12.34",
//! );
//! let decoded = decode_document(&input, &schema)?;
//! drop(input);
//! assert_eq!(
//!     decoded,
//!     Value::Record(vec![
//!         ("title".into(), Value::String("Strategy".into())),
//!         ("count".into(), Value::Int(3_000_000_000)),
//!         ("ratio".into(), Value::Float(0.5)),
//!         ("serial".into(), Value::U64(u64::MAX)),
//!         ("channel".into(), Value::Byte(255)),
//!         ("half".into(), Value::F16(0.5)),
//!         ("single".into(), Value::F32(0.25)),
//!         ("price".into(), Value::Decimal("12.34".into())),
//!     ]),
//! );
//! let encoded = encode_document(&decoded, &schema)?;
//! assert_eq!(decode_document_bytes(encoded.as_bytes(), &schema)?, decoded);
//! assert_eq!(schema.profile(), profile);
//!
//! // Reserved words remain invalid even with different case and leading underscores.
//! let failure = decode_document("__LoOp = 1", &schema).expect_err("reserved label");
//! assert_eq!(failure.code, MonErrorCode::InvalidIdentifier);
//! println!("{failure}");
//! let invalid_schema = Schema::record(vec![Field::required("_U64", SchemaType::Int)]);
//! assert_eq!(
//!     invalid_schema.prepare().expect_err("reserved schema field").code,
//!     MonErrorCode::InvalidSchema,
//! );
//! # Ok(())
//! # }
//! ```

mod budget;
mod error;
mod map_keys;
mod model;
mod numeric;
mod reader;
mod schema;
mod writer;

#[cfg(test)]
#[path = "tests/uint_tests.rs"]
mod uint_tests;

use budget::BudgetState;
use map_keys::{MapKeyIndex, map_key_name, map_key_name_len};
use numeric::{fixed_scalar_to_value, materialize_fixed_value};

pub use budget::Limits;
pub use error::{MonError, MonErrorCode, PathSegment, Span};
pub use model::{Field, PreparedSchema, Schema, SchemaType, Value, Variant};
pub use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};
pub use reader::{decode_document, decode_document_bytes};
pub use writer::{encode_document, encode_value};
