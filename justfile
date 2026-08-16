set windows-shell := ["powershell", "-NoLogo", "-NoProfile", "-Command"]

validate:
    @echo "clippy"
    just ci-clippy
    
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

timers-erasure-check:
    cargo run --package xtask --bin xtask -- timers-erasure-check

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

ci-clippy:
    rustc +1.95.0 -vV
    cargo +1.95.0 clippy -V

    @echo "clippy: native host"
    cargo +1.95.0 clippy --target-dir target/ci-clippy-native --workspace --all-targets --all-features -- -D warnings

    @echo "clippy: linux x64"
    cargo +1.95.0 clippy --target-dir target/ci-clippy-linux --workspace --target x86_64-unknown-linux-gnu --all-targets --all-features -- -D warnings

    @echo "clippy: windows x64"
    cargo +1.95.0 clippy --target-dir target/ci-clippy-windows --workspace --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings
