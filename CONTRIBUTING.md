# Contributing to Moth

Moth is an early-stage project. Right now, things are moving very fast. While core design is shifting less and less, there is still a lot of foundational work to do.

This project is **closed to contributions** until its reached more of a hardening, optimisation and "extra feature proposals" phase.

Get in touch if you're interested in discussing the project. Please don't yeet PRs at the repo, they won't be accepted.

This codebase uses AI in a measured capacity. See the [AI policy](./HUMANS.md) for more info about what this project considers acceptable AI generated code.

---

## Language and current support

- [User-facing documentation](docs/src/docs/)
- [Progress matrix](docs/src/docs/progress/@page.moth)
- [Roadmap](docs/roadmap/roadmap.md)

## Compiler and memory design

More technical details about the language, compiler and build system.

- [Compiler design overview](docs/compiler-design-overview.md)
- [Build system overview](docs/build-system-design.md)
- [Memory-management overview](docs/src/developer-docs/memory-management/overview.mtf)
- [Design scope](docs/src/docs/design-scope/)
- [Repository index](index.md)

## Development standards

- [Code style](docs/src/developer-docs/style-guide/style-guide.mtf)
- [Testing standards](docs/src/developer-docs/style-guide/testing.mtf)
- [Validation gates](docs/src/developer-docs/style-guide/validation.mtf)
- [Benchmark and profiling workflow](benchmarks/README.md)

See the hosted docs site for the compiled versions of these mtf files.

## Testing

The complete testing policy is in [testing.mtf](docs/src/developer-docs/style-guide/testing.mtf).

Use focused tests while working on a feature branch, for example:

```sh
cargo test -p xtask cache::tests
cargo test -p moth-lexical
cargo test -p moth-mon
cargo test -p moth --lib mon_syntax_parity
cargo run --quiet -- tests --case <id>
```

The independent codec suite lives in `crates/moth-mon/tests/`. Shared lexical
and numeric tests live with `moth-lexical`. The real-source cross-parser suite
at `src/compiler_tests/mon_syntax_parity/` compares expected typed values under
equivalent receiving contexts and numeric profiles. Its `gaps.rs` records
intentional data/source differences and the undelivered source `{=}`, Unicode
escape and contextual `::Variant` cases. These gaps do not claim delivered
Moth-native `$mon`, automatic schema extraction or static builder support.

Run `cargo test -p moth-mon --doc` and `cargo test -p moth-lexical --doc` when
changing the crate examples.

Intermediate branch commits use the smallest checks that cover their changes.
Run `just validate` at important boundaries. Reserve `just validate-full` for
final plan or feature merge preparation and release readiness. Every commit
landing on `main`, including documentation-only commits, needs full validation
of its resulting tree. The validation guide owns the exact evidence rules.

## Command guide

- `cargo check -p <package>` - quick compile check during branch work
- `cargo test -p <package> <filter>` - focused Rust tests for a slice
- `cargo run --quiet -- build docs --release` - documentation-only branch checkpoint
- `cargo run --quiet -- tests` - complete integration suite
- `cargo run --quiet -- tests --case <id> [--backend <id>]` - exact focused integration run
- `cargo run --quiet -- tests --tag <tag> [--tag <tag>]` - logical AND tag selection
- `cargo run --quiet -- tests --list [filters]` - list selected suite metadata without compiling
- `cargo run --quiet -- tests --audit` - validate and write the complete suite inventory
- `cargo clippy` - lint the root package with Cargo's defaults
- `just clippy` - lint `moth`, `moth-lexical` and `moth-mon` with default features and targets
- `cargo fmt` - format with Cargo's default package selection
- `just fmt` / `just fmt-check` - format or check the compiler and both extracted libraries
- `just validate` - routine correctness at important feature-branch boundaries
- `just validate-perf` - non-recording benchmark and scaling evidence when the change needs it
- `just validate-full` - final merge preparation, every main commit and release readiness
- `just cache-maintain` - conservative timing-report retention and a soft target size warning
- `just cache-evict-preview` - preview explicit workspace dev/test eviction before `just cache-evict`
- `just bench-check` - non-recording performance evidence
- `just bench` - intentional benchmark-history recording (updates the benchmark log)
- `just bench-report` - inspect local benchmark history
- `just profile-case <case> [filter]` - profile a selected benchmark case

Source audit, feature-lane coverage and the honesty audit include `src`,
`crates/moth-lexical/src`, `crates/moth-mon/src`, `crates/moth-mon/tests` and
`xtask/src`. Routine formatting and Clippy cover both extracted packages.
The first-party dependency gate checks Cargo metadata, including development
and build edges: `moth-lexical` depends on no workspace package and `moth-mon`
may depend only on `moth-lexical`, never `moth` or `xtask`.

[validation.mtf](docs/src/developer-docs/style-guide/validation.mtf) owns exact gate scope,
shared Cargo defaults, cache safety and feature coverage. Rust uses the repository's
unpinned stable channel. Environment overrides remain available.
