# Moth Compiler Source, Token and Diagnostic Data Layout Implementation Plan

> **Repository path:**
> `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`
>
> **Architecture authority:**
> `docs/compiler-data-layout-design.md`
>
> **Status:**
> Active. Phase 0 is complete: activated on branch `token-and-diagnostic-data-layout-changes` from
> `b6f81fe58`, with the Test Suite Hardening prerequisite delivered in `03168082d`. The activation
> baseline, migration inventory summary and layout/correctness evidence are recorded under
> `Data Layout Migration - Phase 0 Activation Baseline` in `benchmarks/frontend-optimization-results.md`.

## Purpose

Replace Moth's allocation-heavy source, path, token, diagnostic and failure representations with
one compact data-oriented architecture before broad diagnostic representation work resumes.

This is a compiler-wide representation change, not a narrow Clippy patch. It must remove the root
causes of `clippy::result_large_err`, remove existing boxed-diagnostic workarounds and leave one clear
extension model for future diagnostics and tooling.

## Temporary validation bridge

The current implementation retains a 192-byte `CompilerError` across many internal and
infrastructure `Result` boundaries. Until the layout migration replaces that mixed representation,
the library crate's `src/lib.rs` carries one documented crate-level `#[allow(clippy::result_large_err)]` for the
192-byte `CompilerError`. The benchmark execution module at `xtask/src/benchmark_execution.rs`
carries one documented module-level allowance for its 224-byte `BenchmarkCaseFailure`.
Together, these temporary allowances keep the native, Linux and Windows Clippy lanes runnable.
These are validation bridges only. They must not be narrowed into copied local allowances or
treated as the target error model.

The migration must remove these allowances after the `CompilerError` sites have been assigned to the
compact diagnostic, infrastructure-failure or compiler-bug lanes and the benchmark failure record
has received its own size fix rather than a lint exemption.
The later migration gate and final validation gate must both prove that the allowances and large-error condition are gone.

Test Suite Hardening landed in `03168082d`. Its remaining large-error variants are this plan's
owned follow-up, not a test-suite regression; do not add another local allowance to mask it.

The implementation must converge on:

- one deterministic build-lifetime source database
- one prepared-source result reused by Stage 0 reachability and module compilation
- exact 4-byte local spans and 8-byte global spans
- genuine 4-byte complete-path identities
- fixed 8-byte token shapes in source-owned token stores
- retained syntax expressed as token ranges rather than cloned token vectors
- one 32-byte durable diagnostic record with typed cold stores
- one declarative diagnostic schema authority
- minimal immutable type-display snapshots
- frozen shared render context rather than deep table clones
- separate user-diagnostic, operational-infrastructure and compiler-bug lanes
- isolated tooling workers that discard failed compiler state

The diagnostics-improvement plan remains paused until this plan is complete. It resumes at its recorded
Phase 4.1c slice after the layout migration, not during it.

---

## Active context capsule

Refresh this block after every accepted slice and immediately before context compaction. Do not resume
from a compressed summary alone.

ACTIVE_PLAN:
- `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`

CURRENT_SLICE:
- Phase: Phase 0 complete; Phase 1 (build-lifetime source identity and exact compact spans) is next
- Checklist item: Slice 1A — introduce `PathId(NonZeroU32)` and the dense parent/component path table
- Goal: land the path identity foundation that `SourceRecord` needs, without migrating the compiler's `InternedPath` owners yet
- Non-goals: no `InternedPath` migration (Phase 2), no token or diagnostic representation change, no filesystem-path semantics change

LAST_GOOD_COMMIT:
- Phase 0 activation baseline: `b6f81fe58` (pre-Phase-0 activation state); Phase 0 documentation/evidence commit recorded below it

CURRENT_WORKTREE_STATE:
- Clean at activation; Phase 0 changed only `AGENTS.md`, `docs/compiler-data-layout-design.md`, this plan and `benchmarks/frontend-optimization-results.md`
- Branch: `token-and-diagnostic-data-layout-changes`
- Dedicated worker worktrees: none; a single worktree at the repository root is the whole inventory

RELEVANT_DOCS_THIS_SLICE:
- `AGENTS.md`
- `docs/compiler-data-layout-design.md`
- `docs/compiler-design-overview.md`
- `docs/build-system-design.md`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`
- `docs/roadmap/roadmap.md`
- delivered compiler diagnostics improvements, and the frontend arena and semantic-invariant work
- `benchmarks/frontend-optimization-results.md`
- the relevant canonical unsuffixed language references selected by `docs/src/developer-docs/language/overview.mtf` only when source-visible diagnostic or span assertions change
- `docs/src/developer-docs/memory-management/overview.mtf` only when borrow facts or tooling worker lifetimes change

RELEVANT_CODE:
- `src/build_system/create_project_modules/source_tree_index.rs`: current single deterministic entry-root traversal to extend with source-registration candidates
- `src/build_system/create_project_modules/module_inventory.rs`: current module reachability scheduling and prepared-source inventory assembly
- `src/build_system/create_project_modules/source_discovery.rs`: current retained-fact traversal, provider resolution and source-closure discovery
- `src/build_system/create_project_modules/source_preparation.rs`: current single-pass source tokenization and retained dependency-clause preparation
- `src/build_system/create_project_modules/prepared_source.rs`: current state-safe source-kind handoff into module preparation
- `src/build_system/create_project_modules/source_loading.rs`: current source IO and byte-loading owner
- `src/build_system/create_project_modules/compilation.rs`: canonical module ordering, local identity forks and module-result merging
- `src/build_system/create_project_modules/module_preparation.rs`: current source attachment and deterministic file-preparation merge. Stage 0 stops at prepared syntax; it owns no frontend stage.
- `src/build_system/create_project_modules/prepared_source.rs::PreparedSourceInput`: the five current source-text/path carriers. There is no `InputFile` type; `src/build_system/build.rs` owns the mutable backend string-table handoff
- `src/compiler_frontend/module_compilation/service.rs::compile_module`: the one production owner of the local semantic sequence, and the consumer of whatever source identity representation this plan lands on
- `src/compiler_frontend/pipeline.rs::CompilerFrontend`: current mutable stage-facade state and per-module `SourceFileTable`
- `src/compiler_frontend/symbols/identity.rs`: current `FileId` and `SourceFileTable`
- `src/compiler_frontend/symbols/interned_path.rs`: current `Vec<StringId>` complete-path owner
- `src/compiler_frontend/symbols/string_interning.rs`: existing immutable-base fork and deterministic delta merge to reuse
- `src/compiler_frontend/tokenizer/tokens.rs`: current `Token`, wide 94-variant `TokenKind` and `FileTokens`
- `src/compiler_frontend/headers/types.rs::Header`: current owned `FileTokens` body and repeated source/path fields
- `src/compiler_frontend/headers/header_dispatch.rs::capture_function_body_tokens`: current token-cloning body capture
- `src/compiler_frontend/compiler_messages/`: current diagnostic kinds, descriptors, payloads, labels, bags, messages and renderers
- `src/compiler_frontend/compiler_messages/compiler_errors.rs`: current mixed error lane, table cloning and full type-context retention
- `src/lib.rs`: temporary crate-level `result_large_err` allowance used only to keep interim validation runnable
- `xtask/src/benchmark_execution.rs`: temporary module-level `result_large_err` allowance for the benchmark failure record
- `src/compiler_frontend/datatypes/display.rs`: current type renderer to preserve as one shared formatting owner
- `src/projects/html_project/html_project_builder.rs` and `BackendBuilder`: current target/project diagnostic production over a mutable `StringTable`
- `src/projects/dev_server/build_loop.rs`: current compiler-message boundary, reusable executor and poisoned-lock recovery
- `src/compiler_frontend/arena/`: existing token/header statistics and capacity-policy owner
- `src/compiler_frontend/instrumentation/`: existing benchmark counters to extend
- `xtask/`, `benchmarks/` and `justfile`: existing benchmark engine, cases, evidence and command owners

ACCEPTANCE_CRITERIA:
- the delivered Test Suite Hardening commit `03168082d` is recorded as the prerequisite
- every hard layout assertion in `docs/compiler-data-layout-design.md` passes
- the old source-location, path, token, payload, message and mixed-error models are deleted
- CI passes native/Linux/Windows Clippy on the repository's current stable toolchain without boxing or lint suppression
- stable diagnostic codes, source ranges, diagnostic order and emitted artifacts remain correct
- aggregate retained frontend memory improves on representative success and failure workloads
- no unaccepted median regression above 5% remains
- one current implementation owner remains for every migrated concern
- roadmap, authority, style, testing, validation and index documentation are current

DECISIONS_ALREADY_MADE:
- decision: the architecture in `docs/compiler-data-layout-design.md` is the physical-layout authority
  - reason: aggressive representation contracts must not drift during a long migration
  - source/user/date: user interview and confirmation, 2026-07-19
- decision: complete Test Suite Hardening before activating this plan
  - reason: the rewrite depends on explicit diagnostic and integration ownership
  - source/user/date: user interview, 2026-07-19
- decision: park the diagnostics-improvement plan behind this migration
  - reason: diagnostic constructors, storage, labels, rendering and type context are replaced here
  - source/user/date: user interview, 2026-07-19
- decision: evolve existing Stage 0, `CompilerFrontend`, string-delta, diagnostic-bag and renderer owners rather than adding parallel frameworks
  - reason: the current repo already has deterministic merge and stage ownership that should be reused
  - source/user/date: repository review, 2026-07-19
- decision: benchmark-selectable details may change only through the procedure in the architecture document
  - reason: optional packing is accepted only when aggregate evidence justifies its complexity
  - source/user/date: user interview, 2026-07-19

BLOCKERS / RISKS:
- earlier queued implementation work has landed; this plan is active
- source/span migration touches nearly every frontend stage
- current boxed diagnostic aliases and style-guide advice are already present and must be removed
- release/profiling currently use aborting panics, which conflicts with thread-isolated tooling recovery
- compact-ID merge order must remain deterministic across file and module parallelism

VALIDATION_STATE:
- last command: `just validate`, the three Clippy lanes, `just bench-frontend-check`, `just bench-check`
- result: fully green on `b6f81fe58`. 4947 + 817 + 17 unit tests, 1951/1951 integration cases, docs clean, 82/82 bench preflight, 40/40 bench-check cases
- known unrelated failures: none

DOCS_IMPACT:
- progress matrix needed: only when current diagnostic/failure/tooling behaviour changes; do not add an internal-refactor status row
- other docs stale: current authorities and style rules still describe `CompilerError`, path-backed locations and boxed large-error boundaries
- authorized docs updates: every authority, style, roadmap, plan, matrix and index edit named below

- next action: Phase 1, Slice 1A

---

## Authority and change control

Read in this order before every implementation slice:

1. `AGENTS.md`
2. this plan and its active context capsule
3. `docs/compiler-data-layout-design.md`
4. `docs/compiler-design-overview.md`
5. `docs/src/developer-docs/style-guide/style-guide.mtf`
6. `docs/src/developer-docs/style-guide/testing.mtf`
7. `docs/src/developer-docs/style-guide/validation.mtf`
8. current owner code and current diff

Also read `docs/build-system-design.md` for source registration, module scheduling, graph outcomes,
compiler/build result boundaries or dev-server changes.

The architecture document owns the end-state representation. This plan owns sequencing. The progress
matrix owns current support. A locked architecture decision may change only with explicit user
approval. A benchmark-selectable decision may change only with recorded evidence and an update to the
architecture document in the same accepted slice.

## Locked completion contract

| Contract | Required end state |
|---|---|
| Source identity | `SourceId` is a non-zero 4-byte build-lifetime ID; each final ID is assigned at a deterministic registration barrier before that source is tokenized |
| Local source range | `LocalSpan` and `Option<LocalSpan>` are exactly 4 bytes and encode exact half-open UTF-8 byte ranges |
| Global source range | `SourceSpan` and `Option<SourceSpan>` are exactly 8 bytes |
| Path identity | `PathId` and `Option<PathId>` are exactly 4 bytes and identify a complete interned path |
| Token shape | `TokenShape` is exactly 8 bytes; `LocalSpan` remains separate |
| Diagnostic token | `DiagnosticToken` is exactly 8 bytes and does not retain source-token cold stores |
| Secondary label | `SecondaryDiagnosticLabel` is exactly 12 bytes |
| Durable diagnostic | `DiagnosticRecord` is exactly 32 bytes |
| Local diagnostic | `DiagnosticDraft` is move-only and at most 48 bytes |
| Diagnostic extension | new diagnostic families never widen the common record; unusual data uses typed cold stores |
| Type rendering | durable reports retain a minimal `DiagnosticTypeStore`, not a complete `TypeEnvironment` |
| Frozen identity | one lookup-only frozen context owns source, string and path tables; one `DiagnosticReport` owns report-local stores for one identity domain, while `DiagnosticReportSet` is the command-level one-or-many container |
| Failure lanes | user mistakes diagnose, expected operations return `InfrastructureFailure`, proven invariants panic through `compiler_bug!` |
| Tooling recovery | long-lived hosts recover only by discarding an isolated worker's complete mutable state |

Additional hard rules:

- no common durable source, token or diagnostic record contains `usize`, `Vec`, `String`, `PathBuf`,
  `HashMap` or a wide payload enum
- no missing identity is represented by a magic valid ID
- no source span asks a later stage to rescan, reparse or infer its end
- no `#[allow(clippy::result_large_err)]`
- no common `Box<CompilerDiagnostic>` or boxed diagnostic result alias
- no stable external diagnostic code is renumbered or repurposed during storage migration
- no raw `SourceId`, `PathId`, `StringId`, token-store ID or diagnostic-store ID crosses a project/package identity boundary without its owning frozen context or an explicit canonical remap; public semantic origin IDs remain the cross-boundary authority
- raw bit masks and shifts remain private to their codec modules
- every narrowing conversion is checked
- exact sizes and alignments use dependency-free const assertions compiled by native and cross-target CI, plus focused semantic/layout tests
- temporary migration adapters are private, named as migration code and deleted at their recorded boundary

## Repository shape to establish at activation

Phase 0 records the current shape of the source, token and diagnostic owners listed below. Do not
carry a shape recorded before activation - it will be stale, and the owners move under ordinary
work. Confirm at that point whether diagnostics implementation is parked or active, and make the
answer explicit rather than inheriting it.

Current pressure points:

- Stage 0 synthetic discovery retains complete prepared-file output, while directory inputs retain
  their single lexical token stream for later header preparation; these two state-safe variants
  still need to converge on the planned build-lifetime `PreparedSource` store
- temporary discovery identities are rebound into the retained module source table during final
  aggregation
- the `PreparedSourceInput` variants own `String`, `PathBuf` and source kind, while `SourceFileTable` is rebuilt per module
- `SourceLocation` owns `InternedPath` plus line/column start and end pairs
- `InternedPath` owns `Vec<StringId>` and allocates on parent/append/join operations
- `FileTokens` mixes immutable storage, source identity, filesystem identity and mutable cursor state
- `TokenKind` is widened to 24 bytes by `NumericLiteralToken`; its path variant already carries a dense `PathSyntaxId`
- header parsing clones tokens into declaration bodies, then each `Header` owns another `FileTokens`
- `CompilerDiagnostic` duplicates its primary location into an allocated primary label
- `DiagnosticPayload` and nested reason enums contain paths, tokens, spans and variable lists
- `DiagnosticLabelMessage` is widened by a substitution vector
- `DiagnosticBag` and `CompilerMessages` are broadly cloneable
- `CompilerMessages` deep-clones `StringTable` and retains complete `TypeEnvironment` ranges
- `CompilerError` combines user-adjacent operational failures with compiler invariants
- current style guidance explicitly recommends boxed local diagnostic results for `result_large_err`
- boxed diagnostic boundaries already exist in `pipeline.rs`, `headers/file_parser.rs`,
  `headers/header_dispatch.rs` and adjacent parser helpers
- the dev server recovers poisoned shared state after a panic instead of isolating compiler state

## Reuse, consolidation and deletion map

This table is normative. Do not create a new subsystem when the listed current owner can be evolved.

| Current owner/system | Required reuse or evolution | Delete or avoid |
|---|---|---|
| `compile_directory_frontend` result sorting | keep canonical module ordering as the module-delta and diagnostic aggregation order | no second graph scheduler or diagnostic-ordering pass |
| `merge_file_preparation_chunks` | keep one canonical file/chunk merge point for string/path deltas, prepared sources and diagnostics | no completion-order merge and no per-subsystem file scheduler |
| `StringTableForkSource` and `merge_delta_from` | preserve immutable-base forks and deterministic file/module merges; add a consuming frozen lookup form | no globally locked interner and no generic identity framework that obscures ownership |
| `SourceTreeIndex`, source-package inventories and path resolution | remain the only filesystem discovery owners; extend their existing deterministic inventories to pre-register source identity without eagerly loading every file | a second filesystem traversal or source-discovery policy inside `SourceDatabase` |
| `SourceFileTable` | absorb into the build-lifetime `SourceDatabase`; assign IDs once in Stage 0 | per-module file tables, fallback path reconstruction and `attach_source_files` |
| `PreparedSourceInput` and source-loading slots | replace with pre-registered source slots whose text allocation is populated once and then addressed by `SourceId`; module inputs become ordered `SourceId` sets | duplicate source strings, per-module input copies and another source cache |
| `CompilerFrontend` | remain the explicit mutable module compiler owner; borrow build-owned source registration, style directives, path resolver and external registries while owning only module-local mutable string/path/diagnostic state | a parallel all-purpose `CompilationContextBuilder` and per-module clones of immutable build services |
| `ModuleCompilationContext` | continue to own one module compilation's immutable inputs and consume already-prepared source IDs | a generic worker/task framework |
| `TokenStats`, `HeaderStats`, `FrontendArenaCapacityEstimate` | extend only with the layout metrics explicitly required by Phases 0, 3 and 7 | another token-statistics or capacity subsystem |
| `FrontendCounter` and `xtask` benchmarks | add layout counters and reports here | ad hoc timers, counters or standalone benchmark runners in data modules |
| `FileFrontendPrepareOutput` | evolve into one cached `PreparedSource` result per `SourceId`, consumed first by structural reachability and later by module aggregation | retokenization for dependency scanning and token stores copied into every header or declaration |
| `Header` | keep semantic shell facts but replace `tokens: FileTokens` with `TokenRange`; derive source from IDs/spans | repeated `source_file`, cursor state and token-vector ownership |
| token classification matches | replace with one `TokenDescriptor` table generated/validated from `TokenTag` | separate spelling, precedence, continuation and diagnostic-name matches |
| `numeric_text` | retain lexical/semantic numeric parsing; only move retained token data into a typed store | a second numeric parser in token storage |
| `DiagnosticKind` plus descriptor table | transform into the single declarative diagnostic schema with explicit numeric codes | a parallel code registry or duplicated descriptor match |
| `DiagnosticBag` | become the move-only draft accumulator; capture a small module-local type-display draft before the live environment is released, then own the one final `freeze` transition after canonical remap | a separate general-purpose `DiagnosticStoreBuilder` layer |
| `CompilerMessages` | replace with one-domain immutable `DiagnosticReport` plus a thin command-level `DiagnosticReportSet` only when independent project/package contexts coexist | a report wrapper around another public message wrapper or raw-ID concatenation across contexts |
| `display_type` | extract one shared type-display view used by both `TypeEnvironment` and `DiagnosticTypeStore` | duplicate old/new type-formatting implementations |
| current render modules | keep terminal, terse and dev-server presentation owners over new typed views | construction-time prose and renderer-specific diagnostic storage |
| `CompilerError` render helpers | retain only presentation helpers that accept the new typed failure/view contracts; delete helpers coupled to the old mixed payload | conversion of infrastructure failures into user diagnostics |
| `Module::warnings` and backend `Project::warnings` | move module warnings into the sibling prepared batch and backend/project warnings into the command-owned diagnostic bag so `CompilationOutcome::Success` is the sole warning owner | warning clones, warning-specific module remapping and warnings embedded in semantic/artifact payloads |
| `BackendBuilder::build_backend(..., &mut StringTable)` and project-builder diagnostics | audit every mutation, preserve lookup through a narrow build context and keep only diagnostic-owned identity extension mutable until the final command outcome | freezing identities before the last target/backend diagnostic producer or keeping an unrestricted mutable interner in lowerers |
| `DevBuildExecutor` / `ProjectBuildExecutor` | evolve the current dev-build seam into a factory/owned-worker boundary whose mutable worker is recreated after failure | a second generic task system or reuse of a panicked executor |

### Narrow context lifecycle

The final build/freeze lifecycle must be implemented without a broad parallel context framework:

```text
Stage 0's existing canonical source inventories pre-register final SourceId values before any source preparation
-> selected source text is loaded once into its preassigned slot
-> module work owns SourcePreparationDelta values plus local string/path deltas
-> each source's preparation result is cached once for Stage 0 structural reachability and later module compilation while its source-local extended-span builder stays owned until the last span-producing stage
-> DiagnosticBag captures only referenced type-display facts while the local TypeEnvironment is live
-> existing file/module merge boundaries merge strings, then paths, then remap prepared diagnostic batches into one command-owned draft stream
-> target/project/backend diagnostic producers receive narrow mutable diagnostic/identity borrows and never a second message system
-> at the last diagnostic-producing boundary for the actual outcome, DiagnosticBag::freeze performs the one draft-to-dense-store transition
-> build ownership collects finalized source records and freezes source/string/path lookup tables once into FrozenIdentityContext
-> each DiagnosticReport owns its DiagnosticStore and DiagnosticTypeStore plus Arc<FrozenIdentityContext>
-> DiagnosticReportSet preserves one or more independent project/package reports in canonical order
-> successful artifacts that retain compact IDs share the same Arc<FrozenIdentityContext>
```

`FrozenIdentityContext` is a lookup-only immutable bundle. It does not own diagnostics, report-local
type-display data, a compiler driver, a scheduler or dependency-injection services. A result retains
it only while that result still contains compact IDs that require it. A completed non-tooling success
with no retained report drops source/token/identity data at its last diagnostic-capable boundary.
Update the companion architecture document's conceptual context example to this exact ownership split
before the first implementation phase that freezes reports.

---

## Roadmap, matrix and overlapping-plan edits

### Current roadmap state

Test Suite Hardening was delivered in `03168082d` and is no longer an active plan. This plan
remains queued after the earlier implementation chain. The diagnostics plan is paused until this
plan completes; this plan owns the source/token/diagnostic representation migration and must not
be activated alongside another migration of the same representation.

Adapt wording to the current roadmap conventions, but preserve the queued order and one owner for
the representation migration.

### At activation

- [ ] make this the sole active source/token/diagnostic representation migration
- [ ] record the delivered hardening commit `03168082d` in this plan and the benchmark report
- [ ] confirm the diagnostics plan remains paused until this representation migration completes
- [ ] prohibit old-model diagnostic payload work while this plan is active
- [ ] keep unrelated scope-frame, arena and semantic-invariant work deferred in the roadmap

### Progress matrix

Do **not** add a phase-by-phase internal data-layout row. The matrix owns current user-visible support,
not implementation project tracking.

- [ ] review the matrix after every phase
- [ ] do not edit it for a semantics-neutral storage refactor
- [ ] update the existing **Structured diagnostics** row when the implemented failure lanes or tooling behaviour actually change
- [ ] at final completion, replace references to `CompilerDiagnostic`/`CompilerError` with the current `DiagnosticReport`, `InfrastructureFailure` and compiler-bug contract
- [ ] place rejected or postponed implementation optimisations in the roadmap, not the matrix, unless they affect current tooling support

### Required documentation changes

The following edits are authorized in their owning phases:

- `AGENTS.md`: add the data-layout authority to task-specific reading and update panic/result guidance
- `docs/compiler-data-layout-design.md`: replace its broad conceptual context example with the lookup-only frozen identity/report split, then record every benchmark-selected constant
- `docs/compiler-design-overview.md`: source identity, prepared syntax, diagnostic reports, type-display snapshots and failure lanes
- `docs/build-system-design.md`: source registration, deterministic merges, graph outcomes and tooling worker ownership
- `docs/src/developer-docs/style-guide/style-guide.mtf`: compact-record rules and removal of boxed large-error advice
- `docs/src/developer-docs/style-guide/testing.mtf`: layout/property/schema/render-equivalence/failure-worker test ownership
- `docs/src/developer-docs/style-guide/validation.mtf`: new manual architecture and failure-lane audit wording
- the paused user-facing diagnostics improvement work: keep it parked, then refresh it against the final schema APIs when this migration completes
- `docs/src/docs/progress/@page.moth`: only when current support wording changes
- `benchmarks/README.md` and `CONTRIBUTING.md`: document the alternate data-layout benchmark case list/command if that surface is added
- `index.md`: final source, token, path and diagnostic module map
- generated `docs/release/**`: rebuild only; never edit directly

---

## Evidence, slicing and validation protocol

### Evidence owner

Continue the existing evidence authority:

```text
benchmarks/frontend-optimization-results.md
```

Add one indexed top-level section for this plan and keep its phase evidence there. Do not create a
second optimisation report or benchmark runner. Use existing `FrontendCounter`, benchmark runner,
fixture groups and profiling commands. Raw inventories, allocator logs, per-run history and profiler
captures remain uncommitted under `target/` or `benchmarks/local-data/`.

Record for every material phase:

- date, accepted commit, machine/OS/CPU and Rust toolchain
- exact commands
- relevant common-record sizes and alignments
- common-array and cold-store bytes separately
- retained source snapshot bytes separately
- success, warning-heavy and diagnostic-heavy memory separately
- string/path remap and full-context clone counts
- five-run medians where timing is material
- accepted and rejected optional experiments
- known noise and unrelated failures

A benchmark-only counting allocator may be added only if current counters cannot provide a repeatable
peak-allocation proxy. It must have zero normal-build cost.

Keep the existing default `core` and scaling case groups in `benchmarks/manifest.toml` success-only.
`benchmarks/manifest.toml` is the corpus authority; `xtask/src/benchmark_manifest.rs` is only its
parser. Extend the existing in-process frontend benchmark engine, manifest parser and report types so
a `data_layout` case group can declare success or an **expected diagnosed outcome**. Give these
runs their own `BenchmarkSuiteKind::DataLayout` history/summary identity and add thin
`just bench-data-layout-check` / `just bench-data-layout` recipes that delegate to the same engine.
Reuse the existing warmup, measurement, observation and history machinery rather than copying
`frontend_bench`. A diagnosed case returns timing/counter evidence instead of becoming a runner
failure. An infrastructure failure or compiler bug still aborts the run. Reuse hardened canonical case
inputs where they provide a stable workload. Do not treat benchmark cases as correctness coverage.

### Performance policy

- a median regression above 5% in docs, focused frontend or a representative targeted workload blocks the phase unless explicitly accepted by the user
- the terminator-span experiment uses the stricter thresholds in the architecture document
- common-record shrinkage without aggregate retained-memory improvement is not enough
- an optional optimisation that adds substantial complexity without a repeatable memory or throughput gain is reverted
- benchmark fixtures are evidence, never correctness owners

### Slice protocol

Each `Slice` heading and each bold numbered batch inside a slice group is intended for one
coding-agent context. Split a batch before coding when the refreshed repository shows it cannot reach
a coherent focused-green checkpoint within one context.

Before coding:

- [ ] refresh the active context capsule, branch, HEAD, worktrees and diff
- [ ] read authorities and current owner code
- [ ] identify duplicate helpers, adapters, tests and callers
- [ ] state exact goal and non-goals in the capsule

Before accepting the slice:

- [ ] remove every superseded owner named by the slice; a surviving bridge must be one of the explicitly allowed cross-phase bridges below
- [ ] run focused tests and relevant benchmark checks
- [ ] inspect ownership, duplication, raw-bit boundaries and compatibility shims
- [ ] record temporary migration state explicitly
- [ ] record the accepted commit and refresh the capsule/report before compaction

### Common phase close

Every phase ends with an explicit **Audit / style-guide review / validation** subsection containing:

- [ ] one-owner and no-duplicate-path audit
- [ ] obsolete adapter/helper/comment/test sweep
- [ ] architecture and stage-boundary audit
- [ ] style-guide and module-organization review
- [ ] focused invariant and integration validation
- [ ] `cargo fmt` when Rust changed
- [ ] required documentation build/check
- [ ] `just validate`; when the refreshed Phase 0 baseline fails, record the exact failure and run/report every independently runnable component without claiming the full gate
- [ ] benchmark evidence required by that phase
- [ ] roadmap/matrix/docs impact review
- [ ] accepted commit and active-context refresh

A temporary adapter normally dies inside its owning phase. A bridge may cross a phase boundary only
when this plan names its exact deletion slice, the active capsule lists every caller and new callers are
forbidden. Such a bridge is migration-only, private and never a compatibility API. The source
`PathId`/legacy-path boundary ending in Slice 2D, the transitional `CompilerMessages` identity-context
bridge ending in Slice 4I and any measured early token projection ending in Slice 3H are the only
anticipated cross-phase cases.

---

## Phase 0 — Activation, current-state audit and evidence baseline

### Summary, reasoning and context

Activate when this plan reaches the queued implementation order after the delivered hardening prerequisite and earlier queued work. This phase replaces stale repository facts, parks overlapping work and
records the exact baseline before representation changes make comparison impossible. It changes no compiler semantics.

### Slice 0A — Activate from the final hardening commit

- [x] confirm the delivered hardening prerequisite at `03168082d`
- [x] confirm the parent worktree is clean and inventory all worker worktrees
- [x] create or reuse one dedicated implementation worktree according to current repository policy —
  the repository has exactly one worktree and no worker-worktree policy; work proceeds on branch
  `token-and-diagnostic-data-layout-changes` in the root worktree
- [x] record the activation baseline and inventory what changed under the owners below
- [x] refresh every path and symbol in the active context capsule
- [x] re-read the progress matrix and all authority documents

### Slice 0B — Serialize roadmap ownership

- [x] apply the activation roadmap order defined above
- [x] confirm the diagnostics plan remains paused until this representation migration completes
- [x] add `docs/compiler-data-layout-design.md` to task-specific `AGENTS.md` reading
- [x] update the design document's audit anchor, implementation map, deterministic source-registration barriers and lookup-only frozen-context example to the refreshed repo
- [x] do not edit the progress matrix unless current support changes during activation or migration —
  no current support changed, so the matrix was not edited
- [x] build documentation and inspect all plan/authority links

### Slice 0C — Produce the migration inventory

Generate the complete searchable inventories under `target/data-layout-audit/`. Commit only a
concise count/high-risk-owner/owning-phase summary in `benchmarks/frontend-optimization-results.md` and
keep the active slice's exact affected symbols in the context capsule.

Inventory:

- [x] every `SourceLocation`, `CharPosition`, `FileId`, `SourceFileTable` and durable location field
- [x] every source-text/path ownership, source cache, source reread and dependency-scan
  retokenization — the carriers are `PreparedSourceInput` variants, not an `InputFile` type
- [x] every `InternedPath` field, clone, append, parent, join and remap path
- [x] every `Token`, `TokenKind`, path row, `FileTokens`, token clone and retained token vector
- [x] every diagnostic kind, payload/reason/label-message variant, renderer, stable external code and `StringId`/string field containing compiler-generated prose rather than authored facts
- [x] every `Box<CompilerDiagnostic>` alias or conversion
- [x] every complete `StringTable` clone, diagnostic-only `TypeEnvironment` retention, backend/project-builder `StringTable` mutation and renderer query that depends on more than type spelling
- [x] every `CompilerError` producer, macro, conversion, consumer and immediate-print path
- [x] every panic catch, poisoned-lock recovery and panic-profile setting
- [x] planned owning phase for each item

### Slice 0D — Record layout, distribution and memory baseline

- [x] record size/alignment of current location, path, path row, token, diagnostic, largest reason
  types, labels, messages, render contexts and the retained `TypeEnvironment`, through a throwaway
  probe deleted after recording. Durable layout assertions land with the replacement types in
  Phase 1, so no predecessor-only layout-report module was added.
- [x] record the corpus source-size distribution, which decides the `LocalSpan` start-bit gates
- [ ] **owned by Slice 1C2:** exact span start/length histograms with boundary buckets for every
  candidate split. The current model stores line/column, not byte offsets, so the census cannot run
  against today's spans. Slice group 1C was reordered at activation so the byte cursor and line
  index land first and the census runs against real offsets where the selection decision lives.
- [x] add the `data_layout` case group to `benchmarks/manifest.toml`, extend `BenchmarkExpectation`
  with expected warning and diagnosed outcomes, add `BenchmarkSuiteKind::DataLayout` and add the
  `just bench-data-layout-check` / `just bench-data-layout` recipes. Built in Phase 0 rather than
  deferred: the predecessor diagnostic and warning memory baseline is unrecoverable once Phases 3
  and 4 delete the token and diagnostic models it measures.
- [x] reuse existing benchmark inputs where they exercise the required workload; new diagnosed and
  warning-heavy inputs live under `benchmarks/`, with correctness assertions left in `tests/cases/`
- [x] record source, path/dependency, token, template, type/generic, warning-heavy and
  malformed-source workloads
- [x] ensure instrumentation is feature-gated or otherwise zero-cost in normal builds

### Slice 0E — Establish correctness and performance baseline

- [x] run the native/Linux/Windows Clippy commands and record all failures — all three lanes pass
  under Rust 1.97.1, the repository's actual toolchain
- [x] identify every existing boxed boundary and whether unboxed failures remain
- [x] run full `just validate` when the baseline is green — green
- [x] run `just bench-frontend-check` and `just bench-check`
- [x] run five recorded frontend and end-to-end benchmark invocations — three recorded suites
  (frontend phases, end-to-end CLI, diagnosed data layout), each ten iterations per case, plus five
  independent repeatability invocations per suite; the noise floor is +/-1ms on the suite average
- [x] record retained source bytes, common data, cold data and clone/remap pressure separately
- [x] capture focused profiles only where attribution is unclear — attribution was clear; none captured

### Phase 0 — Audit / style-guide review / validation

- [x] confirm no compiler or language semantics changed
- [x] confirm every current owner appears in the migration ledger
- [x] confirm locked design decisions still match the refreshed repo
- [x] confirm no unrecorded lint allowance, boxing workaround or new compatibility path was added; the temporary pre-activation `result_large_err` bridge is recorded with its removal owner
- [x] confirm instrumentation reuses existing owners and has no normal-build cost
- [x] run the documentation-only gate for documentation-only commits
- [x] record the exact green baseline
- [x] record the Phase 0 commit and refresh the capsule/report

### Phase 0 exit criteria

- [x] this plan is the sole active owner of the source/token/diagnostic representation migration
- [x] diagnostics work remains paused cleanly behind this migration
- [x] every stale snapshot fact is refreshed
- [x] migration and failure-site inventories are complete
- [x] baseline correctness, layout, memory, timing and CI evidence is recorded

---

## Phase 1 — Build-lifetime source identity and exact compact spans

### Summary, reasoning and context

Source locations are the most pervasive representation problem. This phase moves source ownership to
Stage 0, introduces exact packed byte spans and migrates every compiler stage and renderer. It also
removes duplicated location data and existing boxed-diagnostic workarounds so the full CI gate becomes
green before later layout work proceeds.

### Slice 1A — Add the final path foundation required by source records

- [ ] introduce `PathId(NonZeroU32)` and a dense parent/component path table
- [ ] intern source logical paths into the build base before string/path forks are created
- [ ] use `PathId` in `SourceRecord` immediately; do not store an interim `InternedPath`
- [ ] keep filesystem `PathBuf`/`Box<Path>` separate from compiler logical identity
- [ ] add layout, root, parent, append, equality and rendering tests
- [ ] defer the full compiler `InternedPath` migration to Phase 2


### Slice group 1B — Replace per-module source tables with build-lifetime registration

- [ ] **1B1 — registration index and ID domain:** add the `compiler_frontend/source/` owner; keep `SourceTreeIndex`, source-package inventories and path resolution as the only filesystem discovery path; have those owners produce/move compact candidate rows into one sorted compiler-facing `SourceRegistrationIndex` rather than duplicating their tree/root metadata; implement `SourceId(NonZeroU32)` and the deterministic `CompilationRoot` record at ID 1
- [ ] **1B2 — registration barriers:** register config/bootstrap sources before config tokenization, then each project/package registration index before structural preparation; keep config and `ProjectGlobalsInterface` in the same project identity context; give separately compiled packages their own context; sort by canonical logical identity rather than reachability or completion order
- [ ] **1B3 — single-file, directory and synthetic sources:** build a bounded candidate inventory before the single-file entry scan; pre-register directory/source-package candidates before parallel work; reuse authored `SourceId`s for header/adaptor provenance; permit genuinely late synthetic sources only through deterministic deltas merged before an ID escapes
- [ ] **1B4 — source slots and loading:** move each loaded text allocation into its preassigned slot with no second full copy; enforce the monotonic registered → loaded → finalized lifecycle; represent registered-but-unloaded candidates with a compact slot/index rather than allocating empty full records; keep loaded records dense behind a `SourceId` slot map; deduplicate canonical physical sources and reject conflicting logical identity, kind or a second different snapshot
- [ ] **1B5 — module inputs and worker ownership:** replace `PreparedSourceInput` payloads with ordered `SourceId` sets; make structural preparation/module work borrow registered identity/text and own per-source `SourcePreparationDelta`; place finalized records into preassigned slots at the existing canonical merge; validate every selected slot was loaded/prepared exactly once
- [ ] **1B6 — remove per-module service copies:** absorb `SourceFileTable`, `FileId`, `FrontendSourceFileIdentity` and `attach_source_files`; make `CompilerFrontend<'build>` and header-parse options borrow immutable source registration, style directives, path resolver and external registries; retain canonical OS paths only as cold source-record data
- [ ] **1B7 — failures and tests:** preserve typed source-size, UTF-8 path and source-registration failures in their correct lanes; add config-to-project, direct-service, serial/parallel ID, slot, deduplication and source-order determinism tests

### Slice group 1C — Implement `LocalSpan`, line indexes and exact resolution

Slice order note, recorded at activation: the encoding cannot be selected before byte offsets exist.
The original order put selection (`1C1`) before the byte cursor (`1C3`), which would have forced a
throwaway offset tracker duplicating the line-index builder. The byte cursor now comes first.

- [ ] **1C1 — byte cursor and line index:** thread one line-index builder and byte-offset cursor through each source kind's existing traversal; use byte-aware iteration such as `char_indices()`; do not add a second pre-scan unless a non-tokenized source kind has no existing traversal
- [ ] **1C2 — span census and encoding selection:** with real byte offsets available, record exact span start/length histograms with boundary buckets for the architecture document's 8–12 length-bit splits over the weighted corpus; implement benchmark-only candidate codecs, run the bounded terminator experiment once, select by the accepted gates and record/freeze the constants in the architecture document and evidence report. The Phase 0 source-size census already proved every candidate is start-overflow-free on the current corpus, so this census decides the split on length overflow alone.
- [ ] **1C3 — exact span codec:** implement the selected `LocalSpan(NonZeroU32)`, one append-only `ExtendedSpanBuilder` per source and one private source-local factory/codec for exact construction, join, insertion-point and resolution; expose the same read-only resolver over a live source builder and a frozen source record so consumers never freeze/copy just to inspect an existing span; reject cross-source joins and expose named source-order, overlap and containment operations
- [ ] **1C4 — conversion semantics:** define CRLF, empty-file, final-newline, long-line and zero-width EOF behaviour; implement lazy line, Unicode-scalar-column and UTF-16-column conversion
- [ ] **1C5 — invariants:** add hard layout assertions plus exhaustive inline/extended boundary, malformed-capacity, join, ordering, Unicode and conversion property tests

### Slice 1D — Migrate tokenization and source preparation

- [ ] make tokenization emit `LocalSpan` and source-scoped diagnostics emit `SourceSpan`
- [ ] replace transitional `FileTokens::file_id` and every header/source identity field with final `SourceId`; any remaining path fields are display/migration data only and disappear in Phase 3
- [ ] finalize line starts and immutable token preparation at file-preparation completion, but keep the source-local extended-span builder mutable until the final span-producing stage
- [x] preserve the dependency-clause plan's deletion of the duplicate scanner: Stage 0 consumes
  retained prepared facts without rereading, cloning or owning a second source snapshot
- [ ] move the current `source_preparation.rs` and `PreparedSourceInput` handoff onto final
  `SourceId` records without adding another structural scan
- [ ] make file workers return `SourcePreparationDelta` values keyed by final `SourceId`; each delta owns its `DiagnosticBag`, token/header preparation and span builder, and moves into module/build ownership by existing chunk/file order without shared mutation or SourceId remapping
- [ ] make any diagnostic produced before that merge retain only exact final SourceId plus local span data owned by the same delta
- [ ] migrate path-item and alias locations to source-local spans
- [ ] migrate headers, dependency clauses, declaration shells, source contracts, fragments and source-kind adapters
- [ ] remove source-location string-ID remapping from file-preparation outputs
- [ ] preserve stable diagnostic codes, source ranges and ordering
- [ ] add one owning-module test-only `TestSourceContext` that creates a source record/span builder for focused Rust tests; migrate repeated ad hoc path/location constructors to it without exposing a production convenience API

### Slice group 1E — Migrate all downstream source spans

Each checked batch below is an independent accepted agent slice. Split a batch by its listed submodule
before coding when it cannot reach focused green validation in one context. Do not accept a commit with
a public boundary supporting both location models.

- [ ] **1E1 — headers and ordering:** header/dependency/declaration-shell records, module symbols, dependency edges and sorted headers
- [ ] **1E2 — core AST:** declarations, types, expressions, statements, calls, assignments, generic inference/evidence and generated-function requests
- [ ] **1E3 — templates:** template/TIR nodes, views, overlays, slots, control flow, formatting and runtime handoff metadata
- [ ] **1E4 — backend-facing frontend:** HIR nodes, locals, places, statements, terminators, validators, borrow facts and target-contract validation
- [ ] **1E5 — orchestration and support:** project config, Stage 0, build-system diagnostics, source adapters, compiler test helpers and direct location constructors
- [ ] in the owning batch, replace ambiguous location `PartialOrd` use with named source-order, overlap and containment operations

### Slice group 1F — Establish the frozen identity/render boundary

- [ ] **1F1 — frozen lookup foundation:** add consuming string/source/minimal-path freeze operations that move or share current allocations and create the final lookup-only `FrozenIdentityContext`
- [ ] **1F2 — pre-merge boundary cleanup:** make file stages return `SourcePreparationDelta` with a move-only diagnostic bag and make module stages return a move-only legacy diagnostic batch plus their local identity deltas; create a boundary message set only after the final canonical build/package merge instead of cloning `StringTable` through `from_*_ref` helpers
- [ ] **1F3 — transitional message ownership:** only at the final build/package render boundary, make current `CompilerMessages` temporarily own diagnostics, existing type context and `Arc<FrozenIdentityContext>` rather than a mutable/deep-cloned string table; make it move-only and use an outer `Arc` only where a host genuinely shares it; module outcomes must not freeze or clone a context before their deltas merge; name this bridge and delete it in Slice 4I
- [ ] **1F4 — renderer migration:** resolve paths, excerpts, line/column and UTF-16 positions through retained source snapshots and remove filesystem rereads used only for excerpts
- [ ] preserve terminal, terse and dev-server code/span identity
- [ ] define synthetic/compilation-root display and provenance explicitly
- [ ] keep non-UTF-8 filesystem display in infrastructure/path handling, not fabricated source paths
- [ ] add rendering tests for changed-on-disk files, Unicode, CRLF, long lines, config/bootstrap sources and synthetic sources

### Slice group 1G — Remove immediate diagnostic bloat and recover green CI

- [ ] **1G1 — remove duplicated source facts:** make the primary span canonical, keep only secondary labels in the transitional vector and remove payload/reason spans already represented by labels; cover generic inference, borrow conflicts, duplicate/shadow/dependency sites and assignment declarations first
- [ ] **1G2 — remove infrastructure widening:** stop converting `CompilerError` into `DiagnosticPayload::InfrastructureError`, carry the legacy outer failure separately until Phase 5 and delete the infrastructure payload plus its cloned `String`/`HashMap`
- [ ] **1G3 — enforce a non-throwaway size fix:** measure the transitional diagnostic; do not build a temporary old-payload cold-store system; when simplification is insufficient, pull forward only final schema/projection components retained by later phases
- [ ] **1G4 — token projection gate:** if the predecessor still exceeds 128 bytes, pull forward only final `TokenTag`, `TokenDescriptor` and 8-byte `DiagnosticToken` foundations from Phase 3; do not create another wide or temporary diagnostic-token enum
- [ ] **1G5 — remove workarounds:** require `size_of::<CompilerDiagnostic>() <= 128`, delete every `Box<CompilerDiagnostic>` alias/conversion and remove style-guide advice recommending local boxing; run native/Linux/Windows Clippy and confirm `result_large_err` is gone
- [ ] remove both temporary `result_large_err` allowances from `src/lib.rs` and `xtask/src/benchmark_execution.rs` rather than relocating either one

### Slice 1H — Delete the old location and source identity model

- [ ] delete `SourceLocation`, `CharPosition`, their constructors, path replacement and remap methods
- [ ] delete `SourceFileTable`, `FileId` and fallback path-based identity comparison
- [ ] delete line/column mutation in tokenization
- [ ] delete location filesystem fallback helpers and obsolete tests
- [ ] search the repository for old type names and construction patterns
- [ ] update compiler/build authorities and source-location style rules

### Phase 1 — Audit / style-guide review / validation

Complete the common phase close, plus:

- [ ] audit exactness: no consumer guesses or reconstructs a span end
- [ ] audit ownership: source text exists once, each extended-span builder has one owner and every builder freezes exactly once after its last producer
- [ ] audit determinism: serial/parallel SourceIds, spans, diagnostics and outputs match
- [ ] audit no raw source span crosses a project/package identity boundary without context ownership or canonical remap
- [ ] audit user-input limits diagnose rather than panic or truncate
- [ ] audit no path/line-column durable location remains
- [ ] audit related diagnostic locations were not lost while duplication was removed
- [ ] review codec isolation, module size, comments and stage ownership
- [ ] run source/span/tokenizer/header/renderer property tests and affected integration cases
- [ ] record source-snapshot, span-table and timing deltas

### Phase 1 exit criteria

- [ ] one build-lifetime source database is canonical and its mutable/frozen source lifecycle is explicit
- [ ] all compiler source positions are exact compact byte spans
- [ ] retained snapshots render diagnostics
- [ ] old location/file identity types are gone
- [ ] current boxed-diagnostic workarounds are gone
- [ ] full CI is green before Phase 2 begins

---

## Phase 2 — Genuine complete-path interning

### Summary, reasoning and context

The source database now uses `PathId`, but the rest of the compiler still owns vector-backed complete
paths. This phase replaces those paths without adding a contended global interner or another
scheduling system.

### Slice 2A — Extend the final path-table foundation beyond source registration

- [ ] keep the exact dense parent/component representation introduced in Phase 1; do not replace it with a second interner
- [ ] add module-local delta, remap and consuming frozen-table support needed by later compiler stages
- [ ] make parent/append/join identity operations allocation-free after interning
- [ ] add a consuming freeze operation that drops reverse lookup state when no further interning occurs
- [ ] define portable and native rendering through `PathTable` plus string lookup
- [ ] classify path domains before migration: compiler logical/semantic component paths may share the table, filesystem paths remain cold `Path` values and rendered free text is never interned as a path
- [ ] add layout, depth, prefix/suffix, equality, domain-wrapper and invalid-context tests

### Slice 2B — Reuse existing deterministic identity merges

- [ ] mirror the existing string-table immutable-base fork at module scope only where PathIds must exist during module compilation
- [ ] merge string IDs before path nodes because path records contain `StringId`
- [ ] merge path deltas in the same file/chunk and module order already used by frontend orchestration
- [ ] remap PathIds exactly once at each existing merge boundary
- [ ] keep source logical paths in the immutable build base so they never remap
- [ ] do not add a new scheduler, global lock, atomic ID allocator or generic interner framework
- [ ] extend existing counters for path nodes, unique complete paths, depth, merges and remaps

### Slice group 2C — Migrate compiler path owners

Each checked batch is an independent accepted slice. Delete old fields and conversion helpers in the
same batch.

- [ ] **2C1 — tokenizer and dependencies:** tokenized path rows, clause-owned dependency aliases,
  dependency-shell provider paths and path diagnostics
- [ ] **2C2 — headers and graph facts:** header identities, dependency collections, exports, module symbols, path resolution and source/package identities
- [ ] **2C3 — semantic types and interfaces:** parsed type paths, nominal/type lookup maps, traits, generic identities and public-surface facts; keep stable semantic origin IDs as the cross-package authority and remap or context-own every display `PathId` at interface binding
- [ ] **2C4 — AST/TIR/HIR metadata:** declarations, scopes, constants, template metadata, HIR/link facts and build metadata
- [ ] **2C5 — diagnostics and support:** diagnostic places/path facts, renderers, test support, snapshots and debug output
- [ ] replace retained per-header `HashSet<InternedPath>` dependency storage with deterministic sorted/deduplicated `PathId` slices or typed arena ranges; temporary sets may exist only while collecting

### Slice 2D — Delete vector-backed canonical paths

- [ ] delete `InternedPath`, its vector constructors and string-ID remap implementation
- [ ] delete repeated path clones and allocation-based parent/append/join helpers
- [ ] remove `HashMap<InternedPath, ...>`/`HashSet<InternedPath>` owners in favour of PathId keys
- [ ] keep `PathBuf` only at filesystem boundaries and source cold data
- [ ] update codebase index and module docs

### Phase 2 — Audit / style-guide review / validation

Complete the common phase close, plus:

- [ ] audit one path identity domain per compilation context and no detached raw path ID crosses a project/package boundary
- [ ] audit source paths stay stable while worker-created paths remap deterministically
- [ ] audit no path identity is reconstructed from rendered text
- [ ] audit no lock or `Arc` exists per path
- [ ] review path APIs for explicit context and no hidden allocation
- [ ] run path/dependency/module/type/diagnostic tests and serial/parallel determinism tests
- [ ] record path bytes, allocation/remap counts and timing

### Phase 2 exit criteria

- [ ] `PathId` is the only complete logical path identity
- [ ] `InternedPath` is deleted
- [ ] common compiler records carry IDs rather than owned component vectors
- [ ] path construction and merge order are deterministic

---

## Phase 3 — Fixed tokens and source-owned retained syntax

### Summary, reasoning and context

This phase replaces the wide token enum and the current cursor/storage mixture. It also removes token
cloning from declaration shells by giving each source one immutable token store and representing
retained syntax as ranges.

### Slice 3A — Select the token array layout before migration

Evaluate only these production candidates:

1. compact AoS `TokenRecord { shape: TokenShape, span: LocalSpan }` with a required 12-byte layout
2. SoA `Vec<TokenShape>` plus `Vec<LocalSpan>`

- [ ] prototype both behind benchmark-only code or short-lived branches
- [ ] measure parser iteration, cache behaviour, retained capacity and validation overhead
- [ ] choose AoS when results are materially tied because consumers normally need shape and span together
- [ ] choose SoA only for a repeatable material memory or throughput improvement
- [ ] record the decision in the architecture document and remove the rejected implementation

### Slice 3B — Introduce one token taxonomy and descriptor authority

- [ ] define or reuse the final explicit `TokenTag(u16)`, flags and `TokenShape { tag, flags, data }`; when Phase 1 pulled the tag foundation forward, extend it rather than declaring another taxonomy
- [ ] declare every tag once through a small internal `token_schema!`/const-table authority containing its explicit numeric tag and static descriptor facts; do not add a procedural macro or a second hand-maintained tag list
- [ ] generate or validate assignment, expression-continuation, operand, keyword, delimiter, literal, precedence and diagnostic-name APIs from that one authority
- [ ] keep dynamic token values in typed payload accessors rather than descriptor data
- [ ] preserve exact source spelling and current lexical semantics
- [ ] add all-tag coverage and reserved-bit/layout tests

### Slice 3C — Add typed source-local cold stores

- [ ] move numeric literal retained data into a numeric store while reusing `numeric_text` parsing
- [ ] move canonical path rows into typed path stores using `PathId` and `LocalSpan`; keep
  dependency aliases under their retained clause owner and migrate only their locations
- [ ] encode symbol, string, bool and char payloads directly when they fit the token word
- [ ] use checked `u32` indexes and typed capacity failures
- [ ] add a direct single-path fast form only if measured aggregate evidence justifies it
- [ ] add remap/freeze tests for every cold store

### Slice 3D — Separate immutable storage from parser cursor

- [ ] implement `SourceTokens` as the one immutable token owner for a source
- [ ] implement `TokenCursor` as short-lived index/range state over a borrowed store
- [ ] implement compact `TokenRef`/views without cloning cold payloads
- [ ] use `u32` token indexes and checked range construction
- [ ] remove source path, canonical OS path, `index` and `length` from token storage
- [ ] add cursor boundary, EOF, peek, nested-range and malformed-index tests

### Slice group 3E — Make prepared syntax source-owned

- [ ] **3E1 — prepared-source store:** evolve `FileFrontendPrepareOutput` into one move-owned `PreparedSource` slot per selected `SourceId`; it owns the source token store and syntax preparation exactly once; Stage 0 may borrow structural facts, then the unique owning module/direct service takes the record without per-source `Arc` or cloning
- [ ] **3E2 — structural reachability reuse:** make prepared dependency shells expose final local-source `SourceId` edges plus typed provider request records; Stage 0 traverses those facts without reading token stores or rendered path text; keep provider mutation/resolution on its serial owner after workers return structural references
- [ ] **3E3 — consolidate retained preparation:** preserve the current exactly-once lexical and
  preparation paths while moving both directory-token and synthetic-prepared variants into the
  same build-lifetime `PreparedSource` store; module aggregation must consume those records without
  adding another tokenizer, parser or cache
- [ ] **3E4 — contiguous retained syntax:** add half-open `TokenRange { source, start, end }`, replace contiguous `Header::tokens` bodies with ranges, remove repeated `Header::source_file` and change function/template body capture to record boundaries instead of cloning tokens
- [ ] **3E5 — segmented start-body syntax:** add `TokenSequenceId` into a source-local range-list store whose entries are 8-byte `{ start, end }` token-index pairs and whose owner stores `SourceId` once; expose one `TokenSequenceView` so contiguous and segmented bodies use the same `TokenCursor`; never add a copied start stream or parallel parser path
- [ ] **3E6 — source-kind adapters:** represent non-tokenized adapter payloads directly, preserve plain Markdown's no-token path and keep declaration-shell parsing single-owner through token/sequence views

### Slice group 3F — Migrate parser and semantic consumers

Each checked batch is independently accepted and must remove the old token API from its owner.

- [ ] **3F1 — Stage 0 and header cursor:** structural reachability, provider-reference collection,
  header splitting, dependency clauses, declaration dispatch, retained dependency-fact reads and
  source-kind preparation
- [ ] **3F2 — declaration syntax:** signatures, declarations, types, structs, choices, traits and generic parameter parsers
- [ ] **3F3 — AST core:** expression, statement, call, field, match, loop and assignment parsers
- [ ] **3F4 — template parser:** template heads, TIR emission, slots, control flow and formatter-facing token reads
- [ ] **3F5 — support surfaces:** token-based diagnostics, tests, debug/show-token output, `TokenStats` and benchmark classification; extend the owning test source helper with token-store construction rather than adding parser-specific fixture builders
- [ ] in every batch, use short-lived token views only; no durable Rust reference or self-referential structure may be introduced

### Slice 3G — Add the durable diagnostic token projection

- [ ] implement exact 8-byte `DiagnosticToken`
- [ ] reuse `TokenTag` and descriptor spelling
- [ ] preserve only the one ID/immediate needed for useful diagnostics
- [ ] collapse detailed path and numeric payloads when full retained data is unnecessary
- [ ] prove diagnostics render after source token cold stores are dropped

### Slice 3H — Delete the old token architecture

- [ ] delete `Token`, `TokenKind` and `FileTokens`
- [ ] delete clone-based `current_token()` and body-capture helpers
- [ ] delete duplicate token spelling/classification matches
- [ ] delete token string/path remapping that the new stores no longer require
- [x] delete the former tokenizing dependency-scanner path, provider-free classification/retry
  scans and `ReachableSourceInventory::source_cache`; the dependency-clause plan replaced them with
  `source_preparation.rs`, retained prepared facts and state-safe `PreparedSourceInput` variants
- [ ] replace the transitional `PreparedSourceInput` variants only when the build-lifetime
  `PreparedSource` store owns their data without reintroducing tokenization or preparation
- [ ] update module docs, codebase index and style rules

### Phase 3 — Audit / style-guide review / validation

Complete the common phase close, plus:

- [ ] audit one token store per tokenized source
- [ ] audit retained syntax contains ranges/IDs only
- [ ] audit descriptor data has one authority and no duplicate classification tables remain
- [ ] audit token/cold-store indexes are checked and deterministic
- [ ] audit source tokens drop after their final semantic/tooling owner and are never retained merely because a source snapshot remains renderable
- [ ] run tokenizer, header, parser, template, diagnostic-token and lifecycle tests
- [ ] run full integration output/diagnostic equivalence checks
- [ ] record common/cold token bytes, clones, capacity and timing

### Phase 3 exit criteria

- [ ] `TokenShape` and the selected source-token array layout are canonical
- [ ] complex token data lives only in typed source-local stores
- [ ] declaration shells use `TokenRange`
- [ ] no wide token enum or cloned retained token vector remains
- [ ] each tokenized source is prepared once and reused by reachability and module compilation

---

## Phase 4 — Compact diagnostics, type snapshots and frozen reports

### Summary, reasoning and context

This phase replaces the complete diagnostic model in one migration phase. Temporary conversion code
may exist only inside this phase. The exit commit must have one schema, one draft accumulator, one
32-byte durable record, one type-display renderer and one immutable report boundary.

### Slice 4A — Declare the complete diagnostic schema

- [ ] inventory every current stable diagnostic family after the parked diagnostics checkpoint
- [ ] assign explicit non-zero internal `DiagnosticCode` values; never derive them from enum order
- [ ] preserve each external `MOTH-*` code, category, title and default severity
- [ ] declare fact-word meaning, optional codecs, allowed extra kind, secondary-label roles, type-rewrite markers and renderer entry once per diagnostic
- [ ] replace compiler-generated prose stored in payload/label string fields with typed reason/message codes and facts; an authored string may remain a `StringId`, but `RenderedText` is not a generic escape hatch
- [ ] use one small `macro_rules!` vocabulary and const tables
- [ ] keep one registry module and split domain declaration files once the registry would exceed the repository's practical file-size guidance; each family still has exactly one declaration
- [ ] generate/validate descriptor lookup, all-code iteration, typed draft constructors, durable accessors and schema tests
- [ ] prohibit raw fact indexing outside diagnostic storage modules

### Slice 4B — Implement compact records and cold stores

- [ ] implement `DiagnosticRecord`, `DiagnosticDraft`, `DiagnosticId`, `DiagnosticExtraId` and hard layout assertions
- [ ] implement packed code/severity flags with reserved-bit validation
- [ ] implement fixed `DiagnosticExtraRecord`, typed list ranges and typed arenas
- [ ] implement 12-byte secondary labels; primary span exists only in the record and every compiler-owned label phrase uses a compact `LabelMessageCode` plus typed immediate/side data
- [ ] implement 4-byte `DiagnosticPlace` and checked optional/packed ID codecs
- [ ] implement deterministic capacity-exhaustion diagnostics without recursive extra allocation
- [ ] make common drafts allocate nothing and rare drafts allocate at most one root auxiliary object
- [ ] remove broad `Clone` from drafts, bags and owning stores

### Slice 4C — Define the one draft, preparation and compaction path

- [ ] make `DiagnosticBag` a move-only `Vec<DiagnosticDraft>` accumulator
- [ ] let each `SourcePreparationDelta` carry its local `DiagnosticBag`; at the existing file/chunk merge, merge strings then paths and schema-remap those drafts into the module identity domain
- [ ] define one internal `PreparedDiagnosticBatch` only for a module-local identity/type domain crossing the module/build merge boundary
- [ ] keep prepared batches as drafts plus compact type-display draft data; they are not another durable report/store API
- [ ] implement `DiagnosticBag::freeze`/equivalent as the one final draft-to-dense-store transition after canonical remapping and after the actual outcome's last diagnostic producer
- [ ] avoid a separate public `DiagnosticStoreBuilder` layer
- [ ] compact labels, lists, token projections and extras into dense arenas
- [ ] preserve deterministic production order
- [ ] expose typed immutable `DiagnosticView` and iterators; renderers never mutate records
- [ ] compute error/warning/note counts during freeze and expose allocation-free severity-bucket iteration without building a reordered index vector
- [ ] keep `DiagnosticReport` and `DiagnosticReportSet` move-only; a long-lived host wraps the whole set/report in `Arc` only when multiple host consumers genuinely share it, never per record or per side store

### Slice group 4D — Build the minimal diagnostic type-display store

- [ ] **4D1 — shared display/query view:** inventory every renderer query against `TypeEnvironment`, define the smallest frozen query surface beyond spelling and extract one read-only view from `datatypes/display.rs` so live and frozen types share one formatter
- [ ] **4D2 — compact store:** implement `TypeDisplayId`, fixed records and typed child ranges for every currently rendered builtin, nominal, choice, generic, function, option, collection, map, tuple/multi-value, fallible and external shape
- [ ] **4D3 — snapshot algorithm:** while the producing `TypeEnvironment` is live, collect schema-marked `TypeId`s, reserve before recursion, copy only transitive display/query facts and rewrite draft words to batch-local `TypeDisplayId`s before the environment can be released
- [ ] **4D4 — stage boundaries:** refactor diagnosed AST/HIR/target paths so no live environment is dropped before its prepared diagnostic batch is captured; successful semantic owners keep their environment only for real backend work
- [ ] **4D5 — equivalence:** add exhaustive live/snapshot formatting and query-equivalence tests; a valid frozen report never falls back to a raw internal ID

### Slice group 4E — Consolidate file, module and command diagnostic aggregation

- [ ] **4E1 — file/module domains:** let `SourcePreparationDelta` carry file-domain drafts; at file/chunk merge, merge strings then paths and schema-remap drafts into the module domain
- [ ] **4E2 — prepared module batch:** add `PreparedDiagnosticBatch` beside `CompiledModuleResult`, never inside `Module`; capture drafts plus the small module-local type-display store before a diagnosed module drops its environment
- [ ] **4E3 — canonical module merge:** use existing module result sorting as the only cross-module diagnostic order; merge module strings then paths, remap drafts/labels/type records, merge batch-local `TypeDisplayId`s and append into command-owned builders without freezing early
- [ ] **4E4 — warnings and success artifacts:** move successful warnings out of `Module` and backend `Project` payloads, remove warning-specific clones/remaps and preserve success-only linkable/artifact payloads; all warnings remain command-owned drafts and become only `CompilationOutcome::Success::warnings`
- [ ] **4E5 — independent contexts:** define thin move-only `DiagnosticReportSet` as the command-level canonical-order collection of one-domain reports; keep project/package identity domains separate rather than concatenating raw IDs, and preserve one canonical diagnostic set per shared module

### Slice 4F — Freeze lookup context without a parallel compiler framework

- [ ] audit every target/project/backend use of mutable `StringTable` or path construction; convert lookup-only consumers to immutable views, keep generated output names/text in backend-owned strings rather than compiler identity tables and route only legitimate diagnostic-owned additions through narrow command-owned builders
- [ ] reuse and complete the consuming string/source/path freeze foundation from Phase 1; move or share existing allocations and drop reverse lookup state without rebuilding every string
- [ ] finalize each source record when its owning module/direct-service outcome reaches its last span producer; at the final build/package boundary assemble those already-finalized records into their preassigned slots and freeze only the table/container indexes still mutable
- [ ] keep the narrow lookup-only `FrozenIdentityContext` containing source, string and path data; it owns no diagnostics or report-local type data
- [ ] let existing `CompilerFrontend` and Stage 0 owners supply mutable state; do not add a second compiler driver
- [ ] freeze a diagnosed frontend/graph outcome immediately after its canonical failure aggregation; on a successful frontend path, carry warning drafts through target/project/backend work and freeze only after the last producer
- [ ] let each `DiagnosticReport` own its dense `DiagnosticStore` and `DiagnosticTypeStore` plus `Arc<FrozenIdentityContext>`; `DiagnosticReportSet` owns only the ordered report collection and no duplicate context/store layer
- [ ] let successful artifacts/build results that retain `SourceSpan`, `StringId` or `PathId` share the same identity context rather than cloning tables
- [ ] do not attach the context to final outputs that contain no such IDs; define and test the earliest drop boundary for ordinary successful CLI builds, warning reports and long-lived tooling separately
- [ ] skip building a frozen report/context entirely for a clean success whose final outputs contain no compact compiler IDs; otherwise freeze once and share
- [ ] remove full table clones from normal diagnostic and warning boundaries
- [ ] ensure raw compact IDs never escape without their owning context
- [ ] update the companion architecture context example in this slice if Phase 0 did not already make this ownership correction

### Slice group 4G — Migrate all diagnostic families

Each batch is one independently accepted agent slice. A batch must migrate its constructors, schema,
renderers, unit tests and integration-facing stable-code behaviour together. Before using extra data,
simplify the semantic facts and place related source sites in secondary labels.

- [ ] **4G1 — token/lexical:** expected/unexpected tokens, delimiters, strings, numbers, characters, spacing and paths
- [ ] **4G2 — template syntax:** tokenizer, structure, directive, slot and template-control diagnostics
- [ ] **4G3 — dependencies/config/project:** dependencies, source kinds, packages, config, Stage 0 and project structure
- [ ] **4G4 — names/declarations:** names, declarations, assignment, shadowing, warnings and deferred features
- [ ] **4G5 — calls/data operations:** calls, returns, casts, fields, collections, maps and copy/access targets
- [ ] **4G6 — choices/control flow:** choices, matches, loops, statements, fallible handling and non-token template semantics
- [ ] **4G7 — generics/traits/API:** generic application/inference/instances/generated functions, traits, conformances, public API and visibility
- [ ] **4G8 — types/borrows/targets:** type mismatch/operator/type shape, borrow/move/alias and target/backend-feature diagnostics
- [ ] in every batch, use typed list/extra stores only where fixed facts and labels cannot preserve the diagnostic clearly

### Slice 4H — Replace boundary APIs and renderers

- [ ] replace `CompilerMessages` with one-domain `DiagnosticReport` and command-level `DiagnosticReportSet` for diagnosed/warning outcomes
- [ ] migrate terminal, terse, dev-server, test and tooling renderers to `DiagnosticView`
- [ ] use the schema's descriptor/renderer entry and the shared token/type display authorities
- [ ] preserve diagnostic display ordering with allocation-free severity-bucket passes over stored production order; delete the current `Vec<usize>` display-order construction
- [ ] keep integration assertions on stable external codes, source positions and contractual rendered fragments; use typed fact views only in focused Rust schema/renderer tests, never expose raw fact words to fixtures
- [ ] preserve existing rendered wording unless a fact simplification requires an explicitly accepted correction

### Slice 4I — Delete the old diagnostic architecture

- [ ] delete `CompilerDiagnostic`, `DiagnosticPayload` and nested storage-only reason variants
- [ ] delete `DiagnosticKind`, domain kind enums and the separate descriptor mapping
- [ ] delete old label style/message enums and primary-label construction
- [ ] delete `CompilerMessages`, `RenderTypeContext`, diagnostic-context range shifting/prepending and full diagnostic type-environment retention
- [ ] delete diagnostic string/path/type remap methods superseded by canonical freeze
- [ ] delete all boxed diagnostic results and old-model adapters
- [ ] search all code, tests and docs for obsolete names and assumptions
- [ ] update style/testing/validation rules and codebase index

### Phase 4 — Audit / style-guide review / validation

Complete the common phase close, plus:

- [ ] audit every external diagnostic code has exactly one schema entry
- [ ] audit every schema fits four fact words and one optional extra without widening the record
- [ ] audit common diagnostics allocate no cold data
- [ ] audit primary/secondary spans have one owner and no payload duplication
- [ ] audit one type formatter serves live and frozen contexts
- [ ] audit no full token store, path vector or type environment is retained by a report
- [ ] audit no full source/string/path context clone remains on normal boundaries
- [ ] audit deterministic file/module diagnostic ordering and remapping
- [ ] review schema readability, generated surface, codec isolation and renderer ownership
- [ ] run layout/schema/store/property/type-equivalence and all diagnostic integration tests
- [ ] record draft/store/cold/type/context memory and timing

### Phase 4 exit criteria

- [ ] every user diagnostic uses the schema and `DiagnosticDraft`
- [ ] every durable record is exactly 32 bytes
- [ ] prepared module batches contain no full type environment and no second durable diagnostic model
- [ ] `DiagnosticReport`/`DiagnosticReportSet` are the only diagnosed/warning render boundaries and are frozen only at the actual outcome's last diagnostic producer
- [ ] full type environments and mutable reverse-lookup tables are not retained solely for rendering
- [ ] all old diagnostic/message models are deleted

---

## Phase 5 — Three explicit failure lanes

### Summary, reasoning and context

Now that user diagnostics have their final model, audit every old `CompilerError` site and separate
recoverable operational failures from proven compiler bugs. The compiler must never use panic merely
to simplify a signature, and expected infrastructure failures must never widen user diagnostics.

### Slice 5A — Refresh and approve the failure classification ledger

For every remaining old error producer, record:

- [ ] path and symbol
- [ ] stage and current trigger
- [ ] whether adversarial user input can reach it
- [ ] chosen lane: user diagnostic, infrastructure or compiler bug
- [ ] exact earlier invariant that proves a proposed bug path impossible
- [ ] host/owner that renders or reacts to an infrastructure failure

Review the complete ledger before broad signature changes.

### Slice 5B — Move remaining user-caused failures to diagnostics

- [ ] convert a remaining source/config/project/type/rule/target failure to an existing schema family only when the semantic family and stable-code contract match; otherwise add one explicit new schema family
- [ ] add a stable code only for a genuinely new semantic family
- [ ] preserve exact spans and related labels
- [ ] add adversarial integration ownership for every old user-reachable `CompilerError` path
- [ ] default an uncertain site to recoverable failure until an invariant is proved

### Slice group 5C — Implement and migrate `InfrastructureFailure`

Define the final type first, then migrate these independent batches:

- [ ] **5C1 — data and rendering:** compact typed kind/stage, optional exact source or owned filesystem-path context, deterministic typed detail slice and terminal/terse/dev-server renderers
- [ ] **5C2 — filesystem/build:** source open/read, canonicalization, output writing, manifests, permissions and other expected operating-system failures
- [ ] **5C3 — providers/backends:** tool invocation, provider loading, backend emission/validation IO and other recoverable external-system failures
- [ ] **5C4 — tooling hosts:** watcher, socket/server, routing and dev-server operational failures
- [ ] replace string-keyed metadata `HashMap`s with typed details in the owning batch
- [ ] preserve useful underlying IO/tool information without pre-rendering all presentation text
- [ ] when a failure carries `SourceSpan`/`PathId`/`StringId`, make the host-facing failure share the matching frozen identity context; pre-source failures use owned filesystem context instead
- [ ] never let a compact ID escape detached from its context and never deep-clone that context for one failure
- [ ] never convert infrastructure data into a `DiagnosticRecord`

### Slice group 5D — Implement structured compiler bugs

- [ ] **5D1 — bug payload:** define `CompilerBugReport` with stage, Rust caller, version/commit, invariant message and optional self-contained resolved source facts consisting of logical display path, byte/line/column range and a bounded excerpt; it never retains a worker context or whole source; add `#[track_caller]` `compiler_bug!` that raises the typed payload through `panic_any` rather than a formatted string panic
- [ ] **5D2 — compact storage invariants:** source/path/token/diagnostic IDs, ranges, reserved bits and store bounds that are already proven by validated construction
- [ ] **5D3 — AST/TIR/HIR invariants:** impossible validated semantic/IR states and missing stage authority
- [ ] **5D4 — borrow/link/backend invariants:** impossible analysis, call-target, capability and backend metadata states
- [ ] route every intentional production compiler-invariant panic through `compiler_bug!`; debug assertions may supplement but never replace release validation
- [ ] resolve any optional bug source span into enough owned display/provenance/range data before panicking so the report remains useful after the complete worker state is discarded
- [ ] never rely on reading logically suspect worker tables after unwind merely to render the bug
- [ ] let unknown dependency/runtime panics become generic compiler-bug reports only at the host boundary
- [ ] ensure user-driven capacity/limit failures diagnose instead of panic

### Slice 5E — Make result boundaries explicit

Use one boundary shape rather than a family of wrapper enums:

```rust
pub type CompilationResult<T> = Result<CompilationOutcome<T>, InfrastructureFailure>;

pub enum CompilationOutcome<T> {
    Success { value: T, warnings: Option<DiagnosticReportSet> },
    Diagnosed(DiagnosticReportSet),
}
```

Exact names may follow existing module/graph outcome types, but the lane placement is fixed. This slice must adapt the graph/result owner present at activation; it must not implement the separately queued canonical-module graph architecture merely to introduce these lanes.

- [ ] keep stage-local short-circuit helpers free to return `Result<T, DiagnosticDraft>`; convert to `Diagnosed` only at the owning compilation boundary
- [ ] keep warnings only on `Success`; a diagnosed or infrastructure-failed outcome does not carry unrelated prior warnings, and any source context essential to an error belongs in that diagnostic's labels/facts; use a one-element report set for the common single-context case
- [ ] update current warning-prepend helpers and tests to this accepted boundary rather than carrying mixed-lane production-order streams forward
- [ ] keep graph `blocked` and independent-branch continuation inside the successful orchestration outcome, not as infrastructure errors
- [ ] remove internal-bug variants from `Result` signatures after they become invariant panics
- [ ] preserve operational failures explicitly as the outer `Err`
- [ ] remove immediate printing from compiler internals
- [ ] preserve independent graph continuation for diagnosed branches, not corrupted compiler state
- [ ] apply the same contract to generated-function and target-validation boundaries
- [ ] avoid aliases or wrappers that merely rename another identical result shape

### Slice 5F — Delete the mixed error model

- [ ] delete `CompilerError`, `ErrorType`, metadata keys/maps, the temporary outer legacy failure bridge from Phase 1 and compiler-error-to-diagnostic conversion
- [ ] delete `return_compiler_error!`, `return_hir_transformation_error!` and `return_file_error!`
- [ ] delete temporary internal lanes and infrastructure diagnostic remnants
- [ ] update compiler/build authorities, style, testing, validation and current matrix wording
- [ ] search code, tests and docs for old terms and implicit conversions

### Phase 5 — Audit / style-guide review / validation

Complete the common phase close, plus:

- [ ] audit every old site appears once in the approved classification ledger
- [ ] audit malformed source cannot reach infrastructure or panic lanes
- [ ] audit expected operational failures do not panic
- [ ] audit every compiler bug names a proven invariant and stage
- [ ] audit no infrastructure data enters the 32-byte diagnostic model
- [ ] review result signatures for clarity and no wrapper proliferation
- [ ] run adversarial diagnostic, infrastructure rendering and invariant-panic tests
- [ ] run current graph-outcome and generated-function boundary tests without pulling queued graph redesign into this plan
- [ ] record successful-path and failure-lane overhead

### Phase 5 exit criteria

- [ ] exactly three failure lanes exist
- [ ] user and operational failures are recoverable and separately renderable
- [ ] only proven compiler invariants panic
- [ ] `CompilerError` and its bridge are deleted

---

## Phase 6 — CLI and long-lived tooling isolation

### Summary, reasoning and context

A compiler bug is non-recoverable for the owning compilation. The CLI terminates after a structured
report. A long-lived host may continue only by running compilation in isolated owned state and
discarding that state completely after a panic. This phase removes poisoned-state recovery rather
than normalizing it.

### Slice 6A — Audit current host ownership

- [ ] trace every mutable value shared between CLI/dev-server state and build execution
- [ ] identify locks held while compiler/build code runs
- [ ] identify panic catches, poisoned-lock recovery and immediate-print paths
- [ ] identify all affected Cargo panic profiles and binaries
- [ ] state which host state may update only after a coherent worker result

### Slice 6B — Add one owned compilation worker boundary

- [ ] evolve the existing `DevBuildExecutor`/`ProjectBuildExecutor` seam into a factory or immutable request that creates one fresh mutable worker per build
- [ ] use a fully owned same-thread worker closure as the baseline isolation mechanism; add a dedicated thread, channel or pool only when current host concurrency requires it and benchmark the added boundary
- [ ] define a narrow worker request containing immutable build/configuration facts
- [ ] make the worker own all mutable compiler, source, token, identity and diagnostic builders
- [ ] move the complete worker into the unwind boundary; do not use broad `AssertUnwindSafe` over borrowed host state; a narrowly documented wrapper around fully owned state is allowed only because that state is always discarded after unwind
- [ ] return success, diagnosed or infrastructure outcomes only after coherent freeze/finalization
- [ ] catch unwind at exactly one long-lived host boundary
- [ ] install one process-level panic-hook policy once through a race-free global initialization owner: suppress immediate output only for recognized `CompilerBugReport` payloads, delegate unknown panics to the previous hook and never swap hooks per build; let the owning host boundary render the recognized report exactly once
- [ ] recognize the self-contained `CompilerBugReport`; treat an unknown panic payload as a generic compiler bug without inspecting discarded compiler state or duplicating terminal output
- [ ] discard the complete failed worker and construct a new one for the next rebuild
- [ ] prohibit compiler/build calls while host shared-state locks are held
- [ ] avoid a generic task framework or reusable panic-catching abstraction inside compiler stages

### Slice 6C — Integrate the CLI

- [ ] run the CLI compilation in one fully owned top-level host closure so a panic can be rendered before process exit without reusing state
- [ ] install one structured panic/reporting policy that avoids duplicate default and host-rendered output
- [ ] render diagnosed and infrastructure outcomes through separate boundaries
- [ ] let a compiler bug terminate the CLI process after the structured report
- [ ] preserve distinct exit-status policy
- [ ] add portable subprocess tests for diagnosed, infrastructure and compiler-bug exits; where one host path is platform-specific, pair its focused host-function test with the closest portable subprocess boundary

### Slice 6D — Integrate the dev server

- [ ] run each initial build/rebuild through the owned worker boundary
- [ ] update shared dev-server state only after worker completion
- [ ] on a compiler bug, keep last known-good served artifacts and publish a compiler-bug failure state/page
- [ ] discard all failed worker compiler state
- [ ] keep watcher scheduling alive at the host level
- [ ] keep watcher/server IO failures on infrastructure/host IO paths
- [ ] remove poisoned build-state lock recovery and tests that normalize poisoned compiler state
- [ ] prove a failed worker or executor instance is never reused and cannot corrupt or poison the next rebuild

### Slice 6E — Align panic profiles

- [ ] use `panic = "unwind"` for binaries/profiles that implement host recovery
- [ ] keep structured panic output for binaries that still terminate
- [ ] measure binary size and representative compile-time impact
- [ ] record the accepted cost and exact profile policy in the architecture/build documents
- [ ] leave process-isolated workers to the deferred item below

### Slice 6F — Expose the future tooling boundary

- [ ] expose immutable source/report views and owned worker inputs suitable for a future LSP host
- [ ] do not implement LSP protocol, cancellation or incremental document state
- [ ] ensure frozen source/diagnostic context can outlive the worker safely
- [ ] document restart/cancellation as future host policy, not a fourth failure lane

### Phase 6 — Audit / style-guide review / validation

Complete the common phase close, plus:

- [ ] audit no compiler mutation occurs while host locks are held
- [ ] audit failed worker state is never reused
- [ ] audit `catch_unwind` exists only at the host isolation boundary
- [ ] audit CLI compiler bugs remain non-recoverable
- [ ] audit dev-server continuation uses last known-good host state, not recovered compiler state
- [ ] review `Send`/`Sync`, panic-hook, ownership and state-update boundaries
- [ ] run CLI subprocess, worker panic, dev-server rebuild and infrastructure tests
- [ ] record panic-profile, binary-size and host overhead

### Phase 6 exit criteria

- [ ] compiler bugs terminate the owning compilation
- [ ] long-lived hosts survive only by discarding isolated worker state
- [ ] poisoned compiler-state recovery is gone
- [ ] panic strategy matches the implemented host model

---

## Phase 7 — Final measured optimisation, cleanup and handoff

### Summary, reasoning and context

Re-measure the complete architecture after all ownership changes. Accept optional packing only where
it produces a repeatable compiler-wide benefit. Then remove every migration remnant, converge
documentation and reactivate semantic diagnostic improvement work.

### Slice 7A — Account for final retained memory

Measure separately:

- [ ] source text, line starts, provenance and extended spans
- [ ] mutable/frozen string and path tables
- [ ] token records, spans, numeric/path cold stores and token ranges
- [ ] diagnostic records, extras, labels, lists, type-display records and frozen context
- [ ] successful, warning-heavy and diagnostic-heavy builds
- [ ] allocation counts, peak retained bytes and drop timing where reliable
- [ ] serial versus parallel merge/remap costs

Identify the top remaining owners before changing anything.

### Slice 7B — Run bounded optional reviews

- [ ] extended-span deduplication only if repeated extended ranges are material
- [ ] hot/cold `SourceRecord` splitting only if cold fields pollute measured source access
- [ ] source snapshot compression/mapping remains deferred unless source bytes dominate and a separate design is approved
- [ ] path-node AoS/SoA only if path traversal is a measured hotspot
- [ ] token cold-store packing only with formal bounds and aggregate evidence
- [ ] direct path-token fast form only if it improves aggregate memory or parse time
- [ ] extra/list record packing only if diagnostic cold data is material
- [ ] verify the selected `LocalSpan` split against the final corpus
- [ ] do not reduce `TokenShape` below 8 bytes or change the 32-byte diagnostic budget in this plan

For each experiment:

- [ ] record hypothesis and bounded prototype
- [ ] run retained-memory and five-run median comparisons
- [ ] record maintenance/audit cost
- [ ] accept only if all architecture gates pass and benefit is repeatable
- [ ] otherwise revert all code/tests/flags and record rejection plus re-entry criteria

### Slice 7C — Remove obsolete code and names

- [ ] search for all old source/location/path/token/diagnostic/message/error type names and macros
- [ ] classify any intentional remaining name; rename it if it implies deleted ownership
- [ ] delete migration modules, adapters, aliases and deprecated constructors
- [ ] delete stale remap/clone helpers and obsolete counters
- [ ] delete tests that protect only removed API shapes
- [ ] confirm no `#[allow(clippy::result_large_err)]`, including the temporary allowances in `src/lib.rs` and `xtask/src/benchmark_execution.rs`, and no boxed diagnostic boundary remains
- [ ] run ordinary dead-code/unused checks through Clippy

### Slice 7D — Converge authority and policy documentation

- [ ] finalize `docs/compiler-data-layout-design.md` with selected constants and final file map
- [ ] update compiler and build-system authorities to the implemented ownership/failure model
- [ ] update `AGENTS.md`, style, testing and validation guidance
- [ ] update `index.md` and module-level docs/comments
- [ ] review language and memory authorities; change only stale implementation references
- [ ] rebuild generated documentation and inspect changed routes/diff

### Slice 7E — Close roadmap ownership and resume diagnostics work

- [ ] mark this plan complete using current roadmap convention
- [ ] move rejected/postponed items to the roadmap with the deferral table below and links here
- [ ] remove duplicate deferred bullets owned by another plan
- [ ] update the existing Structured diagnostics matrix row to the implemented report/failure contract
- [ ] unpause the user-facing diagnostics improvement work immediately after this plan
- [ ] refresh its paths, capsule and next semantic slice against the schema/store APIs
- [ ] remove old payload, label, token and type-context assumptions from that plan
- [ ] require future diagnostics to fit the 32-byte schema and side-store policy

### Slice 7F — Final correctness, performance and determinism gate

- [ ] run every layout, property, schema, range, type-render and failure-worker test
- [ ] run all Rust tests and the complete canonical integration suite/audit
- [ ] confirm stable diagnostic codes, source spans and renderer identity across terminal/terse/dev server
- [ ] confirm successful artifacts/goldens are unchanged except explicitly authorized output
- [ ] run docs check and release build
- [ ] run native/Linux/Windows Clippy with warnings denied after removing the temporary `result_large_err` suppressions from `src/lib.rs` and `xtask/src/benchmark_execution.rs`, and confirm the large-error lint is absent without boxing the common diagnostic
- [ ] run full `just validate`
- [ ] run the complete five-run recorded benchmark protocol
- [ ] compare against Phase 0 and each material phase
- [ ] confirm aggregate retained frontend and failure-path memory improves
- [ ] confirm no unaccepted median regression above 5%
- [ ] confirm serial/parallel IDs, diagnostics and outputs are deterministic

### Phase 7 — Audit / style-guide review / validation

Complete the common phase close and Slice 7F, plus:

- [ ] perform a repository-wide ownership and duplicate-path audit
- [ ] perform a repository-wide module/style/comment audit
- [ ] audit every narrowing codec, reserved bit and capacity path
- [ ] audit every user diagnostic, infrastructure failure and compiler-bug site
- [ ] audit test ownership after migration cleanup
- [ ] audit documentation consistency across authority, roadmap, matrix, plans and code comments
- [ ] confirm no temporary adapter, stale owner or compatibility path remains
- [ ] record final evidence, final accepted commit and closed context capsule

### Phase 7 exit criteria

- [ ] one canonical source/path/token/diagnostic/failure architecture remains
- [ ] every hard layout and semantic gate passes
- [ ] aggregate memory improvement is demonstrated
- [ ] all required documentation is current
- [ ] every deferred item is explicit and owned
- [ ] the diagnostics-improvement plan is ready to resume without reopening layout architecture

---

## Deliberately deferred work

Maintain one `Compiler data-layout follow-ups` subsection under the roadmap's deferred-design area.
Add an item when this plan activates or when an experiment is rejected, link back to this plan/design,
and remove overlapping bullets from other plans. Do not add these implementation optimisations to the
progress matrix unless they change current tooling or user-visible support.

| Deferred item | Owner / re-entry criteria |
|---|---|
| Process-isolated compiler workers | separate tooling/reliability plan after thread-isolation evidence shows process isolation is needed |
| Persistent serialization of `SourceId`, `PathId`, token or diagnostic IDs | persistent artifact format must define canonical identities and remapping; process-local IDs are never serialized directly |
| Persistent/incremental source database and token reuse | incremental invalidation and source-hash ownership must be defined first |
| Source snapshot compression or memory mapping | retained source bytes must be a measured top memory owner and platform/lifetime policy must be designed |
| Terminator-based span encoding | only reconsider with a new exact constant-time design that passes every architecture gate and materially beats exact overflow storage |
| Token records below 8 bytes | token shape must become a measured common-memory hotspot after this plan |
| Procedural/build-script diagnostic schema generation | `macro_rules!` schema must first become demonstrably unmanageable |
| Broader compiler-wide ID bit packing | each domain must have a formal bound and measured retained-memory pressure |
| Cross-build/context diagnostic deduplication or caching | repeated frozen reports must be material and stable invalidation/identity must be specified |
| Full LSP protocol, incremental documents and cancellation | separate tooling plan consuming this source/report/worker architecture |
| Further path/token/diagnostic cold-store packing | the exact cold store must appear in a later retained-memory profile |
| Unrelated diagnostic wording and coverage improvements | existing compiler diagnostics improvement plan after this plan completes |

## Risk controls

| Risk | Required control |
|---|---|
| Migration creates two permanent architectures | bridges are private, caller-frozen and tied to the exact deletion slice named above; all are gone before final handoff |
| Packed records become unreadable | typed constructors/accessors, private codecs, semantic debug views and schema tests |
| Integer narrowing truncates authored input | checked construction and typed user-facing capacity diagnostics |
| Retained source text erases memory wins | move existing strings, measure snapshots separately and drop source/token data at explicit lifetimes |
| Path interning adds contention | immutable bases, worker-local deltas, canonical existing merge points and no global lock |
| Token ranges create lifetime complexity | IDs/ranges plus short-lived views; no durable Rust references or self-referential storage |
| Four diagnostic facts are insufficient | simplify semantics, derive descriptor facts, use labels and one typed extra; never widen the record |
| Type snapshots lose useful rendering | one shared formatter and exhaustive live/snapshot equivalence tests |
| Panic reclassification hides user bugs | site ledger, adversarial tests and named prior invariant for every compiler bug |
| Dev server catches too broadly | one host boundary, complete worker-state discard and no compiler-internal catch |
| Clever packing regresses throughput | five-run medians, focused profiles and mandatory revert when benefit is not repeatable |

## Final handoff contract

After every Phase 7 exit criterion is checked:

1. Keep `docs/compiler-data-layout-design.md` as the active architecture authority.
2. Delete this plan and its roadmap entry in the commit that completes it, as `Adding and maintaining plans` in `docs/roadmap/roadmap.md` requires. Git history is the implementation record.
3. Unpause the user-facing diagnostics improvement work.
4. Refresh that work from the current branch; do not preserve old file paths or payload assumptions.
5. Implement each future diagnostic through one schema entry, typed facts, labels and optional cold data.
6. Require every diagnostic review to answer:
   - Does the diagnostic fit four fact words?
   - Can a related source site be a secondary label instead of payload data?
   - Is variable data genuinely required?
   - Can the semantic facts be simplified before adding cold storage?
   - Does the stable external code still identify the same diagnostic family?
7. Keep future wording/presentation work separate from data-layout work unless correctness requires a fact change.

This plan is complete only when future compiler and diagnostic work can proceed without reopening
source identity, span encoding, token ownership, path identity, diagnostic record width, type-display
retention or failure-lane architecture.
