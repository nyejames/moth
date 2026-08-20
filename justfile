set windows-shell := ["powershell", "-NoLogo", "-NoProfile", "-Command"]

validate:
    @echo "clippy"
    just ci-clippy-native

    just validate-common

validate-common:
    @echo "feature lane coverage"
    just feature-lane-check

    @echo "source audit"
    just source-audit

    @echo "unit tests"
    cargo test --workspace --quiet -- --format terse

    @echo "integration tests"
    cargo run --quiet -- tests --terse

    @echo "docs build"
    cargo run --quiet -- check docs --terse

    @echo "benchmark sanity"
    cargo run --package xtask --bin xtask -- bench-ci

    @echo "timers erasure"
    just timers-erasure-check

ship:
    cargo fmt
    just validate
    just bench

release version:
    just validate
    git tag -a v{{version}} -m "Moth v{{version}}"
    git push origin v{{version}}

bench:
    cargo run --package xtask --bin xtask -- bench

bench-frontend:
    cargo run --package xtask --bin xtask -- bench-frontend

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

# Build a no-timer release binary and prove no timer-only marker survives into its bytes.
# The timer *source* rules are applied by `just source-audit`, which owns the single walk.
timers-erasure-check:
    cargo run --package xtask --bin xtask -- timers-erasure-check

# The one broad-source architecture audit: timer source rules plus the removed-name tripwires.
source-audit:
    cargo run --quiet --package xtask --bin xtask -- source-audit

# Run every curated feature lane. Lanes are package-scoped: `cargo test --workspace` unifies
# features across the resolve graph and always enables `timers` through xtask's dependency, so it
# can never run the default configuration.
test-feature-matrix:
    cargo run --package xtask --bin xtask -- feature-matrix

# Prove every declared Cargo feature has an executing lane, without running one.
feature-lane-check:
    cargo run --quiet --package xtask --bin xtask -- feature-lane-check

# The canonical test-honesty audit.
#
# The suite inventory runs first because the audit composes it: `xtask honesty-audit` runs the
# source and feature-lane audits itself, but the integration suite inventory is written by the
# compiler binary, and an audit that read a stale one would report counters no current run
# measured. Every report lands under target/test-reports/.
test-honesty-audit:
    @echo "integration suite inventory"
    cargo run --quiet -- tests --audit

    @echo "honesty audit"
    cargo run --quiet --package xtask --bin xtask -- honesty-audit

# Refresh the tracked durable inventory from the audit that measures it.
#
# Separate from `test-honesty-audit` because a CI gate must not modify the checkout, and because
# the durable copy is a reviewed artifact: it changes when someone decides it should.
test-honesty-evidence:
    cargo run --quiet -- tests --audit
    cargo run --quiet --package xtask --bin xtask -- honesty-audit --update-evidence

# Independently reported CI gates.
#
# CI runs each as its own job so one failed validation family never hides another; `just validate`
# keeps the fail-fast local ordering. `ci-gate-unit-tests` is the workspace-unified configuration
# a developer runs locally, and `ci-gate-feature-matrix` is what actually covers the default and
# per-feature configurations.
ci-gate-clippy:
    just ci-clippy-native

ci-gate-unit-tests:
    cargo test --workspace --quiet -- --format terse

ci-gate-feature-matrix:
    just test-feature-matrix

ci-gate-integration:
    cargo run --quiet -- tests --terse

ci-gate-docs:
    cargo run --quiet -- check docs --terse

ci-gate-benchmarks:
    just bench-ci

ci-gate-timers-erasure:
    just timers-erasure-check

ci-gate-source-audit:
    just source-audit

ci-gate-honesty-audit:
    just test-honesty-audit

# Repeat the unit and integration suites at one, default and 16 threads.
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

[unix]
profile-build:
    RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling --features detailed_timers --bin moth

[windows]
profile-build:
    $env:RUSTFLAGS = "-C force-frame-pointers=yes"; cargo build --profile profiling --features detailed_timers --bin moth

ci-clippy-native:
    rustc -vV
    cargo clippy -V

    @echo "clippy: native host"
    cargo clippy --target-dir target/ci-clippy-native --workspace --all-targets --all-features -- -D warnings
