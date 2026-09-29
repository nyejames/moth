set windows-shell := ["powershell", "-NoLogo", "-NoProfile", "-Command"]

# Routine correctness gate. Cargo.toml, .cargo/config.toml and rust-toolchain.toml
# also govern direct Cargo commands. No recipe selects another Rust version or profile.
validate:
    just fmt-check
    just clippy
    just validate-common
    just cache-maintain

validate-common:
    @echo "feature lane coverage"
    just feature-lane-check
    @echo "source audit"
    just source-audit
    @echo "first-party dependency audit"
    just first-party-deps
    @echo "unit tests"
    just validate-unit-tests
    @echo "integration tests"
    just ci-gate-integration
    @echo "docs check"
    just ci-gate-docs

# Measurements stay non-recording and retain their existing workloads and budgets.
validate-perf:
    just bench-ci
    just bench-scaling
    just cache-maintain

# All standard validation families, including configurations absent from workspace tests.
# Opt-in Boracle, stress and recording benchmarks remain separate.
validate-full:
    just validate
    just fmt-workspace-check
    just clippy-features
    just test-feature-matrix
    just test-honesty-audit
    just validate-perf
    just timers-erasure-check
    just cache-maintain

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

fmt-workspace-check:
    cargo fmt --all --check

clippy:
    cargo clippy

# Broader coverage is named explicitly rather than hidden inside ordinary `cargo clippy`.
# The manifest's shared warning policy applies here and to direct commands alike.
clippy-features:
    cargo clippy --workspace --all-targets --features moth/timers,moth/detailed_timers,moth/benchmark_counters,moth/show_tokens,moth/show_headers,moth/show_ast,moth/show_eval,moth/show_hir,moth/show_codegen,moth/show_borrow_checker,moth/checked_blocks,moth/async_blocks

validate-unit-tests:
    cargo test --workspace --quiet -- --format terse

# Automatic housekeeping keeps build reuse. It never evicts Cargo artifacts.
cache-maintain:
    cargo run --quiet --package xtask --bin xtask -- cache-maintain

# Explicit maintenance only. Stop editors/agents running Cargo in this target directory first.
# This discards current workspace artifacts too, and therefore costs a subsequent rebuild.
cache-evict-preview:
    cargo clean --profile dev --package moth --package xtask --dry-run --verbose

cache-evict:
    cargo clean --profile dev --package moth --package xtask

ship:
    cargo fmt --all
    just validate-full
    just bench

release version:
    just validate-full
    git tag -a v{{version}} -m "Moth v{{version}}"
    git push origin v{{version}}

bench:
    cargo run --package xtask --bin xtask -- bench

bench-frontend:
    cargo run --package xtask --bin xtask -- bench-frontend

bench-data-layout:
    cargo run --package xtask --bin xtask -- bench-data-layout

bench-data-layout-check:
    cargo run --package xtask --bin xtask -- bench-data-layout-check

bench-check:
    cargo run --package xtask --bin xtask -- bench-check

bench-ci:
    cargo run --package xtask --bin xtask -- bench-ci

bench-report:
    cargo run --package xtask --bin xtask -- bench-report

bench-frontend-check:
    cargo run --package xtask --bin xtask -- bench-frontend-check

bench-validate:
    cargo run --package xtask --bin xtask -- bench-validate

# Fit the growth exponent of every declared scaling series and hold it to budget.
bench-scaling:
    cargo run --package xtask --bin xtask -- bench-scaling

# Preserve the exact no-timer release-artifact check. Source rules stay in source-audit.
timers-erasure-check:
    cargo run --package xtask --bin xtask -- timers-erasure-check

source-audit:
    cargo run --quiet --package xtask --bin xtask -- source-audit

span-census:
    cargo run --quiet --package xtask --bin xtask -- span-census

# First-party dependency and runtime-import policy, not arbitrary host-driven loading.
first-party-deps:
    cargo run --quiet --package xtask --bin xtask -- first-party-deps

# Package-scoped executing lanes. Workspace tests unify features through xtask's timers dependency.
test-feature-matrix:
    cargo run --package xtask --bin xtask -- feature-matrix

# Checks the declared coverage map without executing its lanes.
feature-lane-check:
    cargo run --quiet --package xtask --bin xtask -- feature-lane-check

boracle:
    cargo fmt --check
    cargo clippy -p moth --all-targets --features boracle
    cargo test -p moth --quiet borrow_problem -- --format terse
    cargo test -p moth --quiet last_use -- --format terse
    # Static solver, source service and bounded operational oracle. The measured campaign is separate.
    cargo test -p moth --quiet --features boracle boracle -- --format terse

boracle-campaign:
    cargo clippy -p moth --all-targets --features boracle,boracle_campaign
    cargo test -p moth --quiet --features boracle,boracle_campaign boracle_generated_differential_campaign -- --format terse

# Refresh integration inventory before the composed honesty audit reads it.
test-honesty-audit:
    cargo run --quiet -- tests --audit
    cargo run --quiet --package xtask --bin xtask -- honesty-audit

# Deliberate evidence refresh, never part of a read-only validation gate.
test-honesty-evidence:
    cargo run --quiet -- tests --audit
    cargo run --quiet --package xtask --bin xtask -- honesty-audit --update-evidence

# Independent CI results preserve coverage beyond the routine local gate.
ci-gate-format:
    just fmt-workspace-check

ci-gate-clippy:
    just clippy
    just clippy-features

ci-gate-unit-tests:
    just validate-unit-tests

ci-gate-feature-matrix:
    just test-feature-matrix

ci-gate-integration:
    cargo run --quiet -- tests --terse

ci-gate-docs:
    cargo run --quiet -- check docs --terse

ci-gate-benchmarks:
    just bench-ci

ci-gate-scaling:
    just bench-scaling

ci-gate-timers-erasure:
    just timers-erasure-check

ci-gate-source-audit:
    just source-audit

ci-gate-first-party-deps:
    just first-party-deps

ci-gate-honesty-audit:
    just test-honesty-audit

# Deliberate stress, not a safe first experiment on a memory-constrained host.
stress repeats="3":
    cargo run --package xtask --bin xtask -- stress --repeats {{repeats}}

profile filter="terse":
    cargo run --package xtask --bin xtask -- bench-profile --filter {{filter}}

profile-case case filter="terse":
    cargo run --package xtask --bin xtask -- bench-profile --case {{case}} --filter {{filter}}

profile-symbolicated filter="terse":
    cargo run --package xtask --bin xtask -- bench-profile --filter {{filter}} --presymbolicate

profile-case-symbolicated case filter="terse":
    cargo run --package xtask --bin xtask -- bench-profile --case {{case}} --filter {{filter}} --presymbolicate

# Profiling intentionally needs symbols and frame pointers. It uses the same Rust toolchain.
[unix]
profile-build:
    RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling --features detailed_timers --bin moth

[windows]
profile-build:
    $env:RUSTFLAGS = "-C force-frame-pointers=yes"; cargo build --profile profiling --features detailed_timers --bin moth
