# Moth Compiler Source, Token and Diagnostic Data Layout Implementation Plan

> **Repository path:**
> `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`
>
> **Architecture authority:**
> `docs/compiler-data-layout-design.md`
>
**Status:**
Phase 1 is delivered on main; Phase 2 is accepted at `c17672bb5`; the Phase 3 implementation
cutover is recorded at `6309adf6d`, and the R5 closeout source corrections are accepted
at `4cfd9d492`. The canonical `SourceTokens` owner, bounded parser views, typed payload stores and
preparation lifecycle are complete; `Token`, `TokenKind` and `FileTokens` have been deleted.
Phase 3 is accepted with the explicitly inherited generic-scaling exception recorded below; Phase 4
remains gated behind the roadmap sequence and explicit reactivation.
Validation provenance: the executable Phase 3 closeout is attached to its recorded code/evidence
checkpoint. The manually dispatched `Validate and deploy` workflow at `7afdcc4e16` is the final
pre-merge platform evidence and is intentionally not green: native and feature-configured Clippy
configurations at that checkpoint report the known `clippy::result_large_err` failures from the
`CompilerMessages` representation recorded there, including `Result` call sites whose `Err`
variant is at least 128 bytes. These are deferred diagnostic-layout failures owned by later phases; this closeout does
not box `CompilerMessages` or suppress the lint. The other exercised validation families remain
recorded as passed, and the generic scaling exception remains separately accepted without changing
its budget. Detailed measurements and the complete disposition live in
`benchmarks/frontend-optimization-results.md`.
After the Phase 3 closeout, this plan pauses through the roadmap order: MON
syntax and nested const records, MON Rust tooling, Wiring V1, then native result slots and Core
const evaluation, with Phase 4 resuming only after explicit reactivation.

## Purpose

Replace Moth's allocation-heavy source, path, token, diagnostic and failure representations with
one compact data-oriented architecture before broad diagnostic representation work resumes.

This is a compiler-wide representation change, not a narrow Clippy patch. It must remove the root
causes of `clippy::result_large_err`, remove existing boxed-diagnostic workarounds and leave one clear
extension model for future diagnostics and tooling.

## Historical activation validation bridge

At plan activation, the implementation still carried a 192-byte `CompilerError` across internal
and infrastructure `Result` boundaries. The activation record also documented crate-level and
benchmark-module `result_large_err` allowances for that predecessor state and its 224-byte
`BenchmarkCaseFailure`.

Those allowances and boxed-diagnostic explanations are historical activation evidence, not current
guidance. The current boundary carries diagnosed failures as plain `CompilerDiagnostic` values and
keeps typed infrastructure failures in the `CompilerError` lane. The Phase 1 closeout records the
removal of the activation bridge and its lint allowances.

Test Suite Hardening landed in `03168082d`. Its activation-era large-error variants are this plan's
owned follow-up, not a test-suite regression; no local lint workaround remains.

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

The diagnostics-improvement plan remains paused until this plan is complete. It resumes at its
recorded Phase 4.1c slice after the layout migration, not during it; Phase 4 is the only phase
that continues user-facing diagnostics work, and that continuation is preserved there.

---

## Active context capsule

Refresh this block after every accepted slice and immediately before context compaction. Do not resume
from a compressed summary alone.

ACTIVE_PLAN:
- `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`

- Phase: Phase 3 fixed-token/source-owned implementation is accepted at checkpoint `6309adf6d` on
  `diagnostic-data-layout-changes`; R5 source corrections are accepted at `4cfd9d492`.
- Goal: `SourceTokens` is the canonical immutable token owner. Headers, AST and generic bodies
  retain checked ranges and sequence IDs over that owner. The former `Token`, `TokenKind` and
  `FileTokens` representations, compatibility cursors and copied parser windows are deleted.
- Current code evidence: `src/compiler_frontend/tokenizer/` owns canonical `SourceTokens`, SoA
  storage, typed cold stores and bounded `TokenCursor`/`TokenRef` views. Headers, prepared sources
  and generic bodies retain canonical ranges or sequence IDs; `SourceTokenOwner` and
  `StableBodyOwner` carry the required source and donor identity without cloned token vectors.
- Validation provenance: executable R5 evidence is attached to `4cfd9d492`; the final manual
  `Validate and deploy` workflow is recorded at `7afdcc4e16`. That workflow is intentionally red
  only on the known `clippy::result_large_err` diagnostic-layout failures, including `Err` carriers
  at least 128 bytes. Do not interpret the red Clippy/feature configurations as a Phase 3 token
  ownership regression or fix them by boxing/suppression.
- Scaling status: exact final-checkpoint constant and nominal fits are within `n^1.25`; generic
  is `n^1.770` against the unchanged `n^1.70` budget. The generic exceedance is the explicitly
  accepted inherited exception. No budget was raised or loosened and no no-worsening claim is made.
- Retention status: the feature-gated owner ledger records source-token arrays, cold stores,
  transient construction pressure, requester remaps, donor identity tables and generic owner
  live/peak bytes for clean, warning, diagnosed, generic-heavy and early-malformed smokes.
- Closeout status: R3 diagnostic projection, R4 preparation ownership and R5 evidence disposition
  are complete; deferred large-error carriers remain owned by later diagnostic-layout phases.
- NEXT_SUBSTEP: remain paused while MON syntax, MON Rust tooling, Wiring and native
  result-slot/Core const-eval checkpoints land, then explicitly reactivate Phase 4.
- Checkpoints: `b5e1b8fa3`, `1e39f7678`, `a80fa63d6`, `77c0c6fc8`, `8fc783a9d`, `f60def921`,
  `aed38042f`, `72f30dcfb`, `e7d9a7ab5`, `c17672bb5`, `98040fbd0`, `fbbe0119a`, `6309adf6d`,
  `4cfd9d492`.
- Non-goals: diagnostic compact-record work; package implementation; MON syntax and nested const
  records, MON Rust tooling, Wiring V1 and native result-slot/Core const-eval
  implementation before their owning roadmap checkpoints; Phase 4 reactivation before the explicit
  gate.

Phase 1 code closeout is `3c9c776a8`; Phase 2 continuation acceptance is `c17672bb5`, with
diagnostic correction `e7d9a7ab5`; Phase 3F5/3G acceptance is `98040fbd0`, implementation
checkpoint `6309adf6d` contains the completed code cutover, and R5 source corrections are accepted
at `4cfd9d492`. Detailed implementation and review checkpoints remain in Git history, while the
summaries below retain only contracts and evidence needed by later phases.

CURRENT_WORKSPACE_STATE:
- Phase 1 remains complete. Phase 2 PathId cutover, generated identity pairing and report-owner
  retention metrics are accepted at `c17672bb5` with the diagnostic correction and validation-lane
  stabilisation; refreshed probe evidence is recorded.
- Phase 3 fixed-token/source-owned implementation is accepted at `6309adf6d`, with R5 source
  corrections and exact evidence accepted at `4cfd9d492`. `SourceTokens` is the only token owner;
  retained ranges and sequence IDs, typed payload provenance, preparation ownership and diagnostic
  projection provenance are final.
- Exact final R5 evidence records constant and nominal fits within budget and an explicitly accepted
  inherited generic `n^1.70` exception. No budget was raised or loosened and no no-worsening claim is
  made.
- Known deferred validation: the manual workflow at `7afdcc4e16` reports `result_large_err` in
  the Phase 3 checkpoint's `CompilerMessages` result boundaries, including `Err` variants at least
  128 bytes. The later diagnostic-layout phases own the compact representation; no boxing or lint suppression is
  accepted, and fresh validation is required when the next implementation checkpoint activates.
- After Phase 3 closeout, this plan pauses through the roadmap order: MON
  syntax and nested const records, MON Rust tooling, Wiring V1, then native result slots and Core
  evaluation. Phase 4 resumes only after this branch is rebased and Phase 4 is explicitly
  reactivated (see the Phase 4 reactivation gate in the Phase 4 section).

HISTORICAL_PHASE_3_MERGE_STATE:
- branch: `diagnostic-data-layout-changes`
- candidate HEAD: `7afdcc4e16fc72e8cb66c2139eabd4e23634f797`
- current `main`: `b1d2d2f0edc09db8a9dca25df29d7a5669c06775`
- merge base: `b1d2d2f0edc09db8a9dca25df29d7a5669c06775`
- worktree: clean before this documentation-only closeout
- post-implementation history: `7576bccb1` changed validation configuration/docs only;
  `b79113ad2` carried the memory-safe validation closeout (31 Rust files plus 13 docs/benchmark
  files); `34c1f45e5` merged MON documentation; `7c0d41ee2` regenerated docs; `4cfd9d492` changed
  five R5 source files; `7afdcc4e1` changed twelve benchmark/authority/roadmap files.
- this closeout adds documentation/roadmap changes only. No Rust source, tests, manifests, build
  scripts, benchmark implementation or fixtures are in scope.

HISTORICAL_ACCEPTED_SLICES:
Phase 0 and Phase 1 are complete on main; Phase 2 is accepted on the continuation branch at
`c17672bb5`. Per-slice delivery, review and validation logs live in Git. The compact Phase 0/1/2
summaries below keep standing contracts, later-phase prerequisites and ownership notes that later
slices still need. Do not resume accepted-phase work from this capsule.

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
- `src/build_system/create_project_modules/prepared_source.rs::PreparedSourceInput`: the three
  `PreparedSourceKind` variants keyed by final `SourceId` — canonical `Moth`, retained
  `MothPrepared` output and deferred text; source kind, paths and snapshots remain
  `SourceDatabase`-owned
- `src/compiler_frontend/module_compilation/service.rs::compile_module`: the one production owner of the local semantic sequence, and the consumer of whatever source identity representation this plan lands on
- `src/compiler_frontend/pipeline.rs::CompilerFrontend`: stage facade with remaining immutable-service copies
- `src/compiler_frontend/source/`: build registration, loaded snapshots, exact spans and line indexes
- `src/compiler_frontend/symbols/path_interner/`: current dense parent-linked path table owner
  (builder, fork, delta, remap, frozen lookup; complete-path identity since Phase 2)
- `src/compiler_frontend/symbols/string_interning.rs`: existing immutable-base fork and deterministic delta merge to reuse
- `src/compiler_frontend/tokenizer/`: canonical `SourceTokens` schema, SoA storage, typed cold
  stores and bounded `TokenCursor`/`TokenRef` views; the former `Token`, `TokenKind` and `FileTokens`
  representations are deleted
- `src/compiler_frontend/headers/types.rs::Header`: retained `TokenRange` bodies, optional
  `TokenSequenceId` start syntax and `SourceTokenOwner` containing only the canonical owner; logical
  and OS paths resolve through `SourceDatabase`, with no cloned token vectors
- `src/compiler_frontend/headers/header_dispatch.rs`: canonical header cursor range helpers and
  bounded range capture, with no clone-based body capture
- `src/compiler_frontend/ast/cursor.rs` and `src/compiler_frontend/declaration_syntax/mod.rs`:
  canonical bounded cursor handoffs; no compatibility parser lane
- `src/compiler_frontend/ast/generic_functions/materialisation/frozen_syntax.rs`:
  `StableBodySyntax` owns donor/source identity, frozen identity tables and resolved file-reference
  facts; `StableBodyOwner` retains only the canonical `Arc<SourceTokens>`
- `src/compiler_frontend/compiler_messages/`: current diagnostic kinds, descriptors, payloads, labels, bags, messages and renderers
- `src/compiler_frontend/compiler_messages/compiler_errors.rs`: current mixed error lane, table cloning and full type-context retention
- `src/lib.rs` and `xtask/src/benchmark_execution.rs`: activation-era lint-bridge locations retained only as historical removal records; current workspace status makes no validation claim
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

- Phase 3 is accepted at implementation checkpoint `6309adf6d` with R5 closeout checkpoint
  `4cfd9d492`; the post-Phase-3 compiler tidy-up is also accepted on main.
- User-facing diagnostic improvement remains paused until the Phase 4 reactivation gate is satisfied.
- release/profiling currently use aborting panics, which conflicts with thread-isolated tooling recovery.
- compact-ID merge order must remain deterministic across file and module parallelism.
- The first source-freeze experiment was deliberately rejected after mapping the final owner
  boundary: a `FrozenSourceDatabase` without an immediate render/identity consumer would create a
  dead parallel owner and warnings. Revisit source freezing together with that terminal owner.
VALIDATION_STATE:

Executable Phase 3 closeout provenance belongs to the recorded source/evidence checkpoint
`4cfd9d492`, not to later documentation HEAD. The manually dispatched `Validate and deploy` workflow
at `7afdcc4e16` is the final pre-merge platform evidence for that code checkpoint. It is intentionally
not green: the blocking native and feature-configured Clippy failures are the known
`clippy::result_large_err` failures from the current `CompilerMessages` representation, including
call sites where the `Err` variant is at least 128 bytes.

The workflow still exercised and recorded the successful feature coverage, source and first-party
audits, workspace tests (`5210 + 17 + 839`), integration (`1973/1973`), docs and benchmark
preflight/quick-sanity (`82/82`) families. Those results do not make the aggregate workflow green.
Do not box `CompilerMessages`, add a lint allowance or suppress `result_large_err`; the later
diagnostic-layout phases own the compact diagnostic/error-carrier migration. Fresh validation is
required when the next implementation checkpoint activates.

The exact release R5 evidence records constant `n^0.597`, nominal `n^0.919` and generic `n^1.770`;
constant and nominal remain within budget, while the inherited generic `n^1.70` exception remains
accepted without changing the budget or claiming no worsening. `just timers-erasure-check` passed
separately with an `8,862,096`-byte no-timer binary. Earlier per-slice validation is Git history,
not a current workspace claim. The plan pauses after Phase 3 through the roadmap order above; Phase
4 reactivation remains gated.
Gate hygiene: `just validate` diffs tracked files during its benchmark stage — edit only before it
starts or after it exits. `cargo test -p moth --lib` misses test targets; use the featured
all-target Clippy gate before accepting a slice.

DOCS_IMPACT:
- progress matrix needed: only when current diagnostic/failure/tooling behaviour changes; do not add an internal-refactor status row
- current authorities, implementation map and style/validation records now describe the accepted
  canonical token owner, bounded parser views, preparation lifecycle and inherited generic-budget
  exception
- authorised docs updates for this 3H closeout: the plan, the architecture status, the compiler
  implementation overview and the benchmark evidence
- next action: preserve the MON syntax, MON Rust tooling, Wiring and native result-slot/Core
  const-eval sequence before Phase 4
- `R10a`–`R10d` remain prerequisites for their owning later phases (see the integrated list in the
  Phase 1 standing-contracts section).

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
| Source identity | `SourceId` is a non-zero 4-byte build-lifetime ID, final before inventory-backed tokenization or at the one private discovery-finalization barrier, then immutable within its context |
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

## Repository shape recorded at activation (historical)

Phase 0 recorded the predecessor shape of the source, token and diagnostic owners below. This
activation inventory is historical, as of plan activation; it is not a current workspace status.
Do not carry it forward as a current owner map.

Historical activation pressure points:

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
Stage 0 inventories pre-register final SourceIds before preparation; traversal-only discovery normalizes private provisional IDs before publishing success or diagnosis
-> selected source text is loaded once into its preassigned slot
-> module work owns SourcePreparationDelta values plus local string/path deltas
-> each source's preparation result is cached once for Stage 0 structural reachability and later module compilation while its source-local extended-span builder stays owned until the last span-producing stage
-> DiagnosticBag captures only referenced type-display facts while the local TypeEnvironment is live
-> existing file/module merge boundaries merge strings, then paths, then remap prepared diagnostic batches into the module domain's DiagnosticBag; the command/report owner collects them without a second durable store
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

Test Suite Hardening was delivered in `03168082d`. This plan was the active representation migration
through Phase 3, with implementation checkpoint `6309adf6d` and R5 closeout checkpoint `4cfd9d492`.
The canonical `SourceTokens` owner, bounded parser views, typed payload stores and preparation
lifecycle are complete; Phase 3 is accepted with its explicitly recorded inherited generic-scaling
exception before the plan pauses through the roadmap sequence toward Phase 4. The diagnostics plan
remains paused until that gate.

### Approved private discovery-finalization contract

User-approved authority: `docs/compiler-data-layout-design.md` >
`Source identity and database > Private discovery finalization`.

Directory/package inventories and standalone templates keep upfront final IDs. Synthetic
single-file and recursive direct-template discovery may use private provisional IDs, normalized
once before success or diagnosed publication. The authority owns ordering, disposal, visibility
and context rules. This exception adds neither an eager inventory scan nor a second ID type.

Required acceptance:
- [x] prove a traversal whose provisional and final numeric IDs differ normalizes retained facts
- [x] prove different discovery orders yield identical final identity and source-shell facts
- [x] prove early preparation failure after a prior prepared source leaves only final-domain
  diagnostics and attached source context

### At activation

- [x] make this the sole active source/token/diagnostic representation migration
- [x] record the delivered hardening commit `03168082d` in this plan and the benchmark report
- [x] confirm the diagnostics plan remains paused until this representation migration completes
- [x] prohibit old-model diagnostic payload work while this plan is active
- [x] keep unrelated scope-frame, arena and semantic-invariant work deferred in the roadmap

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
`PathId`/legacy-path bridge is historical: it ended at Slice 2D when Phase 2 was accepted. The
transitional `CompilerMessages` identity-context bridge ending in Slice 4I and any measured early
token projection ending in Slice 3H remain the only anticipated cross-phase cases.

---

## Phase 0 — Activation, current-state audit and evidence baseline (historical)

Phase 0 is an activation baseline, not a current workspace report. Evidence:
`benchmarks/frontend-optimization-results.md`; searchable inventories: `target/data-layout-audit/`.
Detailed execution history remains in Git.

Delivered: hardening prerequisite `03168082d`; this plan activated as the sole representation
migration with diagnostics work parked; source/location, path, token, diagnostic, context-copy and
failure owners mapped to phases; layouts, corpus sizes and representative workloads recorded;
`data_layout` benchmark support added with no normal-build cost; activation validation passed on
Rust 1.97.1 native/Linux/Windows Clippy plus `just validate`.

Limits that still apply: memory evidence covers bounded capacities and aggregate live/peak
allocation proxies, not a complete heap partition, nested heaps or path-only remap counts. Exact
span histograms required byte offsets and were delivered in 1C, not Phase 0.

---

## Phase 1 — Build-lifetime source identity and exact compact spans

### Summary, reasoning and context

Source locations are the most pervasive representation problem. This phase moved source ownership to
Stage 0, introduced exact packed byte spans and migrated every compiler stage and renderer. It also
removed duplicated location data and obsolete diagnostic indirection so later layout work proceeds
from one source-span boundary.

Phase 1 is complete on main. Implementation tasks and per-slice checklists are closed; Git holds
the delivery log. The contracts below are standing rules and ownership facts that later phases
still need.

**Source identity and registration.** `PathId(NonZeroU32)` and a dense parent/component path table
intern source logical paths into the database-owned build base before string/path forks; compiler
source registration slots use `PathId` immediately; filesystem `PathBuf`/`Box<Path>` stay separate
from compiler logical identity. `SourceLogicalIdentity` keeps its owned portable spelling as the
`SourceId` sort key; canonical source order is established before path interning, and numeric
`PathId` allocation order must never sort source candidates. Source paths preserve exact
`Path::components()` semantics and strict UTF-8 validation; portable semantic spellings preserve
their component separators. The frozen path table owns nodes and depths only; the child map lives
and dies with the builder.

The source database owns one registration slot per candidate, dense loaded snapshots and a cold
load-failure array. `SourceId(1)` is the compilation root. Config is registered before tokenization
and stays in the project identity domain; independently compiled packages keep separate domains.
Stage 0's `SourceRegistrationIndex` preserves module-origin order; traversal-only single-file and
direct-template lanes use canonical logical-path order, and their provisional identities stay in a
private, disposable discovery-local domain rebound exactly once before success or diagnosed
publication. A flat display-path sort must not replace either lane's canonical ordering policy.
Loaded text moves into its slot once (a second retain is an invariant failure); re-registration of
one canonical source rejects conflicting logical identity or kind; display-path equality is not
source identity. `SourceKind` records the producer's authored classification, not the canonical
target's extension or builder support; provider-owned slots carry identity but acquire no compiler
snapshots. Module inputs carry ordered candidate `SourceId` sets whose external-import scope is the
module's owned candidates. File/chunk merges place prepared results in preassigned slots and reject
duplicate, missing and out-of-range outputs. `SourceDatabaseBuilder` separates live span ownership
from immutable lookup services; `finish` consumes the builder and returns an owned
`SourceDatabase`. Package lookup publication waits for check-only producers.

`SourceDatabase` owns the path interner as its one source identity base. The source
`PathId`/legacy-path bridge ended at Slice 2D; it reconstructed transient components from table
nodes, never rendered text, and no new consumer may adopt it. `ModuleSymbols` no longer copies
canonical OS paths. `FileTokens.canonical_os_path` and `FileFrontendPrepareOutput.canonical_os_path`
remain as explicit preparation and adapter metadata until 3H removes the duplicated fields. The
former canonical-path agreement check was removed with 3D.
Renderers use retained snapshots rather than reopening files; per-diagnostic-range source contexts
preserve package ownership; ambiguous display-path matches omit a frame rather than select the wrong file;
the direct-template API retains its finalized per-document source context; the facade and module
semantic context borrow immutable services.

**Spans.** `LocalSpan`/`Option<LocalSpan>` are 4 bytes; `SourceSpan`/`Option<SourceSpan>` are 8.
Selected codec: the measured 22/10 split (`LENGTH_BITS = 10`) with exact append-only overflow rows;
the census proved every candidate start-overflow-free, so length overflow alone decided the split.
The terminator experiment was deferred undone (not evaluated); its measured maximum prize is under
2 KB and re-entry criteria live in the architecture document and evidence report. One line-index
builder and byte-offset cursor is threaded through each source kind's traversal; the line index
builds once per snapshot; empty snapshots have no lines; EOF after a final terminator resolves to
the preceding visible line end; Unicode scalar columns serve rendering and UTF-16 columns serve
tooling, both derived lazily; empty spans overlap nothing and containment is the operation for
insertion points. The tokenizer's line-break set is LF, CRLF and bare CR; `TokenStream::next`
advances the UTF-8 byte-offset cursor; line indexes derive from retained snapshots. Unreadable-source
failure stays at the slot layer; loaded records own text, line starts and extended spans
unconditionally. Cross-source joins are rejected; named source-order, overlap and containment
operations replace ambiguous location `PartialOrd`. Any remaining allowance must name a real
unreached consumer rather than suppress an entire module.

**Preparation and builder ownership.** Tokenization emits `LocalSpan`; source-scoped diagnostics
emit `SourceSpan`. Authored token streams and header/source identity are keyed by registered
`SourceId`; frozen generic syntax preserves or canonically remaps its owning context; the
compilation root never substitutes for missing identity. The original source builder remains owned
through the last producer; preparation, aggregation and diagnosis publish only final source
identities before downstream consumers run; no repeated preparation or independent builder may
create a parallel source table; later migrations must thread this owner into any new
joined/insertion-span producer. File workers return move-only `SourcePreparationDelta` values keyed
by owning source identity; private discovery domains normalize exactly once at final publication;
diagnostics produced before merge retain the producer identity/local span, then normalize before
downstream publication. The mandatory provider owns a dependency clause's path-token anchor; source
contracts retain the qualifier `#` anchor copied into `DeclarationSyntax.span`, not the header name
anchor. Const-template joins preserve first-interior-token through post-close-token bounds through
the original live builder; runtime fragments continue to use retained tokens without a new record;
synthetic constructors preserve authored anchors and explicit synthetic metadata without
manufacturing source identities. The historical interval bridge (exact spans plus line/column
location) is gone: authored tokens and diagnostics use source-qualified `SourceSpan` values.
Focused Rust tests use one owning-module test-only `TestSourceContext`; no production convenience
API for that helper.

**Downstream cutover.** All compiler source positions are exact compact byte spans, or explicit
absence for generated data; HIR, borrow facts, target-contract validation, Stage 0, config and
build diagnostics share the boundary; header parse-time duplicate sites carry their first authored
`SourceSpan` or an explicit spanless contract, with no path/line reconstruction. Consuming freeze
moves current string/source/minimal-path allocations into lookup-only `FrozenIdentityContext`; file
stages return move-only diagnostic/source deltas; the final build/package render boundary owns
diagnostics, type context and frozen identity; no module-level premature freeze or clone.
Materialised tokens, generated artefacts and diagnostics preserve or canonically remap their owning
`SourceId`, including extended spans after donor builders drop; synthetic/compilation-root display
and provenance are explicit; non-UTF-8 filesystem display stays in infrastructure/path handling.
`CompilerDiagnostic` owns one canonical primary span; secondary labels retain related sites without
copying primary facts; infrastructure failures remain in the typed `CompilerError` lane pending
Phase 5. A throwaway layout probe measured `CompilerDiagnostic` at 96 bytes and `CompilerMessages`
at 112 bytes on `aarch64-apple-darwin` (hard diagnostic bound 128 bytes). Plain `CompilerDiagnostic`
values cross the diagnosed lane with no common diagnostic boxing, no `result_large_err` allowance
and no lint-specific workaround. `SourceLocation`, `CharPosition`, `SourceFileTable`, `FileId`,
fallback path-based identity comparison, line/column mutation and location filesystem fallback
helpers are absent from production sources.

### Phase 1 closeout evidence

Final Phase 1 review-correction closeout is `3c9c776a8` (2026-09-10), after implementation
`a9f9744de`, representation corrections `e1f16cb49` and later correction checkpoints through
`fc9f449e9`. `just validate` passed on Apple M1 Pro / Rust 1.97.1 as recorded in the capsule.
Span-census corpus: 4,626 files walked, 4,585 tokenized, 286,779 spans; selected `LocalSpan`
extended table 1,992 bytes at the 22/10 split. Timing and retained-memory partitions, including
the five-run probe, live in `benchmarks/frontend-optimization-results.md`. Per-command closeout
tables are Git history.

### Phase 1 exit criteria

- [x] one build-lifetime source database is canonical and its mutable/frozen source lifecycle is explicit
- [x] all compiler source positions are exact compact byte spans
- [x] retained snapshots render diagnostics
- [x] old location/file identity types are absent from production sources
- [x] the compact plain diagnostic boundary has no common diagnostic boxing or lint-specific workaround
- [x] full local CI is green before Phase 2 begins

### Standing correction contracts

These remain in force for later phases. They do not reopen Phase 1, the accepted Phase 2 migration
or the accepted Phase 3 migration; Phase 3 is complete at `4cfd9d492`, and 3H old-token deletion is
closed. Current status and ownership are recorded in the active capsule and the completed 3H section
below.

#### Cold ownership

The common `DiagnosticRecord` remains 32 bytes and the common `SecondaryDiagnosticLabel` remains
12 bytes. Their compact spans are meaningful only in the identity domain of the enclosing report
context. Rare mixed-domain sites currently carry an optional typed `FrozenIdentityHandle` in cold
diagnostic or label ownership storage; the handle names the immutable string/source context that
resolves the span. No common record grows an `Arc`, path table, or rendered coordinate, and no
renderer guesses an owner from a colliding requester database. Slice 4B replaces this transitional
handle with the final typed cold-store encoding.

`ProjectEntry` and linked-module views retain the owning boundary domain. Project backend
diagnostics use the domain-less project default only for project modules; package diagnostics use
their exact package domain. Empty package rows are retained only for explicitly domained handles
and never become default render ranges. Distinct domain-less handles compare by allocation identity
rather than `None == None`.

#### Production lifecycle gates

Every Phase 1 outcome must cross one of these gates exactly once:

1. early diagnosed/blocked frontend result;
2. generated post-HIR diagnostic or warning;
3. late backend or output-builder failure;
4. successful build with warnings;
5. clean successful build.

The first four retain the final diagnostic owner and all source-domain associations. The fifth
drops compiler identity storage once no compact-ID consumer remains. A focused test that exercises
only frozen frontend diagnostics does not close the build/dev gates.

Generated materialisation retains the declaring frozen-identity handle through successful warnings,
HIR lowering, borrow checking and backend target validation. Re-anchoring preserves existing
primary/label owners, compares duplicate sites with their identity domains, and never falls back
from an unresolved donor-domain handle to the requester database. The body remains primary when
`call_span` is absent; a body secondary is created only when a real call span exists.

Mutable and frozen string resolvers use checked lookup. An in-range ID still requires the correct
identity domain; a foreign out-of-range ID is a compiler-invariant failure, never undefined
behaviour.

Registration assigns deterministic slots without reading files. Canonical discovery and explicit
check-only preparation load selected semantic sources once; provider-only and unselected sources
remain pending.

Authored table or compact-domain exhaustion is a deterministic typed source-capacity diagnostic;
capture exhaustion is terminal and carries the offending exact range. Compiler-produced impossible
overflow remains a compiler failure. Global `SourceSpan` live-builder operations accept only a
source-qualified resolver view and validate `span.source()` against the resolver source. Database
resolution of the reserved compilation-root `SourceSpan` accepts only the exact empty range
`[0, 0)`.

Scalar source offsets, UTF-16 tooling columns and terminal/HTML display cells are three separate
units. Terminal and HTML format independently; non-tab chunks use string display width while scalar
columns, UTF-16 columns and tab stops remain distinct.

#### Deferred representation work

Token-store consolidation and declarative token/diagnostic schemas remain deferred to their planned
migration slices. Parent-linked path tables and compact span encoding are Phase 1 current state, not
future work. This Phase 1 work adds no second interner, scheduler, observer API or ubiquitous
per-node owner.

The private discovery-finalization barrier, parent-linked path trie and local `_unspanned` wrappers
remain accepted choices for later phases; the ambiguity-failing legacy path lookup was a Phase 1
bridge and ended with Phase 2. None of these choices permits a parallel framework.

#### Later-phase prerequisites (R10)
- [ ] **R10a — diagnostic identity ownership (Phase 4, Slice 4A):** make the optional stable
  `reason_key` contract part of the final diagnostic schema. Preserve every existing reason key
  independently of rendered wording; diagnostics without a reason-key contract remain `None`.
  Do not manufacture keys merely because the schema can store one. Integrated into Slice 4A.
- [ ] **R10b — draft/durable type separation (Phase 4, Slice 4B):** define distinct draft and
  durable diagnostic fact/core types so a pre-compaction `TypeId` cannot masquerade as a durable
  fact. Integrated into Slice 4B.
- [ ] **R10c — infrastructure/bug context (Phase 5):** settle self-contained context ownership for
  infrastructure failures and compiler-bug reports before deleting the mixed error model.
  Integrated into Slice 5C0 and 5D; remains a Phase 5 prerequisite and is not pulled into Phase 4.
- [ ] **R10d — structural type-display dedup (Phase 4, Slice 4C):** keep `TypeId` to
  `TypeDisplayId` memoization mandatory, but require benchmark evidence before adding
  cross-identity structural hash-consing. Integrated into Slice 4C5.

---

## Phase 2 — Genuine complete-path interning

Phase 2 is accepted on `diagnostic-data-layout-changes`. Its continuation checkpoint is
`c17672bb5`, carrying the diagnostic correction `e7d9a7ab5` (path/string attachment and remap
ownership: one attachment contract, `NotExportedByPublicSurface` remaps `public_surface_name`,
required paths keep tables that contain unused nodes, and renderer type contexts retain the tables
they read) plus validation-lane stabilization. Per-slice checklists and review logs are Git
history.

Delivered, in order:

- **2A** extended the Phase 1 dense parent/component path table with module-local delta, remap and
  consuming frozen-table support; allocation-free parent/append/join after interning; portable and
  native rendering through `PathTable` plus string lookup; path-domain classification (compiler
  logical/semantic paths share the table, filesystem paths stay cold `Path` values, rendered free
  text is never interned).
- **2B** mirrored the string-table immutable-base fork at module scope; merges strings before
  path nodes (path records contain `StringId`); reuses the existing file/chunk and module merge
  order; remaps `PathId`s exactly once per boundary; source logical paths stay in the immutable
  build base and never remap. No scheduler, global lock, atomic allocator or generic interner
  framework was added; existing counters record path nodes, unique paths, depth, merges, remaps.
- **2C** migrated every compiler path owner (tokenizer/dependencies, headers/graph facts,
  semantic types/interfaces, AST/TIR/HIR metadata, diagnostics/support) to `PathId`. Stable
  semantic origin IDs remain the cross-package authority; display `PathId`s remap or context-own
  at interface binding. Retained per-header dependency sets became sorted/deduplicated
  `PathId` slices or typed arena ranges.
- **2D** deleted `InternedPath`, its vector constructors and remap implementation, repeated path
  clones and allocation-based parent/append/join helpers, and `HashMap`/`HashSet` keyed by
  `InternedPath`; `PathBuf` remains only at filesystem boundaries and source cold data.

### Phase 2 standing rules

- `PathId` is the only complete logical path identity; one path identity domain per compilation
  context; no detached raw path ID crosses a project/package boundary.
- Source paths stay stable while worker-created paths remap deterministically; no path identity
  is reconstructed from rendered text; no per-path lock or `Arc`.
- Path construction and merge order are deterministic; path bytes, allocation/remap counts and
  timing evidence live in `benchmarks/frontend-optimization-results.md` > `Data Layout Migration -
  Phase 2 Path Identity Retention Probe`.

---

## Phase 3 — Fixed tokens and source-owned retained syntax

### Accepted historical summary

Phase 3 replaced the wide token enum and cursor/storage mixture with one source-owned,
data-oriented representation. The accepted contracts are:

- the canonical production token layout is SoA `TokenShape` plus `LocalSpan`;
- every tokenized source has exactly one canonical `SourceTokens` owner;
- numeric and path payloads live in typed source-local cold stores;
- retained syntax uses checked `TokenRange` and `TokenSequenceId` values, not copied token windows;
- `TokenCursor` and `TokenRef` provide bounded borrowed views over canonical storage;
- preparation loads, tokenizes and prepares each selected source exactly once, then shares that
  move-owned result between reachability and semantic compilation;
- generic syntax retains the canonical owner and persistent donor/requester identity contract, with
  explicit typed provenance and no per-body identity-table copies;
- `TokenPayloadOrigin` translates donor payloads only at consumption time;
- `DiagnosticToken` is a durable independent projection and does not depend on source token storage;
- malformed compact payloads remain compiler/infrastructure failures rather than source diagnostics;
- `SourceTokenOwner` retains only the canonical owner, and old `Token`, `TokenKind`, `FileTokens`,
  compatibility parser cursors and copied parser windows remain deleted.

The implementation checkpoint is `6309adf6d`. R5 source corrections and exact evidence are accepted
at `4cfd9d492`; the rejected shared path-source experiment is not part of the accepted architecture.
The detailed 3A–3H and R0–R5 implementation diary, adapter inventories and correction narratives
remain available in Git history. The durable benchmark distributions and owner-ledger observations
remain in `benchmarks/frontend-optimization-results.md`.

### Phase 3 evidence and merge disposition

The exact final R5 fits are constant `n^0.597`, nominal `n^0.919` and generic `n^1.770`; the
constant and nominal fits remain within `n^1.25`, while the inherited generic `n^1.70` exception
is explicitly accepted without changing the budget or making a no-worsening claim.

The executable closeout evidence is attached to the code/evidence checkpoint, not later
documentation HEAD. The manual `Validate and deploy` workflow at `7afdcc4e16` is intentionally not
green: the known `clippy::result_large_err` failures remain in the Phase 3 checkpoint's
`CompilerMessages` result boundaries, including `Err` variants at least 128 bytes. This is deferred
diagnostic-layout work owned by later phases. Do not box `CompilerMessages`, add lint allowances or suppress the lint.
The other exercised validation families, the owner-ledger smokes and the separate timer-erasure
result remain recorded as evidence; none changes the non-green workflow disposition.

### Standing decisions for Phase 4 reactivation

Phase 4 and later phases must preserve the canonical owner/range/sequence model, exactly-once
preparation, donor/requester identity provenance, consumption-time payload translation and durable
diagnostic-token independence. They must not restore compatibility parser lanes, copied token
windows or a second token owner. The later diagnostic-layout phases own compact
`CompilerMessages`/error-carrier representation and the associated `result_large_err` failures;
Phase 3 closeout does not pull that work forward.

### Phase 3 — Audit / style-guide review / validation

The accepted phase review confirmed one canonical source-token owner, checked bounded parser views,
typed cold-store provenance, exactly-once preparation, generic donor/requester identity, independent
diagnostic projections and deletion of the old token architecture. Validation evidence is
checkpoint-scoped as described above; the generic scaling exception and deferred
`result_large_err` failures are explicit dispositions, not green-gate claims.

### Phase 3 exit criteria

- [x] `TokenShape`/`LocalSpan` SoA storage and typed cold stores are canonical.
- [x] retained syntax uses checked ranges/sequences and bounded borrowed views.
- [x] each tokenized source is prepared once and reused by reachability and compilation.
- [x] generic bodies preserve canonical ownership and explicit donor/requester provenance.
- [x] `DiagnosticToken` survives source-token storage release.
- [x] `Token`, `TokenKind`, `FileTokens`, copied parser windows and compatibility cursor paths remain
  deleted.
- [x] constant and nominal R5 fits remain within budget; the inherited generic exception is
  explicitly accepted without changing the budget.
- [x] Phase 4 remains gated by the complete reactivation gate below and the roadmap sequence.

---
## Phase 4 — Compact diagnostics, type snapshots and frozen reports

### Phase 4 reactivation gate

Phase 4 starts only when all of the following are true. Until then the plan pauses after accepted
Phase 3 and the completed post-Phase-3 tidy-up through the roadmap order: MON syntax and nested const
records, MON Rust tooling, Wiring V1, then native result slots and Core const evaluation.

- [ ] rebase `diagnostic-data-layout-changes` onto a main that contains the accepted MON syntax and
  nested const records checkpoint, then the accepted MON Rust tooling checkpoint, then the accepted
  Wiring V1 checkpoint and then the accepted native result-slot and Core const-eval checkpoint
- [ ] re-run the owning validation gate after the rebase
- [ ] fresh inventory of diagnostic producers, result boundaries, warnings, type environments and
  generated functions as they exist after those semantic checkpoints merged
- [ ] stale name refresh: update schema inventories and slice wording for any diagnostic family,
  result shape or owner renamed or moved by the rebase
- [ ] every locked architecture decision in `docs/compiler-data-layout-design.md` is preserved.
  No reactivation change may weaken a locked size, ownership or failure-lane contract

### Summary, reasoning and context

This phase replaces the complete diagnostic model in one migration phase. Temporary conversion code
may exist only inside this phase. The exit commit must have one schema, one draft accumulator, one
32-byte durable record, one type-display renderer and one immutable report boundary. One report and
one accumulator per identity domain; `DiagnosticReportSet` is the canonical-order collection. The
command/report owner is the sole warning-storage owner in this phase; `CompilationOutcome::Success`
as the final warning owner is installed by Phase 5 alone, and this phase must not introduce that
result model early.

### Slice 4A — Declare the complete diagnostic schema with optional stable reason keys

- [ ] inventory every current stable diagnostic family from the fresh reactivation inventory
- [ ] assign explicit non-zero internal `DiagnosticCode` values; never derive them from enum order
- [ ] preserve each external `MOTH-*` code, category, title and default severity
- [ ] make the schema's stable `reason_key` contract optional; preserve every existing reason key
  exactly and independently of rendered wording. Diagnostics without a reason-key contract remain
  `None`; do not manufacture a reason key merely because the schema can store one. Validate
  uniqueness and well-formedness for every defined key (R10a)
- [ ] declare fact-word meaning, optional codecs, allowed extra kind, secondary-label roles,
  type-rewrite markers and renderer entry once per diagnostic
- [ ] replace compiler-generated prose stored in payload/label string fields with typed
  reason/message codes and facts; an authored string may remain a `StringId`, but `RenderedText`
  is not a generic escape hatch
- [ ] use one small `macro_rules!` vocabulary and const tables
- [ ] keep one registry module and split domain declaration files once the registry would exceed
  the repository's practical file-size guidance; each family still has exactly one declaration
- [ ] generate/validate descriptor lookup, all-code iteration, typed draft constructors, durable
  accessors and schema tests
- [ ] prohibit raw fact indexing outside diagnostic storage modules

### Slice 4B — Define distinct draft/durable fact types and implement compact records

- [ ] define distinct draft and durable fact/core types so a pre-compaction stage-local `TypeId`
  can never be assigned where a durable `DiagnosticRecord` fact is required; the draft type set is
  not implicitly interchangeable with the durable fact set (R10b)
- [ ] implement `DiagnosticRecord`, `DiagnosticDraft`, `DiagnosticId`, `DiagnosticExtraId` and
  hard layout assertions
- [ ] implement packed code/severity flags with reserved-bit validation
- [ ] implement fixed `DiagnosticExtraRecord`, typed list ranges and typed arenas
- [ ] implement 12-byte secondary labels; primary span exists only in the record and every
  compiler-owned label phrase uses a compact `LabelMessageCode` plus typed immediate/side data
- [ ] implement 4-byte `DiagnosticPlace` and checked optional/packed ID codecs
- [ ] implement the final typed cold-store encoding for rare foreign-domain primary/secondary
  sites, replacing the transitional `FrozenIdentityHandle` without widening 32-byte records or
  12-byte labels; carry project/package/generated-generic regression coverage for every mixed-domain
  site class
- [ ] implement deterministic capacity-exhaustion diagnostics without recursive extra allocation
- [ ] make common drafts allocate nothing and rare drafts allocate at most one root auxiliary
  object
- [ ] remove broad `Clone` from drafts, bags and owning stores

### Slice 4C — Build the type-display snapshot representation first

No later preparation or compaction slice may depend on an undefined type-display representation;
this slice defines it before 4D.

- [ ] **4C1 — shared display/query view:** inventory every renderer query against
  `TypeEnvironment`, define the smallest frozen query surface beyond spelling and extract one
  read-only view from `datatypes/display.rs` so live and frozen types share one formatter
- [ ] **4C2 — compact store:** implement `TypeDisplayId`, fixed records and typed child ranges for
  every currently rendered builtin, nominal, choice, generic, function, option, collection, map,
  tuple/multi-value, fallible and external shape
- [ ] **4C3 — snapshot algorithm:** while the producing `TypeEnvironment` is live, collect
  schema-marked `TypeId`s, reserve before recursion, copy only transitive display/query facts and
  rewrite draft words to batch-local `TypeDisplayId`s before the environment can be released
- [ ] **4C4 — type-lifetime proof:** prove the snapshot algorithm captures every schema-marked `TypeId`
  while its `TypeEnvironment` is live; leave prepared-batch stage integration to 4D/4E, after its
  contract exists
- [ ] **4C5 — equivalence and dedup policy:** add exhaustive live/snapshot formatting and
  query-equivalence tests; a valid frozen report never falls back to a raw internal ID.
  `TypeId` -> `TypeDisplayId` memoization is mandatory; cross-identity structural deduplication is
  benchmark-gated and stays off until measured evidence justifies it (R10d)

### Slice 4D — Define the one draft, preparation and compaction path

Prepares batches over the representation 4C already defined.

- [ ] make `DiagnosticBag` a move-only `Vec<DiagnosticDraft>` accumulator, one per identity domain
- [ ] let each `SourcePreparationDelta` carry its local `DiagnosticBag`; at the existing
  file/chunk merge, merge strings then paths and schema-remap those drafts into the module identity
  domain
- [ ] define one internal `PreparedDiagnosticBatch` only for a module-local identity/type domain
  crossing the module/build merge boundary
- [ ] integrate AST/HIR/target stage boundaries only after `PreparedDiagnosticBatch` exists; no live
  `TypeEnvironment` is dropped before its batch is captured, while successful semantic owners retain
  the environment only for real backend work
- [ ] keep prepared batches as drafts plus compact type-display draft data from the 4C
  representation; they are not another durable report/store API
- [ ] implement `DiagnosticBag::freeze`/equivalent as the one final draft-to-dense-store
  transition after canonical remapping and after the actual outcome's last diagnostic producer
- [ ] avoid a separate public `DiagnosticStoreBuilder` layer
- [ ] compact labels, lists, token projections and extras into dense arenas
- [ ] preserve deterministic production order
- [ ] expose typed immutable `DiagnosticView` and iterators; renderers never mutate records
- [ ] compute error/warning/note counts during freeze and expose allocation-free severity-bucket
  iteration without building a reordered index vector
- [ ] keep `DiagnosticReport` and `DiagnosticReportSet` move-only; a long-lived host wraps the
  whole set/report in `Arc` only when multiple host consumers genuinely share it, never per record
  or per side store

### Slice group 4E — Consolidate file, module and command diagnostic aggregation

- [ ] **4E1 — file/module domains:** let `SourcePreparationDelta` carry file-domain drafts; at file/chunk merge, merge strings then paths and schema-remap drafts into the module domain
- [ ] **4E2 — prepared module batch:** add `PreparedDiagnosticBatch` beside `CompiledModuleResult`, never inside `Module`; capture drafts plus the small module-local type-display store before a diagnosed module drops its environment
- [ ] **4E3 — canonical module merge:** use existing module result sorting as the only cross-module diagnostic order; merge module strings then paths, remap drafts/labels/type records, merge batch-local `TypeDisplayId`s and append into command-owned builders without freezing early
- [ ] **4E4 — warnings and success artifacts:** move successful warnings out of `Module` and
  backend `Project` payloads, remove warning-specific clones/remaps and preserve success-only
  linkable/artifact payloads. The command/report owner is the sole warning-storage owner in this
  phase: all warnings remain command-owned drafts in that owner's stores. Installing them into
  `CompilationOutcome::Success::warnings` is Phase 5 work; Phase 4 must not introduce that result
  model early
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

- [ ] every user diagnostic uses the schema and `DiagnosticDraft`, with one report/accumulator per
  identity domain and `DiagnosticReportSet` for canonical ordering
- [ ] every durable record is exactly 32 bytes; no raw ID concatenation replaces the
  `DiagnosticReportSet` canonical ordering
- [ ] prepared module batches contain no full type environment and no second durable diagnostic
  model
- [ ] `DiagnosticReport`/`DiagnosticReportSet` are the only diagnosed/warning render boundaries
  and are frozen only at the actual outcome's last diagnostic producer
- [ ] type-display snapshot representation was defined (4C) before any preparation/compaction slice
  (4D) consumed it
- [ ] the command/report owner is the sole warning-storage owner; `CompilationOutcome::Success` was
  not introduced early
- [ ] full type environments and mutable reverse-lookup tables are not retained solely for
  rendering
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

- [ ] **5C0 — verify the canonical context contract:** before implementing any batch, reconcile
  the published `InfrastructureFailure` example in `docs/compiler-data-layout-design.md` against
  the fresh Phase 5 inventory. It must encode one typed identity-vs-filesystem context: every
  retained `SourceSpan`/`PathId`/`StringId` has its matching frozen identity context, while a
  pre-source filesystem failure carries ordinary filesystem context and no compact ID. Modify the
  canonical example only if the inventory exposes a missing context class; record the accepted
  R10c context ownership.
- [ ] **5C1 — data and rendering:** compact typed kind/stage, exactly one `InfrastructureContext`
  value (identity context with optional exact source span, or filesystem context with owned
  path/tool facts), deterministic typed detail slice and terminal/terse/dev-server renderers
- [ ] **5C2 — filesystem/build:** source open/read, canonicalization, output writing, manifests,
  permissions and other expected operating-system failures
- [ ] **5C3 — providers/backends:** tool invocation, provider loading, backend emission/validation
  IO and other recoverable external-system failures
- [ ] **5C4 — tooling hosts:** watcher, socket/server, routing and dev-server operational failures
- [ ] replace string-keyed metadata `HashMap`s with typed details in the owning batch
- [ ] preserve useful underlying IO/tool information without pre-rendering all presentation text
- [ ] when a failure carries `SourceSpan`/`PathId`/`StringId`, make the host-facing failure share
  the matching frozen identity context; pre-source failures use owned filesystem context instead
- [ ] never let a compact ID escape detached from its context and never deep-clone that context for
  one failure
- [ ] never convert infrastructure data into a `DiagnosticRecord`

### Slice group 5D — Implement structured compiler bugs

- [ ] **5D1 — bug payload:** define `CompilerBugReport` with stage, Rust caller, version/commit, invariant message and optional self-contained resolved source facts consisting of logical display path, byte/line/column range and a bounded excerpt; it never retains a worker context or whole source; add `#[track_caller]` `compiler_bug!` that raises the typed payload through `panic_any` rather than a formatted string panic
- [ ] **5D2 — compact storage invariants:** source/path/token/diagnostic IDs, ranges, reserved bits and store bounds that are already proven by validated construction
- [ ] **5D3 — AST/TIR/HIR invariants:** impossible validated semantic/IR states and missing stage authority
- [ ] **5D4 — borrow/link/backend invariants:** impossible analysis, call-target, capability and backend metadata states
- [ ] route every intentional production compiler-invariant panic through `compiler_bug!`; debug assertions may supplement but never replace release validation
- [ ] require every compiler-bug report that carries a source span to resolve its bounded
  source/display facts (logical display path, byte/line/column range, bounded excerpt) before the
  worker's compiler state is discarded; resolution happens before the discard, never after unwind
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
- [ ] install `CompilationOutcome::Success::warnings` here — Phase 5 alone introduces that result
  model — moving the command/report owner's warning store into the success outcome; a diagnosed or
  infrastructure-failed outcome does not carry unrelated prior warnings, and any source context
  essential to an error belongs in that diagnostic's labels/facts; use a one-element report set for
  the common single-context case
- [ ] update current warning-prepend helpers and tests to this accepted boundary rather than
  carrying mixed-lane production-order streams forward
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

### Slice 6F — Prove the frozen data and owned worker boundary are host-ready

Narrow scope: this slice proves two things and builds nothing speculative. It does not expose
future LSP APIs, frameworks or cancellation models.

- [ ] prove frozen source/report data can outlive a worker: frozen identity contexts, report
  stores and retained facts remain valid and renderable after the worker that produced them is
  dropped
- [ ] prove the existing owned worker boundary is ready for another long-lived host: a fresh owned
  worker per build, coherent freeze before outcomes return, complete failed-worker discard, and a
  documented request/shape a second host could adopt without changing the compiler
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
- [ ] confirm no `#[allow(clippy::result_large_err)]` allowance or boxed-common-diagnostic
  workaround has been reintroduced
- [ ] run ordinary dead-code/unused checks through Clippy

### Slice 7D — Converge authority and policy documentation

- [ ] finalize `docs/compiler-data-layout-design.md` with selected constants and final file map
- [ ] update compiler and build-system authorities to the implemented ownership/failure model
- [ ] update `AGENTS.md`, style, testing and validation guidance
- [ ] update `index.md` and module-level docs/comments
- [ ] review language and memory authorities; change only stale implementation references
- [ ] rebuild generated documentation and inspect changed routes/diff

### Slice 7E — Close roadmap ownership and resume diagnostics work

- [ ] delete this plan and its active roadmap entry in the same completion commit, with no
  intermediate "plan complete" commit; durable deferred work moves to the roadmap and the
  architecture authority without links back to this deletable plan
- [ ] move rejected/postponed items to the roadmap with the deferral table below, named as
  capabilities in the roadmap's deferred-design area, not linked to this plan
- [ ] remove duplicate deferred bullets owned by another plan
- [ ] update the existing Structured diagnostics matrix row to the implemented report/failure
  contract
- [ ] unpause the user-facing diagnostics improvement work immediately after this plan, preserving
  its final continuation at its recorded Phase 4.1c checkpoint
- [ ] refresh its paths, capsule and next semantic slice against the schema/store APIs
- [ ] remove old payload, label, token and type-context assumptions from that plan
- [ ] require future diagnostics to fit the 32-byte schema and side-store policy

### Slice 7F — Final correctness, performance and determinism gate

- [ ] run every layout, property, schema, range, type-render and failure-worker test
- [ ] run all Rust tests and the complete canonical integration suite/audit
- [ ] confirm stable diagnostic codes, source spans and renderer identity across terminal/terse/dev server
- [ ] confirm successful artifacts/goldens are unchanged except explicitly authorized output
- [ ] run docs check and release build
- [ ] run native/Linux/Windows Clippy with warnings denied and confirm no
  `#[allow(clippy::result_large_err)]` allowance or boxed-common-diagnostic workaround has been
  reintroduced
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
Add an item when this plan activates or when an experiment is rejected, name the capability and its
re-entry criteria in the roadmap (do not link back to this deletable plan), and remove overlapping
bullets from other plans. Do not add these implementation optimisations to the progress matrix
unless they change current tooling or user-visible support.

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
