# Contributing to Moth

Moth is an early-stage compiler and language project with ambitious long-term, difficult goals. 

If you are interested in compilers/programming languages or the goals of this project 
and want to contribute or make suggestions, please open a discussion on GitHub.

This is still an undisclosed project (its open sourced but no one really knows about it), so getting in touch prior to contributing is preferred at this stage over yeeting a random PR at the repo.

No PRs making modifications to documentation or other supporting documents in this codebase will be accepted. 

## AI Policy

AI tools are used to help with building this language, but all LLM generated output is meticulously planned and scaffolded ahead of time, carefully reviewed, and has been put through a rigourously tested pipeline with tons of documentation and testing to support that process.

Contributions that are AI generated can be accepted on their own merit, but might have to adhere to a higher standard of scruitiny before they can be accepted. Everything must pass the full validation workflow and follow the compiler documentation strictly. Submissions that show design drift, duplicated implementation paths, weak diagnostics, superficial tests, or unreviewed generated churn will not be accepted.

---

## Learning materials and design guides for this codebase

The [user facing docs](docs/src/docs/**) contain examples, and beginner-oriented explanations for the language and using the tooling.

To see the current plans and priority goals of the compiler and language, see the [roadmap](docs/roadmap/roadmap.md).

### Language and current support

- [User-facing documentation](docs/src/docs/)
- [Progress matrix](docs/src/docs/progress/#page.moth)
- [Roadmap](docs/roadmap/roadmap.md)

### Compiler and memory design

More technical details about the language, compiler and build system.

- [Compiler design overview](docs/compiler-design-overview.md)
- [Build system overview](docs/build-system-design.md)
- [Memory-management overview](docs/src/docs/codebase/memory-management/overview.mtf)
- [Design scope](docs/src/docs/codebase/design-scope/overview.mtf)
- [Repository index](index.md)

### Development standards

- [Code style](docs/src/docs/codebase/style-guide/style-guide.mtf)
- [Testing standards](docs/src/docs/codebase/style-guide/testing.mtf)
- [Validation gates](docs/src/docs/codebase/style-guide/validation.mtf)

These Moth template files are the development-standard references. The generated web page combines them at `/docs/codebase/style-guide/`.

---

## Contribution workflow

1. Discuss broad design or architecture changes before implementing a large slice.
2. Read the relevant language, compiler, memory, style, testing, and validation documents. Build-system work also requires reading `docs/build-system-design.md`.
3. Find the existing owner of the behavior before adding a new path.
4. Keep the change focused and remove obsolete paths instead of preserving compatibility scaffolding.
5. Add or update tests according to the testing standards.
6. Review the progress matrix when support, rejection, backend coverage, or test coverage changes.
7. Run the correct final gate for the files changed.
8. Summarize design impact, test coverage, documentation changes, and validation accurately.

## Testing

The complete testing policy is in [testing.mtf](docs/src/docs/codebase/style-guide/testing.mtf).

In general:

- use integration cases under `tests/cases/` for user-visible behavior;
- use focused unit tests for hidden invariants and side-table facts;
- use backend artifact assertions or contractual goldens for emitted structure;
- prefer stable diagnostic codes over full rendered-message snapshots;
- do not use benchmark fixtures as correctness tests.

Run the integration suite during iteration with:

```sh
cargo run --quiet -- tests
```

Use retained manifest metadata to narrow local runs without changing fixtures:

```sh
cargo run --quiet -- tests --case arithmetic_operator_precedence --backend html
cargo run --quiet -- tests --tag borrows --tag diagnostics --backend html
cargo run --quiet -- tests --contract <contract-id>
cargo run --quiet -- tests --list --tag borrows
```

`--case` and `--contract` match exactly. Repeated `--tag` values use logical AND, and all filters
compose in canonical manifest order. To validate and inventory the entire suite without compiling
cases, run:

```sh
cargo run --quiet -- tests --audit
```

Audit cannot be combined with filters or `--list`. It writes
`target/test-reports/integration_suite_inventory.json`. The report keeps the universal backend
baseline separate from authored acceptance-only intent and other case-specific assertions. Hard
suite-policy findings make audit fail after the report is written and make normal list or execution
fail before compilation. Missing roles and missing contracts on non-smoke cases are hard findings.
A contractless whole-case acceptance-only smoke is valid. Primary-less backend-only and
adversarial-only contract families remain non-fatal advisories, while mixed or ordinary
secondary-only families stay visible for ownership review.

A successful HTML or HTML-Wasm backend always runs its universal backend baseline. Use
`success_contract = "acceptance_only"` only when that backend intentionally has no case-specific
semantic, artifact, golden, absence or expected-warning assertion. A whole-case acceptance-only
fixture uses `role = "smoke"`.

Runtime behavior uses the existing Node harness through `rendered_output_*` fields. Use
`rendered_output_exact` for complete deterministic output, `rendered_output_contains_in_order` for
chronology, and `rendered_output_contains_exactly_once` for activation or mount multiplicity. Exact
mode normalizes line endings only. Prefer narrow artifact assertions for static output and reserve
goldens for exact emitted text that is genuinely contractual.

Failure `diagnostic_codes` and exact `warning_codes` are unordered multisets, so duplicate codes
must be repeated. Exact diagnostic matching is the default:

```toml
[backends.html]
mode = "failure"
warnings = "forbid"
diagnostic_codes = ["MOTH-RULE-0033", "MOTH-RULE-0033"]
```

When a broad code needs a stable reason or source remapping is contractual, add a structured
`diagnostic_assertions` table. Select the diagnostic by code plus one-based occurrence, use only a
compiler-owned qualified reason key, and write normalized paths relative to the fixture:

```toml
[[backends.html.diagnostic_assertions]]
code = "MOTH-RULE-0057"
reason = "invalid_generic_instantiation.recursive_function_instantiation"
path = "input/#page.moth"
line = 3

[[backends.html.diagnostic_assertions.secondary_labels]]
occurrence = 1
path = "input/helpers.moth"
line = 2
```

Lines are one-based. Assert columns only for a real token-span contract. Secondary occurrences
select that diagnostic's secondary labels in emitted order. Do not create runner-owned reason
names or use rendered message text as identity.

Use `diagnostic_match = "contains"` only for an intentional independent cascade or recovery path,
and always add a substantive `diagnostic_match_reason`. The suite audit treats an unjustified
contains block as a hard finding. Expected warnings use `warnings = "exact"` with an exact
`warning_codes` multiset. `warnings = "forbid"` rejects every warning, while `warnings = "ignore"`
intentionally leaves warnings unchecked.

## Validation

The executable validation policy is summarized in [validation.mtf](docs/src/docs/codebase/style-guide/validation.mtf).

### Code-bearing changes

If a change touches Rust, compiler/build-system sources, libraries, tests, benchmarks, manifests, scripts, configuration, or another non-documentation implementation file, run:

```sh
just validate
```

Run `cargo fmt` when Rust files changed.


## Command guide

- `cargo run build docs --release` - required final gate for a strictly documentation-only change
- `cargo run --quiet -- tests` - fast integration-suite iteration
- `cargo run --quiet -- tests --case <id> [--backend <id>]` - exact focused integration run
- `cargo run --quiet -- tests --tag <tag> [--tag <tag>]` - logical-AND tag selection
- `cargo run --quiet -- tests --list [filters]` - list selected suite metadata without compiling
- `cargo run --quiet -- tests --audit` - validate and write the complete suite inventory
- `just validate` - required final gate for code-bearing changes
- `just bench-check` - non-recording performance sanity check
- `just bench` - intentional benchmark-history recording
- `just bench-report` - inspect local benchmark history
- `just profile-case <case> [filter]` - profile a selected benchmark case

## Benchmarks and profiling

Benchmarks are development evidence, not correctness gates by themselves.

- Use `just bench-check` for non-recording performance checks.
- Use `just bench` only when intentionally updating benchmark history.
- Do not add negative diagnostic cases to benchmarks; they belong in `tests/cases/`.
- Commit benchmark source fixtures, not generated project outputs or local profiling data.
- Do not claim an improvement from noisy timing alone.

See [benchmarks/README.md](benchmarks/README.md) for the benchmark and profiling workflow.

## Pull-request expectations

A useful pull request should:

- have one coherent purpose
- explain the relevant design owner and any boundary changes
- avoid compatibility shims and parallel implementations
- include appropriate tests without redundant fixtures
- preserve structured diagnostics and source context
- update the progress matrix when current support changed
- update relevant documentation when the documented behavior changed
- avoid unrelated formatting or generated churn
- state exactly which validation command was run

Do not claim full validation when only a targeted command was used instead of full validation.
