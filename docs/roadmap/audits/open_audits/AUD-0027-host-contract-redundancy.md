# AUD-0027: Host contract redundancy

- State: `complete`
- Kind: `Redundancy`
- Area: `crates.moth_host.contracts` - host requirement construction, export selection and preflight validation.
- Coverage: `partial`
- Reviewed: `2026-11`
- Baseline: main `d6b323082777e73663c8eacd6e03852f97d7e769`. The returned GitHub check runs for that revision were successful, with deploy skipped. This is baseline evidence only. No local checkout or executable validation environment was available to this audit.

## What was inspected

- `crates/moth-host/src/{lib,descriptor,identity,requirements,selection,preflight}.rs`: public entry points, descriptor construction, canonical identities, requirement ordering, selection validation and mismatch collection/rendering.
- `crates/moth-host/src/tests/{requirements_tests,export_tests,preflight_tests}.rs`: ordering, uniqueness, invalid selections, missing/duplicate implementations, exact scalar and optional identities, count/type/channel mismatches and accumulated mismatch order. Also located the descriptor test.
- Context, not independently audited: `src/builder/host_requirements.rs` and `src/builder/tests/host_requirements_tests.rs`, the compiler's selected export/reachable import producer and preflight test consumer. Searched for the mismatch variants across the repository to distinguish host preflight from similarly named compiler diagnostics.

## Authorities read

- `AGENTS.md`, the codebase style and testing/validation guides, the audit guide and Redundancy checklist.
- `docs/developer-docs/project-builders/host-packages.mtf`: shared host descriptor semantics, accepted future generated adapters and explicit registration.
- `docs/developer-docs/memory-management/native-host-abi/native-host-abi.mtf`: import/export identity, signature profile and manifest ownership.
- `crates/lappet/docs/host-abi.md` (installation and matching contract) and relevant milestones in `crates/lappet/roadmap.md`. Runtime adapter installation and physical manifest work remain outside the current shared semantic contract.

## Existing findings and active plans checked

Read the audit log, open findings, completed audit log and bug queue. No existing finding covers the two local redundancies below. AUD-0026 concerns HTML runtime glue, not native host descriptors. The active `docs/const-foundation-profile-independent-semantics` branch was compared with the audited revision: no change to the shared host crate or its direct requirement producer/tests appeared in the returned diff. The const-function plan concerns semantic evaluation and profile independence; this review excludes those semantics, including the compiler's type conversion implementation. No finding depends on const evaluation or target-specific lowering.

## Findings

### AUD-0027-F01: Signature-slot mismatch comparison is duplicated

- State: `candidate`
- Kind: `Redundancy`

#### Evidence

In `crates/moth-host/src/preflight.rs`, `compare_signatures` compares required and declared parameters, then success values. Both inputs are ordered `&[HostType]` from the same `HostSignature` API. Each branch first compares lengths and reports one count mismatch if unequal. Otherwise each zips the slices, enumerates them and emits a mismatch for every unequal pair. Both copy the same canonical import identity and the same required/declared facts. The only difference is which slot category the error names.

The same distinction is expanded into four variants of `PreflightError` (parameter count/type, success-value count/type) and four Display arms. The variants have identical corresponding payloads and their messages differ only in the slot noun. `preflight` calls the comparison only after finding exactly one implementation. It does not validate source types or enforce result arity. Repository search found the host variants in this owner and its tests, with compiler-side preflight assertions in `src/builder/tests/host_requirements_tests.rs`; similarly named compiler trait diagnostic variants are separate owners.

#### Counter-explanation tested

Parameters and success values have distinct ABI roles and registration/selection constraints. That distinction must survive in the reported reason, but it does not require two equality algorithms. Neither current comparison applies a role-specific compatibility rule. Exact `HostType` equality (including optional payloads and Byte versus U8) is the accepted preflight contract for both. Error-channel equality is independent and must remain separate. Explicit variants might make callers easier to read; retain a typed slot category rather than a string, bool, or untyped index. If the consumer inventory establishes a current role-specific comparison requirement, or the proposed change needs more indirection than the duplicated logic, reject or narrow the candidate.

#### Violated contract or cost

The count-before-index policy, error payload construction and formatting have two local owners for one operation. Changing suppression, ordering or attribution requires coordinated edits even though the equality contract is shared. This is a maintenance and drift concern, not a measured execution cost.

#### Root owner

`crates/moth-host/src/preflight.rs`, especially `compare_signatures`, `PreflightError` and its Display implementation.

#### Suggested correction

Consolidate locally. Consider one typed parameter/success-value slot category carried by count and type mismatch reasons and a direct slot comparison operation called in parameter-then-result order. The noun used for rendering should derive from that category. Delete the duplicate comparison blocks and redundant error construction/rendering paths; migrate the current crate and compiler-side test consumers together. Prefer this only if the resulting code is smaller and more direct. Do not introduce a generic diagnostic framework, closures for each case, compatibility variants or another signature representation.

#### Fix scope and preserved invariants

Keep required and declared signatures separate, retain canonical binding identity, precise required/declared counts and types, zero-based indices, distinct parameter/result attribution, count mismatch suppression for that list only, all other list and error-channel mismatches, and canonical import/slot order. Preserve current display meaning and the missing, ambiguous and unused implementation policies. Do not alter the host ABI, descriptor vocabulary, selection validity, registry, compiler type conversion or linker installation.

#### Required validation

Run `cargo test -p moth-host` and the compiler-side `host_requirements_tests` filter. Preserve existing count, type, optional, Byte/U8, error-channel and canonical order assertions. Check a signature with a count mismatch and differing overlapping types in one list, an equal-length mismatch in the other list and an error-channel mismatch: one count reason, no misleading positional reasons for that list, then the remaining reasons in order. Preserve exact rendering if existing consumers depend on it. Review the actual deletion and resulting API at triage; no behaviour or performance improvement is claimed by this audit.

#### Linked findings

None.

### AUD-0027-F02: Requirement export ordering has an unobservable tie-breaker

- State: `candidate`
- Kind: `Redundancy`

#### Evidence

`HostRequirements::new` in `crates/moth-host/src/requirements.rs` sorts exports by `export_name`, then `facade_name`. Immediately afterwards its adjacent-pair check rejects any repeated export name and returns only `RequirementsError::DuplicateExportName { export_name }`. On success all export names differ, so the secondary key cannot affect the published order. On failure the first duplicate name depends only on the primary key, and the error contains no facade-dependent fact. `requirements_tests.rs` asserts canonical order and duplicate rejection. The compiler builds these requirements in `src/builder/host_requirements.rs` from selected exports.

#### Counter-explanation tested

`HostExports::finish` in `selection.rs` has a similar secondary key, but invalid empty/reserved export-name errors can include the facade name. Its tie-breaker can select a deterministic witness and is not part of this finding. The requirements constructor must keep its own canonicalisation and duplicate check because it accepts arbitrary iterators at a public boundary. A future facade-dependent requirement error could justify restoring the key; no current success or error exposes it.

#### Violated contract or cost

One comparison suggests a facade-dependent ordering rule which is unreachable in a valid requirements value and cannot affect the existing duplicate error. This is small unnecessary structure, not a material performance claim.

#### Root owner

`crates/moth-host/src/requirements.rs`, `HostRequirements::new`.

#### Suggested correction

Delete the secondary comparison from the requirements sort only. Keep the selection sort and separate validation boundaries. Do not introduce a shared sort helper.

#### Fix scope and preserved invariants

Preserve export-name ordering, independent import identity ordering, duplicate rejection and deterministic first duplicate name even with reversed input. No change to export selection or error contracts.

#### Required validation

Run `cargo test -p moth-host` and the compiler's host requirement tests. Existing requirement-order and duplicate-name assertions should remain valid. Check reversed invalid export inputs with distinct facade names and multiple duplicate export-name groups if needed to demonstrate identical error selection.

#### Linked findings

None. This is independent of F01.

## Checked and clean

- `HostFunction` and `HostSignature` have distinct registration and signature responsibilities. Their fluent construction shares the signature mutation owner; merging the public types would lose the function name boundary without removing meaningful logic.
- `HostImportRequirement` and `HostImplementation` carry the same kinds of facts from distinct producers. Keeping the types separate prevents required and declared contracts becoming one interchangeable value. Matching uses full structured `HostBindingIdentity`, not a display string or a symbol leaf.
- Requirement normalisation is not a redundant copy of selection or producer validation. The compiler's reachable-import deduplication and selection checks do not establish the invariant for arbitrary inputs accepted by `HostRequirements::new`. Keep its uniqueness checks.
- `preflight` deliberately gathers mismatches for all required identities, ignores unused implementations and diagnoses duplicate implementations before comparing signatures. Replacing the scan with a map purely for speculative performance would add indexing and duplicate handling without measurement.
- The compiler-side producer marked dead-code-allowed pending native manifest work is intentionally deferred by the Lappet roadmap, not abandoned compatibility code. No deletion of the pending integration or consolidation of semantic and physical ABI facts is recommended.

## Limitations

This is a source-trace audit of the shared semantic contract, not a review of all host execution, native memory or Wasm ABI work. Physical manifest generation, Lappet execution/adapter installation, compiler type conversion, transitive reachability correctness, and feature/build integration were excluded. The tests were inspected rather than executed. No local Moth compiler or repository checkout was available; `moth build docs --release` (or its Cargo equivalent) was not run and `just validate-full` was not run on the audit branch. This report records candidate findings for triage, not approved fixes or a merge-ready validation result.
