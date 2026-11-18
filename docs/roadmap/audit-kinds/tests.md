# Tests

Find supported contracts with no effective regression protection, weak or misleading assertions, wrong test ownership and redundant coverage that adds maintenance without detecting distinct failures.

Follow the [audit guide](../audit-guide.md) for formal findings and coverage. Read [testing standards](../../developer-docs/style-guide/testing.mtf) in full before this audit. Use the [validation guide](../../developer-docs/style-guide/validation.mtf) for current commands. Record incidental defects in [`docs/bug-reports.md`](../../bug-reports.md).

**Evidence bar:** name the supported contract, a realistic regression and the gap in existing protection. Point to the relevant tests and assertions, or show that no owner exists. A test count, search hit or coverage percentage is not enough.

## 1. Build a contract-to-test map

Read the canonical owner, [progress matrix](../../docs/progress/@entry.moth), code entry points and the relevant tests. Inventory:

- Supported valid behaviour and its observable outputs or effects.
- Invalid inputs that must be rejected through the correct diagnostic or failure lane.
- Internal facts that cannot be observed externally, such as side-table alignment, ID remapping and graph invariants.
- Meaningful variants across modules, builders, backends, feature configurations, host platforms and CLI paths.
- Boundaries recently changed or historically fragile, including regression cases for fixed bugs.

Separate accepted, partially implemented and deferred behaviour. Do not demand success coverage for unsupported future work, but protect its required rejection and feature gates. Identify the intended **primary test owner** where required and recognise legitimate backend-only or adversarial-only families.

## 2. Identify the correct owner and whether it runs

Use the ownership rules in `testing.mtf`:

- **Local algorithm, parser distinction, diagnostic construction or hidden invariant:** focused Rust unit test in the owning module's `tests/` directory.
- **User-visible language or project behaviour:** real integration scenario under `tests/cases/`.
- **Stage orchestration:** minimal pipeline or build test when integration scenarios cannot isolate the boundary.
- **Backend artefact structure or target legality:** narrow backend-specific assertion.
- **Cross-backend parity:** one input with distinct expectations for supported backends.

Verify that each supposed owner is actually selected and executed. Look for ignored tests, obsolete `cfg` gates, unreachable harness helpers, invalid fixture manifests, feature lanes no command invokes and tests aimed at an entry point production no longer uses. Check filters and exact case or contract IDs, not just the presence of a file.

A contract may appear incidentally in several scenarios without gaining several primary owners. Secondary cases earn their place through different failure mechanisms, stages, targets or interactions.

## 3. Inspect the canonical integration inventory

Review `tests/cases/manifest.toml`, each selected case's `input/` and `expect.toml`, plus the inventory produced by:

```bash
cargo run --quiet -- tests --audit
```

The inventory validates policy and records assertion types. It **does not execute cases or prove their semantics**. Inspect:

- Unique, stable case IDs and contained paths, non-empty tags, valid roles and a declared contract for non-smoke cases.
- At most one `primary` case per contract. Examine primary-less contract-family advisories rather than treating every advisory as a defect.
- Helpers stored within their owning case rather than silently becoming separate cases.
- Per-backend `success_contract = "acceptance_only"` used only when no stronger case-specific assertion exists. Entirely acceptance-only cases use `role = "smoke"`.
- Warnings `forbid`, `exact` or justified `ignore`, and intentional `diagnostic_match = "contains"` with a substantive reason.
- No successful case that passes solely because the universal backend baseline ran.

Use `cargo run --quiet -- tests --list --contract <contract-id>` to check selection, then run focused cases with `--case` and `--backend` to verify their assertions execute. Manifest metadata is authoritative, not names inferred from folders or display text.

## 4. Ask whether the assertion detects the wrong behaviour

For every claimed contract, imagine a realistic incorrect implementation. **Which exact assertion fails?**

### Successful compilation and execution

- Does the test assert meaningful results rather than just successful compilation or non-empty output?
- Would a wrong value, skipped side effect, reordered event or duplicated entry activation still pass?
- Are deterministic runtime results checked with exact output, ordered fragments or exactly-once fragments as appropriate?
- Does a test inspect emitted artefacts only when their structure is the contract, reserving exact goldens for genuinely contractual bytes?
- Could the harness fail or silently skip execution while the case is counted as successful?

### Rejections and diagnostics

- Does a failing input assert stable `diagnostic_codes` rather than merely `mode = "failure"`?
- Where required, does it check exact multiplicity, compiler-owned `reason`, primary source and related sites through `diagnostic_assertions`?
- Are additional diagnostics permitted by `contains` genuinely independent recovery, rather than a way to hide unexpected failures?
- Are warnings, operational failures, compiler bugs and user diagnostics kept distinct?
- Could the case pass because it failed for an unrelated reason?

### Internal and cross-boundary facts

- Do unit tests check semantic relationships instead of fragile arena indexes, block order or implementation text?
- Are expected results independently derived, or does the test use the same flawed algorithm as its oracle?
- Would a cached, generated, imported or parallel path take a different route from the one under test?
- Does a source-text test claim runtime behaviour it cannot prove?

A test that passes for the wrong reason is a coverage gap, even when it has a descriptive name.

## 5. Hunt for missing permutations without multiplying fixtures

For each significant contract, compare a valid case with its nearest invalid or boundary case. Vary a condition that changes the failure mechanism:

- Empty, minimum, maximum, overflow, malformed, repeated and nested inputs.
- Success, early return, alternative branches, failure propagation, recovery and unreachable paths.
- Local versus imported declarations, visibility, generated instances and multi-file identity.
- Debug versus release, supported backend differences, entry selection and artefact absence.
- Fresh versus reused state, parallel execution, deterministic ordering and failure cleanup.
- Platform-specific filesystem identities or feature-gated code where they affect the contract.

Prefer enriching an existing purposeful integration scenario with a distinct interaction or assertion. Keep precise unit tests for local failure modes that broad scenarios cannot isolate. Do not add a matrix of cases that merely changes names, literals or syntax spelling without exposing a new regression.

## 6. Review harness, platform and maintenance quality

- Use `just feature-lane-check` to check feature-to-lane mapping and `just test-feature-matrix` when execution evidence is needed. A declared lane is not proof it ran.
- Check package-scoped features. `cargo test --workspace` cannot prove Moth's feature-free default because Cargo unifies features through `xtask`.
- Use `just stress` for relevant scheduling and repetition contracts. Sleeps and arbitrary timing thresholds cannot replace synchronisation.
- Keep fixtures deterministic and local to their owners. Temporary filesystem resources need owned cleanup.
- Separate actual case failures from runner, Node harness, spawning, protocol and cleanup failures.
- Preserve distinct regression cases during pruning. Check exact assertions and failure paths before merging or deleting tests.
- Benchmarks are performance evidence, never correctness coverage.

For suspected duplication, show which surviving assertion still catches every distinct regression. Do not use a larger integration scenario as automatic grounds to delete focused unit coverage.

## 7. Challenge and record findings

Test the strongest counter-explanation: perhaps another assertion already protects the contract, the behaviour is deferred, a target genuinely differs or an apparent duplicate isolates a distinct failure. Verify the test's selection and actual assertions before filing.

For each formal Tests finding, record:

- The exact contract and its canonical or code owner.
- Current test paths, manifest case IDs, relevant configurations and assertions inspected.
- A realistic incorrect behaviour the existing tests could miss, or the evidence of redundant protection.
- The smallest correction: strengthen an assertion, extend a case, add a focused owner, move coverage or safely remove duplication.
- Why existing protection is insufficient, the counter-explanation tested and the focused run needed to verify the proposed fix.

Report what was inspected and which targets, feature lanes or cases were excluded. The audit makes **no test or fixture edits**. A demonstrated compiler defect belongs in the bug queue with a proposed regression test for its eventual fix. A separate accepted implementation task changes coverage.

## Stale when

Supported semantics, test ownership, manifest or runner policy, assertions, relevant backend and platform lanes or feature selection change materially.
