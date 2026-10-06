# moth-lexical

`moth-lexical` owns the identifier, reserved-word and numeric-text rules shared by the compiler and standalone MON codec. It depends on neither consumer.

`moth_lexical::identifier` checks Unicode identifier shape separately from user-name reservation. Exact source spelling stays case-sensitive without Unicode normalisation. Reservation removes leading underscores and checks whole names without regard to ASCII case, including fixed numeric names and the Dec family with any ASCII-digit suffix. `moth_lexical::words` exposes one neutral source-word inventory without compiler token tags or naming-style diagnostics.

`moth_lexical::is_line_break` shares the logical line boundary used by source
and MON: LF and CR are the only line breaks, with CRLF forming one break.
Other Unicode whitespace remains horizontal trivia.

`moth_lexical::numeric` accepts literal text and caller-selected destination facts. It owns shared parsing, materialisation and formatting for fixed-width integers, Byte, F16/F32/F64 and profile-dependent Int/Uint/Float, plus exact Dec/DecN scale-text facts from 0 through 256. All four combinations of 32/64-bit Int and 32/64-bit Float remain supported, with Int32/Float64 as the default. Compiler/build policy selects compilation profiles and MON schemas capture their caller-selected profile.

The crate provides no file I/O, document parsing, compiler token storage, type lookup, expression evaluation, arbitrary-precision coefficient arithmetic or numeric runtime. Its numeric operations return structured failure reasons for consumers to project into their own error boundary. Immutable prepared schemas, owned document values and `MonError` with `Display`/`std::error::Error` belong to `moth-mon`.

The crate-level Rust documentation shows identifier checks and numeric text parsing. Run its example with `cargo test -p moth-lexical --doc`.
