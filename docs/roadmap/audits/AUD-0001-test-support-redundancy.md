# AUD-0001: Redundancy across integration and unit test support

- State: `complete`
- Kind: `Redundancy`
- Primary scope: `tests.support`
- Required context: `tests.harness`, `docs/src/docs/codebase/style-guide/testing.mtf`, consuming test modules
- Coverage: `partial`
- Reviewed: `2026-08`
- Baseline: no validation gate was run; this audit is read-only and made no code change. Branch `test-suite-honesty` is mid-campaign (Phase 5), so parts of this surface are actively changing.
- Revision: branch `test-suite-honesty` @ `4e2207826`

## Scope, context and exclusions

Primary scope is the comparison scope `tests.support`: the test-only support and helper
modules that the integration harness and the per-subsystem unit tests both depend on.
These files are maintained implementation, so they are auditable under Redundancy even
though they live in the test tree.

Explicitly excluded from exhaustive coverage:

- `src/compiler_tests/integration_test_runner/**` implementation (7,600 lines) — owned by
  `tests.harness`, read here only as context.
- The 1,700 fixture directories under `tests/cases/*/` — owned by `tests.cases`.
- Duplicate or missing *test coverage*. `audit-kinds/redundancy.md` routes that to the
  Tests lane. This report only covers repeated machinery and obsolete support code.

## Coverage inventory

Inspected in full:

- `src/compiler_tests/{test_support,test_fs,test_diagnostics}.rs`
- `src/build_system/test_support.rs`
- `src/compiler_frontend/tests/{ast_fixture_support,hir_fixture_support,borrow_fixture_support,parse_support,external_package_support}.rs`
- `src/compiler_frontend/public_interface/tests/test_support.rs`
- `src/projects/html_project/tests/test_support.rs`
- `src/backends/js/tests/{support,test_symbol_helpers}.rs`
- `src/backends/wasm/tests/lowering/test_support.rs`
- The complete cross-tree inventory of `SourceLocation` construction helpers defined inside
  test modules (23 zero-value wrappers, 6 line-based builders, 5 forwarding aliases).

Signature-level survey only (bodies not read exhaustively):

- `src/compiler_frontend/tests/type_id_fixture_support.rs` (738 lines) — `build_ast`,
  `build_ast_with_choices` and `reference_expr` read in full; remainder surveyed.
- `src/compiler_frontend/ast/templates/tir/tests/{support,store_support,builder}.rs`
- `src/compiler_frontend/ast/expressions/tests/expression_test_support.rs`
- `src/compiler_frontend/external_packages/packages/test_packages.rs`

## Authorities read

- `AGENTS.md` — ownership, one-current-path, data-oriented preference, test location rules
- `docs/roadmap/audit-guide.md` — preservation contract, forbidden fix forms, freshness
- `docs/roadmap/audit-kinds/redundancy.md` — full procedure
- `docs/src/docs/codebase/style-guide/testing.mtf` — read in full, per the audit guide's
  requirement for reviews of test ownership

## Existing findings and active plans checked

`open-audit-findings.md` was empty and `audits/` held no prior report — AUD-0001 is the first
audit in this repository, so no duplicate or superseding relationships exist. The scope
registry was empty and was populated for this run (`tests.harness`, `tests.support`,
`tests.cases`).

The active `test-suite-honesty` branch is a test-honesty campaign (Phases 0-5). Its commits
tighten assertion exactness and helper contracts. None of the findings below overlap that
work: the campaign strengthens what tests assert, while these findings concern repeated and
obsolete helper machinery.

## Findings

### AUD-0001-F01: Twenty-three identical zero-value `SourceLocation` wrappers

- State: `resolved`
- Kind: `Redundancy`
- Scope: `tests.support`
- Priority: `unassigned`

#### Evidence

Twenty-three byte-identical private helpers wrapping `SourceLocation::default()` are defined
independently across the test tree, under three names:

```rust
fn empty_location() -> SourceLocation { SourceLocation::default() }
```

`empty_location` appears in 16 modules: `tir/tests/{fold_cache,expression_traversal,
render_unit,builder_tests,store_tests,view_tests,construction,hir_handoff,slot_composition,
wrapper_context_fold,preparation,subtree_copy,fold_final_view,slot_layout}_tests.rs`,
`ast/const_values/tests/mod.rs` and
`ast/templates/template_slots/runtime_plan/sites/sites_tests.rs`.

`location()` with the same body appears in `ast/templates/tests/reactive_template_metadata_tests.rs:26`,
`tests/canonical_type_identity_tests.rs:168`, `datatypes/tests/generics_tests.rs:23` and
`public_interface/tests/declaration_record_tests.rs:126`.

`default_location()` appears in `public_interface/tests/test_support.rs:161` and again in
`public_interface/tests/folded_value_tests.rs:53` — the second inside a file that already
imports `super::test_support` on line 16 and documents "This module reuses shared fixtures
from `test_support`" on line 10.

Two related aliases sit in the same family: `ast_fixture_support.rs:42`
`test_location(line)` forwards unchanged to `test_source_location(line)` in the same file,
and `public_interface/tests/test_support.rs:165` `immutable()` returns the constant
`ValueMode::ImmutableOwned`.

#### Counter-evidence checked

Considered whether the wrappers carry naming intent that `SourceLocation::default()` lacks —
whether `empty_location()` documents "this test does not care about location". Rejected as
insufficient: the name is not consistent across owners (`empty_location`, `location`,
`default_location` all mean the same thing), so it communicates nothing stable to a reader,
and `SourceLocation::default()` already reads as an explicit don't-care. Also checked whether
any of the 23 diverge — none do; every body is exactly `SourceLocation::default()`.

Considered whether `folded_value_tests::default_location` shadows deliberately to avoid an
import. Rejected: the file already imports from the same support module on line 16.

#### Violated contract or cost

`redundancy.md` section 8: "one-line forwarding functions with no ownership, validation or
policy". `AGENTS.md`: "Does the code you're about to write need to exist at all? If not,
skip it." `testing.mtf` `Test location and module layout`: "Do not create broad shared test
utility modules for one or two callers" — the inverse also holds; a helper that adds nothing
over the `Default` impl is not a helper.

#### Impact

Twenty-three definitions to read past, with three different names for one concept. Low
severity, broad surface.

#### Root owner

No single owner — each test module defines its own. `SourceLocation`'s `Default` impl is the
real owner.

#### Suggested correction

Non-authorising. Delete all 23 wrappers and call `SourceLocation::default()` at the call
sites. Do **not** extract them into a shared helper: that would move the redundancy rather
than remove it. Delete the `test_location` alias in `ast_fixture_support.rs:42` and point
its callers at `test_source_location`. Delete `immutable()` and inline the constant.

Classification per section 15: **delete**, not extract or move.

#### Allowed fix scope

The 23 definition sites and their call sites, `ast_fixture_support.rs:42`, and
`public_interface/tests/test_support.rs:165`.

#### Read-only context

`SourceLocation` and its `Default` impl; all consuming test modules.

#### Must preserve

Every test's assertions and pass/fail outcome must be unchanged — this is a mechanical
substitution of an identical value. No assertion text, exactness or diagnostic expectation
may be touched. Any site where the substitution would require changing what a test asserts
must be left alone and raised as a linked Tests finding instead.

#### Forbidden fix forms

Do not introduce a new shared `test_location`-style utility module. Do not widen
`SourceLocation`'s production API. Do not adjust a test's assertions to accommodate the
substitution.

#### Required validation or measurement

`just validate`.

#### Dependencies and related findings

Related to AUD-0001-F02 (the line-based builder family) and AUD-0001-F04.

#### Triage record

Accepted and corrected on branch `test-suite-honesty`. All 23 wrappers deleted and their 944
call sites now spell `SourceLocation::default()`. Implementation found a 24th the audit missed —
`ast/expressions/tests/runtime_handoff_expression_payload_tests.rs:17` `test_location()`, which
takes no argument and so did not match the report's search shape — and deleted it too. The `test_location` alias in
`ast_fixture_support.rs` was deleted and its 826 call sites repointed at the owner
`test_source_location`; `immutable()` was deleted and inlined. `cargo test --workspace` reports
4373+17+646 passing, 0 failed, 0 ignored — identical to the pre-change baseline, so no test
outcome changed.

### AUD-0001-F02: JS backend test support re-implements HIR fixture constructors the Wasm support already shares

- State: `resolved`
- Kind: `Redundancy`
- Scope: `tests.support`
- Priority: `unassigned`

#### Evidence

`src/backends/js/tests/support.rs` and `src/backends/wasm/tests/lowering/test_support.rs`
define ten same-named helpers. Seven have byte-identical bodies:

`expression`, `unit_expression`, `int_expression`, `bool_expression`, `string_expression`,
`statement`, `local`.

These construct plain HIR nodes. Per `AGENTS.md`, "HIR is the first backend-facing semantic
IR" — both backends consume the same HIR, so HIR fixture construction is not target-specific.

The `loc` helper is the decisive case. The Wasm support already delegates to the canonical
owner:

```rust
// wasm/tests/lowering/test_support.rs:29
use crate::compiler_frontend::tests::ast_fixture_support::test_source_location;
pub(crate) fn loc(line: i32) -> SourceLocation { test_source_location(line) }
```

while the JS support re-copies the same body inline at `js/tests/support.rs:113`, down to the
`char_column: 120, // Arbitrary number` comment. The same body is copied a third and fourth
time into `ast/templates/template_folding_tests.rs:48` and
`ast/templates/tests/template_tests.rs:33`, and independently re-owned a fifth time by
`hir/tests/hir_expression_lowering_tests.rs:103`.

The Wasm import proves cross-subsystem test-support reuse is already sanctioned and
mechanically works.

#### Counter-evidence checked

Applied `redundancy.md` section 12 to each shared name rather than treating the cluster as
uniform. Two of the ten are **correctly divergent and must stay separate**:

- `build_type_environment` — JS registers option, choice, collection, map, fallible-carrier
  and IO-input-handle types and returns an 11-field `TypeIds`; Wasm registers only
  unit/int/bool/string and returns a 4-field `TypeIds`. This reflects a real difference in
  each backend's supported type surface. **Leave local.**
- `build_module` — different signatures and different semantics. JS takes one function plus
  explicit local-name pairs, hard-codes one region and seeds a `HirChoice`; Wasm takes
  multiple `(HirFunction, InternedPath, HirFunctionOrigin)` tuples, derives region count from
  the blocks and auto-names locals. Not equivalent. **Leave local.**

Also checked whether the seven identical constructors might diverge later under target
pressure. They construct stage-owned HIR nodes with no target policy in them, so divergence
would indicate a HIR change affecting both backends equally.

#### Violated contract or cost

`redundancy.md` section 12: behaviour that is "language-owned shared behaviour that should
move before target lowering". `AGENTS.md`: "Before adding a helper... search the current
owner, adjacent stages, backend paths and tests. Share only identical behaviour with a clear
owner."

#### Impact

Seven duplicated constructors plus one duplicated location builder across two backends. A HIR
node-shape change requires editing both, and the JS copy can silently drift from the Wasm one.

#### Root owner

HIR owns the node shapes. `src/compiler_frontend/hir/tests/hir_builder_test_support.rs` is
the natural test-support owner; `ast_fixture_support::test_source_location` already owns the
location builder.

#### Suggested correction

Non-authorising. Move the seven identical constructors to the HIR test-support owner and have
both backend supports consume them, following the import path the Wasm support already uses.
Point `js/tests/support.rs:113` `loc` at `test_source_location`, and do the same for the
copies in `template_folding_tests.rs:48`, `template_tests.rs:33` and
`hir_expression_lowering_tests.rs:103`. Leave `build_type_environment` and `build_module`
local to each backend, and record that reason in each file so a later reader does not
"finish the job" incorrectly.

Classification per section 15: **move to a common owner** for the seven constructors and
`loc`; **leave local** for the two divergent helpers.

#### Allowed fix scope

`src/backends/js/tests/support.rs`, `src/backends/wasm/tests/lowering/test_support.rs`,
`src/compiler_frontend/hir/tests/hir_builder_test_support.rs`, and the four `loc`/
`test_location` copy sites.

#### Read-only context

HIR node definitions; the 17 JS and 2 Wasm test modules that consume these supports.

#### Must preserve

Every existing backend test must pass unmodified. Target-specific behaviour must stay
target-specific — do not create a shared backend abstraction that erases the
`build_type_environment` or `build_module` differences. Determinism of emitted symbol names
and fixture identities must be unchanged.

#### Forbidden fix forms

Do not merge `build_type_environment` or `build_module`. Do not introduce a trait or generic
backend-fixture abstraction over two concrete callers. Do not move JS-specific
`lower_minimal_module_*` helpers to a shared owner.

#### Required validation or measurement

`just validate`.

#### Dependencies and related findings

Shares the location-builder root cause with AUD-0001-F01 and AUD-0001-F03.

#### Triage record

Accepted and corrected on branch `test-suite-honesty`. The seven identical constructors moved to
`compiler_frontend::tests::hir_fixture_support`, which both backend supports now consume. That
module was chosen over `hir/tests/hir_builder_test_support.rs` because `hir::tests` is a private
module: using it would have required widening `mod tests` to `pub(crate)` purely to serve the
backends. `hir_fixture_support` is already `pub(crate)` and already the frontend's HIR fixture
owner, so no visibility was widened. `loc` was deleted from both backends rather than kept as a
delegate, so `test_source_location` is now the single location builder in the tree, and the
copies in `template_folding_tests.rs`, `template_tests.rs` and `hir_expression_lowering_tests.rs`
were deleted too. `build_type_environment` and `build_module` stayed backend-local and each now
carries a doc comment stating why, so a later reader does not merge them. JS's `float_expression`
also stayed local, since only JS consumes it; it now calls the shared `expression`.

### AUD-0001-F03: A test module serves as the de-facto support owner for five sibling HIR test modules

- State: `resolved`
- Kind: `Redundancy`
- Scope: `tests.support`
- Priority: `unassigned`

#### Evidence

`src/compiler_frontend/hir/tests/hir_expression_lowering_tests.rs` exports `pub(crate)`
helpers (`location` at line 103, `register_local`, and others) consumed by five sibling test
modules:

- `float_formatting_lowering_tests.rs:33` — `use ...::hir_expression_lowering_tests::{...}`
- `checked_numeric_lowering_tests.rs:21` — same
- `hir_validation_tests.rs:42` — `use ...::hir_expression_lowering_tests::location`
- `loop_lowering_tests.rs:27` — `super::hir_expression_lowering_tests::location(line)`
- `hir_function_origin_tests.rs:26` — `crate::...::hir_expression_lowering_tests::location(line)`

Meanwhile `src/compiler_frontend/hir/tests/hir_builder_test_support.rs` exists as the declared
support owner for this directory, and contains only two functions.

Three of the five consumers wrap the import in a local forwarding alias that adds nothing:

```rust
// hir_validation_tests.rs:49
fn test_location(line: i32) -> SourceLocation { location(line) }
// loop_lowering_tests.rs:26
fn test_location(line: i32) -> SourceLocation { super::hir_expression_lowering_tests::location(line) }
// hir_function_origin_tests.rs:25
fn location(line: i32) -> SourceLocation { crate::...::hir_expression_lowering_tests::location(line) }
```

So a call to `test_location` in `loop_lowering_tests.rs` resolves through three hops to a body
that duplicates `ast_fixture_support::test_source_location`.

#### Counter-evidence checked

Considered whether co-locating helpers with the test that first needed them is the intended
pattern under `testing.mtf`'s "Test-only utilities should live with the tests that own them".
Rejected: that rule places utilities with their owner, and the owner here is five modules, not
one. The same document's `Test location and module layout` section places shared module
helpers in the module's test-support file, which exists and is nearly empty.

Considered whether the aliases preserve a call-site name that would be churn to change.
Rejected — this is test-internal naming with no external contract.

#### Violated contract or cost

`redundancy.md` section 8 (forwarding functions with no ownership) and section 13 (modules
that "expose internals to avoid a better submodule boundary"). `testing.mtf`
`Test location and module layout`.

#### Impact

Helper ownership is not discoverable: a reader looking for HIR fixture helpers finds a
near-empty support file and must know that a test module is the real owner. Deleting or
renaming a test breaks five unrelated modules.

#### Root owner

`src/compiler_frontend/hir/tests/hir_builder_test_support.rs`.

#### Suggested correction

Non-authorising. Move the cross-module helpers out of `hir_expression_lowering_tests.rs` into
`hir_builder_test_support.rs`, and delete the three forwarding aliases so consumers import the
support owner directly. Fold `location` into `ast_fixture_support::test_source_location` per
AUD-0001-F02 rather than re-homing a fourth copy.

Classification per section 15: **move to a common owner**, plus **delete** for the aliases.

#### Allowed fix scope

`hir_expression_lowering_tests.rs`, `hir_builder_test_support.rs`, and the five consuming
modules' import lines and alias definitions.

#### Read-only context

`ast_fixture_support.rs`; `testing.mtf`.

#### Must preserve

This is a pure relocation of helper definitions. No test body, assertion, fixture value or
outcome may change. If moving a helper would require altering what any of the five modules
asserts, stop and raise a linked Tests finding — a Redundancy finding cannot authorise a
test-behaviour change.

#### Forbidden fix forms

Do not leave a re-export in `hir_expression_lowering_tests.rs` for compatibility — the audit
guide forbids preserving an obsolete path through a forwarding shim. Do not widen the helpers
to `pub`.

#### Required validation or measurement

`just validate`.

#### Dependencies and related findings

Depends on AUD-0001-F02 for the location-builder destination. Related to AUD-0001-F01.

#### Triage record

Accepted and corrected on branch `test-suite-honesty`. `setup_builder`, `register_local` and
`runtime_template_expression` moved into `hir_builder_test_support.rs`, together with the
`expressions_to_owned_render_node`/`expression_to_owned_node` conversion pair that
`runtime_template_expression` depends on. The template branch and loop fixture builders stayed in
`hir_expression_lowering_tests.rs` because only that module consumes them. Consumers now import
through the established `hir::hir_builder` re-export, and the three forwarding aliases were
deleted with no compatibility re-export left behind.

### AUD-0001-F04: Sibling fixture supports export colliding names with different semantics

- State: `deferred`
- Kind: `Redundancy`
- Scope: `tests.support`
- Priority: `unassigned`

#### Evidence

`src/compiler_frontend/tests/` holds three sibling fixture-support modules that export
same-named `pub(crate)` helpers with **different behaviour**:

`build_ast(nodes, entry_path) -> Ast` — identical name and signature in two modules:

- `hir_fixture_support.rs:27` forwards unchanged to the production
  `hir::hir_builder::build_ast`.
- `type_id_fixture_support.rs` delegates to `build_ast_with_choices`, which registers struct
  definitions, choice definitions and collection types into a fresh `TypeEnvironment` and
  constructs the `Ast` literal directly.

`reference_expr` — same name, different fixed policy:

- `ast_fixture_support.rs:102` takes a `DataType`, hard-codes `ValueMode::ImmutableReference`.
- `type_id_fixture_support.rs` takes a `ValueMode`, hard-codes `DataType::Inferred`.

Ten or more test modules import both `ast_fixture_support` and `type_id_fixture_support`,
including `hir/tests/{hir_module,hir_branch,value_block,hir_result,hir_match,hir_reactivity,
hir_local}_lowering_tests.rs` and
`analysis/borrow_checker/tests/{borrow_checker_reactivity,borrow_checker_call_summary}_tests.rs`,
so the collision is live rather than theoretical. `float_formatting_lowering_tests.rs` and
`hir_module_lowering_tests.rs` import a `reference_expr` from these siblings today.

Separately, `hir_fixture_support.rs:27` `build_ast` is a bare one-line forward to a production
function with no added ownership, validation or policy.

#### Counter-evidence checked

Considered whether this is a deliberate in-progress migration, with `type_id_fixture_support`
("TypeId-first test helpers... so HIR test files can remain free of parse-era type-syntax
references") replacing the `DataType`-era `ast_fixture_support`. This is the strongest
counter-explanation and it is partly correct: the module header documents exactly that intent.
But it does not dissolve the finding — `ast_fixture_support` still has 33 importers against
`type_id_fixture_support`'s 16, so the old shape is not being retired, and both are actively
imported into the same files. A migration that has stalled with two live shapes under one name
is the "transitional duplication" the audit guide forbids as a completed state.

Verified the two `build_ast` bodies are genuinely non-equivalent before considering a merge —
they are, so merging them would be wrong.

#### Violated contract or cost

`redundancy.md` section 13: "duplicate types across sibling modules"; section 8 (one-line
forwarding function). `AGENTS.md`: "Keep one current implementation path... and delete the old
path", and "Moth is pre-release. Do not preserve old APIs through... parallel structs".

#### Impact

A test importing the wrong `build_ast` or `reference_expr` compiles cleanly and silently gets
different fixture semantics — a different `TypeEnvironment`, or a different `ValueMode`. This
is a latent correctness hazard in fixtures, not just a readability cost.

#### Root owner

`src/compiler_frontend/tests/` — the three sibling support modules collectively; no single
current owner, which is the defect.

#### Suggested correction

Non-authorising, and this finding needs triage before any code change because the right
outcome depends on migration intent that the roadmap does not currently state.

If the TypeId-first migration is meant to complete: finish it, move the remaining 33
`ast_fixture_support` importers across, and delete the superseded helpers. If both shapes are
genuinely needed: give them distinct, self-describing names (for example
`build_ast_from_production_lowering` and `build_ast_with_registered_types`) so an import
cannot silently pick the wrong semantics. Delete `hir_fixture_support::build_ast` and let
callers call the production function directly.

Classification per section 15: **split** the naming first; the merge-or-delete decision is
blocked on migration intent.

#### Allowed fix scope

`src/compiler_frontend/tests/{ast_fixture_support,hir_fixture_support,type_id_fixture_support}.rs`
and the import lines of their consumers.

#### Read-only context

`hir::hir_builder::build_ast`; `Expression::reference_with_type_id`; the roadmap.

#### Must preserve

No test's fixture semantics may change. Renaming must be a pure rename — if any consumer turns
out to be importing the *other* module's helper than its author intended, that is a separate
Correctness or Tests finding and must not be silently "fixed" under this one.

#### Forbidden fix forms

Do not merge the two `build_ast` bodies. Do not keep a deprecated alias for the old name. Do
not implement the migration's remaining scope under cover of a rename.

#### Required validation or measurement

`just validate`.

#### Dependencies and related findings

Blocked on a roadmap/design answer about the TypeId-first migration. Related to AUD-0001-F02.

#### Triage record

Accepted as a real defect, deferred by design gate. The correction depends on whether the
TypeId-first migration is meant to complete, which the roadmap does not state, so it was routed
into the owning plan rather than guessed at: `docs/roadmap/plans/test-suite-honesty-and-infrastructure-hardening-plan.md`
Phase 11 item 3, with a matching completion criterion ("no two fixture-support helpers share a
name with different fixture semantics"). The plan entry carries the finding's constraint that a
rename must be a pure rename and that a mis-imported consumer is a separate exposed defect.

### AUD-0001-F05: Two dead assertion helpers retained behind `#[allow(dead_code)]`

- State: `partially resolved`
- Kind: `Redundancy`
- Scope: `tests.support`
- Priority: `unassigned`

#### Evidence

`src/compiler_tests/test_diagnostics.rs` marks two `pub fn` helpers `#[allow(dead_code)]`:

- line 133 `assert_diagnostic_reason(messages, code, occurrence, expected_reason)`
- line 173 `error_code_counts(messages) -> BTreeMap<String, usize>`

A tree-wide search for each name outside its defining file returns zero callers. Both
suppressions are bare — unlike the other `allow(dead_code)` sites in this repository
(`hir_display.rs:124`, `traits/environment.rs:468`, `wasm/lir/types.rs:34`), which each carry
a comment justifying why the item is retained.

#### Counter-evidence checked

Checked whether these are a deliberate API for imminent use by the `test-suite-honesty`
campaign, whose Phase 2 commit message mentions "typed error seams". `error_code_counts`'s doc
comment says it is "useful for comparing multisets in tests that need exact cardinality",
which matches that direction. This is a real possibility and is why the suggested correction
below offers retention-with-justification as an equal option rather than mandating deletion.

Also checked whether the integration runner reaches them dynamically — it does not; these are
ordinary Rust functions with no macro or registry indirection.

#### Violated contract or cost

`redundancy.md` section 5: "dead code and unjustified `allow(dead_code)`". `AGENTS.md`
Final audit step 5: "justify every lint suppression". The audit guide's forbidden fix forms
name suppressing a lint instead of fixing its cause.

#### Impact

Two unused public helpers, ~40 lines, with a lint suppression that hides their disuse from
future readers and from the compiler.

#### Root owner

`src/compiler_tests/test_diagnostics.rs`.

#### Suggested correction

Non-authorising. Either delete both helpers and their suppressions, or — if the
`test-suite-honesty` campaign intends to adopt them — keep them and replace each bare
suppression with a comment naming the phase and the intended caller, matching the justified
style used elsewhere in the repository. Deciding between these requires the campaign owner's
input.

Classification per section 15: **delete**, unless triage establishes a named imminent caller.

#### Allowed fix scope

`src/compiler_tests/test_diagnostics.rs`.

#### Read-only context

The active `test-suite-honesty` plan; existing diagnostic assertion call sites.

#### Must preserve

No existing diagnostic assertion may be weakened. If deletion is chosen, confirm no test is
left asserting less than it does today.

#### Forbidden fix forms

Do not keep the helpers with a bare suppression. Do not add a token caller in a test purely to
retire the lint — that would be implementation-shaped coverage.

#### Required validation or measurement

`just validate`.

#### Dependencies and related findings

Independent.

#### Triage record

The bare suppressions were the part that needed no campaign input, and they are corrected: each
`#[allow(dead_code)]` now carries a comment naming the deciding phase, the deletion deadline and
the ban on adding a token caller, matching the justified style used elsewhere in the repository.
The delete-or-adopt decision itself was routed into the owning plan — Phase 7 item 8 decides
adoption, Phase 11 item 2 deletes them if nothing adopted them. Note for that decision: both
helpers also have no self-tests, so adopting either one means proving it works first.

## No-finding checks

Areas inspected that produced no finding, recorded so a later audit does not re-derive them:

- **`portable_path_text` centralisation holds.** `testing.mtf` requires tests to use the
  shared normalisation helper. There is exactly one definition
  (`compiler_frontend/utilities/basic.rs:49`) and every consumer — the integration runner's
  `artifacts.rs`, `expectations.rs`, `goldens.rs`, `fixture.rs`, `diagnostics.rs`, and
  `html_project_builder_tests.rs` — imports it. No competing normaliser exists in the test
  tree.
- **The HTML shell contract has one owner across levels.**
  `projects/html_project/tests/test_support.rs:290` deliberately consumes
  `integration_test_runner::assertions::html_shell_violation` rather than re-implementing the
  check, with a comment stating the reason. This is the correct shape for a contract shared by
  unit and integration tests and should not be "simplified" into a local copy.
- **Stage separation between the frontend fixture supports is deliberate and sound.**
  `parse_support`, `ast_fixture_support`, `hir_fixture_support` and `borrow_fixture_support`
  each document why they must not depend on the next stage down, and the dependency direction
  matches. The F04 naming collision does not undermine this layering.
- **`build_system/test_support.rs` is correctly placed.** It carries an explicit `MUST NOT`
  clause and exists specifically to keep a flat-module test-construction shape out of
  production `build.rs`. This is the right owner, not redundancy.
- **`test_fs.rs` and `test_support.rs` in `compiler_tests` are not duplicative.** `test_fs`
  owns explicit IO-outcome assertions; `test_support` owns temp paths, panic assertions and
  style directives. `unused_temp_path` correctly reuses `test_fs::assert_path_missing` rather
  than re-deriving absence.
- **`js/tests/test_symbol_helpers.rs` mirrors production naming logic by design.** It predicts
  emitted symbol names to verify determinism; sharing the production function instead would
  make the test assert nothing.

## Limitations

- **Coverage is partial.** Four primary-scope files were surveyed at signature level rather
  than read exhaustively: `type_id_fixture_support.rs` (738 lines, of which `build_ast`,
  `build_ast_with_choices` and `reference_expr` were read in full),
  `tir/tests/{support,store_support,builder}.rs`, `expression_test_support.rs` and
  `packages/test_packages.rs`. Additional duplication may exist inside them.
- Test-support modules under `src/build_system/output/tests/`, `src/projects/dev_server/tests/`,
  `src/timing/tests/` and `src/projects/tests/` were not inspected.
- `tests.harness` (the 7,600-line integration runner) and `tests.cases` (1,700 fixtures) were
  out of exhaustive scope by registration. The runner is the largest unexamined redundancy
  surface in the test tree and warrants its own report.
- No validation gate was run. This audit made no code change, so no gate was required, but the
  baseline health of the suite on this branch is therefore unverified here.
- The branch is mid-campaign at Phase 5. Findings F04 and F05 may be affected by in-flight
  intent that the roadmap does not record; both name that dependency explicitly.
- Duplicate *test coverage* between fixtures and unit tests was deliberately not assessed —
  that is the Tests lane. Nothing in this report should be read as a coverage claim.

## Freshness update

`tests.support` Redundancy: `P 2026-08 AUD-0001`.

No other cell is updated. In particular, the `tests.harness` and `tests.cases` Redundancy
cells remain `N`, and no Style, Comments or Tests cell is promoted from observations made
during this run.
