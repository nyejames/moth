# Lappet roadmap

Lappet owns its runtime, embedding and package-manager work here. Develop it in
an isolated worktree, separately from Moth's serial compiler roadmap. Refresh
the current compiler contracts before each integration slice. This roadmap
never resumes a paused compiler plan or treats planned backend support as
available.

## Current state

- Status: initial dependency-free crate and design documents only.
- Current slice: design review and workspace integration.
- Blockers: the checkpoints below, then the relevant delivered compiler APIs.
- Next action: review the CLI separation checkpoint before code-bearing work.

[Runtime design](docs/design.md), [package-manager design](docs/package-manager.md)
and [Wasm optimisation](../../docs/src/developer-docs/wasm-optimisation/overview.mtf)
own the accepted direction. This living roadmap owns delivery, findings and
open decisions. Record a checkpoint's decision in its permanent owner before
implementing the affected slice. Keep unresolved questions explicit rather
than turning example API names into obligations.

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

## 1. Separate the CLI from the compiler library

This is the first code-bearing milestone.

**Checkpoint.** Fix the Cargo dependency direction and physical package layout.
The CLI package produces `moth` and depends on the compiler and Lappet. The
compiler library never depends on Lappet or Wasmtime. Decide which existing
command helpers belong to the CLI and which are genuine compiler services.
Preserve the documented public builder and host facades without forwarding
libraries that only hide a cycle.

**Delivery.** Move the executable and its command ownership. Update workspace
membership, lockfile, default command selection, installation, packaging and
all executable-discovery callers together. Include the integration runner,
benchmark tools, scaffolds and CI recipes. Add no runtime dependency yet.

**Evidence.** Existing `moth` commands behave identically, the compiler remains
usable as a library and Cargo metadata proves the acyclic dependency graph.
Run command tests and affected packaging/benchmark-discovery checks. Update the
index and validation command references when the package selectors change.

## 2. Freeze the native binding and artefact handoff

**Checkpoint.** Define the first concrete Rust-to-Moth ABI using the existing
manual host descriptor path. Specify supported scalars, strings, optionals,
errors, explicit context and opaque resources. Decide exact V1 buffer, record,
choice, collection and multiple-result shapes before expanding the restricted
profile. Resolve allocation, result ownership, cleanup on every failure path,
call-scoped views and runtime-internal helper calls without permitting general
re-entry. Specify explicit host-callable Moth exports and their identity.

**Delivery.** Share one descriptor contract between compiler registration and
runtime adapters. Retain semantic types separately from physical carriers and
implementation selection. Implement only approved adapters, with no guessed
Rust layout, second type registry or general serialisation framework. Attribute
errors to both the Moth call and the registered host binding where available.

**Documentation.** Once the concrete V1 host ABI is accepted, add
`docs/host-abi.md` for approved Rust/Moth type mappings, physical call and
result forms, allocation, cleanup and error paths. Keep compiler memory
semantics in their canonical authority and link each executable adapter example
to tests. This reference must exist before implementing the approved adapters.

**Evidence.** Cover exact integer ranges, finite floats, Byte versus U8, invalid
Unicode/lengths, optional absence versus failure, ordered results and exactly-once
calls. Reject mismatched imports, undeclared exports, missing contexts, stale or
foreign handles and overlapping exclusive resource access. Test independence
of returned storage and every allocation/cleanup failure path actually added.

## 3. Establish Wasmtime execution independently

**Checkpoint.** Select a Wasmtime version and minimal Cargo features. Review
Winch/Cranelift feature compatibility, supported hosts, debugging limitations,
resource accounting and fault disposition. Establish when a trap or adapter
fault makes an instance unusable. Decide memory/stack/execution limits and how
native blocking calls fit the server's cancellation policy. Choose a simple
public configuration rather than expose every Wasmtime option.

**Delivery.** Development defaults to Winch and release to Cranelift, with
explicit builder override. Share immutable compiled code where valid, but keep
mutable stores, contexts and resource tables instance-owned. Calls are
synchronous and scheduling stays in the builder. Persistent AOT caching and
additional Wasm engines remain excluded.

**Evidence.** Run the same small core Wasm fixtures under both strategies on
candidate Linux x86-64/AArch64, macOS x86-64/AArch64 and Windows x86-64 hosts.
Publish only combinations actually verified. Measure clean Rust build cost,
linked size, cold engine compilation, instantiation, execution, host-call cost
and resident memory separately. Test limits, traps, teardown and prohibited
re-entry. Fixtures demonstrate runtime mechanics, not completed Moth lowering.

**Documentation.** Add `docs/wasmtime-integration.md` using the pinned
Wasmtime release and upstream source references. Document a minimal runnable
Rust embedding example, backed by a tested `examples/` program or integration
test. It should show engine setup, a host import, core Wasm instantiation, a
typed exported call and failure/cleanup handling. A hand-authored Wasm fixture
is acceptable until Moth's backend can produce the equivalent program.

## 4. Establish official Core conformance

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

## 5. Connect real Moth applications

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
Cover source-to-host-to-source values, errors, lifecycle cleanup, isolated host
state and capability rejection. Benchmark all four combinations of Moth
output profile and native strategy under the shared optimisation authority.
Never mutate published compiler results to make runtime binding succeed.

**Documentation.** Extend the embedding example to use a real Moth program
when the compiler and runtime handoffs support it. Keep the example checked
against the public embedding API rather than documenting hypothetical calls.

## 6. Deliver package declaration and local catalog foundations

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

## 7. Derive package eligibility and dependency evidence

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

## 8. Design remote admission before enabling managed addition

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
