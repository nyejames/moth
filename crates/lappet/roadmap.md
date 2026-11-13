# Lappet roadmap

Lappet owns its runtime, embedding and package-manager work here. Develop it on
the `lappet-work` branch in an isolated worktree, separately from Moth's serial
compiler roadmap. Refresh the current compiler contracts before each
integration slice. This roadmap never resumes a paused compiler plan or treats
planned backend support as available.

## Current state

- Status: CLI separation and the public `moth::builder` SDK prototype are
  delivered. The `lappet` crate remains dependency-free and documentation-only,
  with no native runtime or host adapter.
- Current milestone: 2, establish Wasmtime execution. Its plan is ready and not
  started.
- Blockers: none for milestone 2. Milestones 4 and 5 wait for work on `main`
  listed under [Waiting on main](#waiting-on-main).
- Next action: activate the
  [Wasmtime runtime foundation plan](../../docs/roadmap/plans/lappet-wasmtime-runtime-foundation-plan.md)
  at Phase 0.

[Runtime design](docs/design.md), [package-manager design](docs/package-manager.md)
and [Wasm optimisation](../../docs/developer-docs/wasm-optimisation/overview.mtf)
own the accepted direction. This living roadmap owns delivery, findings and
open decisions. Record a checkpoint's decision in its permanent owner before
implementing the affected slice. Keep unresolved questions explicit rather
than turning example API names into obligations.

## Sequence

Work top to bottom. A milestone starts only when its row's wait is satisfied.
Milestones 2 and 3 live entirely in `crates/lappet` and never touch the
compiler crate, so they proceed while compiler work continues on `main`.

| # | Milestone | Touches | Starts when | Plan |
|---|---|---|---|---|
| 1 | Separate the CLI from the compiler library | Delivered | Delivered | None |
| 2 | Establish Wasmtime execution | `crates/lappet` only | Now | [Runtime foundation](../../docs/roadmap/plans/lappet-wasmtime-runtime-foundation-plan.md) |
| 3 | Build host boundary tables and fault mechanics | `crates/lappet` only | Milestone 2 delivers instances, limits and fault quarantine | [Host boundary tables](../../docs/roadmap/plans/lappet-host-boundary-tables-plan.md) |
| 4 | Extract the shared host contract crate | New crate, `src/host`, `src/builder`, `xtask` | Const functions are on `main` | [Host contract crate](../../docs/roadmap/plans/lappet-host-contract-crate-plan.md) |
| 5 | Finish the native binding and artefact handoff | `src/builder`, `src/build_system`, `src/builder_surface` | Const functions are on `main` and milestone 4 is delivered | [Native binding handoff](../../docs/roadmap/plans/lappet-native-binding-handoff-plan.md) |
| 6 | Establish official Core conformance | Lappet, Core references | Milestones 3 and 5 and its own checkpoint | Write when the checkpoint is approved |
| 7 | Connect real Moth applications | Lappet, public compiler services | The compiler's native host ABI prerequisites | Write when the gate opens |
| 8 | Package declaration and local catalog foundations | Build system, Stage 0, Lappet | Its checkpoint and the package prerequisites | Write when the checkpoint is approved |
| 9 | Package eligibility and dependency evidence | Lappet, compiler facts | Milestone 8 | Write when the checkpoint is approved |
| 10 | Remote admission | Lappet | Milestone 9 and its checkpoint | Write when the checkpoint is approved |

Milestone 4 is small and independent of milestones 2 and 3. Run it as soon as
its wait is satisfied, even when milestone 2 or 3 is still in progress. The
package lane, milestones 8 to 10, is independent of Wasmtime and may move ahead
of milestones 6 and 7 once its prerequisites and checkpoint are met.

## Waiting on main

This branch tracks `main`. Sync it whenever one of these lands and before every
integration slice. Name the capability a milestone needs, then confirm it in
the current tree instead of trusting this table.

| Lands on `main` | Unblocks | Why Lappet waits |
|---|---|---|
| Const functions, including the `const_work_limit` and `const_memory_limit_mb` project settings | Milestones 4 and 5 | That work edits `src/build_system/project_config.rs`, `src/build_system/project_config/validation.rs`, `src/builder_surface/definition.rs` and `src/projects/settings.rs`. The remaining SDK work edits the same config owners |
| A `ValidatedMemoryPlan` producer for boundary string, frame and scope allocations | Milestone 7 | The native host ABI forbids a temporary planner or guessed layout |
| Host-import lowering to `moth-host-v1` names, fallible-call lowering through `out_frame` and `status` and builtin `Error` value lowering | Milestone 7 and the compiler-level adapter tests | Lappet adapters convert only what compiled Moth really emits |
| `i64` resource-token carriage, the `staged_text_take` import and the call-scope helper exports | Milestone 7 | Milestone 3 exercises these protocols with fixtures only |
| Builder-selected export wrappers, the host manifest and `host.assertion_failed` moved into `moth-abi-v1` | Milestone 7 and manifest-driven preflight | Lappet matches implementations against the manifest, never against guessed names |
| Dependency clauses, canonical project and package graphs, configuration and input isolation, immutable public interfaces and stable capability and ABI fingerprints | Milestone 8 | The manager consumes compiler-owned compatibility facts |

The [native host ABI](../../docs/developer-docs/memory-management/native-host-abi/native-host-abi.mtf)
owns the full compiler prerequisite list. The first registry prerequisite, the
restricted host profile and the package-path `#` rule, is already delivered by
`BuilderDefinition::finish`.

After each sync, rerun the focused Lappet tests and the dependency boundary
check, then re-read any authority the merge changed.

## Open decisions

Each needs Nye's answer before the named work starts. Recommendations are
recorded in the owning plan.

- **Host resource and context API.** The
  [draft design](docs/host-resources.md) awaits review. It ends with four
  questions and five amendments to the adapter responsibilities. Blocks the
  resource phases of milestone 3. The staging table and token generator don't
  wait.
- **Runtime policy defaults.** Provisional limits are recorded in the runtime
  foundation plan. Milestone 2 gathers the evidence and Nye confirms the final
  values before they move into the runtime design.
- **Facade-only projects.** Whether a project with a package facade and no
  normal entry module may build. Blocks nothing yet. The artefact fixtures use a
  project with both.

## Working rules

Read the worktree's root `AGENTS.md`, `crates/lappet/AGENTS.md`, current
permanent authorities and validation routes. Use current `index.md` locations instead of restoring an obsolete owner.
Name prerequisite capabilities rather than depending on temporary plan files.

A checkpoint blocks its own implementation, not unrelated investigation. Small
Rust-produced or authored core Wasm fixtures may establish runtime mechanics
before Moth's backend is ready. They cannot establish Moth ABI, ownership or
language support. Keep runtime, compiler integration and manager progress
separate.

Each code-bearing slice needs a bounded implementation plan, focused failing
coverage, the smallest implementation and the repository Slice review. Retain
useful evidence and actionable findings here without copying raw logs. Final
integration follows the repository's full validation gate. Compare both the
game-engine and server use cases at every public runtime/API decision.

Lappet plans live in `docs/roadmap/plans/` with a `lappet-` prefix and follow
the main roadmap's rules for adding and maintaining plans. Only this roadmap
links them. A plan names the capabilities it needs and never cites another
plan. Delete a plan in the commit that completes it and update this roadmap in
the same commit. Commits are authored by Nye only, with no attribution lines.
Push to `lappet-work` and don't open a pull request unless asked.

## 1. Separate the CLI from the compiler library

Completed on 18 October 2026. This milestone changes package and command
ownership without implementing a native runtime.

**Decision.** The root `moth` package retains the compiler library.
`crates/moth-cli` owns the `moth` executable, depends on `moth` and `lappet` and
is the workspace default. Compiler features forward through matching CLI
features, including their implications. The compiler has no Lappet or Wasmtime
dependency. [The runtime design](docs/design.md#users-and-workspace-dependencies)
records the package and service boundary.

**Delivered.** Argument parsing, terminal presentation and scaffolding belong
to the CLI. Compiler services retain compilation, output planning/writing,
diagnostic contexts, instrumentation, dev execution, Boracle and the integration
runner. The concrete builder constructor keeps its backend private. Installation,
release packaging, benchmark/profiling discovery, feature lanes and source and
dependency checks use the new owner. The index and validation references match.
No runtime dependency or package-manager command was added.

**Evidence.** Local macOS AArch64 checks used Rust 1.99.0 and Node 24.21.0:

- `just validate` passed after the final coverage correction: 6013 compiler unit
  tests, 102 CLI unit tests, 19 subprocess tests, 855 xtask tests and the remaining
  workspace tests passed. All 2787 canonical integration results were correct,
  and docs source checking, Clippy, formatting and architecture checks passed.
- Focused compiler project tests passed with default features and timers. CLI
  tests passed with default features, timers/counters and Boracle. The existing
  help test failed with campaign-only features before the implication fix and
  passed afterwards. The restored terse-summary assertion passed.
- Locked Cargo metadata verified package direction, default selection, binary
  ownership and all 15 feature forwards. A separate Rust client compiled valid
  Moth and retained a missing-input failure. Negative client probes rejected
  access to the raw backend method, field, trait and test-only constructor.
- `cargo install --path crates/moth-cli --locked --offline` with a local install
  root produced `moth`. Its version/help, scaffold, check and build commands passed.
- `just bench-validate` passed all 150 shared preflights without recording results.
- `just timers-erasure-check` built and inspected the isolated no-timer release
  executable successfully. The documentation release build produced 81 outputs,
  and the generated Lappet policy states Cranelift-only V1 and future Winch.

**Slice review.** Independent architecture and tooling reviews checked ownership,
public handoffs, migrated coverage and consumers. The architecture review found
one omitted terse-summary test, restored at its CLI owner and verified by a
focused run and the repeated routine gate. No production correction remained.

These local checks do not claim hosted platform coverage, profiling execution or
`just validate-full` merge readiness.

## 2. Establish Wasmtime execution

Plan: [Wasmtime runtime foundation](../../docs/roadmap/plans/lappet-wasmtime-runtime-foundation-plan.md).

**Checkpoint.** Wasmtime 50.0.1 with the `std`, `runtime` and `cranelift`
Cargo features is selected behind Lappet's `runtime` feature. A trap or fault
quarantines its instance, which is then dropped. The plan records provisional
memory, stack, execution and table limits. Still to confirm with evidence:
supported Wasm proposals, hosts, debugging requirements and those limits.
Compare `OptLevel::None` and `OptLevel::Speed` using representative game and
server workloads before confirming the defaults. Retain the default
backtracking register allocator and disabled inlining unless a separate
investigation justifies changing them. Set a policy for blocking native calls.
Keep the public runtime configuration small.

**Delivery.** Cranelift is V1's sole native compiler. Development defaults
to `OptLevel::None` and release to `OptLevel::Speed`, independent of the
Moth output profile. These defaults remain subject to checkpoint evidence.
Reuse immutable compiled modules where valid and keep mutable stores, host
contexts and resources instance-owned. Execution remains synchronous and
builder-scheduled. No persistent AOT cache or other Wasm engine is included.

**Evidence.** Run identical core Wasm fixtures under both Cranelift
optimisation levels. The plan records evidence from the implementing host
first. Candidate Linux x86-64/AArch64, macOS x86-64/AArch64 and Windows
x86-64 hosts are published only once each is verified. Measure clean
Rust build cost, binary size, cold engine/module compilation, instantiation,
execution, host-call cost and resident memory separately. Test limits,
traps, teardown and prohibited re-entry. These fixtures do not prove a
Moth compiler ABI.

**Documentation.** Keep [the Wasmtime reference](docs/wasmtime-integration.md)
current with the pinned release and record findings that change an
implementation choice. Back its embedding sketch with a tested program. It
shows engine setup, a host import, core Wasm instantiation, a typed exported
call and failure/cleanup handling. A hand-authored Wasm fixture is acceptable
until Moth's backend can produce the equivalent program.

## 3. Build host boundary tables and fault mechanics

Plan: [Host boundary tables](../../docs/roadmap/plans/lappet-host-boundary-tables-plan.md).

**Checkpoint.** Approve the [draft host resource and context API](docs/host-resources.md)
before the resource phases. The staging table, the token generator and their
fault handling follow the accepted [adapter responsibilities](docs/host-abi.md)
and need no further decision.

**Delivery.** Implement the per-instance staging table, the process-wide token
generator, the per-instance resource table with call-scoped access guards and
the per-call host context slot. Wire each into fault quarantine and teardown.
Build no value conversion, descriptor matching or manifest preflight here.

**Evidence.** Cover the fixture-level deferred tests in the adapter
responsibilities: ticket lifetime and take checks, stale, closed,
other-instance, wrong-type and conflicting tokens, physical import mismatches
and a trapping scope helper. Run each under both optimisation levels. Fixtures
follow the V1 protocol by hand and prove Lappet's mechanics only.

## 4. Extract the shared host contract crate

Plan: [Host contract crate](../../docs/roadmap/plans/lappet-host-contract-crate-plan.md).

**Checkpoint.** Accepted on 13 November 2026 and recorded in the
[runtime design](docs/design.md#users-and-workspace-dependencies). Lappet's
runtime depends on a small `moth-host` contract crate and Wasmtime, never on
the compiler. The compiler depends on the same crate and keeps the
`moth::host` path. Only Lappet's future package feature depends on the
compiler.

**Delivery.** Move the host descriptor vocabulary into the contract crate,
keep the registry conversion in the compiler, extend the first-party
dependency boundary check and record the accepted graph in the runtime design.

**Evidence.** The public `moth::host` path and the standalone crate expose the
same types. The contract crate depends on no workspace package. `lappet` with
the `runtime` feature builds without the compiler in its dependency tree.

## 5. Finish the native binding and artefact handoff

Plan: [Native binding handoff](../../docs/roadmap/plans/lappet-native-binding-handoff-plan.md).

**Checkpoint.** The first concrete Rust-to-Moth ABI is accepted. It uses the
existing manual host descriptor path and covers scalars, strings, optionals,
errors, explicit context and opaque resources. Byte buffers, records, choices,
collections, multi-slot host imports and resource results are deferred beyond
V1 and each needs an approved shape before expanding the restricted profile.

**Delivery.** Complete the public builder SDK: the configuration helper and
typed settings, artefact publication, the honest native Wasm availability
error and the external fixture crate. Then publish import and export
requirements and a pure-Rust preflight. Share one descriptor contract between
compiler registration and runtime adapters. Retain semantic types separately
from physical carriers and implementation selection. Implement only approved
adapters, with no guessed Rust layout, second type registry or general
serialisation framework. Attribute errors to both the Moth call and the
registered host binding where available.

**Documentation.** The accepted V1 physical boundary lives in the compiler-owned
[native host ABI](../../docs/developer-docs/memory-management/native-host-abi/native-host-abi.mtf).
[Host ABI adapter responsibilities](docs/host-abi.md) owns Lappet's adapter,
staging, resource-table and fault duties. Link each executable adapter example
to tests.

**Evidence.** A separate Rust crate defines a builder, compiles a real Moth
project, publishes one output and sees the honest native Wasm error. Reject
mismatched imports, undeclared exports, missing implementations and wrong
types, arity or return channels in the pure-Rust preflight. Value-level
evidence needs emitted Wasm and belongs to milestone 7: exact integer ranges,
finite floats, Byte versus U8 at runtime, invalid Unicode and lengths,
optional absence versus failure, ordered results, exactly-once calls,
independence of returned storage and every allocation and cleanup failure
path.

## 6. Establish official Core conformance

**Checkpoint.** Inventory the complete public Core catalogue and distinguish
compiler-internal helpers from optional host facilities. Record the exact
portable behaviour, documented deviations, capability rejections and declared
unsupported results for browser, game and headless server hosts. Define the
trusted official-environment identity used by package admission. A builder's
self-declaration or use of Lappet cannot grant certification.

**Delivery.** Provide reusable native Core implementations through the existing
canonical binding contracts. Retain efficient compiler/Wasm implementations
where already suitable. Custom builders may select implementations for local
code without gaining permission to consume managed Core-dependent packages.
Keep browser implementation ownership with the HTML builder until a real
second consumer justifies extraction.

**Evidence.** Share semantic conformance cases across official browser and
native implementations, including numeric/error behaviour and permitted host
differences. Distinguish unsupported facilities from broken advertised ones.
Check that `io` stays reserved without forcing custom hosts to implement it.
No implementation status is claimed from registering descriptors alone.

## 7. Connect real Moth applications

**Gate.** Wait for delivered structured Wasm lowering, concrete result/error
ABIs, required scalar/aggregate representations, compiler-selected runtime
helpers and validated physical memory plans. Consume the merged expression and
generic contracts rather than reopening their owners. Foreign WAT provider
work neither supplies all these capabilities nor implies WIT support.

**Delivery.** Compile through public Moth services, install matching Rust host
implementations and invoke explicitly selected exports. Prove one embedded game
lifecycle and one server lifecycle without adding a scheduler or web framework.
Keep rendering/output/resource planning with the builder and build system.

**Evidence.** Run actual Moth programs, not hand-authored ABI substitutes.
Repeat every fixture-level boundary test from milestone 3 and deliver the
value-level evidence deferred from milestone 5.
Cover source-to-host-to-source values, errors, lifecycle cleanup, isolated host
state and capability rejection. Benchmark all four combinations of Moth
output profile and native strategy under the shared optimisation authority.
Never mutate published compiler results to make runtime binding succeed.

**Documentation.** Extend the embedding example to use a real Moth program
when the compiler and runtime handoffs support it. Keep the example checked
against the public embedding API rather than documenting hypothetical calls.

## 8. Deliver package declaration and local catalog foundations

This lane is independent of Wasmtime. It replaces the retired main-roadmap
package-manager foundations plan without claiming that its implementation is
complete.

**Prerequisites.** Consume delivered dependency clauses, canonical project and
package graphs, configuration/input isolation, exact numeric identities,
immutable public interfaces and stable capability/ABI fingerprints. Require
backend capabilities only where the chosen artefact actually needs them.

**Checkpoint.** Decide canonical package-name grammar, the project declaration
carrier, alias replacement rules and the exact resolved-catalog input. Choose
programmatic packages or one explicit local development source for the first
implementation. Preserve the separation from config evaluation and file-local
symbol binding. Version solving, remote fetching and lockfile implementation
are not prerequisites for this local slice.

**Delivery sequence.** Extract declarations in authored order, reject duplicate
canonical names and colliding roots, resolve each declared dependency once,
compile separate dependency graphs in deterministic order and publish direct
roots through the normal registration path. Acquisition remains outside Stage
0. Source and compatible precompiled forms share the catalog contract, without
adding a persistence framework merely to support its vocabulary.

**Evidence retained from the former plan.** Test undeclared source dependencies,
duplicate canonical names, alias collisions, invalid roots and missing catalog
entries. Test direct-only visibility, transitive privacy, independent config and
private project globals, generated-instance ownership and failed publication.
Reverse declaration order to verify deterministic graph outcomes while keeping
authored diagnostic order. Verify exact numeric/ABI compatibility and that
aliases never change artefact or resource-output identities. Missing names must
not trigger filesystem probing, a config import preamble or legacy folder
fallback. Incompatible artefacts are rejected or rebuilt through the existing
policy, never repaired with guessed layouts or numeric conversions.

## 9. Derive package eligibility and dependency evidence

**Checkpoint.** Specify how compiler/provider facts establish public Core use,
no-core closure and excluded Builder/foreign dependencies. Distinguish public
Core dependencies from implicit namespace reservation and compiler runtime
helpers. Define trusted official/custom classification, exact version-instance
accounting and the initial dependency metric definitions. Keep numeric policy
thresholds unchosen until reviewed against representative packages.

**Delivery.** Produce deterministic category, depth, breadth and size reports
from one resolved graph. Apply custom-builder no-core-only admission separately
from language target validation. Reuse canonical dependency data rather than
rescan source or introduce a parallel resolver. Reports alone do not provide a
security assessor or authorise installation.

**Evidence.** Cover transitive Core use through signatures, constants and private
helpers, reserved-but-unused `io`, hidden compiler helpers, diamonds and repeated
aliases. Test shallow-wide graphs, deep-small graphs, large dependency-free
packages, version duplication and copied/vendored source. Lead status alone
never decides rejection. Compare candidate closure, project closure and added
cost without counting shared packages repeatedly.

## 10. Design remote admission before enabling managed addition

**Checkpoint.** Approve version/lockfile/override policy, metadata provenance,
source staging and archive limits, source integrity, isolated analysis and the
required checker set. Specify unknown/failure handling, dependency thresholds,
reports, exact-content approval and atomic project updates. Include concurrent
edits, cancellation, stale evidence and unattended operation. Design assessment
of the whole resolved closure, not only the requested root.

**Delivery.** Only after that checkpoint, implement bounded metadata preflight,
isolated staging, reconciliation, fail-closed assessment, reporting and explicit
confirmation for admissible proposals. Keep the project unchanged on rejection
or cancellation. No force flag, missing-check fallback or implicit native build
may turn a rejection into a managed installation.

**Evidence.** Reject unavailable checks, incomplete evidence, tampered archives,
path escapes, misleading metadata, changed dependencies and unsupported builder
categories. Verify that confirmation binds the exact reviewed transaction and
cannot approve different bytes. Prove isolation/resource controls before any
untrusted code or test execution is enabled. A positive assessment is never
reported as proof of harmlessness.

Registry hosting and publication have another review checkpoint. Public topology
metadata and publication checks do not replace add-time inspection. Preserve
Lead packages for policy review rather than a depth-only registry ban.

## Deferred extensions

- Persistent compiled-code caches, AOT deployment and cache invalidation, kept
  separate from package source storage and cold benchmark measurement.
- Channels, async host calls and scheduling coordination after Moth's language
  and runtime contracts exist. No implicit V1 scheduler is reserved for them.
- Foreign WIT components, first through a separately reviewed value-only profile.
  Resources, callbacks and retained state need their own ownership design.
- Winch as a potential future baseline compiler, only after upstream production
  readiness, proposal/CPU support, limit semantics and benchmarks justify the
  extra dependency and test matrix.
- Other execution engines or smaller target environments, with explicit platform
  and semantic support rather than a speculative multi-engine abstraction.
- Managed Builder-dependent packages and versioned provider compatibility.
- Reusable HTTP, cryptography or other Rust host libraries. Decide whether these
  are builder-owned libraries or Lappet conveniences from real consumers.
- Browser implementation extraction when more than one builder needs it.

## Final review for each delivered milestone

Review producer/consumer ownership, duplicated facts, unused abstractions,
legacy paths, failure categories and meaningful test coverage. Reconcile the
permanent docs and implementation status, update the index and perform the Slice
review. Use the current repository validation guide, including
`just validate-full` for the exact tree intended for main. A narrow crate check,
fixture run or documentation build never claims that wider gate passed.
