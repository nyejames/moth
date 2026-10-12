# moth-lexical

`moth-lexical` owns the identifier, reserved-word and numeric-text rules shared by the compiler and standalone MON codec. It depends on neither consumer.

`moth_lexical::identifier` checks Unicode identifier shape separately from user-name reservation. Exact source spelling stays case-sensitive without Unicode normalisation. Reservation removes leading underscores and checks whole names without regard to ASCII case, including fixed numeric names and the Dec family with any ASCII-digit suffix. `moth_lexical::words` exposes one neutral source-word inventory without compiler token tags or naming-style diagnostics.

`moth_lexical::is_line_break` shares the logical line boundary used by source
and MON: LF and CR are the only line breaks, with CRLF forming one break.
Other Unicode whitespace remains horizontal trivia.

`moth_lexical::numeric` accepts literal text and caller-selected destination facts. It owns shared parsing, materialisation and formatting for fixed-width integers, Byte, F16/F32/F64, plus exact Dec/DecN scale-text facts from 0 through 256. Callers pass the exact destination scalar, and MON schemas name the exact scalar of each field.

Numeric destination facts never come from a semantic profile. Whole-number text materialises directly at the requested exact range, including the full `U64` range. Decimal/exponent spelling cannot satisfy an integer destination. Binary-float text rounds directly to the requested precision using nearest, ties-to-even, with finite-result checks and no default `F64` value followed by narrowing. The `f64` carrier preserves completed `F16` and `F32` values without changing their scalar identity.

The compiler owns source inference and the fixed `I32`/`F64` defaults. MON owns explicit schemas and its lossless `Integer` and `Decimal` data families. This crate selects neither. See the [numeric reference](../../docs/src/docs/numbers/numeric-literals.mtf) for source semantics and the [MON reference](../../docs/src/docs/mon/mon-format.mtf) for schema-directed data.

The crate provides no file I/O, document parsing, compiler token storage, type lookup, expression evaluation, arbitrary-precision coefficient arithmetic or numeric runtime. Its numeric operations return structured failure reasons for consumers to project into their own error boundary. Immutable prepared schemas, owned document values and `MonError` with `Display`/`std::error::Error` belong to `moth-mon`.

The crate-level Rust documentation shows identifier checks and numeric text parsing. Run its example with `cargo test -p moth-lexical --doc`.
