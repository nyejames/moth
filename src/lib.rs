//! Moth compiler package root.
//!
//! Targeted `#[allow(...)]` attributes are used where needed, each with a justification
//! comment.

pub(crate) mod timing;

/// Features the running binary was built with, in declaration order.
///
/// WHAT: the enabled Cargo features of this compiled `moth` library, as names.
/// WHY:  a machine-readable report that does not name its build configuration cannot be told
///       apart from one produced by a differently configured run of the same command. This is
///       public because `xtask` links this library and its reports describe the same build.
pub const ENABLED_FEATURES: &[&str] = &[
    #[cfg(feature = "timers")]
    "timers",
    #[cfg(feature = "detailed_timers")]
    "detailed_timers",
    #[cfg(feature = "benchmark_counters")]
    "benchmark_counters",
    #[cfg(feature = "checked_blocks")]
    "checked_blocks",
    #[cfg(feature = "async_blocks")]
    "async_blocks",
    #[cfg(feature = "show_tokens")]
    "show_tokens",
    #[cfg(feature = "show_headers")]
    "show_headers",
    #[cfg(feature = "show_ast")]
    "show_ast",
    #[cfg(feature = "show_eval")]
    "show_eval",
    #[cfg(feature = "show_hir")]
    "show_hir",
    #[cfg(feature = "show_codegen")]
    "show_codegen",
    #[cfg(feature = "show_borrow_checker")]
    "show_borrow_checker",
    #[cfg(feature = "boracle")]
    "boracle",
    #[cfg(feature = "boracle_campaign")]
    "boracle_campaign",
];

mod compiler_tests {
    pub(crate) mod integration_test_runner; // For running all integration tests and report back the results

    #[cfg(test)]
    mod config_input_origin_tests;
    #[cfg(test)]
    pub mod test_diagnostics;
    #[cfg(test)]
    pub mod test_fs;
    #[cfg(test)]
    pub mod test_support;
}
pub mod benchmarking;
pub mod build_system;
mod builder_surface;
mod compiler_frontend;
pub mod first_party_js;
/// Rust-only MON data service.
///
/// This is the sole public MON root. It accepts caller-owned UTF-8 MON text and
/// returns owned data values; it never compiles a module, evaluates a Moth
/// program, reads a file, discovers a project, or writes a build output.
/// The reader, writer and prepared-schema owners stay crate-private; the only
/// compiler-owned facts the boundary publishes are the numeric profile
/// selection (`NumericProfile`, `IntWidth`, `FloatPrecision`) consumed by
/// `Schema::with_profile`.
///
/// The accepted operations share one prepared schema:
///
/// - `encode_document` receives a record value and its prepared static
///   schema and returns one complete ordinary UTF-8 MON `String` or a
///   structured failure.
/// - `encode_value` receives any eligible value and its schema and returns
///   its complete literal text, including string quotes or container delimiters,
///   through the same writer.
/// - `decode_document` receives one complete document and its prepared
///   record schema, validates, applies defaults, and returns an owned
///   schema-checked record or the first structured failure.
/// - `decode_document_bytes` behaves like `decode_document` for raw
///   bytes and reports invalid UTF-8 through `MonErrorCode::InvalidUtf8`
///   with a byte span.
/// - `Schema::with_profile` selects the numeric profile a schema prepares
///   under; schemas prepared without it use `NumericProfile::STANDARD`
///   (`Int32`/`Float64`). Fixed widths and `Byte` are profile-independent.
/// - `SchemaType::Integer` preserves arbitrary-precision whole-number data.
///   `SchemaType::Decimal` carries an exact scale in `0..=256` and requires
///   exact representability at that scale. A scale-two target accepts `1.2`
///   and rejects `1.239`. `Integer` and `Decimal` remain MON data families.
/// - Integer schemas and `Byte` require whole-number spelling. Decimal or
///   exponent forms such as `3.0` and `1e3` never decode as them, even when
///   mathematically integral. Binary floats accept whole and decimal forms,
///   round to nearest with ties to even at their own precision and reject
///   non-finite results. Encoding round-trips supported typed values,
///   including `F16`/`F32` bits and signed zero.
/// - Map keys follow one declared schema type: `String`, `Bool`, `Char`,
///   `Int`, any fixed-width integer or `Byte`. Float and exact-decimal keys
///   are rejected. Duplicate decoded keys and duplicate field/variant names
///   fail rather than keeping a first or last value.
///     
/// Encoding a `Value::String` always encodes string data. It never guesses
/// that the text resembles MON and should be inserted raw or decoded. Decoded
/// values outlive and release the input text without a caller-retained backing buffer;
/// the public boundary has no borrowed document view and publishes no partial result on failure.
///
/// Moth-native integration stays out of this boundary: automatic schemas
/// derived from Moth types, the `$mon` directive, source or runtime codec
/// operations and a static MON builder are separate deferred work.
///
/// ```
/// use moth::mon::{
///     Field, MonErrorCode, Schema, SchemaType, Value, Variant, decode_document,
///     decode_document_bytes, encode_document,
/// };
///
/// let schema = Schema::record(vec![
///     Field::required("title", SchemaType::String),
///     Field::with_default("width", SchemaType::Int, Value::Int(1280)),
///     Field::required(
///         "theme",
///         SchemaType::Choice {
///             name: "Theme".to_owned(),
///             variants: vec![
///                 Variant::unit("Light"),
///                 Variant::payload(
///                     "Custom",
///                     vec![Field::required("name", SchemaType::String)],
///                 ),
///             ],
///         },
///     ),
/// ])
/// .prepare()
/// .expect("static save schema is valid");
/// let document = Value::Record(vec![
///     ("title".to_owned(), Value::String("Strategy".to_owned())),
///     (
///         "theme".to_owned(),
///         Value::Choice {
///             qualifier: None,
///             variant: "Light".to_owned(),
///             fields: Vec::new(),
///         },
///     ),
/// ]);
/// let encoded = encode_document(&document, &schema).expect("save encodes");
/// let decoded = decode_document(&encoded, &schema).expect("save decodes");
/// let from_bytes =
///     decode_document_bytes(encoded.as_bytes(), &schema).expect("valid bytes decode");
/// assert_eq!(decoded, from_bytes);
/// drop(encoded);
/// let Value::Record(fields) = decoded else {
///     panic!("root is a record");
/// };
/// assert!(fields.iter().any(|(name, value)| {
///     name == "width" && *value == Value::Int(1280)
/// }));
/// let failure = decode_document_bytes(&[0xff], &schema).expect_err("invalid UTF-8 fails");
/// assert_eq!(failure.code, MonErrorCode::InvalidUtf8);
/// assert!(failure.span.is_some());
/// ```
pub mod mon {
    pub use crate::compiler_frontend::datatypes::numeric_profile::{
        FloatPrecision, IntWidth, NumericProfile,
    };
    pub use crate::compiler_frontend::mon::{
        Field, Limits, MonError, MonErrorCode, PathSegment, PreparedSchema, Schema, SchemaType,
        Span, Value, Variant, decode_document, decode_document_bytes, encode_document,
        encode_value,
    };
}

mod backends {
    pub(crate) mod backend_feature_validation;
    pub(crate) mod error_types;
    pub(crate) mod external_package_validation;
    pub(crate) mod js;
    pub(crate) mod structural_string;
    #[cfg(test)]
    mod tests;
    pub(crate) mod wasm;
}

pub mod projects {
    #[cfg(feature = "boracle")]
    pub(crate) mod boracle;
    pub mod check;
    pub mod cli;
    pub(crate) mod command_status;
    pub mod dev_server;
    pub(crate) mod html_project;
    // Kept intentionally in pre-alpha as the future CLI entrypoint for interactive
    // template experimentation. This remains outside the default command surface.
    pub(crate) mod repl;
    pub(crate) mod routing;
    pub mod settings;
}
