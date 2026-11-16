# `$supplied` template directive implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans`. Execute the checked tasks in order, use test-first slices and perform the required Slice reviews.

**Status:** Queued immediately after Constraints and fast arithmetic. Start once that prerequisite has landed.

**Goal:** Add `$supplied` as a small, compiler-owned conditional template directive that emits its body once when its target slot receives at least one structurally routed contribution.

**Architecture:** Extend the existing template-head parser and AST-local TIR slot schema. Resolve presence using the existing `TirSlotContributions::slot_has_routed_presence(&SlotKey)` fact during composition, then retain only the selected ordinary content for folding or runtime handoff. Do not turn slot presence into an expression or a backend operation.

**Tech stack:** Rust, Moth template frontend and TIR, existing module-local tests, manifest-driven `tests/cases/` integration suite and canonical `.mtf` documentation.

**Repository destination:** `docs/roadmap/plans/template-supplied-directive-plan.md`. Remove the plan and its roadmap entry in the same commit that completes implementation.

## Global constraints

- Read `AGENTS.md`, `index.md`, the current developer style/testing/validation guides, the canonical template slots/directives/control-flow references and `docs/compiler-design-overview.md > Templates and TIR` before editing. Canonical references own semantics over this temporary plan.
- Keep `$supplied` a narrow composition-time directive. `$unique`, `$only`, typed slots, slot-value comparisons, `if $slot`, runtime DOM behaviour and the `@core/html` / `@core/css` package expansion are separate work.
- Reuse the existing slot schema, routing buckets, exact `TirView` overlays, TIR node walkers and preparation/folding/runtime paths. Each fact has one owner. Avoid a second registry, slot-routing pass, runtime presence map, string scan, generic conditional framework or compatibility shim.
- Follow Moth's diagnostic policy: source errors use typed `DiagnosticDraft` family constructors with accurate spans. Invalid source must not reach `compiler_bug!` or an unchecked panic.
- Use module-local Rust tests for parser/TIR invariants and `tests/cases/` for language behaviour. Extend existing owners when possible. Use meaningful output assertions and exact diagnostic codes rather than redundant smoke cases.
- Work in small reviewed commits. Use targeted checks during implementation, `just validate` at an important integration boundary and `just validate-full` on the final merge-ready tree. The implementation's last gate is the repository's Slice review.
- The feature must remain backend-neutral. No HTML builder special cases or new runtime opcode should be needed.

## Accepted language contract

### Syntax

The directive has one explicit body and accepts the same key kinds as `$slot`:

```moth
[$supplied title:
    <h2>[$slot title]</h2>
]

[$supplied 1:
    src="[$slot 1]"
]

[$supplied:
    <section>[$slot]</section>
]
```

It is a dedicated conditional child template. An ordinary formatter may share the head, for example `[$html, $supplied title: ...]`. Arbitrary template references, `if`, `loop` or another structural condition may not share that head. There is no `else` arm, operand expression or slot default assignment on `$supplied`.

### Semantics

1. **Declare and test.** `$supplied key` declares a routable slot even if no `[$slot key]` occurrence exists. It is valid as a presence-only flag. When both occur, they refer to the same receiving slot and contributions.
2. **Owning scope.** The guard and `[$slot key]` references inside its body use the enclosing receiving template's slot schema. The guard does not create a separate slot-routing scope. Independently applied nested templates retain their normal scopes.
3. **Structural presence.** The guard is true iff at least one contribution was routed to the key for this template application. Explicitly empty contributions count. Occurrence defaults and fallbacks do not. Head-only wrappers remain open under existing composition rules.
4. **Runtime branches.** A routed insert inside a runtime template `if` counts even if the branch later emits nothing. This is intentional: `$supplied` is unsuitable for toggling live HTML attributes based on runtime state.
5. **Cardinality.** The body emits once per true `$supplied` occurrence, regardless of the number of contributions. Multiple guard occurrences for one key each emit their own body. Ordinary `[$slot key]` occurrences preserve concatenation, replay and their per-occurrence defaults.
6. **No output vs empty output.** A false guard yields structural no-output, so its own wrappers and content do not leak. A true guard with an empty body remains selected and obeys the existing selected-empty wrapper rule. Whitespace or HTML delimiters are emitted only if authored.
7. **Unused value liveness.** When the only consumer of a slot is its `$supplied` check, contribution payloads are validated but not evaluated or materialised for runtime output. When a `[$slot key]` consumes them, preserve ordinary expression execution and output. Continue required type, const-required and resource-path validation even for discarded or inactive content.
8. **Not a value predicate.** Presence does not inspect String length or content. The directive adds no slot uniqueness or typed-attribute validation.

Example illustrating the expected HTML whitespace contract:

```moth
paragraph #= [$html:
    <p[$supplied style:[" style=\""][$slot style]["\""]]>[$slot]</p>
]
disabled #= [$in disabled:]
button #= [$html:
    <button[$supplied disabled:[" disabled"]]>[$slot]</button>
]
```

`$supplied` never inserts a leading space or performs escaping. The attribute fragment owns the space.

### Why this boundary

The one-word directive is explicit and supports optional wrappers and boolean presence flags without turning `$slot` into a Bool or general String-inspection expression. More elaborate component logic belongs in ordinary Moth and const functions.

## Ownership and implementation constraints

Current main-branch anchors (reconfirm after prerequisite merges):

- Parsing and head restrictions: `src/compiler_frontend/ast/templates/template_head_parser/core_directives.rs`, `head_parser.rs` and `control_flow_suffix.rs`. Reuse the current `SlotKey` parser checks and existing source-diagnostic families.
- Slot schema and routing: `src/compiler_frontend/ast/templates/tir/slot_layout.rs`, `slot_composition/contributions.rs` and `slot_composition/schema.rs`. The current `TirSlotSchema` registers unique key targets, and `TirSlotContributions` separately records empty routed presence. Reuse that state.
- Representation and phase handoffs: `tir/node.rs`, `store/control_flow.rs`, `preparation.rs`, `fold/`, `handoff_materialization.rs` and the exact-view walkers. Model the guard as structural template control, not a fabricated Bool expression or an HIR/runtime attribute toggle.
- Test owners: `src/compiler_frontend/ast/templates/tir/tests/` for local invariants and `tests/cases/manifest.toml` plus existing template-related cases for user-visible behaviour. Follow `testing.mtf`'s contract/role and backend assertion rules.
- Documentation after implementation: `docs/docs/templates/template-directives.mtf`, `template-slots.mtf`, `template-control-flow.mtf`, `docs/docs/cheatsheet/moth-language-cheatsheet.mtf` and the relevant TIR subsection of `docs/compiler-design-overview.md`.

**Stage ordering:** Discover and register `$supplied`'s slot target during normal schema collection before contributions route. Resolve its guard only against the complete routed-presence fact for that receiving application. Do not prune its declaration or skip required validation because the guard eventually proves false. Preserve exact-view identity through copy, overlay, imported-constant rebuild and runtime materialisation. Reuse a current node/conditional representation where it preserves these distinctions, adding the smallest targeted structural node only if needed.

**Implementation stop condition:** Check the agreement that unused presence-only payloads do not execute against the current runtime-template effect and evaluation contracts. If it contradicts an established observable execution guarantee, stop and raise that concrete incompatibility for design review. Do not silently drop effects or broaden `$supplied` into a runtime condition to work around it.

## Review focus

Each risk below needs an explicit owner and an assertion that would catch the regression:

- Explicitly empty named, positional and default contributions versus no contribution and occurrence fallbacks.
- A structural insert behind a runtime `if` that is false at execution time.
- A presence-only runtime interpolation that would have a visible side effect, contrasted with normal `[$slot]` consumption.
- Forwarded, stored, imported and repeated occurrences using the receiving application's correct slot identity.
- False-guard structural no-output, selected-empty wrapper behaviour, author-owned attribute whitespace and resource validation without unintended emission.

## Task 1: Establish failing coverage and parse the surface

**Files:** Existing head parser and directive registry, local template parser tests. Avoid a new parser subsystem.

- [ ] Confirm the prerequisites, inspect the current owners and existing template cases, then establish a clean baseline and feature branch. Record the activation SHA in working notes, not this queued plan.
- [ ] Add focused failing syntax tests for the three `$supplied` key forms and a formatter plus guard. Add negative cases for malformed keys, default assignment, missing body and prohibited head combinations. Assert precise diagnostic codes/spans.
- [ ] Run the narrow parser test filter and confirm it fails for the new syntax.
- [ ] Extend current directive parsing with a slot-key-bearing structural condition and normal body parsing. Reuse `SlotKey` rules rather than reimplementing them.
- [ ] Re-run focused tests and inspect the diff for new parser branching that duplicates `$slot` or `if`. Commit the coherent slice.

## Task 2: Register and resolve guard presence through existing TIR routing

**Files:** `tir/slot_layout.rs`, `tir/slot_composition/{contributions,schema}.rs`, the minimal TIR node/store/walker owners and module-local TIR tests.

- [ ] Add a failing focused test where `[$supplied disabled:[" disabled"]]` is the only declaration and `disabled #= [$in disabled:]` fills it. Assert a single emitted ` disabled`. Add an absent case, repeated inserts and a sibling `[$slot disabled]`.
- [ ] Add one compact table-driven test covering named, positional and default keys for absent, explicit empty, nonempty and default-only states. Verify false guards produce no output and true guards emit once.
- [ ] Run the targeted tests red. Extend schema collection to recognise the guard's key and resolve it against `slot_has_routed_presence` at the existing composition boundary, including when the only declaration is the guard. Preserve routing across independent nested template boundaries.
- [ ] Add focused checks for head-only open references, multiple guards on one key, wrapper interactions and a guard containing nested insert syntax. Prevent the guard from becoming its own routed contribution or self-satisfying cycle.
- [ ] Run focused composition/fold tests green. Audit all affected exact-view traversals and conditional/wrapper source spans. Commit.

**Review gate A - structure:** Inspect the parser and TIR diff for duplicate presence maps, schema scans, copies, allocations and cross-stage leakage. Confirm the new directive has one authoritative semantic path and no old path left beside it. Correct issues before proceeding.

## Task 3: Runtime semantics, laziness and resource correctness

**Files:** Existing TIR preparation, fold and handoff owners, HIR only if an existing handoff requires it, one cohesive integration case under `tests/cases/` and minimal focused invariants.

- [ ] Extend or create one primary template-composition integration case with meaningful exact output. Exercise optional `style="..."`, a presence-only `disabled` attribute, named/default/positional routing, repeated contribution and an explicitly empty insert. Register a unique case ID, contract and role in `manifest.toml`.
- [ ] Add a runtime path with a routed insert under a dynamic template `if`. Assert presence is structurally true even when the branch renders no content. Verify both true and false runtime conditions.
- [ ] Add a side-effect sentinel: a runtime interpolation routed only to `$supplied` does not execute, but the same insertion consumed by `[$slot]` does. Add a focused rejected-source case proving that a discarded expression still gets normal semantic diagnostics.
- [ ] Test false-guard resource, formatting and wrapper elision and true-guard resource retention. Invalid authored resource paths remain diagnosed even when their output is unused. Check imported/re-exported and stored-insert forms at their existing boundary.
- [ ] Run targeted case(s) red, then use the existing TIR preparation/folding/runtime handoff to preserve the agreed semantics without a runtime `$supplied` opcode or new backend-side routing. Run backend-specific expectations only where those targets support the relevant runtime features, using explicit intended rejections elsewhere.
- [ ] Verify focused cases green, inspect discarded-content liveness and resource tracking for unwanted work, then commit.

## Task 4: Diagnostics, composition and consumer sweep

**Files:** Existing TIR exact-view/copy/overlay walkers, template diagnostic owners, local tests and a small number of distinct negative integration cases.

- [ ] Cover malformed directive placement, illegal head combinations and unknown inserts with typed, source-located diagnostics. Reuse existing reasons where truthful and add one narrow reason only for a genuinely new rule.
- [ ] Add the missing adversarial interaction cases after comparing existing coverage: nested guards, forwarded wrappers, `$before` / `$after` and `$fresh`, imported constant rebuilding, repeated slot occurrences and head-only composition. Use output assertions that expose missing/duplicate text.
- [ ] Check every exhaustive TIR node/selector consumer, including slot layout, subtree copy, expression overlays, formatters, folding, preparation, resource discovery and runtime handoff. Preserve all context and source identity. Avoid redundant variant-specific walkers.
- [ ] Run targeted tests and a relevant integration tag or contract group. Remove any duplicated tests or leftover scaffolding only when the surviving owner protects the same behaviour. Commit.

**Review gate B - correctness and simplicity:** Read the full touched pipeline with the repository's `AGENTS.md` review criteria and style guide. Check evaluation order, no-output semantics, source diagnostics, stage boundaries, duplicate helpers and unnecessary allocation. Review the Slice for untested negative interactions and remove real redundancy before documentation.

## Task 5: Canonical documentation and completion

**Files:** Canonical template pages, cheatsheet and compiler design overview as appropriate. No changes to `@core/html` or `@core/css` exports.

- [ ] Add one compact source-backed example for optional attributes and one for a presence-only boolean. Describe all key types, routing-based presence, defaults, conditional runtime contributions, selected-empty output and the no-evaluation rule for unconsumed payloads.
- [ ] Document syntax restrictions and exclusions alongside existing directives and template control flow. Update the TIR architecture only for actual semantic ownership changes, without duplicating the full user reference.
- [ ] Run focused compiler and integration checks and the docs source check. At an important integration boundary run `just validate`. Run `just validate-full` on the final merge-ready implementation tree as required by `validation.mtf`.
- [ ] Perform the final **Slice review** required by `AGENTS.md`: verify every design rule has a primary assertion, scan for duplicate/legacy paths and stale documentation, ensure no user-input panics or new unneeded abstraction, and review the progress matrix and index update rules.
- [ ] Re-run affected checks after final changes. Record exact commands and outcomes, commit the final fixes, then hand over branch/commit IDs and remaining deferred work. Do not merge without the normal approval workflow.

## Acceptance

- [ ] All three source forms work and `$supplied` alone declares its target.
- [ ] Only routed presence controls the guard. Defaults, nonempty text and runtime execution do not redefine it.
- [ ] Multiple contributions trigger one body per guard occurrence, with no accidental wrapper or whitespace output when absent.
- [ ] Discarded presence-only payloads remain semantically validated without runtime evaluation. Ordinary `[$slot]` consumers keep existing evaluation.
- [ ] Constant, runtime, nested, forwarded, imported and resource cases preserve current TIR and backend contracts.
- [ ] Diagnostics remain precise and user input cannot panic.
- [ ] The implementation reuses existing owners, passes reviewed tests and full merge validation, and completes both interim reviews and the final Slice review.

## Deferred

`$unique`, `$only`, typed slots, matching or inspecting slot values, new behaviour packages and `@core/html` / `@core/css` changes remain out of scope.