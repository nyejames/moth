//! Runtime mechanics tests over hand-authored core Wasm fixtures.
//!
//! The fixtures under `fixtures/` prove Wasmtime integration only. They say nothing about the
//! Moth host ABI or compiled Moth output.

mod engine_tests;

/// Assembles a WAT fixture into the binary Wasm the runtime compiles.
fn assemble(wat_source: &str) -> Vec<u8> {
    wat::parse_str(wat_source).expect("the fixture is authored, valid WAT")
}
