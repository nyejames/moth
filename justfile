set windows-shell := ["powershell", "-NoLogo", "-NoProfile", "-Command"]

validate:
    @echo "clippy"
    just ci-clippy
    
    @echo "unit tests"
    cargo test --quiet -- --format terse

    @echo "integration tests"
    cargo run --quiet -- tests

    @echo "docs build"
    cargo run --quiet -- check docs --terse

    @echo "benchmark check"
    cargo run --package xtask --bin xtask -- bench-check

    @echo "benchmark compilation validate"
    cargo run --package xtask --bin xtask -- bench-validate

    @echo "frontend benchmark check"
    cargo run --package xtask --bin xtask -- bench-frontend-check

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

bench-report:
    cargo run --package xtask --bin xtask -- bench-report

bench-frontend-check:
    cargo run --package xtask --bin xtask -- bench-frontend-check

bench-validate:
    cargo run --package xtask --bin xtask -- bench-validate

profile filter="terse":
    cargo run --package xtask --bin xtask -- bench-profile --filter {{filter}}

profile-case case filter="terse":
    cargo run --package xtask --bin xtask -- bench-profile --case {{case}} --filter {{filter}}

profile-symbolicated filter="terse":
    cargo run --package xtask --bin xtask -- bench-profile --filter {{filter}} --presymbolicate

profile-case-symbolicated case filter="terse":
    cargo run --package xtask --bin xtask -- bench-profile --case {{case}} --filter {{filter}} --presymbolicate

profile-build:
    RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling --features detailed_timers --bin moth

ci-clippy:
    rustc +1.95.0 -vV
    cargo +1.95.0 clippy -V

    @echo "clippy: native host"
    CARGO_TARGET_DIR=target/ci-clippy-native cargo +1.95.0 clippy --all-targets --all-features -- -D warnings

    @echo "clippy: linux x64"
    CARGO_TARGET_DIR=target/ci-clippy-linux cargo +1.95.0 clippy --target x86_64-unknown-linux-gnu --all-targets --all-features -- -D warnings

    @echo "clippy: windows x64"
    CARGO_TARGET_DIR=target/ci-clippy-windows cargo +1.95.0 clippy --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings
