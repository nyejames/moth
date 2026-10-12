# moth-mon

`moth-mon` provides a standalone Rust codec for literal-only Moth Object Notation (MON). Consumers supply complete UTF-8 text, raw bytes or owned `Value` trees and an explicit schema. The codec returns owned values or encoded text without file I/O, project discovery, compilation or expression evaluation.

Prepare a `Schema` once and reuse its immutable `PreparedSchema` for decoding and encoding. Preparation validates field names, defaults, numeric destinations and receiver limits. Successful values outlive the input. Failures publish no partial result. `MonError` implements `Display` and `std::error::Error` and retains its structured code, path, original-input byte span and detail without holding the input.

Fixed-width integers (`I8`, `I16`, `I32`, `I64`, `U8`, `U16`, `U32`, `U64`), `F16`, `F32`, `F64` and `Byte` each name one exact scalar identity. `SchemaType::U64` and `Value::U64(u64)` carry the full `u64` payload with whole-number spelling. `SchemaType::Integer` retains arbitrary-precision whole-number text and `SchemaType::Decimal { scale }` retains exact decimal text at a declared scale from 0 through 256. These Rust data families add no Moth source types or arithmetic runtime.

Numeric schemas require no profile selection or source inference. Fixed integer and `Byte` schemas require whole-number spelling and check their exact ranges. Unsigned schemas reject negative spelling, including `-0`. Decimal schemas reject inexact scale fitting rather than rounding. Binary-float literals round directly to the declared precision using nearest, ties-to-even and reject non-finite results, while preserving subnormals and signed zero.

Programmatic values and defaults must match their schema variants without implicit conversion. `Value::I32(1)` cannot satisfy `SchemaType::I64`. The float variants use `f64` carriers, and validation rounds a matching carrier at that variant's precision. The carrier never selects the schema's type. The [MON reference](../../docs/src/docs/mon/mon-format.mtf) owns the literal-data contract and the [numeric reference](../../docs/src/docs/numbers/numeric-types.mtf) owns Moth source numeric identities.

The codec depends only on `moth-lexical` within the workspace. Use `moth_mon` for independent Rust applications. Compiler clients may use the deliberate `moth::mon` convenience re-export with the same API and Rust types. Shared name rules reject reserved labels, qualifiers and schema names, including leading-underscore and ASCII-case variants.

The crate-level Rust documentation contains the complete, executable schema and round-trip example. Run it with `cargo test -p moth-mon --doc`.

Moth-native encode/decode, automatic schema extraction, `$mon` and the static project builder remain undelivered. This Rust codec does not deliver source `{=}`, Unicode escapes or contextual `::Variant` construction.
