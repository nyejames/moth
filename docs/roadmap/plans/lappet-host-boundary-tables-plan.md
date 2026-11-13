# Lappet host boundary tables plan

This plan delivers Lappet roadmap milestone 3, "Build host boundary tables and
fault mechanics". It implements the per-instance state Lappet owns at the
native host boundary: the staging table, the resource-token generator, the
resource table with access guards and the per-call host context slot. Delete
this plan in the commit that completes it and update
`crates/lappet/roadmap.md` in the same commit.

## Status

- Status: ready, not started. Phases 3 to 5 wait for the Phase 0 design
  checkpoint.
- Current slice: none.
- Blockers: the runtime foundation isn't delivered yet.
- Next action: Phase 0 once the prerequisites exist.

## Goal

Lappet owns tested staging, resource and context state per instance, wired
into fault quarantine and teardown, with every fixture-level deferred test in
`crates/lappet/docs/host-abi.md` passing under both native optimisation
levels.

Fixtures in this plan are hand-authored core Wasm that follows the V1 protocol.
They prove Lappet's table and fault mechanics. They never prove that compiled
Moth follows the ABI, and no value conversion is built here.

## Prerequisites

These must already be delivered in `crates/lappet`:

- a Cranelift-only engine and compiled modules behind the `runtime` feature
- instances that own their store and host state, with crate-private raw host
  imports and typed export calls
- installed resource limits and execution deadlines
- fault classification with instance quarantine
- runtime feature lanes for `lappet` with and without `runtime`

Nothing is needed from `main`. No file under `src/` changes.

## Read first

Follow the root `AGENTS.md` and `crates/lappet/AGENTS.md` routes for "Host
bindings, ABI and exported calls". These own the contracts:

- `docs/developer-docs/memory-management/native-host-abi/native-host-abi.mtf`:
  `Rust string results`, `Opaque host resources`, `Call protocol` and
  `Ownership and cleanup paths`. It owns the ticket and call-scope protocols
  and token carriage.
- `crates/lappet/docs/host-abi.md`: `Staging table`, `Resource table`,
  `Host context`, `Faults` and `Deferred tests`. It owns Lappet's duties.
- `docs/developer-docs/project-builders/host-packages.mtf`: `Register opaque
  resources` and `Supply host context explicitly`.
- `crates/lappet/docs/host-resources.md`: the resource and context API. It is
  a draft until Phase 0 completes.
- `crates/lappet/docs/design.md`: `Instances, resources and scheduling`.

When this plan and one of those documents disagree, the document wins. Report
the drift.

## Global constraints

- Everything lives in `crates/lappet` behind the `runtime` feature.
- No dependency on the `moth` compiler crate. This plan needs no descriptor
  type. Where a canonical resource or context type identity is required, use
  Rust's `TypeId` of the registered Rust type as the instance-local check and
  leave the canonical Moth identity to the descriptor contract.
- No value conversion table, descriptor matching, manifest preflight or public
  host-function installation. Those need the shared descriptor contract and
  compiled Moth.
- Tokens and tickets are opaque integers. Neither encodes a host address or an
  index that a guest could forge into another entry. No `unsafe`.
- The reserved helper names used by fixtures are the ABI's:
  `moth-abi-v1.staged_text_take` and the `moth_abi_v1_scope_*` exports. Take
  them from the native host ABI and define each name once.
- Every executable change needs a red then green test, run under both
  optimisation levels. Never report a test that wasn't run.

## Process for every phase

1. Write the failing tests, implement the smallest complete slice and remove
   anything it makes obsolete.
2. Run the focused checks: `cargo test -p lappet`,
   `cargo test -p lappet --features runtime`,
   `cargo clippy -p lappet --all-targets --features runtime`,
   `cargo fmt --all -- --check` and `just first-party-deps`.
3. Run the phase review with a fresh reviewer that didn't write the code. The
   reviewer checks the diff against the phase's exit criteria and the owning
   documents, and looks for duplicated owners, unused abstraction, a fault
   turned into a success and tests that assert implementation shape.
4. Fix every blocker, rerun the checks and commit. Push to `lappet-work`.
5. Stop for Nye on any unresolved design or public API choice.

## Provisional policy values

Implement these as defaults on the existing limits value. Phase 6 gathers the
evidence. A value changes only with recorded evidence and Nye's confirmation.

| Policy | Provisional value | Reason | Evidence to gather |
|---|---|---|---|
| Staged text per entry | 16 MiB | Far above ordinary text, far below the ABI's 2 GiB length limit | Largest realistic host string in the game and server cases |
| Staged text live per instance | 64 MiB | Bounds host memory held for one guest | Peak live staged bytes in the fixtures |
| Live resource entries per instance | 1,048,576 | A long-lived game world may hold many resources | Table memory per entry and lookup cost at this size |
| Closed entries | Follow the Phase 0 answer. The draft API removes the entry at close and returns its value | A long-lived instance mustn't pay for every resource it ever closed | Table memory per live entry |
| Token generator | One process-wide `u64` counter starting at 1. Exhaustion is a host fault | Accepted V1 decision | None |
| Ticket generator | Per-instance `u64` counter starting at 1. Never reused while the instance lives | The ABI requires uniqueness within the instance only | None |

## Phase 0: design checkpoint for the resource and context API

The adapter responsibilities leave the Rust host resource API and the
Rust-facing call API as later runtime design. Decide the first before Phase 3.

- [ ] Confirm the prerequisites in the current tree and sync with `main`.
- [ ] Get Nye's answers to the four questions at the end of the draft in
  `crates/lappet/docs/host-resources.md` and any changes to the draft itself.
- [ ] Remove the draft banner from that page, apply its accepted amendments to
  `crates/lappet/docs/host-abi.md` under `Resource table` and `Host context`
  and remove the roadmap's open decision.
- [ ] Everything stays `pub(crate)` until the manifest-driven public API
  exists. Phases 3 to 5 implement the accepted page, not this plan's summary
  of it.

Exit: the page is accepted and the adapter responsibilities agree with it.
Phases 1 and 2 don't wait for it.

## Phase 1: staging table

- [ ] Implement the per-instance staging table: stage bytes and receive a
  nonzero ticket, take by ticket with an expected length and query whether any
  ticket is live. Enforce the per-entry and per-instance byte limits.
- [ ] The empty string stages nothing and uses ticket zero, as the ABI
  defines.
- [ ] Clear the table on fault and on teardown.
- [ ] Unit tests without a guest: ticket uniqueness, single use, wrong length,
  unknown ticket, both limits, the empty-string rule and clearing.

Review focus: the table holds no host address, a failed take consumes nothing
it shouldn't and limits are read from the one limits owner.

Exit: unit tests pass.

## Phase 2: staged text through a real instance

- [ ] Install the reserved `staged_text_take` import for fixtures. It checks
  the ticket, the length and the destination range, copies into guest memory
  and consumes the ticket. Any mismatch is a boundary fault.
- [ ] Track live tickets across the call. Entering any other host import with
  a ticket live is a boundary fault. A ticket live when the outermost entry
  returns is a boundary fault.
- [ ] Write a fixture that receives a staged ticket from a test host import,
  allocates in its own memory and takes it, plus variants that break each
  rule.
- [ ] Tests: the successful copy, an out-of-bounds destination, a wrong
  length, a double take, a live ticket at another import and a live ticket at
  entry return. Each failure quarantines the instance and leaves the table
  empty.

Review focus: fault ordering matches the ABI's cleanup table, no guest helper
is called after a fault and the import can't re-enter the guest.

Exit: fixture-level deferred test 4 passes under both levels.

## Phase 3: token generator and resource table

Needs the Phase 0 decision.

- [ ] Implement the process-wide token generator. Tokens are nonzero and never
  reused. Exhaustion is a host fault.
- [ ] Implement the per-instance resource table: insert a value and receive a
  token, close by token and resolve a token to its entry with a registered-type
  check. Enforce the live-entry limit. Teardown drops every entry.
- [ ] Unit tests: tokens from two instances never collide, a token from another
  instance isn't live here, zero is invalid, a closed token is stale, a
  wrong-type use is rejected and teardown drops each value exactly once.

Review focus: every rejection happens before a Rust reference exists, Rust
drop timing decides no observable behaviour and the table isn't a general
object registry.

Exit: unit tests pass.

## Phase 4: access guards and conflicts

- [ ] Implement the call-scoped guard. Shared access may coexist with shared
  access. Exclusive access conflicts with any other access to the same
  underlying object. Dropping the guard releases the access.
- [ ] Key conflict tracking on the table entry, as the accepted API states.
- [ ] A conflict is a boundary fault and the host function isn't called.
- [ ] Write a fixture whose export receives tokens as `i64` arguments and
  passes them to test host imports.
- [ ] Tests: stale, closed, other-instance, wrong-type and conflicting tokens
  are each rejected before the host function body runs, guards release after
  the call and a fault releases every guard.

Review focus: no guard outlives its host call, a panic inside a host function
releases guards and quarantines the instance and the tests assert behaviour,
not table layout.

Exit: fixture-level deferred test 5 passes under both levels.

## Phase 5: host context and export call scope

- [ ] Implement the per-call context slot. The embedder supplies context for
  each guest entry call. A missing or wrong-type context is a host fault and
  the host function isn't called. Mutable context is exclusive for the call.
- [ ] Implement the host side of the call-scope protocol against a fixture
  that exports hand-written `moth_abi_v1_scope_open`, `scope_reserve` and
  `scope_close`: open, reserve, call once, close. When any step traps, the
  whole call fails, no further guest call runs and scope close is skipped.
- [ ] Tests: one instance serves two successive entry calls with different
  context, a missing context, a wrong-type context, a trapping scope helper
  and a trapping export inside an open scope.
- [ ] Extend the physical import tests: a missing, extra or mismatched
  physical import fails before guest start under both levels.

Review focus: context never persists in the instance between calls, the
call-scope driver matches the ABI's five host steps and a faulted instance is
never asked to close its scope.

Exit: fixture-level deferred tests 6 and 7 pass under both levels.

## Phase 6: policy evidence

- [ ] Measure staging cost for small and limit-sized strings, resource lookup
  and guard cost per host call and table memory at the live-entry limit. Use
  ignored measurement tests in a release build, under both optimisation
  levels.
- [ ] For each row of the policy table, write one line: confirmed, or the
  proposed change with its evidence. Summarise in the roadmap. Keep raw data
  local. Stop for Nye's confirmation.

Exit: every provisional value confirmed or revised by Nye.

## Phase 7: integration and closeout

- [ ] Record the confirmed limits in `crates/lappet/docs/host-abi.md` in place
  of the wording that defers them.
- [ ] Mark each fixture-level deferred test in that document with the test
  that covers it. Leave the descriptor and compiler tests listed as blocked,
  naming what each waits for.
- [ ] Update `crates/lappet/roadmap.md` with delivered status, evidence and the
  next action. Update `index.md` for the new modules.
- [ ] Run `just validate` and record the actual result.
- [ ] Run an independent end-to-end review covering ownership against the
  adapter responsibilities, every fault path, teardown, both the game and
  server cases and anything built that no test or later milestone needs.
- [ ] Delete this plan in the completing commit.

## Out of scope

- Scalar, string, option and error conversion and result-frame encoding.
- Descriptor or manifest matching and any canonical Moth identity.
- A public resource, context or call API.
- The `assertion_failed` helper, which waits for the compiler to freeze its
  signature.
- Running compiled Moth.
