//! Engine construction and module compilation.

use super::assemble;
use crate::runtime::RuntimeEngine;

#[test]
fn trivial_module_compiles() {
    let engine = RuntimeEngine::new().expect("the default engine configuration is supported");
    let wasm_bytes = assemble(include_str!("fixtures/empty.wat"));

    engine
        .compile_module(&wasm_bytes)
        .expect("an empty module is valid core Wasm");
}
