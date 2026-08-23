# Value-Producing Control-Flow Receiver Consolidation and Block Single-Predicate Match Plan

> **Repository path:**
> `docs/roadmap/plans/value-producing-control-flow-receiver-consolidation-plan.md`
>
> **Implementation branch:**
> `value-producing-control-flow-receiver-consolidation`
>
> **Planning parent and initial PR base:**
> `const-folding-and-types-optimisation`
>
> **Planning snapshot:**
> `e782e79d8c4113de4dd835f777bc51b773cffe40`
>
> **Status:**
> Plan-only checkpoint. Implementation has not started.

## Purpose

Consolidate Moth's value-producing control-flow receiver architecture, fix the existing block
value-`if` correctness gaps and add block-form single-predicate value matches without adding a new
AST or HIR representation.

The accepted new source form is:

```moth
label = if maybe_name is |name|:
    then name
else
    then "guest"
;
```

It is the block equivalent of the existing inline form:

```moth
label = if maybe_name is |name| then name else "guest"
```

The work has three equal outcomes:

1. Replace duplicated `if` header detection and split pattern ownership with one clear parser path.
2. Correct block value-production completeness, result inference, arity and coercion across every
   supported closed receiver.
3. Extend the resulting architecture with option-capture and choice-predicate block value matches.

Deletion is part of the deliverable. The final implementation should contain less routing and
validation code than the current tree even after adding `receiver/block_match.rs`.

---

## Active context capsule

ACTIVE_PLAN:
- `docs/roadmap/plans/value-producing-control-flow-receiver-consolidation-plan.md`

BRANCH:
- `value-producing-control-flow-receiver-consolidation`

BASE_COMMIT:
- `e782e79d8c4113de4dd835f777bc51b773cffe40`

PARENT_BRANCH:
- `const-folding-and-types-optimisation`

PARENT_STATE_AT_SNAPSHOT:
- Phase B of the constant and type-system optimisation plan is complete.
- Phase C, module-local folded-value authority, is active.
- Static Bool `if` specialisation remains the later Phase G semantic gate.

CURRENT_SLICE:
- Phase 0, re-anchor and semantic freeze, after this plan is accepted.

NEXT_ACTION:
- Establish the baseline on this branch, record current passing and failing receiver cases, then
  implement Phase 1 without changing header routing.

INTEGRATION_MODEL:
- Work only on this branch while the parent optimisation branch continues independently.
- Do not repeatedly rebase this branch during implementation.
- After the parent branch is complete and squash-merged to `main`, rebase this branch onto that new
  `main`, reconcile the final static-`if` architecture, rerun every final gate, retarget the PR to
  `main` and only then prepare the final squash merge.

ROADMAP_POLICY:
- Do not edit `docs/roadmap/roadmap.md` anywhere in this plan.
- The absence of a roadmap entry is deliberate conflict avoidance, not an omitted task.

---

## Required reading before implementation

Read these sources from the active worktree before Phase 0 and re-read the affected owners after
compaction or a parent-branch rebase:

- `AGENTS.md`
- `docs/compiler-design-overview.md`
- `docs/src/docs/codebase/language/overview.mtf`
- `docs/src/docs/branching/value-producing-if.mtf`
- `docs/src/docs/branching/pattern-matching.mtf`
- `docs/src/docs/branching/patterns-and-exhaustiveness.mtf`
- `docs/src/docs/errors/options.mtf`
- `docs/src/docs/choices/payload-patterns.mtf`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.md`
- `docs/src/docs/progress/@page.moth`
- `docs/src/docs/codebase/style-guide/style-guide.mtf`
- `docs/src/docs/codebase/style-guide/testing.mtf`
- `docs/src/docs/codebase/style-guide/validation.mtf`
- `docs/roadmap/plans/constant-folding-and-type-system-hot-path-optimization-plan.md`

Read the complete compiler architecture because this plan changes a broad Stage 4 parser boundary.
Read the build-system authority only if implementation expands beyond local AST construction, which
would require a design review because no build-system change is expected.

---

## Accepted source contract

### Block single-predicate value matches

The block form supports the same pattern subset as the existing inline single-predicate value form.

Option present capture:

```moth
label = if maybe_name is |name|:
    then name
else
    then "guest"
;
```

Choice unit predicate:

```moth
score = if status is Ready:
    then 1
else
    then 0
;
```

Choice payload capture:

```moth
Response ::
    Complete,
    Failed | message String |,
;

label = if response is Failed(message):
    then message
else
    then "complete"
;
```

Qualified choice variants remain valid where the existing single-predicate parser accepts them.
Choice payload captures keep the existing declaration-order, exact-field-name, alias, no-shadowing
and nested-pattern restrictions.

### Deliberately unchanged pattern boundaries

This plan does not broaden single-predicate value matching to:

- option `none` patterns
- option literal or relational patterns
- arbitrary scalar literal or relational patterns
- match guards
- nested choice payload patterns
- wildcard patterns

Preserve the current Bool-condition and diagnostic behaviour of forms such as option equality. Do
not reclassify them as single-predicate matches merely because the shared header scanner can see an
`is` token.

The current precedence for choice single predicates at a closed value receiver is retained. The same
source spelling in statement or template control flow keeps its current statement/template meaning.
This consolidation must not add statement or template choice-predicate pattern matching.

### Supported receivers

This plan makes these closed receivers consistent:

- declaration initialisers
- assignment right-hand sides
- returns
- multi-bind with fully known slot types
- multi-bind with one or more inferred slot types

Value-producing control flow remains invalid as a general expression inside calls, operators,
constructors, collection literals, templates or expression statements.

Nested value-producing control flow directly inside another `then` remains deferred. The unused
`ValueReceiverKind::NestedThen` marker does not make that surface implemented. Remove dead internal
scaffolding where possible and correct documentation or status text that currently claims nested
`then` support.

### Branch completeness

Every reachable path through a value-producing branch must either:

- produce the receiver's required values with `then`, or
- terminate through an accepted terminal path such as `return`, `return!` or a statically terminal
  assertion

A branch may contain ordinary statements before its final control-flow outcome. A branch does not
need one syntactically final top-level `then`.

A branch that mixes producing paths and terminating paths is complete. For example, this is valid:

```moth
choose_label |ready Bool, use_fallback Bool| -> String:
    label = if ready:
        if use_fallback:
            then "fallback"
        else
            return "returned"
        ;
    else
        then "waiting"
    ;

    return label
;
```

A branch with any reachable fallthrough path is incomplete. Across the whole value-producing
construct, at least one reachable path must produce values. A construct whose every path terminates
has no value to provide and remains invalid at a value receiver.

Every producing path must match the receiver's arity. Every produced value must satisfy the
receiver's type or an allowed contextual coercion. When the receiver type is inferred, inference
must inspect every producing path rather than only the first `ThenValue` found.

### Required default branch

Inline and block single-predicate value matches always require `else` because one pattern cannot
produce a value for every unmatched input.

The parser must reject a missing `else` before constructing `ValueMatchBlock`. The shared
completeness validator remains responsible for branch flow and value production. It must not be
silently repurposed as the default-arm validator.

Full value matches retain their separate exhaustiveness rules. An exhaustive choice or option full
match may still omit `else =>` where the canonical pattern rules allow it.

---

## Branch and PR contract

The plan and implementation live on `value-producing-control-flow-receiver-consolidation`, created
from the exact planning snapshot above.

While parallel work is active:

- the PR targets `const-folding-and-types-optimisation`
- the branch does not merge back into the parent branch
- the parent branch may continue through its remaining phases
- this branch does not repeatedly rebase to follow it
- `docs/roadmap/roadmap.md` remains untouched

After the parent branch is squash-merged to `main`:

1. Fetch the resulting `main`.
2. Rebase this branch onto that `main`.
3. Resolve the final Stage 4 static-`if` integration deliberately.
4. Re-run the ownership, style, test, documentation and validation gates in Phase 8.
5. Retarget the PR to `main`.
6. Prepare this branch for its own squash merge only after the final audit is clean.

The plan file remains the execution record. Each phase must fill in its `Outcome` section before the
phase commit is accepted.

---

## Current repository shape and root causes

The planning snapshot exposes these concrete owners and gaps.

| Current owner | Current responsibility or gap | Required end state |
|---|---|---|
| `ast/statements/if_headers.rs` | Independently scans top-level `if` header shape and also constructs option-capture scopes | One structural header classifier remains here. Pattern meaning and capture construction move to the match owner |
| `ast/statements/match_headers.rs` | Owns shared option/choice pattern parsing but imports option-capture construction from `if_headers.rs` | Own all option and choice pattern resolution plus capture-scope construction with no reverse dependency on `if_headers.rs` |
| `value_production/receiver/detect.rs` | Re-scans `if` headers for full match, inline predicate and Bool routing, and exports one scan into multi-bind | Delete the file and every duplicate scan it owns |
| `receiver/inline_match.rs` | Speculatively parses the scrutinee, checks pattern eligibility and builds the inline match | Consume a shared single-predicate header result and own only inline-body parsing plus final assembly |
| `receiver/full_match.rs` | Parses the same scrutinee boundary again and owns a generic completeness validator | Reuse the shared scrutinee parser. Move generic value-body completeness to the value-production owner |
| `receiver/block_if.rs` | Parses both bodies, forwards warnings and duplicates completeness validation | Reuse one shared block-body parser and all-path completeness owner |
| `value_production/completeness.rs` | Uses a lossy `BranchFlow` tri-state and stops at the first non-fallthrough statement | Publish an all-path exit summary that can represent produce, terminate and fallthrough independently |
| `value_production/multi_bind.rs` | Has a second parser for partially inferred slots, only recognises full match or Bool, duplicates body parsing and builds an inline block before overwriting its bodies | Keep slot inference and coercion here, but consume shared header, body, completeness and construction owners |
| `receiver/expression_build.rs` and multi-bind builders | Construct the same `ValueIfBlock` and `ValueMatchBlock` shapes in separate layers | One value-production construction owner shared by receivers and multi-bind |
| `ValueIfBlock.result_type_ids` | Block Bool construction copies the pre-parse expected IDs, which may be empty for an inferred declaration, while HIR allocates result locals directly from this field | Store the final inferred or explicit result slot IDs before AST leaves Stage 4 |
| `hir/hir_statement/value_blocks.rs` | Already lowers `ValueIfBlock` and `ValueMatchBlock` through shared result locals and merge blocks | Remain unchanged except for tests or invariant comments required by the corrected AST input |
| Canonical docs and progress matrix | Describe option/choice single predicates as inline-only and list nested `then` as supported | Document block predicates and mark nested `then` as deferred |

### Root cause 1: duplicated header classification

`if_headers.rs` and `receiver/detect.rs` each scan top-level nesting, locate `is`, skip newlines and
interpret the following delimiter. Multi-bind depends on the receiver-local scan, which creates a
third routing surface once its own inferred parser is included.

The fix is one syntax classifier in `if_headers.rs`. Consumers map those structural facts onto their
own semantic surface. The classifier must not decide that an `is` header is a choice pattern based
on source tokens alone.

### Root cause 2: split pattern ownership

`match_headers.rs` owns general pattern parsing and choice captures, but option capture scope
construction lives in `if_headers.rs`. This creates the wrong dependency direction and invites
parallel implementations.

The match owner must build both option and choice capture scopes. Statement, template, inline value,
block value and full match callers then consume one pattern contract.

### Root cause 3: lossy completeness analysis

`BranchFlow` can report only `FallsThrough`, `ProducesValue` or `Terminates`. It therefore classifies
a valid alternative where one path produces and another terminates as fallthrough. Its sequential
walker also returns the first non-fallthrough result, which cannot describe several distinct exits
from nested control flow.

The replacement must track independent exit facts and compose them through statement sequences and
alternative branches.

### Root cause 4: first-produced-value inference

Single-result inference and partially inferred multi-bind currently search for one produced value
per body or arm. Nested control flow can contain several producing paths with different types or
coercion needs. Every authored producing path must participate in arity, inference and coercion.

### Root cause 5: partially inferred multi-bind parser fork

Known multi-bind slots delegate to the normal receiver. Partially inferred slots enter a separate
parser which only distinguishes full match from Bool `if`, duplicates inline and block parsing and
constructs a temporary inline `ValueIfBlock` before replacing its bodies.

This plan does not redesign multi-bind's slot-inference model. It removes its structural parser fork
while leaving slot inference and post-inference coercion under the multi-bind owner.

---

## Architectural invariants

- Stage 4 AST remains the sole owner of this source syntax and semantic validation.
- Value-producing control flow remains accepted only at closed receiving sites.
- One structural `if` header classifier exists in `if_headers.rs`.
- Header classification is syntax-only. Pattern eligibility remains type-aware semantic work.
- `match_headers.rs` owns reusable option and choice pattern parsing plus every capture scope.
- Statement and template `if` preserve their current accepted syntax and diagnostics.
- Single-predicate value matches reuse the existing pattern subset and do not grow guards or scalar
  patterns.
- Block Bool `if`, block single-predicate match and partially inferred multi-bind use one block-body
  parsing protocol.
- Every producing path is checked for completeness, arity, type and coercion.
- A missing `else` is rejected before a single-predicate `ValueMatchBlock` is constructed.
- `ValueIfBlock` and `ValueMatchBlock` remain the complete AST-to-HIR contract.
- HIR, borrow validation and backends receive no new node or operation for this feature.
- A final inferred value block carries non-empty, correct `result_type_ids` for every produced slot.
- User-authored failures remain `CompilerDiagnostic`. Retained-data or internal invariant failures
  remain `CompilerError` through `ExpressionParseError`.
- The implementation deletes obsolete scans, helpers, builders, comments and tests rather than
  retaining compatibility paths.
- No edit in this plan touches `docs/roadmap/roadmap.md`.

---

## Locked implementation decisions

### 1. `if_headers.rs` remains the structural classifier owner

Do not add `if_header_syntax.rs` or another broad scanner module.

The classifier should expose enough structured facts that callers do not rescan the token stream.
Exact Rust names may change, but the result must distinguish at least:

- ordinary Bool-condition shape
- full match shape, `if <scrutinee> is:`
- potential single-predicate shape followed by inline `then`
- potential single-predicate shape followed by block `:`
- statement/template body boundaries where relevant

Useful facts such as the top-level `is` index, first meaningful token after `is` and body delimiter
should be retained when they prevent another walk. Do not retain raw copied token slices or create a
second token authority.

### 2. Semantic interpretation stays consumer-specific

The shared classifier does not make statement and template control flow accept choice patterns.

- Statement/template parsing consumes Bool, option capture and full-match facts as it does today.
- Value receivers may interpret an eligible option capture or choice pattern as a single-predicate
  value match.
- Non-eligible candidates follow the existing Bool or diagnostic path.
- Current option `none` and literal-predicate diagnostics remain stable per source form.

Preserve speculative parsing's two-lane error contract. An authored diagnostic may allow the exact
existing Bool fallback only before a single-predicate shape is committed. An infrastructure error is
never discarded.

### 3. `match_headers.rs` owns patterns and captures

Move `build_option_present_capture_scope_and_pattern` out of `if_headers.rs`.

The final match-header owner should provide narrow functions for:

- parsing a scrutinee through the top-level `is` boundary
- parsing the accepted single-predicate pattern
- building option present-capture scope and final binding path
- building choice payload-capture scope and final binding paths
- parsing full match-arm headers and guards

Do not create temporary `MatchArm` bodies just to reuse pattern logic.

### 4. All-path exit summary replaces `BranchFlow`

Use a small data-oriented summary with independent facts, conceptually:

```rust
struct BranchExitSummary {
    can_fall_through: bool,
    produces_value: bool,
    terminates: bool,
}
```

Exact naming may change. A bitset is acceptable if it remains clearer than the current enum.

Statement sequence composition must model that only paths which still fall through execute the next
statement. Alternative branches union their possible exits. Required behaviour:

- `ThenValue` produces and does not fall through
- `return`, `return!` and accepted terminal assertions terminate and do not fall through
- `if` with `else` unions both branch summaries
- `if` without `else` includes an implicit fallthrough path
- exhaustive match unions every arm and default without inventing fallthrough
- loops and unsupported compound constructs remain conservative unless their current semantics prove
  an all-path exit
- statements after all paths have produced or terminated do not affect the summary

Audit every `NodeKind` that can contain statements. Do not silently omit a lexical block or another
compound node that should recurse.

### 5. Produced-value traversal is complete, not first-match

Create one narrow value-production traversal policy that can:

- inspect every reachable `ThenValue` group for inference and validation
- visit every reachable `ThenValue` group mutably for post-inference coercion
- stop scanning a sequence once no path can reach later statements
- recurse through the same compound nodes as the exit summary

Avoid a broad generic AST visitor framework. Two small read and mutable helpers are preferable when
Rust borrowing makes one abstraction obscure.

Delete `extract_first_multi_produced_values` and any single-result helper that hides later producing
paths once the shared traversal replaces them.

### 6. One block-body parser serves Bool and match forms

A focused receiver-local owner such as `receiver/block_body.rs` should parse the two bodies. It owns:

- consuming or validating the block separator at one documented boundary
- installing `ActiveValueProductionTarget`
- creating the caller-supplied then and else branch contexts
- parsing both statement bodies
- forwarding branch-local warnings
- requiring `else`
- returning both bodies and their exit summaries

`block_if.rs` and `block_match.rs` supply their differing semantic inputs and build their final AST
shapes. The option/choice capture scope applies only to the matched branch. The default branch starts
from the outer receiver context.

### 7. One value-block construction owner serves receiver and multi-bind

Move or widen `receiver/expression_build.rs` only as far as the
`ast/statements/value_production/` owner. Use it for:

- `ThenValue` node construction
- `ValueIfBlock` expression construction
- `ValueMatchBlock` expression construction
- final single or tuple result type construction
- final `result_type_ids` storage

Delete multi-bind's duplicate builders and the path that constructs an inline block with empty
bodies before overwriting them.

Do not move these helpers into a broad compiler utility module.

### 8. `receiver/block_match.rs` owns only match-specific block assembly

The new file should:

1. Consume the shared parsed scrutinee, pattern and capture scope.
2. Parse bodies through the shared block-body owner.
3. Confirm a real `else` exists before AST construction.
4. Create one `MatchArm` with no guard.
5. Store the else body as `Some(default)`.
6. Set `MatchExhaustiveness::HasDefault`.
7. Reuse generic value-match completeness and result inference.
8. Build the existing `ValueMatchBlock` expression.

It must not contain another header scan, option parser, choice resolver, arity parser or completeness
algorithm.

### 9. Multi-bind keeps only its distinct semantic work

Multi-bind continues to own:

- target slot count
- known versus inferred slot types
- per-slot result inference
- validating known slots
- applying coercion after inferred slots are known

It must reuse the common structural parser path for inline Bool, block Bool, full match, inline
single predicate and block single predicate. Do not replace this focused plan with a general receiver
API redesign around `Vec<Option<TypeId>>` unless the current implementation proves that smaller than
the agreed path and a phase audit approves it.

### 10. Downstream lowering remains unchanged

`hir/hir_statement/value_blocks.rs` already allocates hidden result locals from
`result_type_ids`, lowers `ThenValue` into assignments and reuses statement match lowering for
`ValueMatchBlock`.

If implementation appears to need a new HIR node, terminator, backend helper or borrow rule, stop and
re-open the design. The intended fix is to deliver complete existing AST facts to existing lowering.

---

## Expected final module shape

Exact file names may change when a clearer narrow owner emerges. The intended structure is:

```text
src/compiler_frontend/ast/statements/
├── if_headers.rs
├── match_headers.rs
└── value_production/
    ├── completeness.rs
    ├── expression_build.rs
    ├── multi_bind.rs
    ├── parse_values.rs
    ├── types.rs
    └── receiver/
        ├── mod.rs
        ├── block_body.rs
        ├── block_if.rs
        ├── block_match.rs
        ├── full_match.rs
        ├── inline_if.rs
        ├── inline_match.rs
        ├── inline_then_else.rs
        └── result_type.rs
```

`receiver/detect.rs` is absent.

`receiver/token_checkpoint.rs` survives only if the final committed/fallback parser still needs it.
Its survival must be justified by the final ownership audit rather than assumed from the current
shape.

---

## Non-goals

- Implement nested value-producing control flow inside `then`.
- Add statement or template choice-predicate pattern matching.
- Add option `none`, literal, relational or guarded single-predicate value matches.
- Redesign full match arm syntax or exhaustiveness.
- Implement static Bool specialisation. The parent plan owns that work.
- Fold matches or perform general AST partial evaluation.
- Add a general expression form for value-producing control flow.
- Add a new HIR or backend representation.
- Redesign all of multi-bind or the general expected-type system.
- Change build-system, module, package or target architecture.
- Edit `docs/roadmap/roadmap.md`.
- Edit generated files under `docs/release/**` directly.

---

## Mandatory phase workflow

Every phase below is one stable implementation checkpoint and one natural context compaction point.
A phase is not complete until all of these steps are recorded in its `Outcome` section.

1. Re-read the changed module from its entry point and confirm one clear owner.
2. Search changed and adjacent paths for duplicate scans, parsers, validators, builders, legacy
   comments and redundant tests.
3. Review every changed Rust file against the complete codebase style guide.
4. Review test placement, ownership and assertion strength against `testing.mtf`.
5. Run the phase's focused unit and integration commands.
6. Run `cargo fmt --all -- --check`.
7. Run `just validate`.
8. Review progress, documentation, index and audit-freshness effects under repository rules.
9. Fill in the phase `Outcome`, including changed owners, deleted code, diagnostics, tests and exact
   validation results.
10. Commit the phase before starting the next one.

Do not claim a command passed when it was not run. If a gate fails for an unrelated parent-branch
reason, record the exact failure and resolve or explicitly block the phase rather than weakening the
gate.

---

## Phase 0 - Re-anchor, baseline and semantic freeze

### Goal

Confirm the implementation branch still matches this plan's evidence, establish a clean baseline and
freeze current statement, template, full-match and inline-predicate behaviour before refactoring.

### Work items

- [ ] Verify the worktree is on `value-producing-control-flow-receiver-consolidation` and records
      base commit `e782e79d8c4113de4dd835f777bc51b773cffe40`.
- [ ] Confirm no implementation commit from the parent branch was accidentally mixed into this
      branch after creation.
- [ ] Re-read every file named in `Current repository shape and root causes`.
- [ ] Inventory all calls to `parse_if_header`, `find_expression_end_index` used for `if` routing,
      `analyze_branch_flow`, `extract_single_produced_type`,
      `extract_first_multi_produced_values`, `build_value_if_expression` and
      `build_value_match_expression`.
- [ ] Record current file sizes and symbol ownership for `receiver/`, `completeness.rs` and
      `multi_bind.rs`. These are deletion evidence, not performance benchmarks.
- [ ] Use untracked `tmp/` snippets to reproduce:
  - [ ] a block Bool branch whose nested paths mix `then` and `return`
  - [ ] a partially inferred multi-bind using an inline option capture
  - [ ] a partially inferred multi-bind using a block choice predicate
  - [ ] the new option-capture and choice-predicate block forms
- [ ] Record the exact current diagnostic or failure for each reproduction in this phase's
      `Outcome`.
- [ ] Confirm current statement and template option capture still work.
- [ ] Confirm statement/template choice comparison keeps its current Bool meaning.
- [ ] Confirm full value matches and existing inline option/choice predicates still pass.
- [ ] Confirm optional `none` and literal inline predicate rejection cases retain their current
      diagnostic codes.
- [ ] Make no semantic or production-code change in this phase.

### Focused validation

Run at minimum:

```bash
cargo test --lib compiler_frontend::ast::statements::tests::value_production_tests
cargo test --lib compiler_frontend::ast::statements::tests::branching_tests
cargo test --lib compiler_frontend::ast::templates::tests
cargo test --lib compiler_frontend::hir::tests::value_block_lowering_tests
cargo run --quiet -- tests --case value_if_block_declaration_init --backend html
cargo run --quiet -- tests --case value_if_multi_bind_block_terminating_branch --backend html
cargo run --quiet -- tests --case value_if_inline_choice_predicate --backend html
cargo run --quiet -- tests --case option_value_if_inline_unwrap --backend html
cargo run --quiet -- tests --case value_match_declaration_init --backend html
cargo fmt --all -- --check
just validate
```

### Mandatory audit and style review

- [ ] Confirm the plan's current-owner table still matches the branch.
- [ ] Confirm no later parent commit has already solved or moved one of these owners.
- [ ] Review the baseline test inventory for redundant or misleading ownership.
- [ ] Complete the mandatory phase workflow.

### Acceptance

- [ ] The baseline is green except for deliberately untracked reproductions of the known gaps.
- [ ] Current diagnostic identities and contextual syntax precedence are recorded.
- [ ] No semantic code changed.

### Outcome

_To be completed by the implementing agent._

---

## Phase 1 - All-path value-production flow and result discovery

### Goal

Replace the lossy branch-flow model with exact all-path exit facts and ensure inference sees every
producing path before any header-routing refactor begins.

### Why this phase comes first

The new block match must not copy the existing correctness bug. Fixing completeness first gives Bool
blocks, full matches, catch handlers, multi-bind and the later block predicate one shared semantic
foundation.

### Work items

- [ ] Replace `BranchFlow` with a small all-path exit summary that independently records
      fallthrough, value production and termination.
- [ ] Implement statement-sequence composition so only surviving fallthrough paths execute the next
      statement.
- [ ] Implement alternative composition for `if` and exhaustive `match`.
- [ ] Audit every compound `NodeKind` and recurse where its language semantics require it.
- [ ] Preserve the current conservative handling of loops and blocked constructs unless exact proof
      already exists.
- [ ] Preserve current literal-`false` assertion terminality. Do not add constant evaluation or
      Phase G behaviour here.
- [ ] Replace the current value-if and value-match completeness checks with one shared validator
      over exit summaries.
- [ ] Keep explicit `else` validation outside that completeness validator.
- [ ] Update catch-handler value-required validation to accept mixed produce/terminate paths and
      reject every real fallthrough path.
- [ ] Add a complete read traversal for every reachable `ThenValue` group.
- [ ] Add the narrow mutable traversal needed for post-inference coercion, or defer the mutable half
      to Phase 4 only if no Phase 1 caller needs it.
- [ ] Replace first-produced-value inference for inferred single declarations with all-produced-path
      inference.
- [ ] Replace first-produced-value collection in partially inferred multi-bind with all produced
      groups.
- [ ] Diagnose mismatched types from the actual conflicting produced value location where the
      current diagnostic model permits it.
- [ ] Ensure every final `ValueIfBlock` and `ValueMatchBlock` stores the final inferred or explicit
      `result_type_ids` used by HIR.
- [ ] Delete `BranchFlow`, `extract_first_multi_produced_values` and superseded first-result helpers
      when no caller remains.

### Required tests

- [ ] Rewrite the unit test that currently expects produce plus terminate to become fallthrough.
- [ ] Add unit coverage for:
  - [ ] direct production
  - [ ] direct termination
  - [ ] true fallthrough
  - [ ] mixed produce/terminate alternatives
  - [ ] produce/fallthrough alternatives
  - [ ] terminate/fallthrough alternatives
  - [ ] nested `if` and match composition
  - [ ] sequential statements after partial fallthrough
  - [ ] statements after every path has exited
- [ ] Add an integration regression where one branch contains nested produce and terminate paths and
      the other branch produces normally.
- [ ] Add the equivalent partially inferred multi-bind regression if the bug reaches that owner.
- [ ] Add or update catch coverage only when it protects the shared summary at a distinct consumer
      boundary.
- [ ] Add an AST or HIR invariant test proving inferred block value `result_type_ids` are non-empty
      and match the expression result slots.

### Focused validation

Run at minimum:

```bash
cargo test --lib compiler_frontend::ast::statements::tests::value_production_tests
cargo test --lib compiler_frontend::ast::statements::fallible_handling
cargo test --lib compiler_frontend::hir::tests::value_block_lowering_tests
cargo run --quiet -- tests --tag value-blocks --backend html
cargo run --quiet -- tests --tag results --tag value-blocks --backend html
cargo fmt --all -- --check
just validate
```

### Mandatory audit and style review

- [ ] Confirm one exit-summary owner and one produced-value traversal policy remain.
- [ ] Search for every old `BranchFlow` match and first-produced-value helper.
- [ ] Review the sequence algorithm for unreachable-code contamination and conservative loop
      behaviour.
- [ ] Review diagnostics for source location and typed payload preservation.
- [ ] Complete the mandatory phase workflow.

### Acceptance

- [ ] Mixed produce/terminate paths are complete.
- [ ] Any real fallthrough path is rejected.
- [ ] At least one producing path is still required for a value construct.
- [ ] Every producing path participates in inferred type and arity validation.
- [ ] HIR receives final non-empty result slot IDs for inferred value blocks.

### Outcome

_To be completed by the implementing agent._

---

## Phase 2 - One structural `if` header classifier and one capture owner

### Goal

Make `if_headers.rs` the only structural `if` header classifier and make `match_headers.rs` the only
option/choice pattern and capture-scope owner, without changing accepted source behaviour.

### Work items

- [ ] Design one narrow classification result in `if_headers.rs` that retains the structural facts
      needed by statement, template, receiver and multi-bind callers.
- [ ] Perform one nesting-aware scan for top-level `is` and the following body delimiter.
- [ ] Distinguish full-match `is:` from potential inline and block single predicates.
- [ ] Preserve template body boundary recognition without importing template construction into
      `if_headers.rs`.
- [ ] Keep classification syntax-only. Do not inspect `TypeEnvironment` in the scanner.
- [ ] Rewrite `parse_if_header` to consume the classifier while preserving its current
      `ParsedIfHeader` semantics or a cleaner equivalent API.
- [ ] Move option present-capture parsing and scope construction into `match_headers.rs`.
- [ ] Remove `match_headers.rs`'s import of
      `build_option_present_capture_scope_and_pattern` from `if_headers.rs`.
- [ ] Factor one shared scrutinee parser that stops at the top-level `is` token and preserves the
      two-lane expression error boundary.
- [ ] Keep full match-arm guard parsing under `match_headers.rs`.
- [ ] Update file-level documentation to state exact ownership and exclusions.
- [ ] Add no new accepted source form in this phase.

### Required tests

- [ ] Add focused classifier tests for nested parentheses, calls and other expressions containing
      nested tokens.
- [ ] Protect full match, option capture, ordinary Bool and malformed header classification.
- [ ] Protect newline handling around `is`, pattern and delimiter.
- [ ] Protect statement `if` and template option-capture behaviour.
- [ ] Protect statement/template choice equality from accidental pattern reclassification.
- [ ] Protect precise diagnostics for missing conditions and malformed pattern suffixes.

### Focused validation

Run at minimum:

```bash
cargo test --lib compiler_frontend::ast::statements::tests::branching_tests
cargo test --lib compiler_frontend::ast::templates::tests::create_template_node
cargo test --lib compiler_frontend::ast::templates::tests::template_control_flow
cargo run --quiet -- tests --case option_value_if_inline_unwrap --backend html
cargo run --quiet -- tests --case value_if_inline_choice_predicate --backend html
cargo run --quiet -- tests --case value_match_declaration_init --backend html
cargo fmt --all -- --check
just validate
```

### Mandatory audit and style review

- [ ] Search the AST statement tree for every remaining top-level `if`/`is` header scan.
- [ ] Confirm `match_headers.rs` no longer depends on `if_headers.rs` for pattern or capture work.
- [ ] Confirm statement and template consumers have no knowledge of value-receiver-only semantics.
- [ ] Review the classification enum for meaningful states rather than boolean flags.
- [ ] Complete the mandatory phase workflow.

### Acceptance

- [ ] `if_headers.rs` owns one structural classification pass.
- [ ] `match_headers.rs` owns both option and choice capture scopes.
- [ ] Existing source acceptance and diagnostics are unchanged.
- [ ] No broad utility module or copied token authority was added.

### Outcome

_To be completed by the implementing agent._

---

## Phase 3 - Delete receiver detection and unify single-predicate header parsing

### Goal

Route closed receivers through the shared classifier, remove `receiver/detect.rs` and make inline,
full and inferred multi-bind callers consume shared scrutinee and pattern facts.

### Work items

- [ ] Replace `detect::classify_value_if_header` in `receiver/mod.rs` with the shared
      `if_headers.rs` classification.
- [ ] Replace multi-bind's `current_if_header_is_full_match` dependency with the same classification
      result.
- [ ] Preserve the existing receiver-only option `none` and literal-predicate diagnostics without
      another token scan.
- [ ] Create one shared type-aware single-predicate header parser which:
  - [ ] parses the scrutinee once
  - [ ] confirms option present-capture or choice eligibility
  - [ ] consumes `is`
  - [ ] calls the shared match-pattern parser
  - [ ] returns scrutinee, pattern, capture scope and authored body form
- [ ] Preserve Bool fallback only where the existing contextual grammar allows it.
- [ ] Preserve infrastructure errors during speculative parsing.
- [ ] Refactor `inline_match.rs` to own only inline body parsing and final match assembly.
- [ ] Refactor `full_match.rs` to reuse the shared scrutinee parser.
- [ ] Refactor the partially inferred multi-bind entry so it receives shared classification facts,
      even before predicate support is added in Phase 6.
- [ ] Delete `receiver/detect.rs` and remove its `mod` declaration, re-exports, tests and comments.
- [ ] Audit `receiver/token_checkpoint.rs`. Delete it if classification and committed parsing make it
      obsolete. Otherwise document its exact rollback invariant.

### Required tests

- [ ] Protect inline option capture and choice unit/payload predicates.
- [ ] Protect qualified choice predicates.
- [ ] Protect cross-choice and unknown-variant diagnostics.
- [ ] Protect Bool equality fallback for non-pattern subjects.
- [ ] Protect full match routing.
- [ ] Protect optional inline unsupported-predicate diagnostics.
- [ ] Add a focused infrastructure-error test only if the refactor changes the retained-token path.

### Focused validation

Run at minimum:

```bash
cargo test --lib compiler_frontend::ast::statements
cargo run --quiet -- tests --case option_value_if_inline_unwrap --backend html
cargo run --quiet -- tests --case value_if_inline_choice_predicate --backend html
cargo run --quiet -- tests --case value_if_cross_choice_predicate_rejected --backend html
cargo run --quiet -- tests --case option_value_if_none_predicate_rejected --backend html
cargo run --quiet -- tests --case option_value_if_literal_predicate_rejected --backend html
cargo run --quiet -- tests --case value_match_declaration_init --backend html
cargo fmt --all -- --check
just validate
```

### Mandatory audit and style review

- [ ] Confirm `receiver/detect.rs` is deleted.
- [ ] Search for duplicate `next_non_newline_index`, same-line `then` and full-match scans.
- [ ] Confirm inline, full and multi-bind entry paths parse a scrutinee through one owner.
- [ ] Review rollback code for diagnostic versus infrastructure error handling.
- [ ] Complete the mandatory phase workflow.

### Acceptance

- [ ] No receiver-local header classifier remains.
- [ ] One single-predicate semantic header parser serves inline and future block bodies.
- [ ] Existing inline/full/Bool behaviour and diagnostics remain stable.
- [ ] The net parser routing code has decreased.

### Outcome

_To be completed by the implementing agent._

---

## Phase 4 - Shared block-body parsing and value-block construction

### Goal

Fix existing block value-`if` receiver gaps and remove duplicated body and construction code before
adding the new block match.

### Work items

- [ ] Add a narrow shared block-body owner under `receiver/`.
- [ ] Give it explicit then and else parent contexts rather than a boolean capture flag.
- [ ] Install the supplied `ActiveValueProductionTarget` in both body contexts.
- [ ] Parse both bodies through `parse_function_body_statements`.
- [ ] Forward warnings exactly once to the outer receiver context.
- [ ] Require `else` and preserve `ValueIfMissingElse` unless a typed diagnostic review proves a
      distinct reason is required.
- [ ] Return both bodies and their all-path exit summaries.
- [ ] Migrate `block_if.rs` to the shared body owner.
- [ ] Migrate partially inferred multi-bind block Bool parsing to the shared body owner.
- [ ] Move value-block construction helpers to the narrow `value_production` owner so receiver and
      multi-bind share them.
- [ ] Make builders accept final result slot IDs rather than copying pre-parse expected IDs.
- [ ] Delete multi-bind's inline-build-then-overwrite-body path.
- [ ] Delete duplicate multi-bind `ValueIfBlock` and `ValueMatchBlock` expression builders.
- [ ] Reuse the all-path produced-value traversal for partial slot inference and coercion.
- [ ] Keep multi-bind-specific slot inference local and readable.
- [ ] Remove stale comments which describe block value `if` as correct under the old tri-state
      model.

### Required tests

- [ ] Strengthen declaration, assignment, return and multi-bind block success coverage.
- [ ] Cover inferred single-result block declarations through HIR and runtime output.
- [ ] Cover multi-result return and multi-bind result slot allocation.
- [ ] Cover mixed produce/terminate nested paths.
- [ ] Preserve missing `else`, branch fallthrough, arity and type mismatch diagnostics.
- [ ] Add a focused AST construction test only where integration output cannot prove final
      `result_type_ids`.

### Focused validation

Run at minimum:

```bash
cargo test --lib compiler_frontend::ast::statements::tests::value_production_tests
cargo test --lib compiler_frontend::hir::tests::value_block_lowering_tests
cargo run --quiet -- tests --case value_if_block_declaration_init --backend html
cargo run --quiet -- tests --case value_if_return_block --backend html
cargo run --quiet -- tests --case value_if_return_multi_block --backend html
cargo run --quiet -- tests --case value_if_multi_bind_block --backend html
cargo run --quiet -- tests --case value_if_multi_bind_block_terminating_branch --backend html
cargo run --quiet -- tests --case value_if_branch_fallthrough_rejected --backend html
cargo fmt --all -- --check
just validate
```

### Mandatory audit and style review

- [ ] Confirm one block-body parser exists.
- [ ] Confirm one value-block construction owner exists.
- [ ] Search for temporary AST construction followed by body replacement.
- [ ] Search for duplicated warning forwarding, `else` checks and active-target setup.
- [ ] Review parameter lists and use an input struct where it improves data flow.
- [ ] Complete the mandatory phase workflow.

### Acceptance

- [ ] Existing block Bool value `if` follows the accepted all-path contract.
- [ ] Inferred block results carry correct HIR result slots.
- [ ] Multi-bind no longer builds and mutates a fake inline block.
- [ ] Block body and construction duplication is removed.

### Outcome

_To be completed by the implementing agent._

---

## Phase 5 - Block single-predicate matches for explicit and known receivers

### Goal

Add `receiver/block_match.rs` and route option and choice single-predicate block forms through the
consolidated receiver architecture for declarations, assignments, returns and fully known
multi-bind slots.

### Work items

- [ ] Add `receiver/block_match.rs` with file-level ownership and exclusion documentation.
- [ ] Route a shared single-predicate header followed by `:` to the block match parser.
- [ ] Reuse the parsed scrutinee, pattern and capture scope from Phase 3.
- [ ] Parse the matched body with the capture scope as its parent.
- [ ] Parse the default body from the outer receiver context so captures cannot leak.
- [ ] Require `else` before building any `ValueMatchBlock`.
- [ ] Build one `MatchArm` with `guard: None`.
- [ ] Store `Some(else_body)` and `MatchExhaustiveness::HasDefault`.
- [ ] Reuse the shared all-path completeness validator.
- [ ] Reuse value-match result inference and final slot construction.
- [ ] Reuse the shared value-block expression builder.
- [ ] Keep option `none`, literal/relational and guarded forms outside this route.
- [ ] Add no new HIR, borrow or backend operation.

### Required integration coverage

Prefer one strong primary success case with several functions over many tiny duplicate fixtures.
Cover at least:

- [ ] option present and absent runtime paths
- [ ] choice unit predicate
- [ ] qualified choice unit predicate
- [ ] choice payload capture
- [ ] capture alias where already supported
- [ ] declaration initialiser
- [ ] assignment right-hand side
- [ ] return receiver
- [ ] fully known multi-bind receiver
- [ ] ordinary statements before `then`
- [ ] nested produce/terminate branch paths

Add focused failure coverage for:

- [ ] missing `else`
- [ ] matched branch fallthrough
- [ ] default branch fallthrough
- [ ] no producing path
- [ ] wrong arity
- [ ] incompatible result type
- [ ] capture shadowing
- [ ] unknown or cross-choice variant
- [ ] capture use in the default branch

Protect statement and template semantics with existing cases rather than creating redundant copies.

### HIR boundary proof

- [ ] Extend an existing value-block lowering test or add one focused AST fixture showing that the
      new block syntax produces the existing one-arm `ValueMatchBlock` shape.
- [ ] Confirm HIR still lowers through `lower_value_block_match` and statement match lowering.
- [ ] Confirm no HIR production file changed unless an invariant comment or bug fix in existing
      input validation is strictly required.

### Focused validation

Run at minimum:

```bash
cargo test --lib compiler_frontend::ast::statements
cargo test --lib compiler_frontend::hir::tests::value_block_lowering_tests
cargo run --quiet -- tests --case <new-primary-block-predicate-case> --backend html
cargo run --quiet -- tests --case <new-missing-else-case> --backend html
cargo run --quiet -- tests --case value_if_inline_choice_predicate --backend html
cargo run --quiet -- tests --case option_value_if_inline_unwrap --backend html
cargo run --quiet -- tests --case value_match_declaration_init --backend html
cargo fmt --all -- --check
just validate
```

### Mandatory audit and style review

- [ ] Confirm `block_match.rs` owns no header scan, pattern parser or independent completeness logic.
- [ ] Confirm the capture scope is branch-local and the default branch uses the outer context.
- [ ] Confirm `else` is checked before AST construction.
- [ ] Confirm no downstream representation changed.
- [ ] Complete the mandatory phase workflow.

### Acceptance

- [ ] Block option capture and choice predicates compile and run at known receivers.
- [ ] Inline and block forms use one pattern/capture authority.
- [ ] Every invalid branch shape receives the established diagnostic family.
- [ ] HIR consumes the existing `ValueMatchBlock` unchanged.

### Outcome

_To be completed by the implementing agent._

---

## Phase 6 - Partially inferred multi-bind parity and final parser deletion pass

### Goal

Give partially inferred multi-bind the same inline and block single-predicate feature set as every
other closed receiver, then remove the remaining parser and construction duplication.

### Work items

- [ ] Route partially inferred multi-bind through the shared `if` header classification.
- [ ] Support existing inline option and choice single predicates for partially inferred slots.
- [ ] Support new block option and choice single predicates for partially inferred slots.
- [ ] Reuse the shared single-predicate scrutinee and pattern parser.
- [ ] Reuse the shared block-body parser.
- [ ] Set `ActiveValueProductionTarget.expected_arity` from the multi-bind target count.
- [ ] Collect every produced value group across nested paths.
- [ ] Infer each unknown slot from every producing path.
- [ ] Validate every known slot against every producing path.
- [ ] Apply contextual coercion to every `ThenValue` group after final slot inference.
- [ ] Preserve terminating paths as valid non-producing paths.
- [ ] Keep actual slot inference and mismatch diagnostics under the multi-bind owner.
- [ ] Delete old multi-bind parsing functions whose only role is now shared.
- [ ] Split multi-bind-specific inference/coercion into a narrow submodule only if the final file
      remains hard to review. Do not create a generic parser framework or another receiver path.
- [ ] Re-measure source lines and symbol inventory against Phase 0 and record the deleted paths.

### Required tests

- [ ] Partially inferred inline option capture.
- [ ] Partially inferred block option capture.
- [ ] Partially inferred inline choice unit or payload predicate.
- [ ] Partially inferred block choice payload predicate.
- [ ] A mix of known and unknown slots.
- [ ] Nested producing paths with the same inferred slot types.
- [ ] Nested producing paths with a type mismatch.
- [ ] A terminating path plus producing paths.
- [ ] Arity mismatch at a nested `then`.
- [ ] Capture scope use in produced values and no leakage into default.

### Focused validation

Run at minimum:

```bash
cargo test --lib compiler_frontend::ast::statements
cargo run --quiet -- tests --tag value-blocks --tag multi-bind --backend html
cargo run --quiet -- tests --case <new-partial-inline-predicate-case> --backend html
cargo run --quiet -- tests --case <new-partial-block-predicate-case> --backend html
cargo run --quiet -- tests --case value_if_multi_bind_type_mismatch --backend html
cargo run --quiet -- tests --case value_if_multi_bind_arity_mismatch --backend html
cargo fmt --all -- --check
just validate
```

### Mandatory audit and style review

- [ ] Search `multi_bind.rs` for copied Bool, match, inline and block parser logic.
- [ ] Confirm it owns only slot-specific inference, validation and coercion.
- [ ] Confirm all produced-value traversal uses the Phase 1 owner.
- [ ] Confirm no old builders or body-overwrite paths survive.
- [ ] Review file size and split only along a real remaining responsibility boundary.
- [ ] Complete the mandatory phase workflow.

### Acceptance

- [ ] Known and partially inferred multi-bind accept the same value-control-flow forms.
- [ ] Every produced path participates in slot inference and coercion.
- [ ] Multi-bind has no independent header or body grammar.
- [ ] The receiver and multi-bind subsystem is materially smaller than the Phase 0 baseline after
      accounting for `block_match.rs`.

### Outcome

_To be completed by the implementing agent._

---

## Phase 7 - Diagnostics, canonical documentation, status and ownership closeout

### Goal

Make the user-facing contract, implementation status, diagnostic model and codebase navigation match
the completed architecture without touching the roadmap.

### Diagnostics

- [ ] Reuse `ValueIfMissingElse`, `ValueIfBranchFallsThrough`, `ValueIfNoProducingPath`, existing
      return-shape reasons and existing type mismatch contexts where they accurately describe the
      new form.
- [ ] Preserve option unsupported-predicate and match-pattern diagnostics.
- [ ] Add a new typed reason only if no existing payload accurately describes a real source error.
- [ ] If a new reason is added, update reason keys, renderer coverage, diagnostic model tests and
      integration expectations in the same phase.
- [ ] Confirm infrastructure errors remain distinct from authored diagnostics through speculative
      and recursive body parsing.

### Test ownership and pruning

- [ ] Audit every new integration contract and role in `tests/cases/manifest.toml`.
- [ ] Keep one primary contract owner for block single-predicate value matching.
- [ ] Keep one unit-test owner for the hidden exit-summary invariant.
- [ ] Remove superseded or implementation-shaped tests made redundant by stronger end-to-end cases.
- [ ] Run the integration suite audit and resolve hard findings.

### Canonical documentation

Update the accepted language contract and teaching surface:

- [ ] `docs/src/docs/branching/value-producing-if.mtf`
  - [ ] document inline and block option/choice predicates
  - [ ] document mixed produce/terminate completeness
  - [ ] list only implemented closed receivers
  - [ ] state nested `then` is deferred
- [ ] `docs/src/docs/branching/value-producing-if-basic.mtf`
  - [ ] add the smallest useful block option example
  - [ ] keep the Basic page focused
- [ ] `docs/src/docs/errors/options.mtf`
  - [ ] show block present-value inspection
  - [ ] keep full option-match rules separate
- [ ] `docs/src/docs/branching/pattern-matching.mtf`
  - [ ] explain single-predicate value matches beside full value matches
- [ ] `docs/src/docs/branching/patterns-and-exhaustiveness.mtf`
  - [ ] state the narrower single-predicate subset and required `else`
- [ ] `docs/src/docs/choices/payload-patterns.mtf`
  - [ ] mention payload captures work in inline and block single predicates where helpful
- [ ] `docs/src/docs/cheatsheet/moth-language-cheatsheet.md`
  - [ ] add the block form
  - [ ] remove or correct nested-`then` overstatement
- [ ] Review `docs/compiler-design-overview.md`. Change it only if implementation altered the Stage 4
      ownership contract, which is not expected.

### Progress matrix and deferred work

- [ ] Update the Pattern matching row to mention inline and block option/choice single predicates
      and their focused coverage.
- [ ] Update the Results, options, multiple returns and multi-bind row to list the supported closed
      receivers and the remaining nested-`then` gap.
- [ ] Keep Pattern matching `Partial` while unrelated deferred pattern features remain.
- [ ] Do not invent a new status label.
- [ ] Record nested `then` as an existing deferred implementation gap, not a new language proposal.
- [ ] Do not add a roadmap item for nested `then` in this plan.
- [ ] Do not change status for full relational overlap, nested payload patterns or other unrelated
      deferred work.

### Codebase navigation and dead scaffolding

- [ ] Remove `ValueReceiverKind::NestedThen` and its dead-code allowance unless a real current caller
      requires the marker after the refactor.
- [ ] Update `index.md` only if the final owner boundary is materially unclear without it. Do not add
      a noisy line merely because `detect.rs` was deleted or `block_match.rs` was added.
- [ ] Update file-level documentation for every moved or deleted owner.
- [ ] Confirm `docs/roadmap/roadmap.md` has no diff.
- [ ] Do not edit `docs/release/**` directly.

### Focused validation

Run at minimum:

```bash
cargo run --quiet -- tests --audit
cargo run --quiet -- tests --tag value-blocks --backend html
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
cargo fmt --all -- --check
just validate
```

### Mandatory audit and style review

- [ ] Review every changed diagnostic constructor and renderer.
- [ ] Review integration contract ownership and remove redundant fixtures.
- [ ] Read every changed canonical page directly and through the docs build.
- [ ] Confirm progress status matches implementation and tests.
- [ ] Confirm roadmap and generated release sources were untouched.
- [ ] Complete the mandatory phase workflow.

### Acceptance

- [ ] Documentation describes the implemented syntax and no broader surface.
- [ ] Nested `then` is truthfully deferred.
- [ ] Progress notes and coverage are current.
- [ ] Diagnostic identities are stable or deliberately extended through typed owners.
- [ ] The roadmap has no change.

### Outcome

_To be completed by the implementing agent._

---

## Phase 8 - Parent completion rebase, Phase G reconciliation and final audit

### Goal

Integrate the completed receiver work with the final constant-folding/static-`if` architecture after
the parent branch has squash-merged to `main`, then prove the combined tree is ready for its own
squash merge.

### Entry gate

Do not start this phase until `const-folding-and-types-optimisation` is complete and its final squash
commit exists on `main`.

### Rebase and re-anchor

- [ ] Fetch the completed `main` and record its squash commit in this phase's `Outcome`.
- [ ] Re-read the final current-state and Phase G outcome in
      `constant-folding-and-type-system-hot-path-optimization-plan.md`.
- [ ] Rebase `value-producing-control-flow-receiver-consolidation` onto the completed `main`.
- [ ] Resolve conflicts by preserving one current owner, not by restoring adapters or parallel
      paths.
- [ ] Re-read every changed module from its entry point after conflict resolution.
- [ ] Retarget the PR from `const-folding-and-types-optimisation` to `main` only after the rebase is
      complete.

### Static Bool integration review

- [ ] Confirm static Bool specialisation consumes fully validated `ValueIfBlock` bodies after this
      plan's completeness, arity and type checks.
- [ ] Confirm inactive Bool branches do not publish downstream executable work under the parent
      plan's contract.
- [ ] Confirm single-predicate `ValueMatchBlock` is not accidentally treated as a Bool `if` or folded
      by a new unsupported match specialiser.
- [ ] Confirm selected branch lexical scope and option/choice capture scope remain correct.
- [ ] Confirm final `result_type_ids` survive specialisation and HIR construction.
- [ ] Confirm statement and template control flow still use the shared header owner without gaining
      value-receiver semantics.
- [ ] Confirm parent benchmark counters or generic-request checkpoints in `branching.rs` retain one
      owner and no stale compatibility path was introduced.

### Final deletion and architecture audit

Audit in the repository-required order:

- [ ] Re-check compiler architecture, canonical language references, style guide and this plan.
- [ ] Read `if_headers.rs`, `match_headers.rs`, `value_production/mod.rs`, `receiver/mod.rs`, every
      receiver file, `completeness.rs`, `multi_bind.rs` and HIR value-block lowering in full.
- [ ] Search for duplicate header scans, scrutinee parsers, option capture builders, block-body
      parsers, completeness validators, produced-value walkers and AST builders.
- [ ] Confirm `receiver/detect.rs` is absent.
- [ ] Confirm no compatibility wrapper or forwarding shim survived the rebase.
- [ ] Review API shape, context structs, comments, visibility and imports.
- [ ] Review every diagnostic lane and source location.
- [ ] Review every integration contract and unit-test owner.
- [ ] Review progress, docs, index and audit freshness.
- [ ] Confirm the final diff contains no `docs/roadmap/roadmap.md` change.

### Final validation

Run at minimum:

```bash
cargo test --lib compiler_frontend::ast::statements
cargo test --lib compiler_frontend::ast::templates
cargo test --lib compiler_frontend::hir::tests::value_block_lowering_tests
cargo run --quiet -- tests --tag value-blocks --backend html
cargo run --quiet -- tests --audit
cargo run --quiet -- check docs --terse
cargo run --quiet -- build docs --release
cargo fmt --all -- --check
just bench-check
just validate
```

Run any new Phase G focused cases required by the final parent plan in addition to this list.

### Acceptance

- [ ] The branch is based on completed `main`.
- [ ] Static Bool specialisation and consolidated value receivers have one coherent Stage 4 order.
- [ ] No new HIR or backend representation exists.
- [ ] Every intended deletion remains deleted after rebase.
- [ ] All focused and full gates pass.
- [ ] Every phase `Outcome` is complete and every checkbox accurately reflects the final tree.
- [ ] The PR is retargeted to `main` and ready for final review and squash merge.

### Outcome

_To be completed by the implementing agent._

---

## Final acceptance contract

The plan is complete only when all of the following are true:

### Language behaviour

- [ ] Inline option and choice single predicates still work.
- [ ] Block option and choice single predicates work at declaration, assignment, return and
      multi-bind receivers.
- [ ] Choice payload captures use the canonical capture contract.
- [ ] Single-predicate block matches require `else`.
- [ ] Every producing path satisfies arity and type rules.
- [ ] Mixed producing and terminating paths are accepted.
- [ ] Any real fallthrough path is rejected.
- [ ] At least one path produces values.
- [ ] Nested `then` remains rejected and documented as deferred.
- [ ] Statement and template semantics are unchanged.

### Architecture and deletion

- [ ] `receiver/detect.rs` is deleted.
- [ ] One structural `if` header classifier remains.
- [ ] One option/choice pattern and capture owner remains.
- [ ] One block-body parser remains.
- [ ] One all-path exit-summary owner remains.
- [ ] One produced-value traversal policy remains.
- [ ] One value-block construction owner remains.
- [ ] Multi-bind contains no second `if` grammar.
- [ ] No temporary inline block is constructed and overwritten for block parsing.
- [ ] Dead nested-then scaffolding is removed unless a real caller justifies it.
- [ ] Net receiver and multi-bind routing complexity is lower than the Phase 0 baseline.

### Stage boundaries

- [ ] `ValueIfBlock` and `ValueMatchBlock` remain the AST handoff.
- [ ] Final `result_type_ids` are complete for inferred and explicit receivers.
- [ ] HIR value-block lowering remains the existing result-local and merge-block path.
- [ ] Borrow validation and backends need no feature-specific change.
- [ ] Static Bool specialisation remains owned by the completed parent plan.

### Diagnostics, tests and docs

- [ ] User failures use typed diagnostics with useful locations.
- [ ] Integration cases own observable language behaviour.
- [ ] Unit tests own only hidden flow and construction invariants.
- [ ] The integration suite audit is clean.
- [ ] Canonical docs, Basic docs where useful, cheatsheet and progress matrix are current.
- [ ] `docs/roadmap/roadmap.md` is unchanged.
- [ ] Generated release docs were rebuilt, not edited directly.

### Integration and validation

- [ ] The branch was rebased onto the completed parent squash on `main`.
- [ ] The final PR targets `main`.
- [ ] The repository Final audit is clean.
- [ ] `just validate` passes on the final rebased tree.

---

## Handoff summary for the coding agent

Start with completeness, not syntax. The existing tri-state flow model cannot represent the accepted
produce-or-terminate contract, and the new block match must not copy that bug.

Then establish one `if_headers.rs` classifier and one `match_headers.rs` pattern/capture owner. Delete
`receiver/detect.rs` before adding the new block form. Share block body parsing and value-block
construction, fix inferred result slot storage and collapse the partially inferred multi-bind parser
onto those owners.

Only after that foundation is clean should `receiver/block_match.rs` be added. It is a small adapter:
one parsed predicate arm, one required default body, the existing `ValueMatchBlock` and no new HIR.

Do not touch the roadmap. Do not chase the parent branch during implementation. Reconcile once, after
the parent work has squash-merged to `main`, then perform the full final audit and validation again.
