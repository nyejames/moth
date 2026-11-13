# Lappet Wasmtime runtime foundation plan

This plan delivers Lappet roadmap milestone 2, "Establish Wasmtime execution".
It gives `crates/lappet` its first code: a Cranelift-only Wasmtime engine,
compiled modules, isolated instances, resource limits, execution deadlines and
fault quarantine, proven with hand-authored core Wasm fixtures. Delete this
plan in the commit that completes it and update `crates/lappet/roadmap.md` in
the same commit.

## Status

- Status: ready, not started.
- Current slice: none.
- Blockers: none.
- Next action: Phase 0.

## Goal

A Rust test inside `crates/lappet` compiles a core Wasm fixture, instantiates
it with a host import, calls an export under both native optimisation levels
and observes every failure as a classified Lappet error. Limits and deadlines
are enforced, a faulted instance can't be called again and the provisional
policy values below are either confirmed or revised from recorded evidence.

This plan establishes runtime mechanics only. It proves nothing about the Moth
ABI, compiled Moth output or Core support.

## Prerequisites

None on `main`. Every change stays inside `crates/lappet`, the workspace
lockfile and the feature-lane declarations. No file under `src/` changes.

## Read first

Follow the root `AGENTS.md` and `crates/lappet/AGENTS.md` routes for
"Wasmtime, execution and embedding" and "Tests and validation". These own the
contracts this plan consumes:

- `crates/lappet/docs/wasmtime-integration.md`: the pinned release, the small
  API surface, ownership rules and the excluded features.
- `crates/lappet/docs/design.md`: `Runtime strategy`, `Instances, resources
  and scheduling` and `Capabilities and failures`.
- `crates/lappet/docs/host-abi.md`: `Faults`, for the fault categories this
  plan classifies.
- `docs/developer-docs/wasm-optimisation/wasm-optimisation.mtf`: the
  compiler/runtime measurement policy.
- `docs/developer-docs/style-guide/testing.mtf` and `validation.mtf`,
  including `Feature lanes` and `testing.mtf > Feature and platform lanes`.

Verify every Wasmtime API against the pinned release's source or versioned
rustdoc, never against upstream `main`.

## Global constraints

- Wasmtime is an optional dependency behind a new `runtime` Cargo feature.
  `lappet` with no features stays dependency-free and still builds and tests.
- Keep the dependency direction. `moth` never depends on `lappet` or
  `wasmtime`. `moth-cli` doesn't enable `runtime` in this plan, because no
  builder consumes it yet. `just first-party-deps` must stay green.
- This plan adds no dependency on the `moth` compiler crate.
- Cranelift only. No Winch, Pulley selection, WASI, component model, async,
  cache, serialisation or raw Cranelift flags.
- The public API stays small: the engine, the native optimisation choice, the
  limits, compiled modules and the error types. Raw import installation and raw
  export calls stay `pub(crate)`. The public call surface arrives with the
  manifest-driven API, so no raw-Wasm embedding API is published by accident.
- No `unsafe`. No guest memory view outlives one host call.
- Type and function names in this plan are suggestions. Keep them descriptive
  and record the final names in the roadmap evidence.
- Every executable change needs a red then green test. Run each test under
  both `OptLevel::None` and `OptLevel::Speed` unless the phase says otherwise.
  Never report a test that wasn't run.
- Documentation edits are limited to the ones this plan names.

## Process for every phase

1. Write the failing tests, implement the smallest complete slice and remove
   anything it makes obsolete.
2. Run the phase's focused checks.
3. Run the phase review with a fresh reviewer that didn't write the code. The
   reviewer reads the diff against the phase's exit criteria and the owning
   documents, and checks ownership, duplication, unused abstraction, failure
   classification, test honesty and public surface. Record each finding's
   disposition in the commit message or the roadmap, not in this plan.
4. Fix every blocker, rerun the focused checks and commit. Push to
   `lappet-work`.
5. Stop for Nye on any unresolved design or public API choice, and on any
   evidence that contradicts a provisional value.

Focused checks, run from the worktree root:

```bash
cargo test -p lappet
```

```bash
cargo test -p lappet --features runtime
```

```bash
cargo clippy -p lappet --all-targets --features runtime
```

```bash
cargo fmt --all -- --check
```

```bash
just first-party-deps
```

## Provisional policy values

Nye asked for provisional defaults with evidence gathered during
implementation. Implement these as the defaults. Phase 6 measures them and
Phase 7 records the confirmed values in `crates/lappet/docs/design.md`. A
value changes only with recorded evidence and Nye's confirmation.

| Policy | Provisional value | Reason | Evidence to gather |
|---|---|---|---|
| Wasmtime release and features | `=50.0.1`, `default-features = false`, features `std`, `runtime`, `cranelift` | Smallest graph that compiles and runs core Wasm | Feature tree, clean build time and linked size against a no-feature build |
| Compiler strategy | `Strategy::Cranelift`, set explicitly | `Strategy::Auto` isn't a policy | None |
| Development native optimisation | `OptLevel::None` | Fast native compilation | Cold compile time and execution time against `Speed` |
| Release native optimisation | `OptLevel::Speed` | Optimised native code | Same comparison. Note `SpeedAndSize` only if `Speed` code size is a problem |
| Register allocator, inlining, parallel compilation | Backtracking, `Inlining::No`, not compiled in | Upstream production defaults | None unless compile time blocks a use case |
| Wasm proposals | Multi-value on, Memory64 off, threads and GC not compiled in. Set every proposal toggle Lappet relies on explicitly | An upgrade can't silently change the profile | Record each toggle's pinned default and the explicit choice. The Moth backend's real requirements are confirmed at the real-application gate |
| Linear memory ceiling per instance | 256 MiB | Bounded by default, far below the 4 GiB Wasm32 limit | Peak memory of both workloads. Whether a server needs a lower default |
| Instances, memories and tables per store | 1, 1 and 1 | One Moth module per instance in V1 | Whether any fixture or expected Moth output needs more tables |
| Table elements | 65,536 | Bounds indirect-call tables | Largest table in the workloads |
| Wasm stack | 512 KiB, Wasmtime's default | No evidence yet for another value | Deepest recursion each workload reaches before a stack trap |
| Memory growth failure | `memory.grow` returns `-1`, no trap | Wasmtime's default. Moth's allocator decides what failure means | Confirm against the memory contract at the real-application gate |
| Execution interruption | Epoch interruption enabled on every engine. Each guest entry takes an optional deadline in ticks. With no deadline Lappet sets the furthest deadline explicitly. The embedder advances the epoch. Lappet starts no thread | A game needs a frame budget and a server a request budget, and neither wants a hidden timer thread | Epoch instrumentation overhead under both optimisation levels on both workloads. If it is material for the game workload, propose engine-level opt-in |
| Fuel | Disabled | Deterministic metering has no V1 consumer | None |
| Backtraces | Wasm backtrace capture on, Wasmtime's default. The `backtrace` Cargo feature stays off | Trap attribution without extra dependencies | What a trap error actually carries with the selected features |
| Blocking native host calls | No Lappet timeout. The builder owns host IO policy | Neither fuel nor epochs interrupts native code | None. State it in the public documentation |
| Fault disposition | Any trap, limit or deadline exhaustion or host fault quarantines the instance, which is then dropped | Accepted V1 decision | Tests in Phase 5 |

The staging-table and resource-table limits belong to the host boundary work
and aren't set here.

## Phase 0: activation and baseline

- [ ] Sync `lappet-work` with `main`. Record the baseline commit in working
  notes, not in this file.
- [ ] Check crates.io for a newer 50.0.x patch release. Take it if one exists
  and update the version and release date in
  `crates/lappet/docs/wasmtime-integration.md`. Don't move to another major
  version inside this plan.
- [ ] Confirm the local Rust toolchain meets the pinned release's
  `rust-version`.
- [ ] Read the routed documents above. Report any drift between them before
  writing code.
- [ ] Set the roadmap's current state to this plan and phase.

Exit: baseline recorded, release confirmed, no unreported drift.

## Phase 1: dependency, feature gate and crate layout

- [ ] Add the `runtime` feature and the optional `wasmtime` dependency to
  `crates/lappet/Cargo.toml` with the exact version and features in the policy
  table. Add a WAT-to-binary dev-dependency for fixtures only, pinned to a
  release compatible with the selected Wasmtime. Production code never parses
  WAT.
- [ ] Create `crates/lappet/src/runtime/` behind `#[cfg(feature = "runtime")]`
  with a `mod.rs` that documents the module's ownership, exclusions and data
  flow. Create `crates/lappet/src/runtime/tests/` and a `fixtures/` directory
  under it for `.wat` sources.
- [ ] Write one failing test that compiles a trivial module, then make it pass
  with the minimum engine code. This proves the feature graph links.
- [ ] Add a feature lane for `lappet` with `runtime` and one for `lappet` with
  no features, following `validation.mtf > Feature lanes` and the lane table
  in `testing.mtf`. Confirm `just feature-lane-check` passes.
- [ ] Record `cargo tree -p lappet --features runtime -e features`, the
  clean-build time of `lappet` with and without `runtime` and the count of new
  lockfile packages. Keep raw output local and put the summary in the roadmap.
- [ ] Prove the boundaries: `cargo tree -p moth -i wasmtime` and
  `cargo tree -p moth-cli -i wasmtime` both report no path, and
  `just first-party-deps` passes.
- [ ] Update `crates/lappet/README.md`, the crate documentation in `lib.rs`
  and the `index.md` Lappet entries so none of them still says the crate is
  documentation-only or dependency-free.

Review focus: the feature really isolates Wasmtime, no unneeded Wasmtime
feature is enabled, the lockfile change contains only the expected graph and
the lanes run what they claim.

Exit: both lanes pass, boundary checks pass and build-cost numbers are
recorded.

## Phase 2: engine and compiled modules

- [ ] Implement the native optimisation choice with two values, development
  and release, mapped to `OptLevel::None` and `OptLevel::Speed`. It is
  independent of Moth's build profile and takes no Moth type.
- [ ] Implement the engine wrapper. It builds one `Config` from the policy
  table, sets every relied-on option explicitly and returns a classified
  configuration error when `Engine::new` rejects it.
- [ ] Implement the compiled module wrapper over `Module::new` for binary
  Wasm. It is immutable and shareable across instances of the same engine.
  Using a module with another engine is a classified error, never a panic.
- [ ] Start the error type with the phases Wasmtime distinguishes: engine
  configuration, invalid module bytes and unsupported proposal. Later phases
  extend it. Keep Wasmtime's error as the source without exposing Wasmtime
  types in public signatures.
- [ ] Tests: valid module compiles under both levels, truncated and malformed
  bytes are rejected, a module using a disabled proposal such as Memory64 is
  rejected, a module used with another engine is a classified error and
  two instances share one compiled module without recompiling it.

Review focus: no Wasmtime type leaks through the public surface, each default
is set in one place and the error type carries the cause without string
matching.

Exit: the tests above pass under both levels.

## Phase 3: instances, host imports and export calls

- [ ] Implement the instance. It owns its `Store`, its host state and its
  limits. Dropping it frees the guest. Nothing mutable is shared between
  instances.
- [ ] Implement crate-private raw host import registration on a `Linker`,
  resolved once through `instantiate_pre` so repeated instantiation reuses the
  import check. Keep shadowing off. Never use
  `define_unknown_imports_as_traps` or
  `define_unknown_imports_as_default_values`.
- [ ] Classify instantiation failures separately: a missing import, an import
  with the wrong physical signature and a trap in the Wasm start function.
- [ ] Implement crate-private typed export calls. Classify a missing export, a
  signature mismatch and a trap during the call.
- [ ] Prevent guest re-entry structurally. A host import receives the host
  state and the caller's memory only. It receives no handle that can call an
  export, and an export call needs exclusive access to the instance.
- [ ] Tests: the scalar import and export round trip from the reference
  sketch, two instances of one module with isolated host state and memory,
  each instantiation failure, each call failure, a start-function trap and
  host state dropped exactly once at teardown.
- [ ] Add the tested embedding program the roadmap asks for as one test file
  that reads top to bottom: engine setup, a host import, instantiation, a
  typed call and failure handling. Link it from the sketch in
  `crates/lappet/docs/wasmtime-integration.md` and state there that the raw
  surface is crate-private.

Review focus: store identity can't be confused through the wrapper API, no
default stub exists for a missing import, re-entry is impossible by
construction and the wrappers add an invariant, not just a navigation step.

Exit: the tests above pass under both levels.

## Phase 4: limits and execution deadlines

- [ ] Implement the limits value with the provisional defaults and install it
  on every store through `Store::limiter` before instantiation. Apply the Wasm
  stack limit on the engine.
- [ ] Implement the deadline policy from the table. Set an explicit deadline on
  every store before any guest code runs, because an epoch-enabled store
  otherwise starts with an elapsed deadline. Expose the epoch advance to the
  embedder through the engine wrapper.
- [ ] Classify each exhaustion as a distinct limit fault: memory ceiling at
  instantiation, denied `memory.grow` as the guest observes it, table growth,
  stack overflow and deadline reached.
- [ ] Tests: a module whose initial memory exceeds the ceiling is rejected,
  growth past the ceiling returns `-1` to the guest and the instance keeps
  running, unbounded recursion ends in a classified stack fault, an infinite
  loop ends in a classified deadline fault once the embedder advances the
  epoch, a call with no deadline completes and a second memory or table is
  rejected.

Review focus: limits are installed before any guest code can run including the
start function, no limit depends on Wasmtime's 10,000-object defaults and
blocking host calls are documented as outside every limit.

Exit: the tests above pass under both levels.

## Phase 5: fault classification and quarantine

- [ ] Complete the error type so one value distinguishes configuration
  failure, invalid module, link failure, start trap, guest trap, limit fault
  and host fault. A host fault covers a host import that returns an operational
  failure or panics. Catch an unwinding panic at the host-call boundary and
  report a host fault. None of these is ever a Moth error.
- [ ] Implement quarantine. Any guest trap, limit fault or host fault marks the
  instance faulted. Every later call on it returns a classified
  "instance faulted" error without entering the guest. A denied
  `memory.grow` that returned `-1` isn't a fault.
- [ ] Tests: each fault category quarantines the instance, a quarantined
  instance refuses calls without running guest code, dropping a quarantined
  instance releases host state exactly once, a sibling instance of the same
  module is unaffected and a host panic is reported as a host fault.

Review focus: the categories match the fault table in
`crates/lappet/docs/host-abi.md`, no path converts a fault into a success or a
retry and quarantine is one owned state, not a flag checked in several places.

Exit: the tests above pass under both levels.

## Phase 6: policy evidence

This phase gathers the evidence for the provisional values. It records
measurements, not pass or fail gates.

- [ ] Write two authored workloads as fixtures:
  - game-like: one long-lived instance, many short export calls per frame, each
    making several scalar host calls and touching linear memory
  - server-like: a fresh instance per request from one shared compiled module,
    one call that grows and writes memory, then teardown
- [ ] Add a compute-bound export, such as an integer loop, so the gap between
  the optimisation levels is visible in isolation.
- [ ] Write the measurements as ignored tests run in a release build. Measure
  separately: engine setup, cold module compilation, import preflight,
  instantiation, repeated guest calls, scalar host-call cost, teardown and
  peak resident memory. Run each under both optimisation levels and with epoch
  interruption on and off.
- [ ] Record the Rust build cost and linked size from Phase 1 beside them.
- [ ] Write the summary table and host description into the roadmap's
  milestone 2 evidence. Keep raw data local. State plainly that the numbers
  come from one host and from authored fixtures, not from compiled Moth.
- [ ] For each row of the policy table, write one line: confirmed, or the
  proposed change with its evidence. Stop for Nye's confirmation.

Review focus: each measurement isolates the state it claims to measure, cold
and reused states aren't mixed, the summary makes no claim beyond the
measured host and no measurement doubles as a correctness test.

Exit: evidence recorded and every provisional value confirmed or revised by
Nye.

## Phase 7: integration and closeout

- [ ] Record the confirmed policy values in `crates/lappet/docs/design.md`
  under `Runtime strategy` and `Capabilities and failures`, replacing the
  wording that defers them to a checkpoint.
- [ ] Update `crates/lappet/docs/wasmtime-integration.md` with findings that
  changed an implementation choice. Don't copy upstream documentation.
- [ ] Update `crates/lappet/roadmap.md`: delivered status, evidence, the final
  public names and the first action of the next milestone.
- [ ] Update `index.md` for the new modules.
- [ ] Run `just validate` and record the actual result.
- [ ] Run an independent end-to-end review covering ownership, the public
  surface from an embedder's view, failure categories, test honesty and both
  the game and server cases. Fix blockers.
- [ ] Delete this plan in the completing commit.

Exit: the roadmap names the next milestone as current and no document still
describes Lappet as having no runtime.

## Out of scope

- Value conversion, descriptors, the host manifest and import matching by
  canonical identity.
- The staging table, the resource table and host context.
- Any public API for calling exports or installing host functions.
- Native Core implementations and package management.
- Platform evidence beyond the implementing host.
