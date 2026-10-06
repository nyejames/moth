# moth-mon

`moth-mon` provides a standalone Rust codec for literal-only Moth Object Notation (MON). Consumers supply complete UTF-8 text, raw bytes or owned `Value` trees and an explicit schema. The codec returns owned values or encoded text without file I/O, project discovery, compilation or expression evaluation.

Prepare a `Schema` once and reuse its immutable `PreparedSchema` for decoding and encoding. Preparation validates field names, defaults, numeric destinations and receiver limits. Successful values outlive the input. Failures publish no partial result. `MonError` implements `Display` and `std::error::Error` and retains its structured code, path, original-input byte span and detail without holding the input.

`Schema::with_profile` selects any combination of 32/64-bit `Int` and 32/64-bit `Float`. `Uint` follows the profile's `Int` width, so the standalone default uses Int32/Float64 with Uint32. `SchemaType::Uint` and `Value::Uint(u64)` carry the full `u64` payload with whole-number spelling, shared unsigned policy and profile-width validation. Fixed-width integers, F16/F32/F64 and Byte retain their own identities independently of the profile. Exact Integer data and scale-0-through-256 Decimal data keep lossless text rather than requiring another arithmetic runtime.

The codec depends only on `moth-lexical` within the workspace. Use `moth_mon` for independent Rust applications. Compiler clients may use the deliberate `moth::mon` convenience re-export with the same API and Rust types. Shared name rules reject reserved labels, qualifiers and schema names, including leading-underscore and ASCII-case variants.

The crate-level Rust documentation contains the complete, executable profile-selected schema and round-trip example. Run it with `cargo test -p moth-mon --doc`.

Moth-native encode/decode, automatic schema extraction, `$mon` and the static project builder remain undelivered. This Rust codec does not deliver source `{=}`, Unicode escapes or contextual `::Variant` construction.
