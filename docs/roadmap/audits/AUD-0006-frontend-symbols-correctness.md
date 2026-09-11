# AUD-0006: Frontend symbol and path interning correctness

- State: `complete`
- Kind: `Correctness`
- Area: `frontend.symbols` - `src/compiler_frontend/symbols/**`: string interning with fork/merge/freeze, complete-path interning (`PathId`, `PathInternerBuilder`, `PathTable`), `InternedPath`, identifier and reserved-name policy, compiler-owned symbol preseeding, and dependency identities
- Coverage: `complete`
- Reviewed: 2026-09
- Baseline: `main` at `455c24de7`. The worktree carries uncommitted changes unrelated to this area - `src/first_party_js.rs`, `src/projects/html_project/external_js/parser/**`, `xtask/src/first_party_deps*`, `docs/src/developer-docs/style-guide/validation.mtf` - plus this report and its log entries. Nothing inspected here was modified, and the binary used for evidence was built from that tree. `just validate` was **not** run: this is a read-only audit and no gate is claimed. Evidence commands were `cargo build` (debug, clean) and `./target/debug/moth check` against throwaway fixtures under `tmp/`, since removed. Active plan touching the area: the data-layout plan's Phase 1 is delivered on main and Phase 2 (`Genuine complete-path interning`) has not started, so `InternedPath` coexistence, the absent path-interner delta/remap and `try_intern_portable_path`'s dead-code allowance are accepted deferral rather than defects. Confidence limit: findings about lanes and identities were verified by running the compiler; the parallel merge ordering was verified by reading, not by a determinism stress run.

## What was inspected

Read in full:

- `src/compiler_frontend/symbols/string_interning.rs` (673 lines): `StringId`, `StringTableResolver`, `StringIdRemap`, `StringTableBase`, `StringTableForkSource`, `StringTableFork`, `FrozenStringTable`, `StringTable` and its manual `Clone`
- `src/compiler_frontend/symbols/interned_path.rs`, `identifier_policy.rs`, `compiler_symbols.rs`, `identity.rs`
- `src/compiler_frontend/symbols/path_interner/{mod,id,builder,frozen}.rs`
- `src/compiler_frontend/symbols/tests/{string_interning_tests,compiler_symbols_tests,interned_path_tests,identifier_policy_tests}.rs` and `path_interner/tests.rs` as the executable baseline

Read as context, not audited: `src/compiler_frontend/source/frozen_identity.rs`, `src/build_system/create_project_modules/{module_preparation,source_preparation,module_inventory,compiled_boundary}.rs`, `src/build_system/create_project_modules/compilation/{canonical,single_file}.rs`, `src/compiler_frontend/compiler_messages/{compiler_errors,diagnostic_bag}.rs`, `src/compiler_frontend/ast/module_ast/environment/declaration_table.rs`, `src/compiler_frontend/keywords.rs`, `src/compiler_frontend/tokenizer/lexer.rs` identifier paths, `src/build_system/build.rs` bootstrap.

Consumer traces were delegated to four read-only agents covering fork/freeze/merge callers, `StringIdRemap` consumers, `PathId` consumers and `InternedPath` consumers. Every claim reused below was re-verified against the cited source before being filed; one delegated claim was wrong and is corrected under `Checked and clean`.

Executed evidence (throwaway fixtures under `tmp/`, debug binary):

- user-authored `start` function, root file, valid body
- user-authored `start` function in a non-root helper file
- `start = 1` and `start ~= 1`
- `this`, `Error`, `message`, `code` as authored function and variable names
- Unicode identifiers `café` and `naïve_value`
- multi-file module with a diagnosed file, to exercise the chunk-merge diagnosed lane
- the existing fixture `tests/cases/this_local_declaration_error`

## Authorities read

- `AGENTS.md` core contracts and failure-lane rule
- `docs/compiler-data-layout-design.md`: `Hard representation invariants`, `Compilation identity context`, `Genuine path interning`, `Deterministic parallel construction`, `Failure architecture`
- `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`: current slice status, Phase 1 standing contracts (path foundation 1A, registration 1B, spans 1C), Phase 2 slices and exit criteria
- `docs/src/docs/progress/@page.moth`: `Entry-selected module roots and implicit start` and `Structured diagnostics` rows
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`: naming conventions, top-level work becoming dormant `start`
- `docs/src/docs/project-structure/entry-runtime-and-fragments.mtf`: compiler-synthesised start, helper files have no independent start
- `docs/roadmap/audit-kinds/correctness.md`, `docs/roadmap/audit-guide.md`

## Existing findings and active plans checked

- Open findings AUD-0005-F01/F02/F03 are tokenizer-owned (`<`/`>` spacing, bare CR, Unicode numerics). F02 below concerns Unicode *identifiers* in the naming-policy owner, not numeric scanning, so it is a distinct root cause rather than a duplicate of AUD-0005-F03.
- AUD-0002-F02 and AUD-0002-F03 already corrected per-module base copying by hoisting one `StringTableForkSource`/`ModulePreparationContext` above the directory loop. That correction holds: `module_preparation.rs:544` and `source_preparation.rs:206` build one fork source per batch. `StringTable::fork_for_module` (the inherent method that builds a throwaway fork source per call) has exactly one production caller left, `compilation/single_file.rs:338`, where the lane creates a single fork, so no repeated base copy occurs. Not re-filed.
- AUD-0002-F04 added the counters the area now increments; no finding here duplicates it.
- Data-layout Phase 2 owns `PathId` becoming the only complete-path identity, `InternedPath` deletion, and module-local path deltas with remaps. Nothing below asks for that work.

## Findings

### AUD-0006-F01: A user-authored root-level `start` function reaches the compiler-invariant lane and is reported as a compiler bug

- State: `fixed`
- Kind: `Correctness`

#### Evidence

`CompilerSymbolSet::preseed` interns the compiler-owned symbol set, `IMPLICIT_START_FUNC_NAME` (`"start"`) first, at `src/compiler_frontend/symbols/compiler_symbols.rs:67`. Nothing in the area's reserved-name policy reserves that name against authored declarations. `identifier_policy.rs:33-40` checks only `RESERVED_KEYWORD_SHADOWS` (`src/compiler_frontend/keywords.rs:13-18`), whose list does not contain `start`, and the builtin-name reservation reached from `headers/symbol_collection.rs:121-125` covers only `Error` (`builtins/error_type.rs:22-25`).

The consequence, reproduced on `main` at `455c24de7` with the debug binary:

```text
$ cat tmp/aud0006b/src/@page.moth
start ||:
    io.line("hello")
;
$ ./target/debug/moth check tmp/aud0006b

Compiler Bug 🔥 ヽ༼☉ ‿ ⚆༽ﾉ 🔥
public default synchronization: duplicate emitted function declaration at path InternedPath { components: [StringId(72), StringId(0)] }; the emitted AST must not contain two nodes with the same path
```

`StringId(0)` is the preseeded `start` symbol; `StringId(72)` is the root source component. `moth build` prints the identical message. The message originates at `src/compiler_frontend/ast/module_ast/finalization/normalize_ast.rs:428-438` as a `CompilerError::compiler_error`, propagates through `synchronize_normalized_public_defaults` (`:154-162`), is wrapped as `TemplateNormalizationError::Infrastructure` (`:1328-1343`), and is rendered under the `Compiler Bug` display name by `compiler_messages/display_messages.rs:82-91`.

The frontend has a declaration-table exception that anticipates this exact collision and states the intended outcome, at `src/compiler_frontend/ast/module_ast/environment/declaration_table.rs:188-196`:

```rust
// An authored function named `start` shares the implicit start path. Preserve the
// trailing-path shadow so body validation can emit the authored source diagnostic.
let shadows_authored_start = compiler_owned.kind == CompilerOwnedDeclarationKind::Start
    && existing.is_some_and(|existing| existing.index() < semantic_len);
if existing.is_some() && !shadows_authored_start {
```

That promised source diagnostic is delivered for the value forms and missing for the function form:

| Authored form | Observed result |
|---|---|
| `start ~= 1` | `MOTH-RULE-0038` Shadowed name, exit 1 |
| `start = 1` | `MOTH-RULE-0044` Invalid assignment target, exit 1 |
| `start \|\|:` with a valid body, root file | `Compiler Bug`, exit 1 |
| `this` as function or variable | `MOTH-RULE-0040`, exit 1 |
| `Error` as function | `MOTH-RULE-0027`, exit 1 |
| `Error` as variable | `MOTH-RULE-0039`, exit 1 |

#### Counter-explanation tested

**"Authored `start` is deferred, so this is not a defect."** Rejected. `docs/src/docs/progress/@page.moth:120-125` records the row `Entry-selected module roots and implicit start` as `Supported` and states the contract explicitly: "The implicit `start` function is build-system-owned and not user-bindable or callable." A supported, stated contract is being enforced through the wrong lane, not deferred.

**"The `Compiler Bug` banner is the intended rejection."** Rejected on two independent grounds. `docs/src/docs/progress/@page.moth:133` states "User-facing failures use typed `CompilerDiagnostic` payloads with stable codes ... Infrastructure and invariant failures use `CompilerError`", matching `AGENTS.md` and `docs/compiler-data-layout-design.md` `Failure architecture`. And `declaration_table.rs:189` names the intended outcome as "the authored source diagnostic", which the sibling value forms actually produce.

**"The duplicate-declaration check is a genuine invariant, so `CompilerError` is right."** Rejected as an explanation of the defect, not of the check. The invariant "the emitted AST must not contain two nodes with the same path" is real and worth keeping; it is being violated by unvalidated user input, so the missing upstream rejection is the defect. The check is the detector.

**"An existing test already covers this, so it must be accepted behaviour."** Rejected. `tests/cases/this_local_declaration_error/input/src/@page.moth` does declare `start ||:`, and its `expect.toml` requires exactly `["MOTH-RULE-0040"]`. Running it confirms it passes. It masks this path because its body `this = 1` fails body validation before AST emission; the collision is only reachable when the authored `start` body is otherwise valid. That fixture is therefore evidence the case is untested, not evidence it is supported.

**"A helper-file `start` is the same defect."** Rejected. `./target/debug/moth check` on a non-root helper file declaring `start ||:` exits 0 with no diagnostic, but `docs/src/docs/project-structure/entry-runtime-and-fragments.mtf:37-43` states helper files have no independent `start`, so no implicit `start` exists there to collide with and an ordinary function of that name is legal. Recorded under `Checked and clean` rather than filed.

Disproof would be a canonical document stating that a root-level authored `start` function is an internal invariant breach rather than a user error, or a progress-matrix row moving the implicit `start` out of `Supported`.

#### Violated contract or cost

Three at once:

- `AGENTS.md`: "User-authored failures use structured `CompilerDiagnostic` values with useful source context. `CompilerError` is for internal invariants, filesystem, tooling and backend infrastructure failures."
- `docs/compiler-data-layout-design.md` `Failure architecture` Lane 1 versus Lane 3, and `Adding a compiler bug check` ("A new `compiler_bug!` site must name the proven invariant and the stage that promised it").
- `docs/src/docs/progress/@page.moth:125`: the implicit `start` is not user-bindable, which is currently unenforced for the function form.

The user cost is a message with no source span, no diagnostic code, no indication which line is at fault, and an explicit claim that the compiler is broken, for an ordinary naming mistake.

#### Root owner

The declared-name validation owner, `src/compiler_frontend/headers/symbol_collection.rs` `validate_declared_name` (`:107-136`), which already rejects keyword shadows, `Error` and core trait names and is the stage that could reject this with source context. The reserved-name policy it should consult is split across this audit's area: `symbols/identifier_policy.rs` owns shadow and naming policy, and `symbols/compiler_symbols.rs` owns the set of compiler-owned names including `start`. `normalize_ast.rs:428` is the detector, not the owner; `declaration_table.rs:188-196` is a consumer whose stated expectation is unmet.

This finding sits on the `frontend.symbols` / `frontend.headers` boundary. The area registered by this run owns the reserved-symbol set and the identifier policy; it does not own the header stage that must apply them.

#### Suggested correction

Non-authorising. Give the compiler-owned symbol set a reserved-declaration predicate beside the existing keyword and builtin predicates, and consult it in `validate_declared_name` so an authored root-level declaration named `start` produces a typed `CompilerDiagnostic` with the authored span. Whether that reuses `RuleDiagnosticKind::ReservedBuiltinName`, adds a reason to `ReservedNameOwner`, or introduces a distinct compiler-owned-entry-point identity is a triage and implementation decision. Whether the reservation applies only in selected module roots, where the implicit `start` exists, is part of that decision.

#### Fix scope and preserved invariants

Bounded to reserved-name validation plus its diagnostic identity. Preserve: the `normalize_ast.rs` emitted-path uniqueness invariant, unchanged; the existing `MOTH-RULE-0038` and `MOTH-RULE-0044` outcomes for the value forms; `declaration_table.rs`'s trailing-shadow exception, or its deletion if a header-stage rejection makes it unreachable, in the same change; legality of an authored `start` in a helper file; the preseed order in `CompilerSymbolSet::preseed`, since changing it shifts every fixed symbol ID.

#### Required validation

`just validate`, plus a `tests/cases/` fixture for a root-level authored `start` function with a valid body asserting the new diagnostic code, and a check that `tests/cases/this_local_declaration_error` still yields exactly `MOTH-RULE-0040`.

#### Linked findings

AUD-0006-F06 (Tests lane): the only existing authored-`start` fixture cannot reach this path.

#### Triage record

2026-09-11 — **Accepted.** Reproduced: a root-level `start ||:` with a valid body still reports
`MOTH-INFRA-0001` / Compiler Bug. Authorised fix: `validate_declared_name` rejects an authored
declaration named `IMPLICIT_START_FUNC_NAME` only when `FileRole` is `ActiveModuleRoot`; skip
`HeaderKind::StartFunction`; emit `ReservedNameCollision` (`MOTH-RULE-0039`) with a new
`ReservedNameOwner` variant for the implicit start, not `ReservedBuiltinName`. Preserve helper-file
legality and the `start ~= 1` / `start = 1` diagnostics. Delete the `declaration_table.rs`
authored-start shadow exception if header rejection makes it unreachable. Add the valid-body
`tests/cases/` fixture required by F06. Compare resolved name text; do not plumb `CompilerSymbolIds`.

2026-09-11 — **Accepted and resolved.** Active-root authored `start` now reports `MOTH-RULE-0039`
(`ReservedNameOwner::ImplicitStart`, "compiler-owned entry point") from `validate_declared_name`.
`HeaderKind::StartFunction` is skipped. Helper-file `start` remains legal. `start ~= 1` stays
`MOTH-RULE-0038` and `start = 1` stays `MOTH-RULE-0044`. The `this_local_declaration_error`
wrapper was renamed to `example` so it still reports exactly `MOTH-RULE-0040`. The unread
`CompilerOwnedDeclaration.kind` field was deleted with `CompilerOwnedDeclarationKind`.
`just validate` passed. Phase audit returned clean.

### AUD-0006-F02 (linked, Diagnostics lane): ASCII-only naming predicates warn on legal Unicode identifiers, telling the user to use the convention they already used

- State: `fixed`
- Kind: `Diagnostics`

#### Evidence

The tokenizer accepts Unicode identifiers. `src/compiler_frontend/keywords.rs:160-168`:

```rust
pub(crate) fn is_identifier_continue(char: char) -> bool {
    char.is_alphanumeric() || char == '_'
}

pub(crate) fn is_valid_identifier(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|char| char.is_alphabetic() || char == '_')
```

`char::is_alphabetic` and `char::is_alphanumeric` are Unicode predicates, and `lexer.rs:1276` dispatches identifiers on `current_char.is_alphabetic()`.

The naming policy in this area tests ASCII only. `symbols/identifier_policy.rs:58-77` (`is_lowercase_with_underscores_name`) accepts `is_ascii_lowercase`, `is_ascii_digit` and `'_'` and returns `false` for anything else; `:47-55` (`is_camel_case_type_name`) requires `is_ascii_uppercase` then `is_ascii_alphanumeric`; `:83-99` (`is_uppercase_constant_name`) is ASCII-only in the same way.

Observed:

```text
$ cat tmp/aud0006c/src/@page.moth
café = "x"
naïve_value = "y"
io.line(café)
io.line(naïve_value)
$ ./target/debug/moth check tmp/aud0006c

Warning ⚠️
Identifier naming convention
  [MOTH-RULE-0021]
  --> @page.moth:1:6
  1 | café = "x"
  Identifier 'café' should use lowercase_with_underscores
```

The ASCII control `plain_value = "x"` reports `No errors or warnings`. `café` and `naïve_value` are lowercase with underscores; the warning asks for a convention they satisfy.

#### Counter-explanation tested

**"Moth deliberately restricts identifiers to ASCII, so the warning is the intended nudge."** Rejected. No canonical document states an ASCII restriction, and the tokenizer deliberately uses Unicode predicates in two places; an ASCII-only language would reject the identifier at `is_valid_identifier` rather than accept it and warn. The warning text also does not say "use ASCII" - it names a convention the identifier already follows, so even under that reading the wording is wrong.

**"This is AUD-0005-F03 again."** Rejected. AUD-0005-F03 concerns Unicode numeric characters diagnosed as `_` separator errors in the tokenizer's numeric scanner. This is the naming-convention predicate in `symbols/identifier_policy.rs` on an accepted identifier. Different owner, different input class.

**"It is only a warning, so it is not worth filing."** Not a counter-explanation to the defect. `MOTH-RULE-0021` is `DiagnosticSeverity::Warning` (`compiler_messages/diagnostic_kind_descriptors.rs:294-297`), so legality is unaffected - which is exactly why this is Diagnostics and not Correctness. The cost is a warning that cannot be satisfied, on projects whose fixtures are configured `warnings = "forbid"`.

Disproof would be a canonical statement that Moth identifiers are ASCII-only, which would move the defect to the tokenizer's acceptance instead.

#### Violated contract or cost

`docs/roadmap/audit-kinds/diagnostics.md` `Message content`: a diagnostic "states what failed in user terms" and "does not promise unaccepted behaviour". Here the stated rule and the tested rule differ, so the message is unactionable. The predicate doc comments at `identifier_policy.rs:60-61` and `:85-86` also describe the rule as "lowercase letters/digits/underscores only" and "uppercase letters/digits/underscores only" without the ASCII qualifier the code enforces.

#### Root owner

`src/compiler_frontend/symbols/identifier_policy.rs`, the module that centralises naming policy for header parsing and AST binding creation.

#### Suggested correction

Non-authorising. Decide the identifier character contract once, in the canonical language reference, then make the tokenizer and the naming predicates agree with it. Either the predicates accept Unicode lowercase/uppercase where the tokenizer does, or the tokenizer rejects non-ASCII identifiers and the naming warning becomes unreachable for them. Do not add a second identifier character rule beside the tokenizer's.

#### Fix scope and preserved invariants

Bounded to the three naming predicates and, if the contract needs recording, a linked Documentation finding for the language reference. Preserve: `MOTH-RULE-0021` identity and severity; all current ASCII outcomes, including the existing keyword-shadow behaviour verified by `symbols/tests/identifier_policy_tests.rs`; `is_camel_case_type_name`'s rejection of underscores in type names.

#### Required validation

`just validate`, plus predicate-level cases for a Unicode lowercase identifier, a Unicode-leading type name and a Unicode uppercase constant, asserting the decided contract.

#### Linked findings

None. Independent of F01.

#### Triage record

2026-09-11 — **Accepted.** Reproduced: `café` and `naïve_value` still warn `MOTH-RULE-0021` asking
for `lowercase_with_underscores` they already satisfy. No canonical ASCII identifier restriction
exists; the tokenizer already accepts Unicode identifiers. Authorised fix: align the three naming
predicates with Unicode `is_lowercase` / `is_uppercase` / `is_alphanumeric`, keep camelCase
rejecting underscores, preserve `MOTH-RULE-0021` and all current ASCII outcomes.

2026-09-11 — **Accepted and resolved.** The three naming predicates now use Unicode letter case
and `is_alphanumeric`, still reject underscores in camelCase, and keep ASCII outcomes.
`café` / `naïve_value` no longer warn. `BuildInputName` shares the snake-case helper, so `café`
is now a valid build-input name. `just validate` passed.

### AUD-0006-F03 (linked, Redundancy lane): `StringIdRemap::has_non_identity_after` has no production caller and its only consumers are the tests that pin it

- State: `fixed`
- Kind: `Redundancy`

#### Evidence

`src/compiler_frontend/symbols/string_interning.rs:126-143` defines `pub fn has_non_identity_after(&self, base_len: usize) -> bool`. A repository-wide search for the name returns the definition and two test assertions only: `symbols/tests/string_interning_tests.rs:53` and `:100`. No production module calls it. Its sibling `is_identity` is used widely in production (`module_preparation.rs:626,628,661,676,690`; `compiler_errors.rs:939,1001`; `headers/types.rs:1443,2040`; `compilation/canonical.rs:1022`; `compilation/single_file.rs:522`; `module_compilation/generated/materialisation.rs:340`), so the fast-path predicate that is actually load-bearing is a different method.

#### Counter-explanation tested

**"It is a documented API for a planned consumer."** Rejected: unlike `try_intern_portable_path`, which carries `#[allow(dead_code)] // Phase 2 migrates semantic path producers to this table.`, this method has no deferral marker, and no plan slice in `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md` names a consumer for it.

**"It is reachable through a trait or dynamic dispatch a name search would miss."** Rejected: `StringIdRemap` is a concrete struct with no trait implementations beyond the derived `Debug`/`Clone`, so all calls are by name.

Disproof would be a production call site, or a plan slice that names it as a prerequisite.

#### Violated contract or cost

`AGENTS.md`: "Deletion over addition ... Does the code you're about to write need to exist at all?" and `docs/roadmap/audit-kinds/redundancy.md` dead structure. The concrete cost is that two tests assert behaviour no consumer depends on, so they constrain future changes to the remap representation for no protected contract.

#### Root owner

`src/compiler_frontend/symbols/string_interning.rs`.

#### Suggested correction

Non-authorising. Delete the method and the two assertions that only exercise it, keeping the surrounding fork/merge assertions those tests also make. If a consumer is genuinely planned, record it in the owning plan slice instead of retaining an untethered API.

#### Fix scope and preserved invariants

Preserve `is_identity` and the cached `is_identity` field it reads, `StringIdRemap::get` semantics, and the remaining assertions of `fork_without_new_strings_produces_identity_delta_remap` and `overlapping_module_forks_remap_diverging_local_suffixes`.

#### Required validation

`just validate`.

#### Linked findings

None.

#### Triage record

2026-09-11 — **Accepted.** No production caller. Delete the method and the two assertions that only
exercise it. Keep `is_identity` and the surrounding fork/merge assertions.

2026-09-11 — **Accepted and resolved.** `has_non_identity_after` and its two pinning assertions
were deleted. `is_identity` and the surrounding fork/merge tests remain. `just validate` passed.

### AUD-0006-F04 (linked, Redundancy lane): `path_interner/frozen.rs` defines a private duplicate of the crate's `StringTableResolver` trait

- State: `fixed`
- Kind: `Redundancy`

#### Evidence

`src/compiler_frontend/symbols/string_interning.rs:46-92` defines `pub(crate) trait StringTableResolver` with `resolve` and `try_resolve`, and implements it for `StringTable`, `Box<StringTable>` and `FrozenStringTable`. It is the crate-wide resolver seam, used by roughly forty render and display functions under `compiler_messages/render/**` and `datatypes/display.rs`, and by `InternedPath::to_path_buf`, `name_str` and `to_portable_string`.

`src/compiler_frontend/symbols/path_interner/frozen.rs:145-159` declares a second, private trait of the same name with the same method, and re-implements it for the same two concrete types:

```rust
trait StringTableResolver {
    fn resolve(&self, id: StringId) -> &str;
}

impl StringTableResolver for StringTable { ... }
impl StringTableResolver for FrozenStringTable { ... }
```

`render_portable_with` at `:127` is generic over the private trait. Behaviour is identical: both `resolve` paths reach the same checked lookup and panic on an invalid ID.

#### Counter-explanation tested

**"The local trait narrows the surface deliberately, since path rendering needs only `resolve`."** Rejected as a justification for a duplicate name and duplicate impls. The crate trait is already `pub(crate)` and in the same module tree, one directory up; using it removes two impls and the shadowing name at no cost to the narrowing, because `render_portable_with` can bound on the crate trait and simply not call `try_resolve`. `AGENTS.md` requires searching "the current owner, adjacent stages" before adding an abstraction and sharing "only identical behaviour with a clear owner" - the behaviour here is identical and the owner is the sibling module.

**"The two traits differ, so they are not duplicates."** Rejected: the crate trait's `try_resolve` carries `#[allow(dead_code)] // Retained for deferred checked lookups across identity tables.`, so the only method with live consumers is `resolve`, which both declare identically.

Disproof would be a behavioural difference between the two `resolve` implementations, or a cyclic dependency preventing `path_interner` from importing from `string_interning` - which it already does at `frozen.rs:10-12`.

#### Violated contract or cost

`AGENTS.md`: "Before adding a helper, pass, type, registry, validator, module or test abstraction, search the current owner, adjacent stages, backend paths and tests." Two same-named traits in one module tree make every reader of `frozen.rs` check which one is in scope.

#### Root owner

`src/compiler_frontend/symbols/path_interner/frozen.rs`.

#### Suggested correction

Non-authorising. Import `StringTableResolver` from `string_interning` and delete the private trait and its two impls, keeping `render_portable_with` generic.

#### Fix scope and preserved invariants

Preserve `render_portable` and `render_portable_frozen` signatures and their panic behaviour on an invalid `StringId`, and the `path_interner/tests.rs` rendering expectations.

#### Required validation

`just validate`.

#### Linked findings

None.

#### Triage record

2026-09-11 — **Accepted.** Import the crate `StringTableResolver` and delete the private trait and
its two impls. Keep `render_portable_with` generic.

2026-09-11 — **Accepted and resolved.** `frozen.rs` now imports the crate `StringTableResolver`.
The private duplicate trait and impls are gone. `render_portable_with` stays generic.
`just validate` passed.

### AUD-0006-F05 (linked, Redundancy lane): the typed `CompilerSymbolIds` accessors have no production consumer

- State: `fixed`
- Kind: `Redundancy`

#### Evidence

`symbols/compiler_symbols.rs:21-31` defines `CompilerSymbolIds` with eight typed fields, documented at `:17-19` as existing so "callers receive this struct so they can refer to fixed symbols by field name instead of by raw StringId", and `:44-47` bundles it into `PreseededStringTable`.

Production has one preseed site and it discards the struct, at `src/build_system/build.rs:1890-1894`:

```rust
let preseeded = CompilerSymbolSet::preseeded_table(FILE_MIN_UNIQUE_SYMBOLS_CAPACITY);
let mut string_table = preseeded.string_table;
// The bootstrap path only needs the preseeded table today. File-local preparation will keep
// these typed IDs alongside its local outputs once fixed-symbol IDs are consumed directly.
let _compiler_symbol_ids = preseeded.compiler_symbol_ids;
```

The only consumers of the fields are `symbols/tests/compiler_symbols_tests.rs`. Consumers that need these names re-intern them by string; for example `identifier_policy.rs` resolves names through `string_table.resolve` and the reserved-keyword list is textual.

The module's stated purpose at `:5-7` is that "parallel tokenization and header parsing need stable IDs for fixed language/compiler names without sharing a mutable global table", with each local table getting the same prefix and identical IDs. Production does not depend on that cross-table property: there is one root table, and every fork inherits the preseeded prefix through the shared `Arc<StringTableBase>` (`string_interning.rs:353-360`, `:558-570`), so inherited IDs are identity by construction rather than by independent preseeding. The invariant asserted by `two_independently_preseeded_tables_assign_same_ids` has no production consumer.

#### Counter-explanation tested

**"The comment reserves it for planned file-local preparation."** Partly accepted, and that is why this is filed as a low-severity candidate rather than a defect: the intent is documented at the call site. It still merits triage because the plan does not own it. Searching `docs/roadmap/plans/`, `docs/compiler-design-overview.md` and `docs/compiler-data-layout-design.md` for `preseed`, `CompilerSymbolIds`, `fixed-symbol` and `compiler-owned symbol` returns no slice that names consuming preseeded symbol IDs, so the only record of the plan is an inline comment.

**"The preseed itself is dead."** Rejected. The preseeded *table* is load-bearing: it is the root table for the whole build, and its prefix is what forks inherit. Only the typed ID struct is unconsumed. Any correction must keep the preseed and its ordering.

Disproof would be a production consumer of any `CompilerSymbolIds` field, or a plan slice naming the future consumer.

#### Violated contract or cost

`AGENTS.md` deletion-over-addition and `docs/roadmap/audit-kinds/redundancy.md` unearned layers. The cost is small and mostly comprehension: a documented stability invariant and a test asserting it, protecting a property no production path uses, which invites a future reader to preserve independent-table ID stability at a real cost.

#### Root owner

`src/compiler_frontend/symbols/compiler_symbols.rs`, with the discarding call site at `src/build_system/build.rs:1894`.

#### Suggested correction

Non-authorising. Either record the future consumer in the owning plan slice and keep the struct, or reduce the API to what production uses - preseeding a table in a fixed order - and delete the typed ID struct, `PreseededStringTable` and the independent-table stability test with it. Triage decides which; both are legitimate.

#### Fix scope and preserved invariants

Preserve the preseed call at `build.rs:1890`, the exact interning order in `CompilerSymbolSet::preseed` (changing it shifts every fixed symbol ID, and `StringId(0)` is relied on by the F01 evidence path), and the fork-inheritance behaviour.

#### Required validation

`just validate`.

#### Linked findings

AUD-0006-F06 shares the observation that no harness test drives the production preseeded table.

#### Triage record

2026-09-11 — **Accepted.** No production consumer and no plan slice names one. Reduce the API to
preseeding a table in fixed order: delete `CompilerSymbolIds` and `PreseededStringTable`, return
`StringTable` from `preseeded_table`, keep intern order, and drop the independent-table stability
test that only pinned the unused IDs. Do not keep the struct as a home for sniffer `intern("this")`
consolidation.

2026-09-11 — **Accepted and resolved.** `preseeded_table` returns `StringTable`. `CompilerSymbolIds`
and `PreseededStringTable` are gone. Intern order is unchanged. `just validate` passed.

### AUD-0006-F06 (linked, Tests lane): no Rust harness test drives the frontend with a preseeded string table, and the only authored-`start` fixture cannot reach F01

- State: `closed`
- Kind: `Tests`

#### Evidence

Production always compiles against a preseeded root table (`build.rs:1890`). Every Rust harness entry into `compile_project_frontend` passes a bare, unpreseeded `StringTable::new()`: `src/build_system/tests/compile_project_frontend_tests/package_materialisation_tests.rs:39,155,241,320,395,551,637,844,978,1119,1238,1308`; `provider_sources_tests.rs:345,413,476,564,630,692,767,814,873,920`; `single_file_directory_discovery_tests.rs:17,83,121,150,193,243,320`; `source_identity_loading_tests.rs:35,108,160,210,274,336,407,586,642,711,766`. Two of them intern a single sentinel first (`package_materialisation_tests.rs:1120` interns `"preexisting-global-name"`, `single_file_directory_discovery_tests.rs:18` interns `"preexisting"`), so they exercise a non-empty table but never the production prefix. The result is that no harness test observes production's fixed-symbol ID layout, and `symbols/tests/compiler_symbols_tests.rs` exercises the preseed only in isolation.

Separately, `tests/cases/this_local_declaration_error/input/src/@page.moth` is the repository's only fixture declaring `start ||:`, and its body `this = 1` fails body validation first, so it asserts `["MOTH-RULE-0040"]` and returns before AST emission. No fixture or Rust test declares an authored `start` with a valid body, which is why F01 is unprotected. `symbols/tests/compiler_symbols_tests.rs:45-65` asserts only that a user string differs from the preseeded IDs, never that an authored declaration colliding with a compiler-owned name is rejected.

#### Counter-explanation tested

**"`tests/cases/` integration fixtures run the real binary, so production preseeding is covered end to end."** Accepted for the fixture suite and it is why this finding is scoped to the Rust harness. The gap that remains is real: the Rust harness tests are the ones that inspect `StringId` values, remaps and table lengths directly, and those are the assertions that would catch a preseed-order or prefix-inheritance regression. They currently run against a table shape production never produces.

**"Passing a bare table is a deliberate simplification with no consequence."** Rejected for the F01 case at least, which is demonstrably unprotected, and unproven for the identity assertions, since a preseed-order change would leave the harness green.

Disproof would be a harness test that constructs its table through `CompilerSymbolSet::preseeded_table`, or a fixture asserting the rejection of an authored compiler-owned name.

#### Violated contract or cost

`docs/roadmap/audit-kinds/tests.md` and `AGENTS.md` testing rules: coverage should protect observable behaviour under the correct owner. A stated stability invariant and a stated language contract are both unprotected.

#### Root owner

The frontend test harness under `src/build_system/tests/compile_project_frontend_tests/`, plus `tests/cases/manifest.toml` for the fixture.

#### Suggested correction

Non-authorising. Route harness table construction through `CompilerSymbolSet::preseeded_table` so the Rust tests share production's prefix, and add one `tests/cases/` fixture for a root-level authored `start` function with a valid body once F01 fixes its diagnostic identity. Do not add the fixture before F01 is triaged: pinning today's `Compiler Bug` output would entrench the defect.

#### Fix scope and preserved invariants

Preserve every existing assertion; if preseeding shifts an asserted `StringId` value, the assertion should move to a resolved-name comparison rather than a numeric one. Preserve `tests/cases/this_local_declaration_error`'s exact expectation.

#### Required validation

`just validate`.

#### Linked findings

AUD-0006-F01 (the untested path), AUD-0006-F05 (the unconsumed preseed invariant).

#### Triage record

2026-09-11 — **Split.** The missing authored-`start` valid-body fixture is accepted as part of F01.
The proposed rewrite of ~51 `compile_project_frontend` harness sites to `preseeded_table` is
**rejected**: those tests have no numeric `StringId` assertions, `tests/cases/` already runs
production preseeding, and `compiler_symbols_tests` already pin the preseed prefix.

## Checked and clean

**Fork, merge and freeze pairing.** `merge_delta_from` (`string_interning.rs:603-658`) assumes IDs below `base_len` resolve identically in donor and target. All five production sites establish that pairing from one snapshot: `module_preparation.rs:544-545` binds `fork_source()` and `base_len()` from the same table and merges at `:625` into that identical table; `source_preparation.rs:205-208,228` does the same, interning source files *before* the fork so they fall inside the snapshot; `module_inventory.rs:1091,1144-1145` carries one snapshot's `base_len` per job through reordering into `config_boundary.rs:56` and `compilation/canonical.rs:1013-1014`; `compilation/single_file.rs:338-339,514` interns into the root between fork and merge, but only by appending, which the prefix assumption tolerates. No fork is merged into a non-origin root; mixed-origin tables use whole-donor `merge_from` (`build.rs:224`, `compiler_errors.rs:756`). Debug builds re-verify the prefix per merge at `string_interning.rs:608-612`.

**Delegated claim corrected.** The fork/freeze trace reported `StringTable::freeze` as having zero production callers. That is wrong: `FrozenIdentityContext::from_parts` calls `strings.freeze()` at `src/compiler_frontend/source/frozen_identity.rs:125`, reached in production from `compiled_boundary.rs:1264` and `compiler_errors.rs:612`. The release `assert!(self.base.is_none(), "StringTable::freeze requires a merged root table")` is therefore live in production, not unreachable. The invariant holds at both sites: `compiled_boundary.rs:1265` freezes `messages.string_table`, the root vessel whose table is base-free, and fork-derived tables displaced by `mem::take` (`module_preparation.rs:727,759`) travel as `PremergeDiagnosticBatch` donors and are consumed by `merge_from`, whose `other.iter()` (`string_interning.rs:537-552`) handles a shared base correctly. I probed the diagnosed chunk-merge lane directly with a two-file module whose helper has a syntax error: it renders `MOTH-SYNTAX-0018` with a source snippet and does not panic. No reachable path to the assert was found, so nothing is filed; the residual risk is that the guard is an `assert!` rather than a typed failure, which `docs/compiler-data-layout-design.md` `Failure architecture` Lane 3 will re-home when `compiler_bug!` exists - the macro is absent from the workspace today.

**Determinism and merge order.** `StringTable` is never mutated across threads: rayon sites (`module_preparation.rs:801-818`, `:833-849`) give each worker a private fork sharing only the immutable `Arc<StringTableBase>`. Merges are serial in canonical order - `sort_by_key(chunk_index)` at `module_preparation.rs:616-625`, wave iteration with `ready.sort_by_key(module_id.index())` at `canonical.rs:930,991-1014`, and deterministic Kahn waves at `project_module_graph.rs:593-636`. No ID is allocated from a timing-dependent atomic, satisfying `Deterministic parallel construction`.

**Remap application.** Remaps are applied exactly once per consumer, before publication, at every boundary: `module_preparation.rs:660-674` remaps `ChunkLocal` payloads then freezes; `source_preparation.rs:228-239` and `headers/parse_file_headers.rs:191-199` remap before returning; `canonical.rs:1017-1035` and `single_file.rs:518-531` remap before `publish_module_and_generated`. `AlreadyGlobal` payloads are never remapped, guarded by the domain enum at `module_preparation.rs:111-122`. No double-remap path is live. `StringIdRemap::get` indexes `mapped_suffix` without a bounds check (`string_interning.rs:112-119`), and no production path was found that can hand it an out-of-domain ID: donor structures carry only donor-local IDs, and the public interface is correctly excluded from remapping because it carries no donor-local identities (`public_interface/model.rs:6-7,360-362`). `InternedPath::try_remap_string_ids` (`interned_path.rs:139-148`) is exhaustive over components.

**Path table.** `PathId` is `NonZeroU32` with the required niche, asserted by `path_interner/tests.rs:7-13` (4 bytes, `Option<PathId>` 4 bytes, `PathNode` 8 bytes), matching `Hard representation invariants`. `PathId::try_from_index` (`id.rs:26-31`) rejects the full `u32` domain edge and one past it without a lossy cast, covered by `path_interner/tests.rs:180-194`. `try_append_child` (`frozen.rs:41-56`) computes the child ID and depth before mutating, so exhaustion leaves no partial node. Interning is fallible end to end: `try_intern_filesystem_path` and `try_intern_portable_path` return `PathInternError::{NonUtf8, TableFull}` rather than panicking. Component semantics are exact - empty segments and trailing separators produce distinct identities (`path_interner/tests.rs:126-158`), preserving the 1A contract that portable semantic spellings keep their separators. `try_intern_portable_path` remains production-unreachable behind its Phase 2 `allow(dead_code)`, which the plan owns.

**`PathId` table pairing.** Multiple path tables are live in one build: the only production `PathInternerBuilder` is the one `SourceDatabaseBuilder` owns (`source/database.rs:109`), frozen at `finish` (`:139`), so each project and package `SourceDatabase` issues `PathId`s in its own domain. Nothing compares them across domains. `PathId` is stored only as `SourceSlot::logical_path` (`source/record.rs:103`), and the sole production consumer is `compiler_messages/render/context.rs:187-190`, which reads `frozen.source_logical_path(source)` and renders it through `frozen.render_path` on the same `FrozenIdentityContext` - table and ID come from one owner by construction. `path_id_matches_components` carries `#[allow(dead_code)] // Supports the deferred mutable logical-path lookup contract.` and takes its table as an explicit parameter. `legacy_logical_path` (`database.rs:300`) is the documented migration bridge that rebuilds an `InternedPath` from parent links without retaining it, so its consumers compare component identities inside one string-table domain rather than `PathId`s across domains, which is the case covered above.

**`InternedPath` identity domains.** All production equality, hashing and map-key use of `InternedPath` occurs within one identity domain: per-file forks are merged and remapped before publication (`source_preparation.rs:214-238`), module merges run in chunk order with explicit remaps (`module_preparation.rs:624-673`), and frozen generic bodies cross tables through a text pool rather than raw IDs (`ast/generic_functions/materialisation/frozen_syntax.rs:141-192`). No pre-merge versus post-merge comparison without a remap was found. `starts_with` and `ends_with` (`interned_path.rs:198-206`, `:246-256`) are correct under their length guards - the guard makes the `zip` exact, so no truncation false-positive exists. `rebind_prefix`, the silent variant that returns `self.clone()` on a missing prefix, has zero production callers; every retained source-owned path uses `try_rebind_required_prefix`, which returns `CompilerError` (`:222-236`), and eight production sites use it. All ten `from_components` sites build components from the table the path is later resolved against.

**Non-UTF-8 filesystem lanes.** `try_from_filesystem_path` (`interned_path.rs:60-73`) retains the original `PathBuf` and never converts lossily. All five production callers map it deliberately: `source_preparation.rs:79-87`, `discovery_traversal.rs:599-608`, `public_exports.rs:84-93` and `single_source_compilation/config.rs:133-150` reach `CompilerError::file_error` (infrastructure), and `compilation/single_file.rs:198-216` splits the user-correctable case to `CompilerDiagnostic::invalid_source_file_entry`. No caller panics, unwraps or discards. Unix coverage exists at `symbols/tests/interned_path_tests.rs:82-125`.

**Other compiler-owned symbol names.** `this` is rejected as an authored function or variable with `MOTH-RULE-0040`; `Error` with `MOTH-RULE-0027` as a function and `MOTH-RULE-0039` as a variable. `message` and `code` are accepted, correctly: they are `Error` field names, not reserved identifiers. `<unknown>` is not an identifier spelling. An authored `start` in a non-root helper file is accepted, consistent with `entry-runtime-and-fragments.mtf:37-43` - helper files have no independent `start`, so there is nothing to collide with.

**Keyword-shadow policy.** `keyword_shadow_match` (`identifier_policy.rs:30-41`) strips leading underscores and compares case-insensitively against `RESERVED_KEYWORD_SHADOWS`, matching the whole stripped identifier rather than a prefix; `identifier_policy_tests.rs:36-38` pins that `configure` and `_configuration` stay ordinary. Behaviour matches the documented examples.

**Layout invariants.** `StringId` is a 4-byte `u32` newtype with typed accessors and no arithmetic on the raw value outside the module. `FrozenStringTable::freeze` moves the existing `Box<str>` allocations rather than copying, asserted by pointer identity in `string_interning_tests.rs:120-141`. `Clone for StringTable` flattens the shared base into independent storage and resets `next_id` consistently; `clone_preserving_inherited_prefix` retains the base and offsets local keys by `base_len`, and its single production caller is `ast/generic_functions/materialisation.rs:3426`.

**Dependency identities.** `DependencyShellId` and `DependencySelectionId` (`symbols/identity.rs`) join by `SourceId` plus ordinal rather than by path text, matching the 1B contract that provider binding must not compare path components.

## Limitations

- Coverage is `complete` for the area's source and tests. Consumer traces outside the area were sampled at the boundaries named above rather than exhaustively re-read; where a delegated trace made a load-bearing claim I re-verified it at the cited line, and the one incorrect claim is recorded under `Checked and clean`.
- Determinism was verified by reading the ordering code and its comments, not by running a serial-versus-parallel differential build. `docs/compiler-data-layout-design.md` `Validation and anti-drift tests` expects such a test; whether one exists is a Tests-lane question this run did not pursue.
- Performance was not measured and no performance claim is made. `StringTableBase::from_table` copying and the twice-merged contracts delta noted during the fork trace are unmeasured and are not filed; the counters AUD-0002-F04 added would double-count the second merge, which is a counter-accuracy question for a Performance run.
- `just validate` was not run. No gate is claimed to have passed.
- The `Compiler Bug` reproduction and all CLI probes used the debug profile. The `start` collision is a typed `CompilerError`, not a `debug_assert`, so release behaviour is expected to match, and the `moth build` probe produced the identical message; a release-profile confirmation was not run.
