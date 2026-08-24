# Diagnostics

Looking for: source the compiler correctly accepts or rejects, reported badly - wrong lane, wrong identity, missing context, unhelpful wording or a cascade.

If the legality outcome itself is wrong, that is Correctness.

**Evidence bar:** the invalid condition, the current diagnostic, the expected one, and the owner with enough context to construct it.

## Lane selection

- Source, config, dependency, type, rule, borrow and target-contract failures use `CompilerDiagnostic`.
- Impossible compiler states and infrastructure failures use `CompilerError`.
- Filesystem errors are classified by whether the user can correct project input.
- User input never reaches `panic!`, `todo!`, `unreachable!` or a user-data-driven `.unwrap()`.
- Deferred-feature rejection stays distinct from outside-design-scope rejection.
- A diagnosed module result contains no partial interface.

A wrong lane that also changes the compilation outcome needs a linked Correctness finding.

## Stable identity

- One code identifies one durable semantic family; an existing code is not reused for a different failure.
- Typed payloads and reason enums carry the cause - not pre-rendered strings.
- `TypeId`, symbols and package IDs stay structured until rendering.
- Wording can improve without changing identity.
- Repeated occurrences keep exact multiplicity.

Watch for catch-all codes hiding materially different corrections.

## Construction and context

- Constructed at the owner with the best semantic context, through one typed constructor rather than near-identical prose in several callers.
- Renderers never infer semantic meaning from message text.
- Consumers do not reopen source or an earlier IR just to build a better error.
- Every user-facing diagnostic has a useful `SourceLocation`. A file-level span is acceptable only when no narrower owner exists, and the report should say why.
- Interned paths stay interned until rendering; render context outlives the diagnostics that need it.
- Diagnostic data is remapped before later consumers read it. Wrong attribution from remapping is also a Correctness finding.

## Message content

States what failed in user terms, names the relevant token, symbol, type or capability, distinguishes expected from found, and explains why rather than only that. Suggests a correction when one is stable and unambiguous. Avoids internal compiler names and indexes, avoids "invalid syntax" when the owner knows the specific rule, and does not promise unaccepted behaviour.

## Conflicts and related labels

For duplicate, collision, visibility, alias and cross-file errors: all materially relevant declarations labelled, the primary location on the thing the user should change first, secondary labels identifying origins without drowning the message, and deterministic order. A complete-collision contract must not silently pick one conflict and hide the rest.

## Recovery and cascades

- The stage continues only where remaining work can be trusted.
- A diagnosed provider blocks its consumers without repeating its root error.
- One malformed construct does not cascade into unrelated errors - but genuinely independent failures stay visible rather than over-suppressed.
- Warnings are retained only on successful artefacts where the architecture requires it, with their own stable identity and deterministic emission.

## Command and target consistency

`check`, `build` and `dev` share semantic diagnostic identities where they share validation. Target failures occur before lowering. Build-system context adds project information without replacing the underlying compiler diagnostic identity.

## Smells

`format!` strings in semantic payloads; ad hoc error text from low-level helpers; `CompilerError` for user-authored failures; source diagnostics without locations; broad generic "invalid" variants; rendered type names stored instead of IDs; cloned `PathBuf`s in payloads; renderers branching on codes to recreate meaning; diagnostics merged in hash-map or completion order; placeholder messages.

## Tests

Read them for the baseline. Weak assertions - "must fail" rather than an exact code, or a missing multiplicity or location check - become a linked Tests finding.

## Stale when

Diagnostic identity, construction ownership, source-location handling, recovery behaviour or warning policy changes materially.
