# Lappet roadmap

Lappet owns its runtime, embedding and package-manager work here. Develop it
on a dedicated Lappet branch in an isolated worktree, separately from Moth's
serial compiler roadmap. Refresh the current compiler contracts before each
integration slice. This roadmap never resumes a paused compiler plan or treats
planned backend support as available.

**Sequencing gate:** Further Lappet resume work remains paused until the
compiler delivers exact ObVer package build declarations, project-local package
libraries and deterministic resolved package graphs. The compiler roadmap owns
that foundation. This gate applies before native readiness work and milestones
6 through 10, in addition to each milestone's existing prerequisites.

## Current state

- Status: CLI separation, the public `moth::builder` SDK, the shared host
  contract crate and the native binding handoff are delivered. The `lappet`
  crate has a Wasmtime runtime and host boundary tables. It has no host adapter
  yet.
- Current milestone: the native readiness and hardening prerequisite is queued
  behind the compiler's exact ObVer package-resolution foundation. No further
  Lappet roadmap milestone is currently startable.
- Blockers: compiler-owned package libraries and dependency graphs must land
  first. Selected Rust export adapters also require the compiler's native
  host ABI capabilities listed under [Waiting on main](#waiting-on-main).
- Next action: after the compiler prerequisite lands, sync and verify the
  earlier runtime findings, apply focused corrections and complete the Core
  implementation/conformance inventory for approval.

[Runtime design](docs/design.md), [package-manager design](docs/package-manager.md)
and [Wasm optimisation](../../docs/developer-docs/wasm-optimisation/overview.mtf)
own the accepted direction. This living roadmap owns delivery, findings and
open decisions. Record a checkpoint's decision in its permanent owner before
implementing the affected slice. Keep unresolved questions explicit rather
than turning example API names into obligations.

## Sequence

Work top to bottom. A milestone starts only when its row's wait is satisfied.
Milestones 2 and 3 lived entirely in `crates/lappet` and never touched the
compiler crate.

| # | Milestone | Touches | Starts when | Plan |
|---|---|---|---|---|
| 1 | Separate the CLI from the compiler library | Delivered | Delivered | None |
| 2 | Establish Wasmtime execution | Delivered | Delivered | None |
| 3 | Build host boundary tables and fault mechanics | Delivered | Delivered | None |
| 4 | Extract the shared host contract crate | Delivered | Delivered | None |
| 5 | Finish the native binding and artefact handoff | Delivered | Delivered | None |
| Prerequisite | Native readiness and hardening | Lappet runtime, Core and host contracts | After the compiler package-resolution foundation is delivered | [Phased plan](../../docs/roadmap/plans/lappet-native-readiness-and-hardening-plan.md) |
| 6 | Establish official Core conformance | Lappet, Core references | Prerequisite, milestones 3 and 5 and its own checkpoint | Write when the checkpoint is approved |
| 7 | Connect real Moth applications | Lappet, public compiler services | Prerequisite and the compiler's native host ABI prerequisites | Write when the gate opens |
| 8 | Consume compiler-resolved package catalog | Lappet package infrastructure | Prerequisite and delivered compiler package-resolution foundation | Write when the consumer checkpoint is approved |
| 9 | Package eligibility and dependency evidence | Lappet, compiler facts | Milestone 8 | Write when the checkpoint is approved |
| 10 | Remote admission | Lappet | Milestone 9 and its checkpoint | Write when the checkpoint is approved |

The package lane, milestones 8 to 10, is independent of Wasmtime and may move
ahead of milestones 6 and 7 after the compiler package foundation and common
readiness prerequisite are delivered. Its own consumer and admission checkpoints
still apply. Milestone 8 must not repeat compiler dependency syntax or resolution.

## Waiting on main

This branch tracks `main`. Sync it whenever one of these lands and before every
integration slice. Name the capability a milestone needs, then confirm it in
the current tree instead of trusting this table.

| Lands on `main` | Unblocks | Why Lappet waits |
|---|---|---|
| A `ValidatedMemoryPlan` producer for boundary string, frame and scope allocations | Milestone 7 | The native host ABI forbids a temporary planner or guessed layout |
| Host-import lowering to `moth-host-v1` names, fallible-call lowering through `out_frame` and `status` and builtin `Error` value lowering | Milestone 7 and the compiler-level adapter tests | Lappet adapters convert only what compiled Moth really emits |
| `i64` resource-token carriage, the `staged_text_take` import and the call-scope helper exports | Milestone 7 | Milestone 3 exercised these protocols with fixtures only |
| Builder-selected export wrappers, the host manifest and `host.assertion_failed` moved into `moth-abi-v1` | Milestone 7 and manifest-driven preflight | Lappet matches implementations against the manifest, never against guessed names |
| Exact ObVer-qualified config declarations, project-local package libraries, immutable resolved package catalog and separate package graphs | Readiness prerequisite and milestone 8 | This compiler foundation is a hard gate before Lappet resumes. Lappet consumes it without another resolver |
| Canonical public interfaces, stable dependency capabilities, ABI fingerprints and configuration/input isolation | Milestones 8 and 9 | The manager consumes compiler-owned compatibility and closure facts |

The [native host ABI](../../docs/developer-docs/memory-management/native-host-abi/native-host-abi.mtf)
owns the full compiler prerequisite list. The first registry prerequisite, the
restricted host profile and the package-path `#` rule, is already delivered by
`BuilderDefinition::finish`.

V1 const functions landed on `main` and this branch merged them on 13 November
2026.

After each sync, rerun the focused Lappet tests and the dependency boundary
check, then re-read any authority the merge changed.

## Accepted decisions and remaining checkpoints

**Accepted for native readiness:** retain the restricted V1 host ABI and route
Core operations through compiler-owned direct, helper or native providers.
Official Core eligibility uses a trusted first-party builder catalogue, not
custom builder self-identification. The Rust adapter uses manifest-validated
selected exports, canonical resource/context descriptors and no implicit start
for host-callable artefacts. The exact operational bound for copied export
strings remains a prerequisite checkpoint, not a chosen number. These decisions
live in [the runtime design](docs/design.md),
[the host ABI adapter responsibilities](docs/host-abi.md) and
[the package-manager design](docs/package-manager.md).

**Accepted compiler dependency foundation:** `@config.moth` declares an exact
external ObVer build through `@package::2026.32.5`. `as` replaces the visible
root without changing canonical identity. A fixed project-local `packages/`
library is the default. Explicit fixed-only configuration can replace it with
other project-local libraries. No global directory is created or searched.
The compiler inspects only declared exact builds under
`library/package_name/version/`. Duplicate matches are errors, not precedence.
Each package version must have a matching name/version and `@package.moth`.
Registered library subtrees are outside the consumer's source index, and each
package gets its own config, graph and private globals. Direct-only dependency
visibility applies across the entire closure. The package's compiler-produced
catalogue is the sole source of graph and compatibility facts. `::` is a
provider-build qualifier conceptually, but only exact external ObVer selection
in config is supported in V1. Ordinary imports never select builds or override
module compilation settings.

The precise fixed-only package-library field spelling and empty-list handling
remain a compiler implementation checkpoint. Those questions do not reopen
Lappet's declaration syntax, version policy or catalogue ownership.

**Remaining Lappet checkpoints:** (1) the complete Core provider and host
capability/conformance matrix, including opaque Core result representation,
(2) the internal trusted first-party environment certification handoff and
(3) the operational byte bound and configuration surface for copying exported
Moth strings into Rust. These are required implementation choices, not an
invitation to broaden the general V1 host ABI. Native export adapters also
wait for the compiler's selected-export manifest and concrete ABI lowerings.

**Previously decided:** Nye confirmed the runtime policy defaults and host
boundary limits on 14 November 2026, with the values unchanged.

**Facade-only projects: decided.** A project with a valid package facade and no
executable entry is a valid build target for a package-capable builder. The
project compiles normally and exposes its package assembly. No executable entry
is synthesised or activated. The selected builder decides whether to publish a
package artefact or return successfully without outputs. Builders that require
executable entries enforce that requirement themselves.

Delivered on 14 November 2026.
[The build system design](../../docs/build-system-design.md#build) owns the
policy. The shared build compiles an entryless project and the public SDK
reaches `ProjectBuilder::build` for it. The HTML builder reports the no-entry
diagnostic itself against the source root the frontend compiled. One ordering
change follows: an HTML project with no entry and a source error now reports
the source error first, because compilation runs before the builder's check.
The `just validate` run recorded under milestone 2 covered this tree.

## Native readiness findings

The pre-roadmap plan owns verification, implementation and tests. An earlier
inspection identified four concrete correctness/diagnostic leads. Reconfirm
them against the current Lappet baseline before changing code:

- `instance.rs` can validate a token before reporting a quarantined instance,
  and a rejected call can still install an epoch deadline.
- `staging.rs` checks configurable byte limits without independently enforcing
  the native ABI's `i32::MAX` text-length ceiling.
- The scope driver reports oversized Rust caller input as an internal host
  fault. It should reject the input before guest entry without quarantine.
- The single-string fixture driver reports declared errors without decoding
  their record and uses the noncanonical-status fault for malformed result
  spans and UTF-8. The manifest adapter must replace this path in milestone 7.

Core conformance also has a documented integration blocker: `@core/time` and
`@core/io` return opaque Core types, while general V1 host imports disallow
resource results. The compiler's existing host requirement collection does
not include Core-backed external bindings. The prerequisite inventory must
assign explicit Core provider routes before native implementation starts.

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
Push to the designated Lappet branch and don't open a pull request unless asked.

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

Completed on 14 November 2026.

**Checkpoint.** Wasmtime 50.0.1 with the `std`, `runtime` and `cranelift`
Cargo features is selected behind Lappet's `runtime` feature. A trap, limit or
host fault quarantines its instance, which is then dropped. Nye confirmed every
policy value on 14 November 2026, and the confirmed values live in
[the runtime design](docs/design.md#runtime-strategy).

**Delivery.** Cranelift is V1's sole native compiler. Development uses
`OptLevel::None` and release uses `OptLevel::Speed`, independent of the Moth
output profile. Compiled modules are immutable and reused where valid. Mutable
stores, host contexts and resources stay instance-owned. Execution is
synchronous and builder-scheduled. No persistent AOT cache or other Wasm
engine is included. The public runtime configuration stays small.

**Evidence.** Core Wasm fixtures ran identically under both Cranelift
optimisation levels. Measurements come from the implementing host first.
Candidate Linux x86-64/AArch64, macOS x86-64/AArch64 and Windows x86-64 hosts
are published only once each is verified. Clean Rust build cost, binary size,
cold engine/module compilation, instantiation, execution, host-call cost and
resident memory are recorded separately. Limits, traps, teardown and prohibited
re-entry have coverage. These fixtures do not prove a Moth compiler ABI.

**Documentation.** [The Wasmtime reference](docs/wasmtime-integration.md)
records the pinned release and the findings that changed an implementation
choice. Its embedding sketch links to a tested program that shows engine setup,
a host import, core Wasm instantiation, a typed exported call and
failure/cleanup handling.

**Progress.** Evidence so far comes from one host, macOS AArch64 with Rust
1.99.0, and from authored fixtures.

- Phase 0, 13 November 2026: the branch was current with `main`, 50.0.1 was
  the newest 50.0.x release and the routed documents showed no drift.
- Phase 1, dependency and feature gate: `runtime` enables Wasmtime `=50.0.1`
  with `std`, `runtime` and `cranelift` only. The fixture assembler is `wat`
  `=1.261.0`, matching the `wasmparser` 0.261.0 that Wasmtime validates with.
  The feature adds 82 packages to a `lappet` build. The lockfile grew from 81
  to 159 packages. A clean `lappet` build with `runtime` took 78 s in the dev
  profile and 53 s in release, against under 1 s with no feature. `moth` and
  `moth-cli` have no path to Wasmtime, and `just first-party-deps` and
  `just feature-lane-check` pass with the new `lappet` and `lappet-runtime`
  lanes. The independent review was clean.
- Phases 2 to 5, engine, instances, limits and faults: delivered as one slice
  because the instance, its limits and its quarantine share one small owner.
  The public names are `RuntimeEngine`, `NativeOptimisation`,
  `CompiledModule`, `InstanceLimits`, `RuntimeError`, `LinkError`,
  `LimitFault` and `HostFault`. The crate-private mechanism is `HostImports`,
  `PreparedModule`, `Instance` and `Deadline`. 26 fixture tests pass under
  both optimisation levels, including the embedding program the Wasmtime
  reference links to. The tests were written with the implementation, so the
  plan's red-then-green sequence was not observed. A later mutation check on
  a scratch copy caught every probed regression: quarantine set and check,
  limit precheck, both deadline installations, the store limiter, swapped
  start and guest traps, an uncaught host panic, an unguarded panic-payload
  drop, relaxed SIMD left on and a stack trap reported as a guest trap.

The independent review required four corrections, all applied in the
follow-up:

- Relaxed SIMD is switched off explicitly. The pinned Cranelift profile
  enables it, and its results can differ between targets.
- A panic payload is dropped under a second unwind guard. A payload whose
  destructor panics no longer escapes the host boundary or skips quarantine.
- The start deadline is proven by a start function that returns in time after
  an epoch has already passed. An epoch-enabled store begins with a deadline
  that has already elapsed, so a spinning-start test alone could pass without
  the deadline being installed.
- A module-level dead-code expectation stays on the runtime module. Narrowing
  it to each crate-private item would repeat one reason across the private
  surface, and the test build still compiles that surface without the
  expectation, so unused production code is still reported there.

Two review points were recorded rather than changed. A typed export can
accept a function reference owned by another instance, and Wasmtime rejects
that before guest entry. The call still quarantines, reported as an internal
host fault, because passing that reference is a host-adapter bug. `define`
accepts numeric Wasm carriers only by contract. A reference type built for
another engine panics inside Wasmtime, and no adapter in this milestone
builds one.

Findings from the pinned release that shaped the implementation:

- Wasmtime reports a module needing a disabled proposal through the same
  untyped validation failure as malformed bytes. Both are
  `RuntimeError::InvalidModule`. A separate unsupported-proposal category
  needs a second validation pass with a direct `wasmparser` dependency or
  Wasmtime's `reexport-wasmparser` feature. That changes the selected feature
  set, so it waits for Nye.
- Wasmtime reports a limiter denial at instantiation and a cross-engine
  module as untyped text. Lappet checks the module's declared memories and
  tables and the engine identity itself, before instantiation, and classifies
  from those typed facts. The store limiter still enforces every limit.
- The Wasm 3.0 profile Cranelift enables accepts modules with several
  memories and tables. The per-instance count limit rejects them, not
  validation.
- `Config::wasm_backtrace` is deprecated in 50.0.1. The engine pins
  `wasm_backtrace_max_frames` to the release default of 20. Trap errors carry
  function names without the `backtrace` Cargo feature.
- The limit checks assume the standard 64 KiB page, so the custom page sizes
  proposal is switched off explicitly, as are Memory64 and relaxed SIMD.
  Multi-value stays on. The other proposal toggles follow the pinned
  Cranelift profile until compiled Moth output shows which of them it needs.
- Raw host imports use Wasmtime's dynamically typed `func_new`, so one
  wrapper catches panics and withholds any handle that could re-enter the
  guest. The measured difference is about 20 ns per scalar host call. That
  does not justify one typed wrapper per arity.
- A host fault keeps what the host reported. It drops the Wasm backtrace that
  Wasmtime attached while unwinding.
- The runtime foundation gave host imports no guest memory access. The staging
  table, delivered in milestone 3, is its first consumer.

**Phase 6 evidence.** Recorded on 13 November 2026 on one host, macOS AArch64
with Rust 1.99.0, from authored core Wasm fixtures. The numbers are not
compiled Moth and say nothing about the Moth host ABI. Timings are release
builds. A cold compilation uses a fresh engine each time. Preflight,
instantiation and teardown are timed only after one untimed call. Peak
resident memory is the process maximum from `/usr/bin/time -l` and includes
the test harness. Raw output stays local.

Published engine, release optimisation, epoch interruption on:

| Measurement | Median |
|---|---|
| Engine setup | 2.8 us |
| Cold module compilation | 938 us |
| Import preflight | 0.2 us |
| Instantiation | 3.6 us |
| Teardown | 3.5 us |
| Game frame, 1000 entity steps | 222 us |
| One entity step, including its export lookup | 0.2 us |
| Server request, 1 MiB written, teardown excluded | 223 us |
| Compute, Collatz to 100000 | 16.3 ms |
| Scalar host call, difference | 20 ns |
| Recursion depth before the stack fault | 10918 frames |
| Process peak RSS, game | 8.5 MB |
| Process peak RSS, server | 9.5 MB |

The smallest embedder links to 0.36 MB stripped without the runtime feature
and 8.05 MB stripped with it. Unstripped, those figures are 0.45 MB and
11.0 MB. A clean `lappet` build with the feature took 78 s in dev and 53 s
in release, against under 1 s with no feature.

Epoch interruption and the two optimisation levels, median:

| Measurement | Development, on | Development, off | Release, on | Release, off |
|---|---|---|---|---|
| Cold compilation | 738 us | 515 us | 938 us | 626 us |
| Game frame | 224 us | 224 us | 222 us | 224 us |
| Server request | 255 us | 213 us | 223 us | 229 us |
| Collatz to 100000 | 16.1 ms | 15.9 ms | 16.3 ms | 16.3 ms |
| Recursion depth | 10918 | 16377 | 10918 | 16377 |

The scalar host-call figure is the difference between two counted loops that
both store on every iteration. On the published engine the host-call loop is
20.7 ns per iteration and the loop without a host call is 0.7 ns, so the call
costs 20 ns. The recursion probe makes one host call per frame. The game and
server workloads do not recurse. A repeat of the release server samples was
223 us with the check and 224 us without it.

Pinned Cranelift profile is Wasm 3.0 plus wide arithmetic. Threads and GC are
absent because those Cargo features are off, and the engine then removes
exceptions with GC. Explicit toggles are multi-value on, Memory64 off, custom
page sizes off and relaxed SIMD off. The other profile toggles are not set
explicitly.

Policy lines, confirmed by Nye on 14 November 2026 and recorded in
[the runtime design](docs/design.md#runtime-strategy):

- Wasmtime `=50.0.1` with `std`, `runtime` and `cranelift`: confirmed. The
  feature costs about 7.7 MB of stripped code and a clean release build of
  about 53 s. No feature is proposed.
- Cranelift, set explicitly: confirmed. No measurement was required.
- Development optimisation `OptLevel::None` and release `OptLevel::Speed`:
  confirmed. This integer loop and the game step show no execution gap, and
  `Speed` adds about 0.2 ms to compiling this small module. `SpeedAndSize` is
  not proposed.
- Backtracking, no inlining, parallel compilation not compiled in: confirmed.
  Compile time does not block either workload.
- Proposal toggles: confirmed as pinned above. Compiled Moth output still has
  to show which of the unpinned toggles it needs.
- Linear memory ceiling 256 MiB: confirmed for these fixtures. The server
  workload peaked at 9.5 MB of process RSS after writing 1 MiB. They do not
  show that a server needs a lower default.
- One instance, one memory and one table: confirmed. No fixture needs more.
- Table elements 65,536: confirmed. The workloads use no table, and the limit
  tests use at most two elements.
- Wasm stack 512 KiB: confirmed. The probe still reaches about 11000 frames
  with a host call on every frame.
- `memory.grow` returns `-1` without a trap: accepted as the V1 default. The
  real-application gate confirms it against Moth's memory contract, and these
  fixtures do not replace that.
- Epoch interruption on, embedder-advanced ticks, no timer thread: confirmed.
  The game frame is the same with the check on or off. The server sample is
  slower with the check in the development build and not in the release build.
  Opt-in is not proposed.
- Fuel disabled: confirmed. No V1 consumer appeared.
- Wasm backtrace capture on, `backtrace` Cargo feature off: confirmed. Trap
  errors carry function names with the selected features.
- No Lappet timeout around a native host call: confirmed. The builder still
  owns host IO policy.
- A fault quarantines the instance: confirmed by the Phase 5 tests.

**Validation.** `just validate` passed on 14 November 2026 on macOS AArch64
with Rust 1.99.0 and Node 24.21.0. Formatting, Clippy, feature lanes, the
source audit and the first-party dependency audit were clean. All 7159
compiler unit tests and the remaining workspace tests passed, 3410 integration
results were correct and the docs check passed. The validated tree also
carried the facade-only build change. `cargo test -p lappet --features runtime`
passed 71 tests with 8 measurements ignored. The independent end-to-end review
required no production change.

## 3. Build host boundary tables and fault mechanics

Completed on 14 November 2026.

**Checkpoint.** Accepted on 13 November 2026. The
[host resource and context API](docs/host-resources.md) owns resource
ownership, access, per-call context and the sendable and thread-bound instance
kinds. V1 is strict: every boundary violation quarantines the instance. The
staging table, the token generator and their fault handling follow the
[adapter responsibilities](docs/host-abi.md).

**Delivery.** The per-instance staging table, the process-wide token generator,
the per-instance resource table with call-scoped access guards, the per-call
host context slot and both instance kinds are built and wired into fault
quarantine and teardown. No value conversion, descriptor matching or manifest
preflight is included.

**Evidence.** The fixture-level deferred tests in the adapter responsibilities
have coverage: ticket lifetime and take checks, stale, closed, other-instance,
wrong-type and conflicting tokens, physical import mismatches and a trapping
scope helper. Each runs under both optimisation levels. Fixtures follow the V1
protocol by hand and prove Lappet's mechanics only.

**Progress.** Phase 0, 13 November 2026: the runtime foundation is in this
tree and the branch is not behind `main`. Wasmtime 50.0.1 does not require
store data to be `Send` for construction, limits, epoch deadlines,
instantiation or a typed call, so a thread-bound instance can use the calls
Lappet makes. One drift: the plan calls token-generator exhaustion a host
fault, but the resource API returns that as an insertion error because no
guest call is running. The API wins. Phase 1: the staging table issues
nonzero tickets that are never reused, keeps a failed take, and reads both
byte ceilings from `InstanceLimits`. A fault clears the instance's table.
Phase 2: `moth-abi-v1.staged_text_take` copies staged text into guest memory
and consumes the ticket. A bad take, a live ticket at another import and a
live ticket at entry return are boundary faults that quarantine the instance
and clear the table. Fixture tests pass under both optimisation levels.
Phase 3: one process-wide token generator and one resource table for both
instance kinds. A sendable instance rejects a value that is not `Send`. A
thread-bound instance is not `Send` and can store one. Close returns the
value, drain hands ownership back and teardown drops what remains in
creation order. A faulted instance can still close and drain. The live-entry
ceiling is 1,048,576. Phase 4: a host call checks every token before any Rust
reference exists. Shared borrows coexist and an exclusive borrow conflicts
with any other borrow of that entry. The borrows are released when the call
returns or panics. Stale, closed, other-instance, wrong-type and conflicting
tokens are boundary faults and the host body does not run. Phase 5: context
moves into the instance for one call and back to the holder on every outcome,
including a caught host panic. A missing or wrong-type context is a host fault
and the host function does not run. A thread-bound instance accepts context
that is not `Send`. A scoped export opens, reserves, calls once, copies the
returned span and closes. Status `1` is a declared error: the scope closes and
the instance stays usable. A trap, a noncanonical status or a noncanonical span
skips close and makes no further guest call. A bad token argument is rejected
before guest entry and the instance stays usable.

**Decision.** `HostState` stays the instance-owned slot the crate-private
import closures use. It is not a third ABI role. Resources and context are the
two roles a host function receives. Adapters will see those through one call
view and will not grow a parallel state generic.

**Phase 6 evidence.** Recorded on 13 November 2026 on the same host, from
authored fixtures and one release run. The numbers are not compiled Moth.
Staging and the resource table do not use Cranelift. The guard figure is the
difference between a bare host call and a host call that passes one token and
borrows it, so it includes the argument and the lookup.

| Measurement | Median |
|---|---|
| Stage 16 bytes | 42 ns |
| Stage 16 MiB | 387 us |
| Guarded host call above a bare call, release | 84 ns |
| Guarded host call above a bare call, development | 125 ns |
| Lookup of a live token in a full table | 78 ns |
| Resident growth at 1,048,576 live entries | 126 bytes per entry |

A full table of `u32` values grew the process from 7.7 MB to 137 MB resident.
That growth includes the allocator, not only the table nodes. Closing one
entry freed its slot. One 16 MiB staged entry is under the 64 MiB live
ceiling. The game and server fixtures stage no host string.

Policy lines, confirmed by Nye on 14 November 2026 and recorded in
[the host ABI adapter responsibilities](docs/host-abi.md):

- Staged text per entry, 16 MiB: confirmed as the V1 default. The workloads
  stage no host string. A 16 MiB stage succeeds in 387 us. The timer covers
  `stage` only. The guest copy no longer clones the entry before writing it,
  and that copy was not in this figure. They do not show a realistic string
  that needs another cap.
- Staged text live per instance, 64 MiB: confirmed for these fixtures. The
  largest staged entry measured is 16 MiB. They do not show a peak that needs
  a lower default.
- Live resource entries, 1,048,576: confirmed as a ceiling. Filling it costs
  about 129 MB of resident growth, 126 bytes per live entry, and a lookup is
  78 ns. No lower default is proposed.
- Closed entries removed at close: confirmed. A full table accepted another
  entry after one close. A long-lived instance pays for live entries only.
- Token generator, one process-wide counter from 1: confirmed. Accepted. No
  measurement was required.
- Ticket generator, one per-instance counter from 1, never reused: confirmed.
  The ABI requires uniqueness within the instance only. No measurement was
  required.

**Validation.** The milestone 2 `just validate` run covers this milestone. The
independent review required one test, a trap in scope close, and it passes
under both optimisation levels.

## 4. Extract the shared host contract crate

**Delivered.** 13 November 2026. `HostType` and `HostFunction` live in
`crates/moth-host`, which has no dependencies. The compiler re-exports that
crate as `moth::host` and keeps the registry conversion beside its only
caller. `moth-host` is an extracted package with no permitted workspace
dependency and a feature-free lane. Lappet does not depend on it yet, because
the runtime has no consumer for the descriptors. That edge waits for the host
adapters. `HostType` is non-exhaustive, so the compiler's match has a
rejecting arm no current variant reaches.

`just validate` passed on this tree: formatting, Clippy, feature lanes,
source audit and first-party dependencies were clean, and the workspace unit
tests, integration cases and docs check passed. The independent review found
no change to a builder author's public path or rejection message.

`CONTRIBUTING.md` and other `validation.mtf` lists still omit `moth-host`.
The approved documentation edit was the first-party dependency paragraph only.
A later closeout edit corrected [the runtime design](docs/design.md) to record
the extracted crate.

## 5. Finish the native binding and artefact handoff

**Delivered.** 15 November 2026.

**Checkpoint.** The first concrete Rust-to-Moth ABI is accepted. It uses the
existing manual host descriptor path and covers scalars, strings, optionals,
errors, explicit context and opaque resources. This milestone delivers the
descriptor and requirement contract for scalars, strings, optionals and
errors. Context and opaque resources remain required V1 features and their
descriptors move to milestone 7. Byte buffers, records, choices, collections,
multi-slot host imports and resource results are deferred beyond V1 and each
needs an approved shape before expanding the restricted profile.

**What exists.**

- The public builder SDK, `moth::builder`. A definition registers source
  packages, host packages and one config section through
  `BuilderDefinition::new`. `build_project` compiles a project and calls
  `ProjectBuilder::build` with a `CompiledProject`, which gives typed
  `settings` and an owner-bound `ProjectPackage`. `BuildArtifacts::add` records
  byte artefacts and `BuiltProject::write_outputs` publishes them through the
  central output writer, its manifest and stale cleanup.
  `BuiltProject::into_warnings` finishes a build that writes nothing.
- `BuildContext::compile_wasm` returns `ErrorKind::UnsupportedCapability`. It
  never produces placeholder Wasm.
- The `moth-builder-fixture` workspace crate depends only on `moth`. It
  defines a builder, compiles a real project, publishes one artefact and
  observes the capability error. The first-party dependency audit holds it to
  that single dependency.
- The shared contract in `crates/moth-host`, re-exported as `moth::host`:
  `HostType`, `HostSignature` and `HostFunction` descriptors,
  `HostBindingIdentity`, `HostImportRequirement`, `HostExportRequirement`, the
  checked `HostRequirements` value in canonical order, the `HostExports`
  selection and `preflight`.
- Compiler-owned requirement production, `host_requirements` in
  `src/builder/host_requirements.rs`. It resolves each selected export against
  the facade's public interface, then lists each host binding reachable from
  the selected exports once by canonical identity. An unselected function adds
  no import. It is internal until `compile_wasm` can lower a package.

**Evidence.** `crates/moth-builder-fixture` runs the whole workflow from
outside the compiler. `tests/builder_sdk.rs` owns each public rule: config
sections and settings, artefact paths and collisions, output roots, failed
builds, ownership conflicts, repeated builds and stale cleanup.
`src/builder/tests/host_requirements_tests.rs` compiles real projects and
checks the produced requirements: canonical identity and order, selected-root
reachability, each rejected selection and capability isolation between two
definitions. The preflight rejects a missing implementation and a wrong type,
arity or return channel against those produced requirements.
`crates/moth-host` owns the preflight, selection and requirement rules alone.
None of this proves a physical import name, export wrapper, frame or native
execution, which need emitted Wasm.

**Decisions.** Nye decided these on 15 November 2026.

- The requirements handoff stays internal to the compiler. The future
  `compile_wasm` result exposes the authoritative host manifest, which carries
  these semantic requirements with the physical ABI facts. No temporary public
  accessor exists.
- A builder selects exports by facade public name with an explicit Wasm export
  name, as in `HostExports::new().export("update", "game_update")`. Only
  selected facade functions become host-callable. Nothing is exported
  implicitly and there is no `moth_start`. A missing name or an unsupported
  declaration or signature is a `Build` error.
- Host context and opaque resources stay required V1 features. Their
  descriptor and compiler registration support moves to the milestone 7 gate.

**Context and resource descriptor gap.** `HostType` and `HostFunction` describe
scalars, strings, one optional layer and the builtin `Error!` channel. They
cannot yet describe an opaque resource parameter, its shared or exclusive
access or a host context parameter. A signature outside the vocabulary fails
explicitly at registration or export selection and never yields a partial
requirement. The eventual work extends these descriptors, the one conversion
table in `src/builder/host_conversion.rs` and `preflight`. It adds no second
binding system. It needs:

- `i64` resource-token carriage in the compiler, replacing the `i32` handle
  note on `ExternalAbiType::Handle`
- opaque type registration on a host package and the compiler type it creates
- a descriptor form for a resource parameter with its access mode and for one
  context parameter that the Moth caller never supplies
- the manual registration spelling, since the accepted
  `#[moth::host_type]` and `#[moth(context)]` forms are macros that don't exist

**Runtime handoff.** This is what milestone 7 picks up.

- Descriptor API: `moth::host` as listed above. Lappet gains its dependency on
  `moth-host` when the first adapter consumes a descriptor.
- Import and export check: `preflight` compares a `HostRequirements` value with
  declared `HostImplementation` descriptors. It reports a missing
  implementation, a duplicate declaration and each parameter, success value
  and error channel mismatch by canonical identity. An unused implementation is
  not an error. Exports have no Rust-side declaration until the Rust call API
  is designed.
- Export selection: `compile_wasm(package)` takes no selection today because
  it cannot lower. When it can, the builder passes a finished `HostExports`
  selection with the package and the result carries the manifest. The builder
  guide's example still shows the call without a selection.
- Existing conversions: none at value level. `src/builder/host_conversion.rs`
  converts descriptors to compiler signatures and back. Lappet's staging,
  resource and scope mechanics from milestone 3 are ready for adapters.
- Moving the fixture-level preflight: Lappet's `HostImports::prepare` proves
  only that Wasmtime can resolve the physical imports. When the manifest
  lands, adapter installation takes the manifest's requirements, runs
  `preflight` against the registered implementations first and then defines
  exactly the required imports on the linker by the manifest's physical names.
  The physical link check stays as the last line of defence.
- Adapter tests: no adapter library was written, which the plan allowed.
  Nothing in the SDK or the preflight calls a value conversion yet. The
  [deferred tests](docs/host-abi.md) each name their covering test or what
  they wait for. Tests 2 and 3 and the manifest half of test 6 wait for
  compiler lowering. The value half of test 1 and the `String` conversion of
  test 4 wait for the host adapters, which arrive with milestone 7. Every test
  needs a Wasmtime store, which milestone 2 already provides, so none is
  blocked on Wasmtime.

**Still missing on `main`.** Every row of [Waiting on main](#waiting-on-main)
that names milestone 7. The physical half of the manifest also needs the
result-frame layout, the physical Wasm signatures, the memory and helper
identities and the compatibility fingerprint.

**Known limits.**

- The central output writer is not transactional. Preflight and ownership
  checks run before any change and stale cleanup runs only after every
  artefact is written. A filesystem failure while writing can leave some new
  files, and a failure persisting the manifest happens after stale removal.
  This is shared with the HTML builder.
- A write failure discards the build's staged compiler warnings, as the CLI
  does.
- `build_project` accepts a project directory only. A single source file is a
  `Build` error before compilation.
- The source and host package overlap check does not cover JS packages
  registered during compilation.

**Documentation follow-ups.** These are reported, not edited.

- `docs/build-system-design.md` and `docs/compiler-design-overview.md` don't
  describe the host manifest, host requirement production or the host-binding
  package backing.
- `creating-a-builder.mtf` doesn't describe the reserved first-party builder
  identities.
- `validation.mtf`, `testing.mtf` and `CONTRIBUTING.md` omit `moth-host` and
  `moth-builder-fixture` from their package and lane lists and don't describe
  the public SDK consumer rule in the first-party dependency audit.
- `crates/lappet/README.md` and the intro of `docs/host-resources.md` describe
  the runtime as it stood before milestone 3.
- `creating-a-builder.mtf` shows `compile_wasm(package)` with no export
  selection and says it compiles the selected assembly and its reachable
  dependencies. The decided shape roots the artefact at explicitly selected
  exports.

**Validation.** `just validate` passed on 15 November 2026 on macOS AArch64
with Rust 1.99.0 and Node 24.21.0, on the final tree after the review
corrections. Formatting, Clippy, feature lanes, the source audit and the
first-party dependency audit were clean. All 7195 compiler unit tests passed,
with 44 SDK tests, the fixture crate's workflow test, 25 `moth-host` tests and
the remaining workspace tests. 3410 integration results were correct and the
docs check passed. `cargo tree` finds no path from `moth` or the fixture crate
to Lappet or Wasmtime. Independent architecture, test and integration reviews
ran before closeout and their required findings are applied.

## 6. Establish official Core conformance

**Checkpoint.** Inventory the complete public Core catalogue and distinguish
compiler-internal helpers from optional host facilities. Record the exact
portable behaviour, documented deviations, capability rejections and declared
unsupported results for browser, game and headless server hosts. Define the
trusted official-environment identity used by package admission. A builder's
self-declaration or use of Lappet cannot grant certification.

**Accepted direction.** The compiler selects direct lowering, a compiler
helper or a native Core provider by canonical identity and target capability.
The prerequisite inventory must resolve opaque return values from `@core/time`
and `@core/io` without widening general host bindings. Official certification
comes from the trusted first-party catalogue and complete applicable Core
conformance, not a builder name or the presence of native implementations.

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
work neither supplies all these capabilities nor implies WIT support. The gate
also needs the host context and opaque resource descriptor work listed under
[milestone 5](#5-finish-the-native-binding-and-artefact-handoff), which V1
requires.

The pre-roadmap readiness gate must also be completed. Its selected-export
contract requires the compiler manifest, no implicit Wasm start function and
an approved bounded-copy policy before a public Rust call adapter ships.

**Delivery.** Compile through public Moth services, install matching Rust host
implementations and invoke explicitly selected exports through checked,
manifest-validated typed handles. Preserve multiple ordered success slots,
builtin declared errors and distinct runtime faults. Extend the `moth-host`
descriptors, the compiler's conversion table and `preflight` to host context
and opaque resource parameters. Replace the fixture-only export driver rather
than maintaining a second semantic signature or call path. Prove one embedded
game lifecycle and one server lifecycle without adding a scheduler or web framework.
Keep rendering/output/resource planning with the builder and build system.

**Evidence.** Run actual Moth programs, not hand-authored ABI substitutes.
Repeat every fixture-level boundary test from milestone 3 and deliver the
value-level evidence deferred from milestone 5: exact integer ranges, finite
floats, Byte versus U8 at runtime, invalid Unicode and lengths, optional
absence versus failure, ordered results, exactly-once calls, independence of
returned storage and every allocation and cleanup failure path.
Cover source-to-host-to-source values, errors, lifecycle cleanup, isolated host
state and capability rejection. Benchmark all four combinations of Moth
output profile and native strategy under the shared optimisation authority.
Never mutate published compiler results to make runtime binding succeed.

**Documentation.** Extend the embedding example to use a real Moth program
when the compiler and runtime handoffs support it. Keep the example checked
against the public embedding API rather than documenting hypothetical calls.

## 8. Consume compiler-resolved package catalog

This lane is independent of Wasmtime. The compiler now owns the package
declaration, local library selection and canonical catalogue foundations as a
separately sequenced implementation item. Lappet must not reimplement them.

**Prerequisites.** The compiler delivers exact ObVer-qualified declarations
in `@config.moth`, project-local library selection, source exclusion and a
deterministic immutable resolved package graph. It also supplies canonical
public interfaces, separate dependency config/input scopes and stable package,
capability and ABI fingerprints. The common native readiness gate must be
complete before this milestone begins.

**Checkpoint.** Identify the first real Lappet package consumer of that
catalogue and the minimal existing compiler/builder API it needs. Confirm how
the future manager receives the selected package/build identities and
compatibility facts without a new resolver, path scanner or parallel graph.
The package manager does not create a global directory, download anything or
implicitly add packages to a project in this milestone. If the compiler
catalogue is already sufficient, delete the redundant proposed adapter rather
than create a future-only wrapper.

**Delivery.** Connect the needed manager-facing package records through the
approved compiler API when a real consumer is present. Preserve the compiler
catalogue's exact ObVer build identities, aliases, direct dependency edges,
source/compatible-precompiled metadata and immutable public-interface facts.
Keep any future managed package installation separate from local manual package
resolution. Do not parse `@config.moth`, reconstruct package identity,
search `packages/` again or introduce version ranges, lockfiles or
source-specific fallback paths.

**Evidence.** Tests cover exact build identity and labels, public namespace
routes, aliases, declared direct-only visibility, independent package configs,
private project globals, diamond graphs, deterministic graph order, source
boundary isolation, incompatible cached artefacts and output resource
identity. Demonstrate that a manager consumer cannot access an undeclared
transitive dependency or alter a compiler-selected package by spelling its
path differently. Do not repeat compiler tests solely to add a second owner.

## 9. Derive package eligibility and dependency evidence

**Checkpoint.** Consume the compiler's exact ObVer package/build identity and
immutable resolved dependency closure. Specify how compiler/provider facts establish public Core use,
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

**Checkpoint.** Approve version ranges (if supported), lockfile/override policy,
metadata provenance,
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
