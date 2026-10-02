use super::*;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};

// ---------------------------------------------------------------------------
//  Emitted-default collection
// ---------------------------------------------------------------------------
//
// Focused tests for the extracted `collect_emitted_declaration_defaults`
// helper. These are internal side-table invariants integration output cannot
// inspect, so they own a focused test beside the synchronization owner.

fn function_node(path: PathId) -> AstNode {
    AstNode {
        kind: NodeKind::Function(
            path,
            FunctionSignature {
                parameters: Vec::new(),
                returns: Vec::new(),
            },
            Vec::new(),
        ),
        span: None,
        scope: PathId::ROOT,
    }
}

fn struct_node(path: PathId) -> AstNode {
    AstNode {
        kind: NodeKind::StructDefinition(path, Vec::new()),
        span: None,
        scope: PathId::ROOT,
    }
}

#[test]
fn collect_emitted_declaration_defaults_rejects_duplicate_function_paths() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork
        .try_intern_portable_path("dup_func", &mut string_table)
        .expect("test path fits");
    let emitted = vec![function_node(path), function_node(path)];

    let result = collect_emitted_declaration_defaults(&emitted);

    assert!(
        result.is_err(),
        "duplicate emitted function declaration paths must be rejected"
    );
}

#[test]
fn collect_emitted_declaration_defaults_rejects_duplicate_struct_paths() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork
        .try_intern_portable_path("dup_struct", &mut string_table)
        .expect("test path fits");
    let emitted = vec![struct_node(path), struct_node(path)];

    let result = collect_emitted_declaration_defaults(&emitted);

    assert!(
        result.is_err(),
        "duplicate emitted struct declaration paths must be rejected"
    );
}
