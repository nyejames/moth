# AUD-0025: Numeric proof side-table style

- State: `complete`
- Kind: `Style`
- Area: `analysis.numeric_proofs` - the bounded-integer proof side table, its production invocations and publication, and its backend proof queries
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: Branch `audit-guide-improvements` at `279eb7b63`. Production code is identical to `main` at `49714f1b8`. `cargo test -q -p moth --lib numeric_proof` passed (60 tests) at `e49a166e8` during AUD-0022 and was not rerun, since no code changed since then. No formatter, clippy or validation gate was run for this read-only run.

## Question

Can a new maintainer follow the proof analysis and its backend queries without memorising positional state or misreading identity domains? Do names and local structure match the documented contract?

## What was inspected

These are the same surfaces as [AUD-0024](./AUD-0024-numeric-proofs-comments.md#what-was-inspected), read as code rather than prose.

Paths followed:

- **Normal path.** `analyse_numeric_proofs`, then `count_incoming_edges`, then the block walk. That walk runs `visit_statement` and `prove_operation` / `expression_interval`, then `visit_terminator` seeding.
- **Exceptional paths.** Unproven operations clearing the cache, out-of-range block targets, late seeds and non-integer domains.
- **Consumer paths.** JS statement and Invoke lowering, `invoke_failure_edge_is_live` from reachability, branch inspection and cast-policy demand, and Wasm `IntegerCheckMode` selection.

Call sites of every renamed-candidate parameter were traced:

- `js_statement.rs` lines 300 and 814 for `lower_numeric_operation`.
- Line 828 for `proven_narrowing_expression`.
- `emitter.rs` line 507, `reachability.rs` line 42 and `js_statement.rs` line 375 for `integer_narrowing_is_proven`.
- `js_statement.rs` line 327 and `wasm/hir_to_lir/stmt.rs` line 345 for `integer_operation_is_safe`.

## Authorities read

`AGENTS.md`; `docs/developer-docs/style-guide/style-guide.mtf` in full; `docs/roadmap/audit-guide.md`; `docs/roadmap/audit-kinds/README.md`; `docs/roadmap/audit-kinds/style.md`; `docs/roadmap/audits/README.md`; `docs/compiler-design-overview.md` `Frontend stages > Stage 5: HIR and validation > Numeric ownership`.

## Existing findings and active plans checked

- AUD-0022-F01 proposes moving the duplicated `Add`/`Subtract`/`Multiply`/`Negate` interval arithmetic into `datatypes/numeric_operators.rs`.
- AUD-0022-F02 covers the JS narrowing re-classification and its `expect` calls.
- AUD-0022-F03 covers the dead `is_integer` guard in `prove_operation`.

None is re-filed. AUD-0024-F02 covers the prose half of F01 below. No active plan or bug entry touches the area's style.

## Findings

### AUD-0025-F01: Proof queries name operation ids `statement`

- State: `closed`
- Kind: `Style`

#### Evidence

Five parameters are named `statement` but receive Invoke terminator ids:

- `NumericProofs::integer_operation_is_safe(&self, statement: HirNodeId)` (`mod.rs` line 90) is called from `classify_numeric_operation` with Invoke ids during Invoke lowering and edge liveness.
- `NumericProofs::integer_narrowing_is_safe(&self, statement: HirNodeId)` (line 95) only ever matches Invoke `CastOp` ids, because narrowing facts are inserted at line 284.
- `JsEmitter::integer_narrowing_is_proven(&self, statement: HirNodeId, ..)` (`js_statement.rs` line 397) is called only with Invoke ids: `emitter.rs` line 507, `reachability.rs` line 42 and `js_statement.rs` line 375.
- `proven_narrowing_expression(.., statement: HirNodeId)` (`js_statement.rs` line 372) has one caller, Invoke lowering at line 828, which passes the terminator `id`.
- `lower_numeric_operation(.., statement: HirNodeId, ..)` (line 342) receives a statement id at line 300 and an Invoke `id` at line 814.

Neighbouring code already uses neutral names for the same value:

- `classify_numeric_operation(op, node, delivery)`.
- `invoke_failure_edge_is_live(id, ..)`.
- Wasm `lower_checked_integer_operation(.., statement, ..)`, where `statement` is accurate because it is reached only from statements.

#### Counter-explanation tested

`HirNodeId` is the id statements carry, and the overview says statement-to-Invoke rewrites preserve identity, so "statement" could name the identity's origin. Rejected. Narrowing Invokes come straight from fallible casts and never had a statement. The authority describes the key as "the operation's HIR node identity". The inconsistent names also make the call sites in `reachability.rs` and `emitter.rs` look like they pass the wrong id.

#### Violated contract or cost

Names and inputs tell different stories, which the style guide asks to avoid. A reviewer has to trace each caller to learn that the parameter is an Invoke id. A maintainer extending the table to a new consumer could reasonably pass a statement id to `integer_narrowing_is_safe` and never get a fact.

#### Root owner

`NumericProofs` query signatures in `analysis/numeric_proofs/mod.rs`. Also the three JS helpers in `backends/js/js_statement.rs`.

#### Suggested correction

Non-authorising. Rename the five parameters to `operation` or `node`, matching `classify_numeric_operation`. Update the query docs with AUD-0024-F02. Leave the Wasm `statement` parameter unchanged.

#### Fix scope and preserved invariants

Rename only. Key domain, proof results and JS and Wasm output unchanged.

#### Required validation

`cargo check -p moth` and `cargo test -q -p moth --lib numeric_proof`.

#### Linked findings

AUD-0024-F02 (Comments). The `artefact.rs` and query-doc wording describes the same key domain.

### AUD-0025-F02: Divisor guards spell interval membership through positional endpoints

- State: `closed`
- Kind: `Style`

#### Evidence

`Interval` is `(i128, i128)` with the documented invariant `low <= high` (`mod.rs` line 66). Most arms read it clearly: `Add`, `Subtract` and `Multiply` add endpoints pairwise, and `Negate` destructures to `(low, high)`. The two divisor arms instead encode "this interval contains this value" through positional fields:

- The zero-divisor guard `right.0 <= 0 && 0 <= right.1` appears twice, at lines 348 and 375.
- The signed-minimum guard (lines 352-356) is `domain.0 < 0 && left.0 <= domain.0 && domain.0 <= left.1 && right.0 <= -1 && -1 <= right.1`.

The second guard means "the domain is signed, the dividend may be its minimum and the divisor may be -1". A reader has to decode five positional comparisons, and confirm that `domain.0` is the minimum, to recover that sentence. The final domain check at line 408 uses the same pattern in the other direction.

#### Counter-explanation tested

Endpoint pairs are the usual interval notation. `NumericScalar::integer_range` and `checked_result_range` share the tuple, so changing the representation would cross the `numeric_operators` owner. Accepted for the representation, and this finding does not propose a new interval type. It is rejected for the membership guards. They need no new type, only a named predicate or named bindings, and the zero-divisor test is the same expression twice.

#### Violated contract or cost

The style guide prefers named intermediates and functions that name a step over logic compressed into one expression. The overflow exclusion is the subtlest soundness condition in `prove_operation`. A mis-edited endpoint there would silently prove a failing `MIN / -1`.

#### Root owner

`BlockAnalyser::prove_operation` in `analysis/numeric_proofs/mod.rs`.

#### Suggested correction

Non-authorising. Add a private `interval_contains(interval, value)` and use it for both zero-divisor guards and the signed-minimum guard: `domain.0 < 0 && interval_contains(left, domain.0) && interval_contains(right, -1)`. Alternatively, destructure `domain` into `(domain_min, domain_max)` before the match. If AUD-0022-F01 moves the arithmetic into `numeric_operators.rs`, apply the same change at the new owner.

#### Fix scope and preserved invariants

Local readability only. Every proof decision, the checked `i128` arithmetic and the declining-on-overflow behaviour stay unchanged.

#### Required validation

`cargo test -q -p moth --lib numeric_proof`, which includes the divisor and signed-minimum invariant tests.

#### Linked findings

AUD-0022-F01 (Redundancy). Its consolidation decides where this guard ends up.

### AUD-0025-F03: `visit_statement` spells `HirWriteTarget` through crate paths

- State: `closed`
- Kind: `Style`

#### Evidence

`visit_statement` (`mod.rs` lines 206-218) writes `crate::compiler_frontend::hir::statements::HirWriteTarget::DefineLocal`, `::AssignPlace(place)` and `::AssignPlace(_)` in full. The file already imports `HirStatement` and `HirStatementKind` from the same module at line 59. The JS emitter imports `HirWriteTarget` from the same path at `emitter.rs` line 27. The full paths push the three match arms across eleven lines and hide that this is a three-way split on the write target.

#### Counter-explanation tested

The full path might avoid a name clash or mark the type as owned by another stage. Rejected. The module has no other `HirWriteTarget`, and the sibling HIR types are imported normally.

#### Violated contract or cost

This breaks the style guide's rule to import long paths at module scope. Low cost.

#### Root owner

`analysis/numeric_proofs/mod.rs` imports and `visit_statement`.

#### Suggested correction

Non-authorising. Add `HirWriteTarget` to the existing `hir::statements` import and use the short variant names.

#### Fix scope and preserved invariants

Import and formatting only.

#### Required validation

`cargo check -p moth`.

#### Linked findings

None.

## Resolution (2026-11-24)

**F01 through F03 accepted and closed.** Proof queries use operation identities,
divisor guards name interval membership and the statement visitor imports
`HirWriteTarget` at module scope. Independent review verified the unchanged
proof and lowering behavior. The `numeric_proof` filter passed 60 tests.

## Checked and clean

- **`analyse_numeric_proofs`.** It reads as edge counting, then one block walk, then returning facts. The driver setting `current_block` and `cache` before each walk is short and visible.
- **`count_incoming_edges`.** It is one explicit loop per fact. Its tolerance of out-of-range ids is conservative and stays consistent with the `get` at line 298.
- **`visit_terminator`.** The let-else guard and the `if let ... &&` chain at lines 296-302 state each seed condition once, in the order the module header lists them.
- **`prove_operation`.** The match is grouped one arm per operator with a comment on each non-obvious guard. It is about 95 lines and represents one coherent operation. Splitting it would scatter the shared `domain` and `binary_intervals` state. The dead `is_integer` guard is AUD-0022-F03.
- **`visit_statement`.** The separate `LoopStepFailure` arm has the same body as the wildcard. It is an intentional labelled group, as the style guide's match example shows, not a duplicate path.
- **Tuples.** `seeds` and `cache` hold the same `(LocalId, Interval)` pair under documented field names. A named struct would add a type for two fields used in three places.
- **`NumericProofs` API.** Two `pub(crate)` boolean queries over private sets give the narrowest surface the five consumers need.
- **JS liveness.** `invoke_failure_edge_is_live` is a single owner reused by reachability, branch inspection and lowering. `integer_narrowing_is_proven` lives in `js_statement.rs` beside the lowering it mirrors, which the reachability doc states deliberately.
- **Wasm `check_mode`.** The selection at `stmt.rs` lines 341-350 is a plain `if` over the `Infallible` mode and the table query.
- **`proven_narrowing_expression`.** Its `expect` chain is a structural duplication covered by AUD-0022-F02.

## Limitations

- Threading-only code in `wasm/backend.rs`, `wasm/hir_to_lir/module.rs`, `html_project_builder.rs`, `js_path.rs` and `html_project/wasm/artifacts.rs` was located by search, not read.
- `numeric_carrier.rs` was read only at its classification entry point.
- Test modules and their API usage were not inspected.
- The long `PathInternerBuilder` paths beside the proof publication in `service.rs` and `materialisation.rs` belong to path-table construction, outside this area, and were not assessed.
