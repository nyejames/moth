# Moth Compiler Source, Token and Diagnostic Data Layout Implementation Plan

> **Repository path:**
> `docs/roadmap/plans/compiler-source-token-and-diagnostic-data-layout-plan.md`
>
> **Architecture authority:**
> `docs/compiler-data-layout-design.md`
>
> **Status:**
> Active. Phase 1 is in progress. The current continuation reviews the existing implementation
> and completes Phase 1, then pauses for external user review before Phase 2 begins.
> Resumed on 2026-09-08 during 1D5c1. Phases 2–7 remain pending.
> Test Suite Hardening was delivered in `03168082d`. Baseline evidence lives in
> `benchmarks/frontend-optimization-results.md`.

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
- Phase: 1E1/1E2/1F4, downstream span consumers and render boundary.
- Goal: migrate downstream header/order and AST consumers onto the exact preparation spans while
  retaining the interval bridge for untouched consumers, and make renderers resolve retained
  primary spans through the source snapshot. Dependency-clause and remaining symbol consumers
  remain open; the accepted renderer slice is the first 1F4 checkpoint.
- Keep source ownership explicit across dependency and symbol records; do not add a second source
  table or convert legacy locations into guessed spans.
- Non-goals: resolved semantic spans, new source owners, diagnostic
  storage redesign and token-store migration. The private location bridge ends at 1H.

LAST_GOOD_COMMIT:
- `70a7ce035` — AST signature-member and choice-variant diagnostics retain exact source-owned
  anchors as primary or related spans, and renderer checkpoint `616d9a864` resolves retained
  primary spans through `LineIndex` for terminal, terse and dev-server output. Focused AST,
  renderer, dependency-selection and module-dependency tests pass; the worktree is clean.

CURRENT_WORKTREE_STATE:
- Branch: `token-and-diagnostic-data-layout-changes`, explicitly selected by the user.
- Continuation order: remaining 1E, 1F, 1G and 1H, Phase 1 closeout and final review,
  then pause for external review. Phases 2–7 stay pending until the user authorizes continuation.
- 1D3 (`b1d5a4005`), 1D5a (`0f92205c6`) and 1D5b (`86d4bf508`) are committed and accepted.
- 1D5c1 is accepted in `835253c32`; 1D5c2 is accepted in `c79763fec`. The worktree was clean
  after checkpoint verification. 1D5c3 is implemented, validated and independently reviewed in
  `b1e7dcb15`. Its worktree was clean after verification. 1D5c4 is implemented, validated and
  independently reviewed and committed in `8142352cd`. The worktree was clean after verification.
  Scripted workers now use the implementation-coordinator launcher at the user's request;
  plan-orchestrator still owns the workflow. Parsed generic parameters and bounds
  preserve original anchors. Generic declaration maps retain only kinds. Canonical registered
  parameters own semantic names and arity across local, imported and materialised nominals.
- Accepted 1D5c1: `SignatureMemberSyntax`, `FunctionReturnSyntax` and `ChoiceVariantSyntax`
  copy existing token-local spans by direct indexing. `ReturnSlotSyntax.location` and its now
  redundant remap/rebind forwarding implementation are removed. Trait `This` substitution copies
  the nested return span. Focused fixtures cover long anchors, remapping and authored substitution.
- Accepted 1D5c5 is committed in `6afa29848`. Header names retain exact local anchors; const
  fragments retain explicit final-source `SourceSpan` ownership and join their first-interior
  through post-close bounds through the original live builder; source contracts retain the
  qualifier `#` anchor. Focused coverage includes long multibyte/extended ranges, infrastructure-
  lane selection failure, related labels and joined dependency-clause ends. The scripted
  independent audit could not run because its providers failed before inspection; no files changed
  during those attempts.
- Accepted 1D6 is committed in `cefb61634`. The test-only `source::test_support::TestSourceContext`
  owns an explicit source identity, interned path, string table and live extended-span builder;
  focused header span regressions now share it, and its own test covers a long live-table span.
- Accepted 1E1a is committed in `3f40b6b48`. Stage 0's directory file-reference resolver consumes
  the retained `PreparedFileReference` local span for every user diagnostic, preserving the legacy
  location and label order while removing the field-level dead-code bridge. The focused missing
  resource regression proves the diagnostic span resolves to the authored path bytes in the
  retained source snapshot. Dependency-clause, module-symbol and sorted-header consumers remain
  the next 1E1 batches.
- Accepted the first sorted-header consumer in `73ee265df`. Stage 3 dependency edges retain each
  header's exact `name_span` with its owning `SourceId`; missing-import and circular-dependency
  diagnostics publish that `SourceSpan` while preserving their legacy primary locations and
  ordering. The focused module-dependency suite (30 tests) passes, and infrastructure errors keep
  their existing location lane.
- Accepted dependency-selection spans in `1690f8bd0`. Provider-surface diagnostics now publish
  the selection token's exact `SourceSpan` with its dependency-owned source ID while preserving
  the legacy primary location. The focused namespace-binding regression passes.
- Accepted renderer span consumption in `616d9a864`. Terminal, terse and dev-server renderers
  resolve retained primary spans against the attached source database, derive scalar columns from
  `LineIndex`, and keep legacy-location fallback for diagnostics without a retained span. Unicode
  and half-open multibyte caret regressions pass.
- Accepted AST anchor consumption in `70a7ce035`. Signature-member diagnostics retain the exact
  member anchor as a primary or secondary span, choice payload diagnostics retain the variant
  anchor as a related span, and struct remapping preserves captured primary spans. The two new
  anchor regressions and all 21 type-resolution tests pass.
- Accepted 1D5c2 adds four field-level allowances for the remaining 1E AST consumers:
  trait declaration, requirement, reference and conformance-target spans. 1D5c4 removes the trait
  declaration allowance because synthetic `This` now consumes that span. Remove the remaining
  three in 1E and confirm none survives 1H.
- Two 1D5c1 field-level dead-code allowances name 1E AST consumers: `SignatureMemberSyntax.span`
  and `ChoiceVariantSyntax.span`. Remove them in the owning 1E batch and confirm none survives 1H.
- The user identified `librust_out.rmeta` as an earlier-session Rust metadata artefact;
  it was inspected and removed at their request before this continuation.
- One unrelated `packages-work` worktree and one pre-existing stash remain untouched.

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
- `src/compiler_frontend/pipeline.rs::CompilerFrontend`: stage facade with remaining immutable-service copies
- `src/compiler_frontend/source/`: build registration, loaded snapshots, exact spans and line indexes
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
- Config preparation and AST success warnings still lack a complete build-boundary handoff.
  Preserve them in 1F2; the source ownership checkpoint does not claim to repair that warning loss.
- Repeated canonical/check-only preparation now shares the original builder. Removing the repeated
  syntax preparation itself remains 3E work; later span-producing stages must use the retained owner.

VALIDATION_STATE:
- 1E1a candidate: `cargo fmt --all` and `git diff --check` passed. Focused directory Stage 0
  tests (7) and file-reference tests (25) passed. `cargo check -p moth` passed in the delegated
  worker. The independent scripted audit was attempted but unavailable because the configured
  auditor provider's HTTPS fallback reported `invalid peer certificate: UnknownIssuer`; no audit
  edits occurred. Parent Slice review found no required correction.
- 1E1 sorted-header candidate: `cargo fmt --all`, `git diff --check`, and the focused
  `cargo test -p moth --lib module_dependencies -- --nocapture` suite (30 tests) passed. The
  diagnostic regression checks exact `SourceSpan` ownership and preserved `SourceLocation`; no
  audit worker was available after the coordinator provider block, so parent Slice review is the
  acceptance review for this bounded change.
- 1E1 dependency-selection candidate: `cargo fmt --all`, `git diff --check`, `cargo check -p moth`
  and the focused `missing_provider_record_fails_deterministically` test passed.
- 1F4 renderer candidate: `cargo fmt --all`, `git diff --check` and the focused renderer suite
  (13 tests) passed. The exact-span path uses retained snapshots and scalar `LineIndex` columns;
  legacy locations remain a fallback for older diagnostics.
- 1E2 AST candidate: `cargo fmt --all`, `git diff --check`, `cargo check -p moth` and all 21
  `compiler_frontend::ast::type_resolution_tests` passed, including the two new anchor tests.
- 1D5c4 candidate: `cargo fmt --all && just validate` passed native featured all-target Clippy,
  5,076 compiler tests, 17 CLI tests, 825 xtask tests, 1,951 integrations, docs checking,
  source audit, 82 benchmark preflights, scaling and timer erasure. Focused generic Rust (183),
  trait `This` (1), type syntax (52), header remap (29), parsed remap (10), generic integrations
  (186) and trait integrations (110) passed. The nine-anchor regression resolves original extended
  rows after UTF-8 text, string remapping and nonidentity source rebinding. Synthetic `This`
  retains the parsed trait declaration's name anchor and legacy location. Independent review is
  clean with no required findings or material test gaps. An initial Clippy type-complexity failure
  in the test closure was corrected before the passing full gate.
- 1D5c3 candidate: `cargo fmt --all && just validate` passed native featured all-target Clippy,
  5,075 compiler tests, 17 CLI tests, 825 xtask tests, 1,951 integrations, docs checking,
  source audit, 82 benchmark preflights, scaling and timer erasure. Focused type syntax (52),
  parsed remap (10), shell remap (3), type resolution (19), header remap (29), substitution (1),
  synthetic adapters (2) and the 29-anchor exactness regression (1) passed.
  Independent review is clean. Its non-blocking coverage limitation is that the substitution
  test covers direct `This`, not constructed sibling types traversing recursive reconstruction.
  Composed `This` is rejected by the canonical language contract; the recursive copies were
  inspected and preserve their spans. No additional synthetic fixture is required for this slice.
- 1D5c2 candidate: `cargo fmt --all && just validate` passed native featured all-target Clippy,
  5,075 compiler tests, 17 CLI tests, 825 xtask tests, 1,951 integrations, docs checking,
  source audit, benchmark preflights, scaling and timer erasure. Focused trait (9), remap (29),
  public trait root (8), requirement (1) and header-parser (175) tests passed. The final regression
  also checks decoded byte offsets against the original locations after nonidentity source rebinding.
  Independent review is clean with no required findings or material test gaps.
- 1D5c1 final candidate: `cargo fmt --all && just validate` passed native featured all-target
  Clippy, 5,074 compiler tests, 17 CLI tests, 825 xtask tests, 1,951 integrations, docs checking,
  source audit, 82 benchmark preflights, scaling and timer erasure. Focused substitution (1),
  traits (9), shared signature (5) and long-anchor preparation (1) tests passed.
  Independent review's rebind coverage correction is implemented and focused verification is
  clean. The stale capsule is refreshed here. No required finding remains.
- 1D5b final candidate: `cargo fmt --all && just validate` passed after the anchor consolidation.
  Native featured all-target Clippy, 5,072 compiler tests, 17 CLI tests, 825 xtask tests,
  1,951 integrations, docs check, source audit, 82 benchmark preflights, scaling and timer erasure
  passed. Focused paths (60), headers (328), frozen generics (25) and config-filter (192) passed.
  Independent review's duplicate-anchor finding is resolved; fresh focused verification is clean.
- Accepted `0f92205c6`: `cargo fmt --all && just validate` passed. Native featured all-target
  Clippy, 5,070 compiler tests, 17 CLI tests, 825 xtask tests, 1,951 integration cases,
  docs check, source audit, 82 benchmark preflights, three scaling budgets and timer erasure passed.
- Focused 263 declaration tests, 41 Moth-template tests, seven Markdown tests and the updated
  real-source/extended-anchor regression passed. Featured all-target compilation passed.
- Independent 1D5a review is clean with no required finding.
- The earlier 1D3 carrier/failure-lane and aggregation reviews and focused correction verification
  were clean. No new structured-audit coverage is claimed.
- Gate hygiene: `just validate` diffs tracked files during its benchmark stage — edit only before
  it starts or after it exits. `cargo test -p moth --lib` misses test targets; use the featured
  all-target Clippy gate before accepting a slice.

DOCS_IMPACT:
- progress matrix needed: only when current diagnostic/failure/tooling behaviour changes; do not add an internal-refactor status row
- other docs stale: current authorities and style rules still describe `CompilerError`, path-backed locations and boxed large-error boundaries
- authorized docs updates: every authority, style, roadmap, plan, matrix and index edit named below
- next action: continue 1E1 with dependency-clause and module-symbol consumers, then expand 1E2–1H
  and complete the Phase 1 closeout/final review before the requested external review pause.
  Phases 2–7 remain pending.

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
Stage 0 inventories pre-register final SourceIds before preparation; traversal-only discovery normalizes private provisional IDs before publishing success or diagnosis
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

Test Suite Hardening was delivered in `03168082d`. This plan is the sole active representation
migration. The diagnostics plan remains paused until the full migration completes.

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
`PathId`/legacy-path boundary ending in Slice 2D, the transitional `CompilerMessages` identity-context
bridge ending in Slice 4I and any measured early token projection ending in Slice 3H are the only
anticipated cross-phase cases.

---

## Phase 0 — Activation, current-state audit and evidence baseline

Completed. Evidence: `benchmarks/frontend-optimization-results.md`; searchable inventories:
`target/data-layout-audit/`. Detailed execution history remains in Git.

- [x] **0A — activation:** hardening prerequisite `03168082d`, branch/worktree inventory,
  authority and owner refresh.
- [x] **0B — roadmap ownership:** activated this plan, paused diagnostics work, updated authority
  routing and built documentation. No support change required a matrix edit.
- [x] **0C — migration inventory:** source/location, path, token, diagnostic, context-copy and
  failure/recovery owners mapped to phases.
- [x] **0D — baseline:** layouts, corpus source sizes and representative success/warning/failure
  workloads recorded. Added `data_layout` benchmark support using existing machinery.
  Instrumentation has no normal-build cost; the throwaway layout probe was removed.
- [x] **0E — validation/evidence:** Rust 1.97.1 native/Linux/Windows Clippy, `just validate`,
  frontend/CLI checks and five-run predecessor evidence passed.
- [x] **Phase close:** ownership/style review and exit criteria accepted without semantic changes.

Limits: memory evidence covers bounded capacities and aggregate live/peak allocation proxies,
not a complete heap partition, nested heaps or path-only remap counts. No profiles were needed.
Exact span histograms required byte offsets and were delivered in 1C2, not Phase 0.

---

## Phase 1 — Build-lifetime source identity and exact compact spans

### Summary, reasoning and context

Source locations are the most pervasive representation problem. This phase moves source ownership to
Stage 0, introduces exact packed byte spans and migrates every compiler stage and renderer. It also
removes duplicated location data and existing boxed-diagnostic workarounds so the full CI gate becomes
green before later layout work proceeds.

### Slice 1A — Add the final path foundation required by source records

- [x] introduce `PathId(NonZeroU32)` and a dense parent/component path table
- [x] intern source logical paths into the database-owned build base before string/path forks
- [x] use `PathId` in compiler source registration slots immediately; reuse the existing
  interner in `SourceDatabase` and remove the Stage 0 path table and fields retained only
  for its test-only entry-root-relative lookup
- [x] keep filesystem `PathBuf`/`Box<Path>` separate from compiler logical identity
- [x] add layout, root, parent, append, equality and rendering tests
- [x] defer the full compiler `InternedPath` migration to Phase 2

`SourceLogicalIdentity` keeps its owned portable spelling as the `SourceId` sort key.
Canonical source order is established before path interning. Numeric `PathId` allocation order
must never be used to sort source candidates.

Source paths preserve exact `Path::components()` semantics and strict UTF-8 validation, matching
their predecessor. Portable semantic spellings preserve their component separators instead.
The frozen table owns nodes and depths only; the child map lives and dies with the builder.

### Slice group 1B — Replace per-module source tables with build-lifetime registration

- [x] **1B1 — registration index and ID domain:** one compiler-facing registration index from
  existing Stage 0 inventories, `SourceId(NonZeroU32)` and compilation-root ID 1.
- [x] **1B2 — registration barriers:** config and project share one domain; packages retain their
  own. Canonical inventories register before preparation, independent of completion order.
- [x] **1B3 — traversal and synthetic sources:** directory/packages pre-register inventory IDs.
  Synthetic single-file and recursive direct-template traversal normalize private provisional
  identities once before publication. Authored provenance and deterministic late deltas remain.
- [ ] **1B4 — source slots and loading:** move each loaded text allocation into its preassigned slot with no second full copy; enforce the monotonic registered → loaded → finalized lifecycle; represent registered-but-unloaded candidates with a compact slot/index rather than allocating empty full records; keep loaded records dense behind a `SourceId` slot map; deduplicate canonical physical sources and reject conflicting logical identity, kind or a second different snapshot
- [x] **1B5 — module inputs and worker ownership:** ordered candidate IDs, canonical file/chunk merge checks, per-source deltas, original live builder retention and final table installation are delivered. Move-only diagnostic bags and the complete frozen identity context remain 1F work.
- [x] **1B6 — remove per-module service copies:** absorb `SourceFileTable`, `FileId`, `FrontendSourceFileIdentity` and `attach_source_files`; make `CompilerFrontend` and header-parse options borrow immutable source registration, style directives, path resolver and external registries. The facade and module context now borrow their immutable services. Token and prepared-output canonical-path copies remain assigned to 3D/3E1.
- [x] **1B7 — failures and tests:** preserve typed source-size, UTF-8 path and source-registration failures in their correct lanes; add config-to-project, direct-service, serial/parallel ID, slot, deduplication and source-order determinism tests

#### Delivered registration contracts and remaining ownership

The source database owns one registration slot per candidate, dense loaded snapshots and a cold
load-failure array. `SourceId(1)` is the compilation root. Config is registered before tokenization
and remains in the project identity domain; independently compiled packages keep separate domains.
Stage 0's `SourceRegistrationIndex` preserves its module-origin order; traversal-only single-file
and direct-template lanes use canonical logical-path order because they have no module inventory,
and their provisional identities stay in a private, disposable discovery-local domain, rebound
exactly once before success or diagnosed publication. A flat display-path sort must not replace
either lane's canonical ordering policy.
Loaded text moves into its slot once; a second retain is an invariant failure. Re-registration of
one canonical source rejects conflicting logical identity or supplied kind. Different physical
sources may share a display path (bootstrap `config.moth` vs `src/config.moth`); display-path
equality is not source identity. `SourceKind` records the producer's authored classification, not
the canonical target's extension or the builder's support policy; provider-owned slots carry
identity but acquire no compiler snapshots merely by registration. Module inputs carry ordered
candidate `SourceId` sets whose external-import scope is the module's owned candidates, not every
source in the boundary database. File/chunk merges place prepared results in preassigned slots and
reject duplicate, missing and out-of-range outputs.
`SourceDatabaseBuilder` now separates live span ownership from immutable lookup services.
Private AST handles share the same allocation only during producer calls; finalization regains
exclusive access and returns that same `Arc` after installing the tables. Package lookup publication
waits for check-only producers. This is the source half of the lifecycle; 1F still owns the complete
lookup-only identity/render boundary and its string/path context.
The source path foundation is delivered: `SourceDatabase` owns the existing path interner as its
one source identity base; Stage 0's test-only path table is gone with its stable logical ordering
and authored classification unchanged. The source `PathId`/legacy-path bridge ends at 2D and
reconstructs transient components from table nodes, never rendered text. Current bridge callers:
source discovery/rebinding, frontend identity and AST entry setup, content dependency targets, AST
file-value scope, direct-template entry setup, HTML template bundle rebinding/owner diagnostics,
the source-size error path; test callers: source invariants, frontend pipeline, module
dependencies, template heads, source snapshot rendering, Stage 0 preparation fixtures. No new
consumer may adopt this migration bridge.
`ModuleSymbols` no longer copies canonical OS paths. `FileTokens.canonical_os_path` and
`FileFrontendPrepareOutput.canonical_os_path` remain for 3D and 3E1; their agreement checks stay
until the duplicated fields are removed. Renderers already use retained snapshots rather than
reopening files; until 1F migrates them to source identity, per-diagnostic-range source contexts
preserve package ownership and ambiguous display-path matches omit a frame rather than select the
wrong file. The direct-template API retains its finalized per-document source context with
warnings and diagnosed outcomes.
1B4 remains open for the final mutable/frozen lifecycle and selected-source loading policy;
1B6 is delivered (facade and module semantic context borrow immutable services). Individual
mutation-test results and delivery history remain in Git.

### Slice group 1C — Implement `LocalSpan`, line indexes and exact resolution

Delivered contracts (evidence: layout authority and benchmark report):

- [x] **1C1 — byte cursor and line index:** one line-index builder and byte-offset cursor
  threaded through each source kind's existing traversal; no second pre-scan where a traversal
  already existed.
- [x] **1C2 — span census and encoding selection:** exact span start/length histograms with
  boundary buckets over the 8–12 length-bit splits on the weighted corpus; benchmark-only
  candidate codecs; constants frozen in the architecture document and evidence report. Selected
  codec: the measured 22/10 split (`LENGTH_BITS = 10`) with exact append-only overflow rows. The
  Phase 0 source-size census proved every candidate start-overflow-free, so length overflow alone
  decided the split. **Amended at delivery:** the bounded terminator experiment was deferred
  undone, not evaluated or rejected — its measured maximum prize is under 2 KB; re-entry criteria
  are recorded.
- [x] **1C3 — exact span codec:** `LocalSpan(NonZeroU32)`, one append-only `ExtendedSpanBuilder`
  per source, one private source-local factory/codec for exact construction, join, insertion
  point and resolution; one read-only resolver serves live builders and frozen records;
  cross-source joins rejected; named source-order, overlap and containment operations.
  **Frozen-record half deferred to 1D** and delivered there as 1D1.
- [x] **1C4 — conversion semantics:** CRLF, empty-file, final-newline, long-line and zero-width
  EOF behaviour; lazy line, Unicode-scalar and UTF-16 column conversion. **Widened at delivery:**
  chose the tokenizer's line-break set (LF, CRLF, bare CR) and made `TokenStream::next` the
  single owner of the authored line counter.
- [x] **1C5 — invariants:** hard layout assertions plus boundary, malformed-capacity, Unicode
  and conversion property tests. **Narrowed at delivery:** join/ordering coverage already existed
  in 1C3's named-operation tests.
- [x] **1C6 — registration slot and loaded record:** compact registration slot per candidate plus
  a loaded record owning text, line starts and extended spans unconditionally; unreadable-source
  failure stays at the slot layer; `Unreadable` variant removed from the record. Moved here from
  1B4: the split pays once the three loaded boxes exist.

Standing facts: `LocalSpan`/option are four bytes; `SourceSpan`/option are eight. The line index
builds once per snapshot; empty snapshots have no lines; EOF after a final terminator resolves to
the preceding visible line end. Unicode scalar columns serve rendering, UTF-16 columns are
reserved for tooling (one narrowly documented allowance until that consumer exists); the byte
cursor stays authoritative while legacy line/column fields survive. Empty spans overlap nothing;
containment is the operation for insertion points. 1F4 must consume `LineIndex` in both
renderers, remove its module-wide dead-code allowance and cover code-token carets plus
re-anchored tokens such as a discarded template body's closing bracket — old `CharPosition`
columns or lines can be stale. Frozen-record span APIs exist, but production installation and
builder retention remain 1D/1F work. Codec operations may keep only narrow allowances naming an
actual remaining consumer, not blanket suppressions.


### Slice 1D — Migrate tokenization and source preparation

- [x] make tokenization emit `LocalSpan` and source-scoped diagnostics emit `SourceSpan`
  — **tokens delivered in 1D2a; preparation diagnostics delivered in 1D3**
- [ ] keep every authored token stream and header/source identity keyed by its registered
  `SourceId`; frozen generic syntax remains explicitly identity-free until 1F preserves or remaps
  its owning context. The compilation root must never stand in for that missing identity.
- [ ] finalize line starts and immutable token preparation at file-preparation completion, but keep the source-local extended-span builder mutable until the final span-producing stage
- [x] **delivered as 1D1** — `SourceRecord` gained its extended-span table plus the authority's
  `&SourceRecord`/`&SourceDatabase` span signatures; 1D2a retired 1C3's module-wide
  `allow(dead_code)`/`allow(unused_imports)`, leaving item-level allowances on the consumer half,
  each naming its first caller's slice.
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

#### 1D sub-slices and the interval bridge

1D is split at source ownership boundaries:

- **1D1 — frozen record spans:** delivered codec and one-shot install API; 1D4b supplies production
  installation under the exclusive owner after every current span producer.
- **1D2 — tokenizer spans and identity:** delivered exact local spans and registered inputs for
  authored tokenization. The 1D4 ownership cutover makes tokenization borrow the source's original
  builder; frozen generic materialisation still needs its owning context in 1F.
- **1D4 — builder lifetime and source preparation delta:** delivered across successful and diagnosed
  file/chunk aggregation, semantic calls and module/direct-service outcomes. Identity-rebind errors
  also retain every known original table before finalization. Later migrations must thread this owner
  into any new joined/insertion-span producer, rather than start an independent builder.
  - [x] **1D4a — diagnosed producer ownership:** delivered; returns the existing span builder and
    real source identity on tokenizer and file-header failures, retaining warnings and diagnostic
    facts. Closed producer-local drops only, not later aggregation.
  - [x] **1D4b — aggregation and outcome ownership:** original builders stay outside diagnostic
    bags and survive preparation, aggregation and semantic outcomes. The exclusive source owner
    installs each table once and retains final render context on terminal errors.
  - [x] **1D4b prerequisite — direct-template entry reuse:** delivered; preserves the entry's
    original preparation and builder through final identity rebinding and bundle consumption; the
    compiler service prepares only standalone raw-source inputs, not bundle entries. Brings
    forward the direct-entry portion of 3E without claiming its source-owned token storage
    migration.
  - [x] **1D4b direct-template finalization:** delivered; preserves builders and snapshots through
    bundle diagnosis or compiler folding, then retains finalized per-document contexts with
    warnings.
  - [x] **1D4b config finalization:** delivered; retains the original builder through compiler and
    build-owned config validation, then installs it once before source database sharing.
  - [x] **1D4b synthetic discovery finalization:** retain prior and failed snapshots/builders
    through the borrowed traversal, normalize the known set on abort and publish final context.
- **1D3 — preparation diagnostics carry source spans:** tokenization and preparation diagnostics
  retain exact final `SourceId` plus local span data owned by the same producer.
  - [x] **1D3a — lexical and per-file primary spans:** encode exact producer-owned byte bounds
    before the original builder leaves preparation; normalize the carrier at discovery publication.
  - [x] **1D3b — aggregation and related ranges:** capture source-contract/symbol aggregation
    diagnostics and related preparation labels with explicit source ownership. Per-file primary
    capture alone does not complete 1D3.
- **1D5 — preparation records onto spans:** headers, dependency clauses and aliases, declaration
  shells, source contracts, const fragments and source-kind adapters carry spans.
  - [x] **1D5a — declaration anchors and initializer terminators:** copy the existing exact
    token anchor into shared declaration/binding-target shells, preserve synthetic source-start
    anchors under their real source identity and delete `Token::terminator_at`.
  - [x] **1D5b — path and dependency records:** source-local path rows, provider/selection/alias
    anchors and structural file references. Reuse the minted path token's encoded span and preserve
    source ownership, remapping and wrong-table checks. The mandatory provider owns the clause's
    path-token anchor; remove the outer clause's duplicated location/span. Four field-level
    dead-code allowances name their planned 1E consumers: dependency alias, selection, provider
    and structural file-reference
    spans. Remove each allowance in its owning 1E batch and confirm none survives 1H.
  - [ ] **1D5c — remaining preparation records:** header names and joined const fragments,
    source contracts and signature shells. Preserve each record's existing range meaning;
    runtime fragments already use retained tokens and need no new record.
    - [x] **1D5c1 — signatures and choices:** copy member, return-type and variant token anchors;
      remove `ReturnSlotSyntax.location`, which duplicates its nested return value's location.
      Preserve anchors through remapping and trait `This` substitution.
    - [x] **1D5c2 — trait shells:** declaration, requirement, reference and conformance anchors;
      consolidate duplicate name locations without changing synthesized semantic names.
    - [x] **1D5c3 — parsed types and capacities:** preserve each type constructor's current token
      anchor and all synthetic/materialized constructors. Resolved types remain 1E2.
    - [x] **1D5c4 — generic parameters and bounds:** preserve authored anchors and explicit
      synthetic metadata without manufacturing source identities. Semantic registration consumes
      ordered parameter IDs and names. Canonical type parameters replace duplicated parsed metadata.
    - [x] **1D5c5 — header names, const fragments and source contracts:** thread the original
      builder only where a joined range needs encoding. Keep const-fragment source ownership
      explicit after module aggregation and preserve the infrastructure failure lane. Source contracts
      retain the qualifier `#` anchor already copied into `DeclarationSyntax.span`, not the header
      name anchor. Const-template joins preserve the existing first-interior-token through post-close-token
      bounds by joining those original token spans through the original live builder.
- [x] **1D6 — test source context:** one owning-module test-only `TestSourceContext`, replacing the
  repeated ad hoc path/location constructors in Rust tests.

**Interval bridge.** A token cannot switch representation and migrate its consumers in the same
slice: the tokenizer has hundreds of downstream location readers, and 1E exists precisely to move
them. So from 1D2 until 1H a token carries both its exact span and the legacy `SourceLocation`. The
span is authoritative for tokenizer-minted tokens, and the legacy range is derived from it.
1D5a replaces the former `Token::terminator_at` exception with the declaration shell's exact
local anchor and deletes that constructor. The anchor keeps its existing token-sized meaning;
it is not widened to the whole declaration. 1H deletes the legacy field after consumers migrate.

**1D3 prerequisite.** The 1D4 ownership cutover keeps original builders outside preparation results,
retains them before fallible aggregation and defers installation until all producers have ended.
Identity-rebind failures also finalize the known original tables before publication. Preparation
diagnostics can use the live source resolver; later parsers must keep that same source owner when
they begin producing joined or insertion spans. Shared locks,
`OnceLock` installation and independent builders for repeated source preparation are prohibited.

*The bounding fact.* `SourceLocation` must not gain a required span field. Outside this lane there
are hundreds of `CompilerDiagnostic::`/`CompilerError::` construction and adapter sites across AST,
HIR, backend, project and build code, plus synthetic and path-only diagnostics that have no
authored span to give. Requiring a span on the shared type pulls all of slice group 1E forward and
forces those sites to manufacture one. 1D3 adds an exact `SourceSpan` carrier on the preparation
lane's diagnostics and leaves `SourceLocation` alone until 1H, exactly as the interval bridge says.

*Where the producers are.* Tokenizer: 11 constructor sites in `tokenizer/lexer.rs`, 2 in
`numeric.rs`, 5 in `text_modes.rs`, and 14 more in `paths/const_paths/` reached through path
parsing — all of them call `stream.new_location()` while `TokenStream` still owns both the cursor
and the live builder (`tokenizer/tokens.rs:693-729`), so the span is in scope at every one. The
three escape sites are the exception: `escape_span` (`text_modes.rs:137-145`) takes only
`CharPosition` and never retained the escape's byte start, so it needs that start threaded before
it can be exact. File preparation: `headers/file_parser.rs` 20, `trait_headers.rs` 17,
`header_dispatch.rs` 7 direct plus 10 `CompilerError` adapters, `dependency_clause_syntax.rs` 5
(12 branches through one helper), `parse_file_headers.rs` 5, `hash_items.rs` 5,
`file_dependency_clauses.rs` 3, `dependency_paths.rs` 4; these receive `FileTokens` and
`SourceLocation` but never the builder, though it is alive in their caller. Stage 0:
`source_preparation.rs` constructs one file error and otherwise forwards, and
`source_discovery.rs` has 4 user diagnostics of which 3 synthesise `SourceLocation::from_path` and
so have no authored span at all. `plain_markdown_prepare.rs` produces no diagnostics and
`moth_template_prepare.rs` only propagates infrastructure errors.

The existing token bridge to copy is `TokenStream::new_token` (`tokenizer/tokens.rs:854-878`),
which builds the span first and derives the legacy range from it; its agreement test is
`every_token_legacy_byte_range_matches_its_encoded_span`
(`tokenizer/tests/lexer_tests.rs:2450-2505`).


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
- [ ] **1F4 — renderer migration:** resolve paths, excerpts and line/column positions through retained source snapshots and remove filesystem rereads used only for excerpts. Both renderers must derive line and column from `SourceLocation`'s exact byte offsets through `LineIndex`, which fixes the `char_column` caret that currently points one scalar past the offending text and the stale line an inherited position can carry; add the first caret-alignment regression tests, covering both a captured code-token column and a re-anchored token such as a discarded template body's closing bracket. Consuming `line_of_offset`, `position` and `LinePosition` retires 1C4's module-wide `allow(dead_code)`; `utf16_column` keeps a narrowly scoped documented allowance until the LSP consumer the architecture document defers actually arrives
- [ ] **1F5 — frozen generic source ownership:** preserve each retained generic body's real source identity with its owning frozen identity context, or canonically remap its exact source spans into the consuming context before publication. Materialised tokens, generated artefacts and their diagnostics must never expose detached donor IDs. Delete the identity-free frozen constructor and optional identity carrier introduced by the 1D interval correction, restoring required `SourceId` throughout the token/scope chain. Cover cross-module and independently compiled package materialisation, including extended spans after the donor's mutable builders have dropped.
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

### Phase 1 external review checkpoint

- [ ] complete the remaining 1D, 1E, 1F, 1G and 1H slices and reconcile the open 1B4 lifecycle gate
- [ ] complete Phase 1's independent final reviews and required measurement/validation gates
- [ ] commit final corrections and closeout, verify the checkpoint sequence and worktree state
- [ ] pause for external user review before starting Phase 2

---

## Phase 2 — Genuine complete-path interning

### Summary, reasoning and context

Phase 2 begins only after 1A has migrated compiler source slots to `PathId` and Phase 1 has
passed its exit gates. It replaces the remaining vector-backed complete compiler paths without
adding a contended global interner or another scheduling system.

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
- [ ] preserve the layout authority's SoA baseline when results are materially tied
- [ ] choose compact AoS only for a repeatable material improvement over SoA, as the authority requires
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
- [ ] remove source path, canonical OS path, `index` and `length` from token storage; this inherits 1B6's last sentence, so also drop `Header::canonical_source_file`'s re-derivation, its nine call sites' dependency on the token copy, and `validate_header`'s canonical-path agreement check
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
