# Lappet native readiness and hardening implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the confirmed Lappet runtime defects, establish an actionable Core implementation and certification contract and make the native adapter handoff ready before starting further Lappet roadmap milestones.

**Architecture:** Keep the compiler responsible for semantic Core identities, provider selection, host manifests and validated memory layouts. Keep Lappet responsible for native execution, adapter checks and runtime faults. Keep official builder certification in a trusted Moth distribution path, separate from custom builder identity and output ownership.

**Tech Stack:** Rust workspace, `moth-host`, `lappet`, Wasmtime 50.0.1, compiler/build-system services, Core reference documentation and focused Cargo tests.

**Spec:** `crates/lappet/docs/design.md`, `crates/lappet/docs/host-abi.md`, `crates/lappet/docs/package-manager.md`, the native host ABI and the canonical Core references.

## Status and boundaries

- Status: queued, hard prerequisite for Lappet milestones 6 through 10, after the compiler's project-local package libraries, exact ObVer build selector and resolved dependency graph foundations are delivered.
- Current slice: recheck the baseline and current runtime behaviour after syncing with main, then complete focused runtime hardening.
- Blockers: the shared compiler package-resolution prerequisite. Selected-export adapters also wait for compiler-produced native host ABI capabilities.
- Next action: once the compiler foundation lands, verify the reported runtime defects against the current source and start Phase 1 with focused failing tests.

This plan prepares the roadmap. It does not implement the full Core catalogue,
the manifest-driven adapter or managed package acquisition. Those belong to
their gated roadmap milestones. No task may substitute authored Wasm fixtures
for the missing compiler handoffs or publish a hypothetical Rust API as usable.

External project dependency declarations, ObVer parsing, local package-library lookup and independent dependency graph construction belong to the compiler's accepted package-resolution foundation. Do not duplicate or reimplement that work in Lappet. Before beginning Phase 1, confirm that the compiler foundation is delivered on main, including fixed project-local search roots, direct-only visibility and exact build identities.

## Global constraints

- Work in a clean isolated Lappet worktree on the branch selected at activation. Read the root and Lappet
  `AGENTS.md` files and the affected canonical authorities before each slice.
- Keep one implementation path. Remove obsolete helpers and tests in their
  owning slice, without adding compatibility wrappers or parallel registries.
- Write a focused failing regression test first for each runtime behaviour fix.
  Test authored core Wasm fixtures at both native optimisation levels. They
  prove mechanics, not compiler-produced Moth ABI compatibility.
- Preserve the closed V1 Rust host profile. Core operations returning opaque
  values do not authorise general resource results on host imports.
- Record a phase's evidence and remaining gates in the Lappet roadmap. Each
  code-bearing phase ends with the repository Slice review.

## Phase 1: Correct current runtime fault boundaries

### Task 1. Quarantine takes precedence over argument checks

**Owner:** `crates/lappet/src/runtime/instance.rs` and
`crates/lappet/src/runtime/tests/context_tests.rs` or the existing instance
tests that own the matching behaviour.

- [ ] Confirm the reported earlier ordering in `call`, `call_with_token`, `call_scoped`
  and `with_context`. Extend existing tests so that an invalid resource token
  on an already quarantined instance returns `InstanceFaulted` and never
  touches the context holder or guest.
- [ ] Add a regression check that a call rejected on a quarantined instance
  does not install another epoch deadline. Cover the scoped entry too.
- [ ] Run the new tests and observe failure against the baseline.
- [ ] Move the quarantine guard to the common earliest valid guest-entry
  boundary. Keep the context and resource preflight checks for usable
  instances. Eliminate duplicate guards when they no longer add an invariant.
- [ ] Re-run the focused tests with both engine optimisation levels and review
  fault categories and context restoration.

### Task 2. Enforce ABI string lengths independently of configurable limits

**Owner:** `crates/lappet/src/runtime/staging.rs`,
`crates/lappet/src/runtime/instance.rs`, `crates/lappet/src/runtime/error.rs`
and the focused staging/scope tests.

- [ ] Reproduce the missing unconditional `i32::MAX` staging check by testing
  the length decision with configured limits larger than the ABI ceiling.
  Keep the test bounded in memory. It must not allocate a multi-gigabyte string.
- [ ] Reject lengths above the physical ABI ceiling before allocating a
  ticket, even when the configured entry and live limits permit them.
- [ ] Test the rejected input-string length path separately from guest faults.
  Oversized Rust caller input must be an `ExportArgument` failure before guest
  entry, not `HostFault::Internal`, and must leave a usable instance usable.
- [ ] Use the existing typed fault hierarchy and add a specific length fault
  only if the current variants cannot state the reason honestly. Avoid an
  additional public configuration option for this fixed ABI ceiling.
- [ ] Verify existing staging, scope and fault tests still pass. Inspect
  ticket counts, live bytes, context restoration and quarantine after failure.

### Phase 1 gate

- [ ] Run focused `cargo test -p lappet --features runtime` coverage and
  relevant `moth-host` tests under the repository's selected feature lanes.
- [ ] Review source changes against the code review guide, including dead
  helpers, duplicate test setup, unnecessary clones and failure categorisation.
- [ ] Record actual command results in the roadmap. No merge-readiness claim
  follows from focused fixture tests alone.

## Phase 2: Establish the complete Core implementation matrix

**Owner:** Canonical `docs/docs/packages/core/` references for semantics and a
compact Lappet-owned implementation/conformance inventory. Do not copy Core
semantics into a second authority.

- [ ] Enumerate every public Core package function and type. Separately list
  compiler intrinsics and private runtime helpers. For each operation record
  its canonical identity, target requirements and source reference.
- [ ] Assign one implementation route for each supported native target:
  direct compiler/Wasm lowering, existing compiler helper or reusable native
  Core provider. Record the owning producer and required compiler capability.
- [ ] Inspect `src/builder/host_requirements.rs` and current backing classes.
  Record why the current `PackageBacking::HostBinding` import collection cannot
  by itself supply `Core/ExternalBinding` operations.
- [ ] Explicitly resolve how `@core/time`'s `Duration`, `TimeMark` and
  `Timestamp` returns and `@core/io`'s `input.new()` result are implemented
  without adding general resource-result support to the restricted host ABI.
  If an operation needs a new compiler-owned representation or lowering, name
  its producer as an unmet prerequisite, not as working Lappet support.
- [ ] For each operation identify the official browser, native game and
  headless server capability gate, documented unsupported outcome or required
  behaviour. Confirm that unsupported is not represented by a silent no-op.
- [ ] Specify one conformance-case identifier per distinct observable
  contract, with native/browser parity where the same semantics apply.
  Include Core error codes, numeric edge behaviour, ownership and host IO.
- [ ] Submit the route and conformance inventory for Nye's milestone 6
  checkpoint. Record accepted decisions in the Lappet design, retaining the
  actual operation semantics with their Core references.

### Phase 2 gate

- [ ] Every public Core operation has a route, capability decision and
  conformance obligation. Unknown routes are explicit compiler blockers.
- [ ] No proposed `moth-host` descriptor widens general V1 resource results
  or creates a Lappet-specific semantic registry.
- [ ] The completed matrix is usable to sequence milestone 6 implementation
  without claiming that registration means conformance.

## Phase 3: Specify the trusted official-builder handoff

**Owner:** `crates/lappet/docs/package-manager.md`, the Lappet design and the
build system's existing first-party builder selection mechanism.

- [ ] Trace the existing first-party HTML builder construction and reserved
  `BuilderIdentity` handling. Distinguish output ownership from official Core
  admission authority.
- [ ] Define the smallest internal Moth-maintained catalogue handoff that
  identifies certified first-party environments. Its provenance must come from
  trusted official builder selection, not a public custom-builder declaration.
- [ ] Describe how the selected environment exposes target capabilities and
  supported or documented unsupported Core operations to the future manager.
  Keep compiler validation of actual local code independent of managed
  package eligibility.
- [ ] Specify negative acceptance tests for forged names, matching Core
  signatures, self-declared flags and custom Lappet builders. Specify a
  positive case only for a certified environment with completed conformance
  evidence.
- [ ] Record the accepted handoff in permanent owners. Defer manager runtime
  enforcement until its acquisition/admission milestone has a real consumer.

### Phase 3 gate

- [ ] A future implementation can identify exactly which trusted producer
  supplies certification and which manager consumer reads it.
- [ ] No public setter or identity-string comparison can select the official
  Core-dependent managed-package path.

## Phase 4: Prepare the manifest-driven Rust adapter handoff

**Owner:** `crates/lappet/docs/host-abi.md`, `crates/moth-host/` descriptors,
`src/builder/host_conversion.rs` and the compiler-owned native host ABI.

- [ ] Reconcile existing descriptors, compiler canonical type conversion,
  host requirement production and preflight with opaque shared/exclusive
  resource parameters and host-only context requirements. Record the exact
  producer/consumer fields. Rust `TypeId` remains runtime-local.
- [ ] Define one selected-export handle path backed by the compiler manifest,
  with typed ordered arguments, multiple success slots and distinct builtin
  declared error, guest trap, boundary fault and host failure outcomes.
  Record unsupported signature diagnostics, including resource results and
  custom error types.
- [ ] Confirm the selected-exports artefact contract excludes a Wasm start
  section. Specify an explicit pre-instantiation rejection test and preserve
  separate generic core Wasm fixture semantics.
- [ ] Set and document a concrete operational byte bound for exported guest
  strings, alongside the fixed ABI length limit. Test the check without a
  multi-gigabyte allocation. State whether callers can configure it and the
  exact fault for a violation. Obtain Nye's approval before freezing that
  previously unchosen value.
- [ ] Inventory the existing single-string fixture scope driver. Document its
  replacement by the manifest adapter, including decoding status `1` as
  `AbiGuestError`, checking every ordered result, distinguishing invalid
  status/span/UTF-8 faults and copying output before scope close. Do not
  expand the fixture driver into a second public caller.
- [ ] Build a deferred compiler-integration test list: `ValidatedMemoryPlan`,
  reachable host imports, fallible frame/Error lowering, `i64` tokens,
  `staged_text_take`, scope helpers, selected export wrappers, manifest and
  reserved assertion helper. Each test starts only after its producer exists.

### Phase 4 gate

- [ ] The adapter's next implementation slice can consume compiler-produced
  facts without guessing ABI layout or shipping placeholder public APIs.
- [ ] Deferred integration tests are listed at their real owners and the
  old fixture-only path has an explicit removal point in milestone 7.

## Phase 5: Correct documentation drift and release the roadmap gate

- [ ] Reconcile `crates/lappet/roadmap.md` with delivered code and accepted
  decisions. Remove the stale claim that no design checkpoints remain.
- [ ] Correct the builder guide's reported obsolete `compile_wasm(package)` narrative
  against the delivered selected-export API. Update the affected runtime/host
  introductions and relevant compiler handoff documentation only where the
  current code and authoritative contracts justify it.
- [ ] Inspect changed documentation links, Core references and API examples.
  Confirm Lappet consumes the delivered compiler package declarations, exact ObVer identities and isolated source graphs without introducing a second resolver.
- [ ] Run the repository's documentation release-build gate and focused code
  checks. At final integration readiness run `just validate-full` on the final
  tree, with the required toolchain. Record platform and skipped checks.
- [ ] Perform the repository Slice review, then delete this completed plan
  and remove its roadmap link in the same completion commit. Record the
  delivered readiness evidence and remaining compiler gates in the roadmap.

## Review focus

1. **Quarantine ordering:** Can a faulted instance still update a deadline,
   reject a token differently or move context? Regression tests check every
   entry path before and after a fault.
2. **ABI width:** Can permissive instance limits admit a string whose length
   cannot cross `i32`? Bounded length-policy tests enforce the physical cap.
3. **Core ownership:** Does an opaque Core result accidentally become an
   ordinary host resource result or an unsupported placeholder? The inventory
   must identify its compiler producer and exact conformance test.
4. **Trust confusion:** Can a custom builder acquire Core-dependent managed
   packages by name, declared features or Lappet use? Negative admission
   cases and the trusted producer/consumer handoff prevent this.
5. **Duplicate ABI callers:** Are Wasm signatures, error records or string
   lifetimes reconstructed outside the manifest path? The selected-export
   contract and deferred compiler-backed tests expose that drift.

Roadmap milestones may resume only after the compiler package-resolution foundation is delivered, this plan is retired and its evidence is recorded. Milestone 6 still requires its approved conformance checkpoint. Milestone 7 still requires the compiler ABI capabilities. Managed package work consumes the delivered exact-version catalogue and retains its independent admission and security checkpoints.
