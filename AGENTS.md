# Moth agent rules

Resolve relative paths from the current worktree root. Do not read project references from another worktree unless the user explicitly asks.

## Reading routes

Always read this file. Before loading other project documentation, classify the task by affected domain and load only the routed material. Re-route when scope expands. When ownership is unclear, a change crosses several stages or a review is architectural, read the relevant authority in full.

### Named sections

A route may name an exact Markdown heading path such as:

```text
Frontend stages > Stage 4: AST semantics > Templates and TIR
```

Read the selected heading through the next heading of the same or higher level. Include nested subsections unless the route narrows further. Use heading names rather than line numbers. Use a document's task-reading guide when present. If a heading is missing or no longer owns the task, report the drift and read the broader authority.

### Task routing

- **Code-bearing implementation or review:** read `docs/src/developer-docs/style-guide/style-guide.mtf` in full and ensure all touched code follows the rules in this document.
- **Compiler stages, semantic data or handoffs:** read the opening authority text and `Architectural invariants` in `docs/compiler-design-overview.md`, then its routed task sections and affected producer or consumer handoffs.
- **Build system and project orchestration:** read the opening authority text and `Architectural invariants` in both `docs/compiler-design-overview.md` and `docs/build-system-design.md`, then routed build sections and relevant compiler handoffs. This includes Stage 0, config, imports, modules, packages, builders, tooling, linking, backend project assembly, outputs, incremental builds and the dev server.
- **Memory and value flow:** read `docs/src/developer-docs/memory-management/overview.mtf`, use its task-reading guide and read the selected leaves. This includes access, copies, ownership, borrowing, lifetimes, last use, retained-edge liveness, cleanup frontiers, declared groups, allocation, memory-strategy selection, Retained Edge Counting, drops, reactivity retention, runtime handles and ABI work.
- **Boracle reference solver, normalized borrow problems or modular last-use work:** read `docs/src/developer-docs/memory-management/boracle/boracle-reference-solver.mtf`, `docs/src/developer-docs/memory-management/borrow-validation/borrow-validation.mtf` and the active Boracle implementation plan, together with the routed memory and compiler authorities.
- **General Moth language understanding or source authoring:** read `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` for a compact overview of the accepted end-state language surface. When source must work with the current Alpha compiler or current implementation or target support matters, also read `docs/src/docs/progress/@page.moth`. The cheatsheet is an orientation and source-writing reference, not the authority for exact semantic edge cases.
- **Exact language syntax or user-visible semantic work:** read `docs/src/developer-docs/language/overview.mtf` and every relevant canonical unsuffixed reference it selects. This route is required when a task changes or depends on the precise contract of a language feature. Also read routed memory material when behaviour touches access, copies, borrows, lifetimes, groups or ownership. Read paired `-basic.mtf` files and `@page.moth` only for teaching, presentation or site structure.
- **Tests:** read relevant sections of `docs/src/developer-docs/style-guide/testing.mtf` before choosing, adding, changing or reviewing coverage. Read it in full for test infrastructure, suite policy, broad fixture cleanup or audits.
- **Structured codebase audits and accepted audit fixes:** read `docs/roadmap/audit-guide.md`, the selected guide under `docs/roadmap/audit-kinds/`, `docs/roadmap/audit-log.md`, `docs/roadmap/open-audit-findings.md` and the owning report when one exists. This is the explicitly invoked audit framework, not the Slice review. Audit runs are read-only. Implement accepted findings in a separate task and preserve every invariant and change lane named by the report.
- **Final validation:** read `docs/src/developer-docs/style-guide/validation.mtf` before selecting, running or reporting a final gate. It need not remain loaded during implementation.
- **Architecture plans, cross-stage ownership changes, broad refactors and thorough reviews:** read every relevant authority in full, including adjacent handoff authorities, current status and active sequencing.

Before changing tokenization, parsing, type checking, language semantics, diagnostics, lowering, semantic tests or authoritative language documentation for a feature, read that feature's canonical unsuffixed reference. Also read the canonical reference when correctness depends on precise feature semantics or edge cases not fully specified by the cheatsheet. Do not infer the exact language contract from examples, tests, compiler behaviour, the cheatsheet or a Basic page.

Use:
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` for compact accepted end-state language orientation and ordinary Moth source authoring
- `docs/src/docs/progress/@page.moth` for current implementation status and coverage
- `docs/roadmap/roadmap.md` for sequencing, active plans, genuinely deferred design, and the rules for adding and maintaining plans
- `docs/roadmap/audit-log.md` for what has been audited, when, and what has never been audited
- `index.md` only as a file and module locator

The public unsuffixed files under `docs/src/docs/design-scope/` own accepted deferred implementation, open questions and excluded language boundaries. The progress matrix tracks implementation of accepted design only.

## Authority and architecture

Instruction priority:
1. The explicit user request
2. The most specific relevant design or standards document
3. This file
4. Existing implementation behaviour

`docs/compiler-design-overview.md` owns compiler semantics and stage contracts. `docs/build-system-design.md` owns project and build orchestration. Canonical unsuffixed language references own syntax and observable language semantics. The language cheatsheet is a compact orientation and source-writing reference derived from those contracts and does not override them. Roadmap plans cannot override these authorities. Code may lag accepted design, so report conflicts instead of treating implementation as authoritative. The progress matrix says what works today, not what the accepted design means.

Core contracts:
- Each semantic fact has one owner. Later stages consume owned facts rather than rescanning source, reparsing syntax or reconstructing meaning from an earlier IR.
- Tokenization and declaration-shell parsing happen once. Do not add lightweight scanners or parallel parsing paths for owned syntax.
- Keep build-system, frontend, AST, HIR, analysis, project-builder and backend responsibilities separate. Builders and backends consume explicit validated artefacts rather than rediscovering source structure or semantics.
- Donor-local `TypeId`, HIR, allocation-family, region, counter and other local indexes do not cross module boundaries. Use stable semantic identities and summaries.
- TIR is AST-local. HIR is the first backend-facing semantic IR. Borrow and lifetime analyses read validated HIR and write side tables without rewriting it.
- User-authored failures use structured `CompilerDiagnostic` values with useful source context. `CompilerError` is for internal invariants, filesystem, tooling and backend infrastructure failures. User input must not panic.
- Borrow validation and lifetime-topology validation are mandatory and backend-independent. GC may represent an already legal topology but cannot legalise invalid or unproven topology.
- Missing optional transfer proof falls back conservatively without rejecting legal source. Missing mandatory topology proof is a source diagnostic, not a GC fallback.
- A backend that advertises full memory control must lower release builds without a tracing or reachability collector. A missing physical strategy after successful topology validation is `CompilerError`. There is no source-visible or project-visible no-GC mode.
- Mandatory lifetime topology and backend-neutral memory requirements are target-independent. Build-owned target partition and target validation happen before target/profile-aware physical memory planning. The compiler-owned memory planner produces one `ValidatedMemoryPlan` per physical variant. Backend lowerers only realise that plan.
- Retained Edge Counting is a compiler-selected physical representation, never source semantics. Its canonical page is `docs/src/developer-docs/memory-management/retained-edge-counting/`.
- Backends do not reparse source, reconstruct imports, infer source semantics or reconsider borrow and lifetime legality.

## Rules before writing a patch

- Moth is pre-release. Do not preserve old APIs through compatibility wrappers, forwarding shims, parallel structs, legacy entry points or fallback paths.
- Deletion over addition. Boring over clever. Does the code you're about to write need to exist at all? If not, skip it. Be proactive about cleaning up and removing code that no longer needs to exist after you've made changes.
- Keep one current implementation path. Thread API changes through every owner and delete the old path.
- Fix root causes. Remove transitional duplication, stale helpers, dead code, obsolete comments, superseded fixtures and cleanup debt in the owning change.
- Prefer readable, modular, explicit code with descriptive names, narrow helpers, context structs and concise WHAT/WHY comments. 
- Prefer data-oriented design over object-oriented patterns.
- Before adding a helper, pass, type, registry, validator, module or test abstraction, search the current owner, adjacent stages, backend paths and tests. Share only identical behaviour with a clear owner. Reuse existing utilities. Look before you write; don't re-implement what's a few files over.
- Do not move shared logic into a broad utility module unless it is genuinely shared and ownership remains clear.
- Do not claim validation commands were run when they were not.
- Use `./tmp` for temporary snippets and artefacts that should be untracked by git.

Required workflow:
1. Route and read the required project material and canonical authorities for the task.
2. Inspect the implementation and identify its owner.
3. Search for overlapping, duplicated, legacy and test paths.
4. Decide whether to extend, consolidate, replace or remove the existing path.
5. Implement the smallest coherent slice without transitional duplication.
6. Add or update tests when behaviour or a real internal invariant changed.
7. Review progress, index and audit-log update rules, run the correct final gate and perform the Slice review.

For multi-phase work, re-check ownership, duplication, stale paths and test gaps after each phase. Every non-trivial implementation plan must end with the Slice review. If the user changes accepted behaviour, treat that request as authoritative for the task and call out implementation conflicts.

## Testing

Follow `docs/src/developer-docs/style-guide/testing.mtf`.

- User-visible language and project behaviour belongs under `tests/cases/`.
- Focused subsystem-local invariant tests belong under that module's test directory, not in production implementation files.
- End-to-end or multi-module Rust harness tests belong under `src/compiler_tests/`.
- Prefer one primary test owner per behaviour. Remove redundant, obsolete or implementation-shaped coverage.
- Use the narrowest runtime, artefact or contractual golden assertion that owns backend behaviour. Use one input with backend-specific expectations for parity.
- Do not use benchmark fixtures as correctness coverage.

## Documentation and status

Do not modify documentation unless the user explicitly requests it or approves identified changes.

Exceptions:
- Update the progress matrix when implementation status, rejection behaviour, backend coverage or test coverage changes. Do not edit it for a pure refactor or prose-only correction.
- Update `index.md` when modules, files or folders move, are renamed or fundamentally change behaviour.
- Delete a plan under `docs/roadmap/plans/` in the same commit that completes it, and remove its roadmap entry. Do not mark it complete and commit that state. See `Adding and maintaining plans` in `docs/roadmap/roadmap.md`.
- Structured audit tasks update their report, the open-findings index and the audit log as required by `docs/roadmap/audit-guide.md`.
- Implementation and verification tasks mark an audit-log row stale when they materially change what it records. Only an audit run may record new coverage.

Report documentation made inaccurate by implementation as a separate follow-up. Do not edit generated files under `docs/release/**` directly. Rebuild them through the compiler.

## Validation and benchmarking

Code-bearing work ends with `just validate`. Documentation-only work uses the documentation release-build gate. Read the validation guide before selecting or reporting the final gate. Prefer `--terse` for Moth `check` diagnostics.

- Use `just bench-check` for non-recording performance evidence.
- Use the `timers` or `detailed_timers` feature flags for quick rough stage timings.
- Keep raw profiling and benchmark data local.
- Treat profiling as attribution evidence, not proof of correctness or improvement.

## Compaction rules

After compaction, reread `AGENTS.md`, reclassify the active task and reload the routed material and required canonical sections. Do not continue from compressed recollection of project contracts.

## Slice review

Every non-trivial slice ends with this review. It is a self-review checklist, not a structured audit: it needs no registered scope, produces no report and never updates the audit log. The structured audit framework under `docs/roadmap/` is a separate, explicitly invoked activity - see `docs/roadmap/audit-guide.md`. Do not block a slice review on anything that framework requires.

Review in this order:

1. Re-check the relevant architecture, language, memory, style and build contracts.
2. Read each changed module from its entry point. Confirm one clear owner. File documentation should state ownership and important exclusions, and the main flow should read as named steps.
3. Search changed and adjacent paths again for duplicated, legacy or obsolete logic, including compatibility wrappers and fallback paths.
4. Review API and abstraction shape. Reject broad, premature or wrong-layer abstractions, noisy parameter lists, boolean-heavy state and clever control flow that slows review.
5. Review local readability. Keep imports readable, group matches by meaning, space unrelated blocks and give complex code concise non-local WHAT/WHY comments. Remove stale comments and justify every lint suppression.
6. Review diagnostics. Use the correct lane, preserve source context, avoid user-input panics and centralise repeated diagnostic construction.
7. Review tests. Protect observable behaviour or real invariants, keep each test under the correct owner and remove redundant or implementation-shaped coverage.
8. Review progress, index and documentation effects under their update rules. Mark an audit-log row stale if this slice materially changed an area it records.
9. Confirm the correct validation path ran and report exactly what was and was not validated.
