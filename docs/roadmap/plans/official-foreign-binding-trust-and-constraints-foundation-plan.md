# Official foreign-binding trust and constraint evidence

> **For agentic workers:** implement the phases in order after explicit activation. Read `AGENTS.md`, the canonical owners and the repository review and validation rules first. This plan is a queued implementation task, not permission to begin unrelated backend or package-manager work.

**Goal:** ensure that only compiler-maintained packages or approved vendored Rust libraries can publish closed per-callable evidence for `$safe_math` and `$infallible`. Preserve correct transitive guarantees through imports, source wrappers, public interfaces and final target selection.

**Architecture:** reuse the existing external numeric contract and constraint call summaries. Add a compiler-controlled provenance check for authorised implementations without granting trust through package names, SDK registrations or runtime choice. Trust identifies the implementation supplying evidence. Each callable separately states its fast-numeric, trap and failure behaviour.

**Specification:** `docs/compiler-design-overview.md` > `Public semantic interfaces`, `docs/build-system-design.md` > `Package classification`, `docs/docs/packages/package-origins-and-backing.mtf`, `docs/docs/directives/constraints.mtf` and `crates/lappet/docs/design.md` own the accepted policy.

## Status and sequencing

```text
STATUS: queued, implementation not started
CURRENT_SLICE: activation pending
BLOCKERS: normal serial roadmap position and the required compiler capabilities below
NEXT_ACTION: activate explicitly, inspect current ownership and establish a fresh baseline
```

This foundation is scheduled before expanding scalar-parametric external providers and file-backed JavaScript/WAT authoring. Consume the delivered `$safe_math` and `$infallible` verifier, canonical call summaries, public interface publication, restricted external registry and Core provider contracts. It does not require WIT component loading, general mixed-target Wasm support or a package manager.

## 1. Policy and non-goals

1. Supported execution families are native Wasm on Lappet and browser JavaScript/HTML5 with optional integrated Wasm. The full-stack builder composes them. Check/LSP are analysis overlays. Custom SDK builders may compose documented capabilities but cannot define new language semantics or independently trusted providers.
2. Foreign code has implementation provenance distinct from `PackageOrigin` and `PackageBacking`. User-authored native Rust, browser JS, WAT and WIT bindings are untrusted. Only compiler-maintained packages or Rust libraries vendored and approved as part of Moth's official distribution may publish trusted facts. A custom builder using Lappet or copying an official file has no special authority.
3. An approved implementation is not implicitly infallible or safe-math. Each registered callable supplies closed facts for `uses_fast_semantics`, `may_trap` and `may_fail`, including positive hazards. Unknown absence is never interpreted as false. A published no-hazard claim must be justified against the exact implementation and its adapters.
4. Source packages retain normal Moth analysis. Only reachable foreign calls affect a caller's constraint proof. A source wrapper cannot clear an unknown external leaf. `$fast_math` remains a lexical numeric mode and has no blanket foreign-code restriction.
5. Keep the present numeric constraints. No `$safe` directive, general effects framework, Rust panic/unsafe scanner, blanket package-safe flag, LLVM backend or public trusted-provider SDK is authorised. Future direct WAT authoring and WIT components receive independent interface designs.

The evidence covers Moth semantic traps, including compiler-generated foreign
value guards, and escaping recoverable errors. Rust panics, operational native
host faults and external termination retain the separate runtime fault policy.
A trusted `$infallible` declaration therefore does not prove that a host
implementation can never panic. Official vendor approval must assess those
risks independently rather than silently expanding this directive.

## 2. Starting implementation and ownership

Confirm these paths at activation rather than relying on stale line numbers:

- `src/compiler_frontend/external_packages/definitions.rs`: `ExternalNumericContract::{Unknown, Trusted}` and `TrustedExternalNumericContract` already describe independent fast, trap and failure facts.
- `src/compiler_frontend/external_packages/registry.rs`: registration currently permits trusted numeric contracts only for Core origin and validates a fallible return contract when `may_fail` is declared. Core constant-evaluator trust has a separate policy and must not expand incidentally.
- `src/builder_surface/package_metadata.rs`: `PackageOrigin` and `PackageBacking` describe distribution and implementation, not permission to prove constraints.
- `src/compiler_frontend/hir/constraint_verification.rs`: external leaves enter monotone call summaries and their reachable proof checks. Keep this owner instead of adding a parallel effect graph.
- `src/builder/definition.rs` and `docs/developer-docs/project-builders/host-packages.mtf`: the public host SDK registers implementations but must offer no way to confer official trust.
- `src/builder_surface/core_packages/`: existing trusted Core definitions are the compatibility baseline. The public semantic interface and its current fingerprint mechanism carry exported constraint facts.

## 3. Implementation phases

### Phase 1: seal implementation provenance

1. Inventory every route that creates, copies or transforms foreign callable registrations, including Core, built-in browser packages, Lappet host descriptors and custom builder packages. Identify which code path actually owns an official implementation, not merely a matching package path or type signature.
2. Reuse existing compiler-shipped registration provenance where it already proves identity. Introduce only the smallest private authority needed for officially approved vendored packages that use another origin or backing. Public builders, project packages and dependencies must have no constructor, metadata field, annotation or configuration switch that confers this authority. Audit current publicly named trusted-contract constructors and comments so they do not advertise a permission that registry validation denies.
3. Bind the authorisation to the selected implementation and its approved adapter/target representation. A replacement host function, changed JS/Wasm asset or alternate runtime provider cannot inherit an official guarantee by preserving its public signature. On a mismatch, reject registration/linking or conservatively treat the call as unknown before publishing a proved guarantee. Choose the existing diagnostic/failure lane appropriate to an authored mismatch or broken official installation.
4. Keep managed package Core eligibility and Lappet's privileged native execution separate from constraint trust. Do not change package lookup precedence, namespace ownership or the Core catalogue.

**Checkpoint:** Core-only trusted registration still works. Spoofed Core or Builder paths, copied assets and SDK descriptors cannot publish trusted evidence. An approved vendored registration can be identified without exposing a generic trust toggle.

### Phase 2: register auditable callable facts

1. Extend the existing `ExternalNumericContract` validation to accept the sealed approved-vendor path as well as the current Core path. Do not add a package-wide boolean that silently certifies every export.
2. For each approved callable, require an explicit closed declaration of possible fast behaviour, semantic traps and recoverable failures, including adapter validation. A missing declaration stays unknown and cannot establish absence. A declared fallible result and a `may_fail` fact must remain consistent with the actual semantic error boundary.
3. Keep trusted numeric metadata independent from const evaluation, access/effect summaries, Never return classification and host permissions. These other facts must not be inferred from the numeric evidence.
4. Audit initial approved implementations and record the evidence beside their owned registrations. Reject false metadata as a compiler/provider invariant violation. User-authored provider descriptions remain ordinary unknown external calls even if they annotate or document equivalent claims.

**Checkpoint:** one approved callable may satisfy both constraints, another may violate `$safe_math`, another may violate `$infallible` and unspecified members of the same approved package remain unknown.

### Phase 3: preserve proofs across compilation and linking

1. Feed the registered facts through the existing external leaf classification and transitive call-summary convergence, including recursion, generic instances, package imports and source wrappers. Do not reanalyse foreign source or add a second summary format.
2. Preserve the evidence and uncertainty in exported semantic interfaces, facade re-exports and generated-function summaries. Reuse the existing interface equality and dependency invalidation rather than creating a trust-specific fingerprint domain.
3. Validate target selection and final provider binding against the evidence-owning implementation. Browser JS, browser Wasm and Lappet-hosted Wasm must not silently select an alternate implementation under a previously proved contract. Invalid or unavailable mappings produce the existing target/provider failure, not a substituted safe assumption.
4. Keep reachability precise. Dead imports must not reject an otherwise valid constraint. Optional optimisation, dead-code elimination and constant folding must not erase a selected fast use or an external call before its required proof is finalised.

**Checkpoint:** a guarantee exported by one package still holds for a consuming module after normal link and target selection, including when an approved foreign call is reached through multiple Moth wrappers.

### Phase 4: regressions, integration and publication

Cover at least these cases with the existing fixtures and relevant native/browser harnesses:

- User Rust HostBinding, annotated browser JS, bounded WAT and a WIT fixture where supported all remain unknown by default. Neither safe-looking signatures nor `Error!` absence proves `$infallible` or `$safe_math`.
- An unknown foreign call invalidates both constraints when reachable, including transitively, but an unused external import does not. Ordinary `$fast_math` callers remain valid under their own numeric semantics.
- Approved calls with all three facts false pass both checks. A declared `uses_fast_semantics` fails `$safe_math`. Declared `may_trap` or `may_fail` fails `$infallible` as applicable. Mixed trusted and unknown members in one package remain independent.
- User builder registration, namespace spoofing, path copying, altered provider bytes and same-signature runtime substitution cannot impersonate approved callable evidence.
- Public interfaces, re-exports, generics, recursion, caches and generated calls preserve the same constraint decisions. Existing Core registrations, error-channel validation and diagnostics remain unchanged.

At each phase, run the smallest relevant tests and the repository Slice review. Before final integration use the validation scope required by `docs/developer-docs/style-guide/validation.mtf`, including target integration tests for each exercised provider. Update the canonical package, constraint, SDK and Lappet references only where implementation clarifies an accepted detail. Record supported implementation status in the progress matrices.

## Completion and boundaries

The foundation is complete when only compiler-authorised implementation paths can publish callable constraint evidence, approved vendored packages work through the existing verifier and all negative provenance and transitive tests pass. Remove this plan and its roadmap entry in the completing commit after transferring any new durable contract to its permanent owner.

Separate later work owns WIT component discovery/adapters, the proposed convenient direct-WAT contract, richer Rust host APIs, browser mixed-Wasm support and full-stack package catalogues. None of them weakens the provenance rule or creates an alternate source constraint system.
