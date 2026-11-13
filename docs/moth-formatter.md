# Moth Formatter Design

> **Future design direction.** This document records accepted design decisions for the official Moth source formatter, including its keyboard-accessibility input alternatives. It defines the intended contract independently of current implementation status.
>
> This is a design authority, not a roadmap item or an implementation plan. Formatting style, internal structure and command-line details remain open where this document does not settle them.

Moth keeps one canonical source spelling for each language construct. The official formatter will make that source easier to write and maintain without introducing alternative syntax into the language. In particular, it will convert a small, reserved set of keyboard-accessibility characters into canonical punctuation.

This document owns formatter policy and the reserved input-alternative mapping. The [language references](developer-docs/language/overview.mtf) continue to own valid source syntax, the [compiler design](compiler-design-overview.md) owns compilation and diagnostics, and the [build-system design](build-system-design.md) owns command orchestration. An implementation plan will resolve the remaining technical details when formatter work begins.

## Goals and boundaries

The official formatter should:

- Produce consistent, canonical Moth source independent of keyboard layout or editor.
- Preserve the meaning of valid code and leave authored literal content intact.
- Produce identical output when run repeatedly on already formatted source.
- Reuse the compiler's authoritative syntax and tokenisation infrastructure.
- Work as built-in tooling without requiring an editor extension, keyboard remapping or third-party formatter.
- Allow explicit formatting and future opt-in automatic invocation without slowing ordinary compilation.

Source formatting and template output formatting have different responsibilities. The source formatter edits authored `.moth` and `.mtf` files. Compiler directives such as `$md`, `$raw` and `$code` format or interpret *template output*. They do not control source-code layout or keyboard normalisation.

The formatter does not add compiler syntax, change runtime semantics or require every project to adopt one editor workflow. Initial source-style rules, such as indentation, line wrapping and comment layout, remain separate design work.

## Keyboard-accessibility input alternatives

### Canonical source rule

Moth accepts only its canonical punctuation in compilable source. Keyboard alternatives are reserved **formatter inputs**, not operators, token aliases or additional valid source spellings.

The formatter recognises an alternative in a syntactic position, replaces it with the designated canonical character and writes canonical source. Normal compilation never performs this conversion, including when building or checking a project that has not opted into formatting.

The accepted mapping for formatter V1 is:

- `§` (section sign, U+00A7) becomes `~` for mutability and exclusive access. It helps layouts where the section sign is easier to reach than tilde.
- `¡` (inverted exclamation mark, U+00A1) becomes `~`. It provides an accessible choice on Spanish layouts where neither tilde nor section sign is convenient.
- `°` (degree sign, U+00B0) becomes `^` for exponentiation. It helps layouts where caret is a dead key, notably German keyboards.

All three alternatives are confirmed V1 design requirements unless an explicit later decision supersedes them. They are reserved from future Moth syntax to protect their use by official tooling. The mapping is fixed by the language/toolchain design, not inferred from the user's operating system or keyboard layout.

For example, the formatter converts these invalid input spellings:

```moth
count §= 0
¡count += 1
power = 2 ° 8
```

Into the only valid source spellings:

```moth
count ~= 0
~count += 1
power = 2 ^ 8
```

### Syntactic and literal contexts

Reservation applies to syntactic positions, not to every occurrence of a character in a source file. The formatter must preserve ordinary content in:

- Quoted strings and character literals.
- Source comments.
- Literal template bodies, including nested and implicitly authored `.mtf` template content.

Template heads, interpolated expressions and other code-bearing regions still follow the canonical source rule. Their boundaries must come from the same lexical rules the compiler uses. A blind whole-file character replacement would corrupt documentation, text and code examples embedded in templates.

For example, formatting `count §= 0` in code must not change `"See §4"` or the text `See §4` inside a template body.

### Compiler diagnostics

The normal compiler rejects reserved alternatives when they appear where syntax is expected. The existing lexical or syntax error path should issue a specific diagnostic that:

- Identifies the encountered alternative and its canonical replacement.
- Explains that the character is reserved for formatter input, not valid Moth syntax.
- Suggests running the official formatter to convert it.
- Points to the authored character with the usual source location information.

These diagnostics belong to ordinary compiler error handling. They do not authorise compilation of the alternative character and must not require a separate source scan or normalisation pass during ordinary compilation.

### Formatting incomplete source

Keyboard normalisation should remain useful while a file contains unrelated syntax errors. The formatter should be able to convert safely identifiable alternatives without first requiring a fully valid program.

Recovery must respect strings, comments and template boundaries. When malformed source makes the boundary uncertain, the formatter should leave the ambiguous region unchanged and explain why, rather than risk altering literal content. The exact recovery strategy and how it interacts with general formatting remain open design work.

## Formatter behaviour

### Source stability

For valid Moth source, formatting preserves program semantics. The formatter must respect syntax-sensitive operator spacing, block delimiters, comments and template content. It must not treat template text as ordinary code or change authored string content while improving code layout.

Formatting must be deterministic and idempotent. After one successful run, another run with the same settings produces no further changes. Input alternatives always produce the same canonical result as typing the canonical character directly.

These requirements constrain future style decisions without choosing a particular line width, indentation scheme, blank-line policy or comment-reflow policy now.

### Compiler infrastructure reuse

The formatter should share the compiler's lexical definitions and source-context recognition, particularly the distinction between code, quoted content and template bodies. It should avoid a separate Moth grammar, a competing source scanner or independent interpretation of template structure.

The formatter must also tolerate the deliberately invalid alternative characters and incomplete edits. Whether this uses a tolerant tokenisation mode, retained source tokens or another compiler-owned interface remains an implementation design question. The chosen interface must preserve source locations and UTF-8 correctness when replacements change byte lengths.

### Toolchain integration

The intended user-facing entry point is `moth fmt`. Running it explicitly performs ordinary source formatting and keyboard normalisation together. Keyboard normalisation is part of the standard formatter contract rather than a separate required flag.

Ordinary `moth check`, `moth build` and development compilation leave source files untouched and do not invoke formatting by default. Later tooling may offer explicit opt-in formatting on save or before compilation, reusing the same official formatter. Where a build requests pre-compilation formatting, it should compile the resulting canonical source, rather than privately accepting noncanonical source in memory.

The precise command flags, configuration scope, editor integration and write/check modes remain open. Automatic invocation must never impose scanning or rewriting work on the default compilation path.

## Deferred design decisions

The following questions stay open for later formatter design work:

- **Source style:** indentation, line breaks, wrapping, comment placement and the amount of existing layout to preserve.
- **Partial or malformed input:** the recovery contract for ordinary formatting and safe keyboard normalisation in incomplete files.
- **Tooling surface:** path selection, check-only operation, configuration, formatter-on-save integration and opt-in pre-build execution.
- **`|` accessibility:** European layouts can make pipes cumbersome despite their frequent use in Moth parameters, choices and captures. Evaluate whether a reserved input alternative would materially help new programmers.
- **Backslash (`\`):** a potential compact-keyboard alternative for `|` or `~`, with `|` worth considering first. Backslash already has a defined role in quoted-string escapes and literal template text. **It is not reserved as an alternative.** Reopen only after checking keyboard coverage and lexical safety.
- **Other punctuation:** revisit further alternatives only when keyboard evidence and usage frequency justify the extra reservation. The three accepted V1 mappings do not imply a general-purpose user-configurable character substitution system.

Supporting 40% and smaller keyboards through additional syntax alternatives is outside this design's goals. Those keyboards typically require layers for many common programming characters, making a language-specific replacement catalogue an unsuitable solution.

This document records accepted policy. Future implementation plans should turn the settled rules into precise behaviour and tests without silently deciding the deferred questions.
