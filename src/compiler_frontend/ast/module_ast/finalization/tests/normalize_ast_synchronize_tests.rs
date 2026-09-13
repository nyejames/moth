use super::*;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};

// ---------------------------------------------------------------------------
//  Emitted-default collection and receiver secondary-index synchronization
// ---------------------------------------------------------------------------
//
// Focused tests for the extracted `collect_emitted_declaration_defaults` and
// `synchronize_receiver_secondary_indexes` helpers. These are internal
// side-table invariants integration output cannot inspect, so they own a
// focused test beside the synchronization owner.

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

fn marker_signature(parameter_count: usize) -> FunctionSignature {
    FunctionSignature {
        parameters: (0..parameter_count)
            .map(|_| Declaration {
                id: PathId::ROOT,
                value: Expression::no_value(None, DataType::Inferred, ValueMode::default()),
                binding_span: None,
                config_qualifier: None,
            })
            .collect(),
        returns: Vec::new(),
    }
}

fn receiver_entry(
    function_path: PathId,
    receiver: ReceiverKey,
    source_file: PathId,
    signature: FunctionSignature,
) -> ReceiverMethodEntry {
    ReceiverMethodEntry {
        function_path,
        receiver,
        source_file,
        receiver_mutable: false,
        signature,
    }
}

#[test]
fn collect_emitted_declaration_defaults_rejects_duplicate_function_paths() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork.try_intern_portable_path("dup_func", &mut string_table).expect("test path fits");
    let emitted = vec![function_node(path.clone()), function_node(path.clone())];

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
    let path = path_fork.try_intern_portable_path("dup_struct", &mut string_table).expect("test path fits");
    let emitted = vec![struct_node(path.clone()), struct_node(path.clone())];

    let result = collect_emitted_declaration_defaults(&emitted);

    assert!(
        result.is_err(),
        "duplicate emitted struct declaration paths must be rejected"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_copies_signatures_and_preserves_order() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let struct_a = path_fork.try_intern_portable_path("StructA", &mut string_table).expect("test path fits");
    let struct_b = path_fork.try_intern_portable_path("StructB", &mut string_table).expect("test path fits");
    let source_file = path_fork.try_intern_portable_path("root.moth", &mut string_table).expect("test path fits");

    // Both methods share the bare method name "shared" but live on different receivers, so
    // by_method_name holds two entries under one name while by_receiver_and_name splits them
    // across two keys. The paths differ by their receiver parent, so the function paths are
    // distinct while the method names match. The insertion order [a, b] must survive
    // synchronization.
    let shared_name = string_table.intern("shared");
    let method_a = path_fork
        .try_intern_child(struct_a, shared_name)
        .expect("test method path fits");
    let method_b = path_fork
        .try_intern_child(struct_b, shared_name)
        .expect("test method path fits");

    let primary_a = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(2),
    );
    let primary_b = receiver_entry(
        method_b.clone(),
        ReceiverKey::Struct(struct_b.clone()),
        source_file.clone(),
        marker_signature(3),
    );

    // Secondary entries intentionally carry stale zero-parameter signatures so the copy from
    // the primary index is observable.
    let secondary_a = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(0),
    );
    let secondary_b = receiver_entry(
        method_b.clone(),
        ReceiverKey::Struct(struct_b.clone()),
        source_file.clone(),
        marker_signature(0),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_a.clone(), primary_a);
    catalog.by_function_path.insert(method_b.clone(), primary_b);
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_a.clone()), shared_name),
        vec![secondary_a],
    );
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_b.clone()), shared_name),
        vec![secondary_b],
    );
    catalog.by_method_name.insert(
        shared_name,
        vec![
            receiver_entry(
                method_a.clone(),
                ReceiverKey::Struct(struct_a.clone()),
                source_file.clone(),
                marker_signature(0),
            ),
            receiver_entry(
                method_b.clone(),
                ReceiverKey::Struct(struct_b.clone()),
                source_file.clone(),
                marker_signature(0),
            ),
        ],
    );

    synchronize_receiver_secondary_indexes(&mut catalog, &path_fork)
        .expect("a consistent catalog must synchronize without error");

    // by_receiver_and_name entries received the synchronized primary signatures.
    let synced_a =
        &catalog.by_receiver_and_name[&(ReceiverKey::Struct(struct_a.clone()), shared_name)][0];
    assert_eq!(
        synced_a.signature.parameters.len(),
        2,
        "by_receiver_and_name entry for method_a must copy the primary signature"
    );
    let synced_b =
        &catalog.by_receiver_and_name[&(ReceiverKey::Struct(struct_b.clone()), shared_name)][0];
    assert_eq!(
        synced_b.signature.parameters.len(),
        3,
        "by_receiver_and_name entry for method_b must copy the primary signature"
    );

    // by_method_name preserves insertion order [method_a, method_b] and copied signatures.
    let name_entries = &catalog.by_method_name[&shared_name];
    assert_eq!(
        name_entries.len(),
        2,
        "both shared-name methods are retained"
    );
    assert_eq!(
        name_entries[0].function_path, method_a,
        "vector order is preserved"
    );
    assert_eq!(
        name_entries[1].function_path, method_b,
        "vector order is preserved"
    );
    assert_eq!(
        name_entries[0].signature.parameters.len(),
        2,
        "by_method_name entry for method_a must copy the primary signature"
    );
    assert_eq!(
        name_entries[1].signature.parameters.len(),
        3,
        "by_method_name entry for method_b must copy the primary signature"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_rejects_missing_by_receiver_and_name_entry() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let struct_a = path_fork.try_intern_portable_path("StructA", &mut string_table).expect("test path fits");
    let source_file = path_fork.try_intern_portable_path("root.moth", &mut string_table).expect("test path fits");
    let method_a = path_fork.try_intern_portable_path("method", &mut string_table).expect("test path fits");
    let method_name = path_fork
        .component(method_a)
        .expect("single-component path has a name");

    let primary = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(1),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_a.clone(), primary);
    // Omit by_receiver_and_name; by_method_name is present and consistent.
    catalog.by_method_name.insert(
        method_name,
        vec![receiver_entry(
            method_a.clone(),
            ReceiverKey::Struct(struct_a.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );

    let result = synchronize_receiver_secondary_indexes(&mut catalog, &path_fork);

    assert!(
        result.is_err(),
        "a primary with no matching by_receiver_and_name entry must be rejected"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_rejects_duplicate_by_receiver_and_name_entry() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let struct_a = path_fork.try_intern_portable_path("StructA", &mut string_table).expect("test path fits");
    let source_file = path_fork.try_intern_portable_path("root.moth", &mut string_table).expect("test path fits");
    let method_a = path_fork.try_intern_portable_path("method", &mut string_table).expect("test path fits");
    let method_name = path_fork
        .component(method_a)
        .expect("single-component path has a name");

    let primary = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(1),
    );
    let duplicate = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(0),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_a.clone(), primary);
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_a.clone()), method_name),
        vec![duplicate.clone(), duplicate],
    );
    catalog.by_method_name.insert(
        method_name,
        vec![receiver_entry(
            method_a.clone(),
            ReceiverKey::Struct(struct_a.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );

    let result = synchronize_receiver_secondary_indexes(&mut catalog, &path_fork);

    assert!(
        result.is_err(),
        "two by_receiver_and_name entries joining one primary must be rejected"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_rejects_wrong_receiver_key() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let struct_a = path_fork.try_intern_portable_path("StructA", &mut string_table).expect("test path fits");
    let struct_b = path_fork.try_intern_portable_path("StructB", &mut string_table).expect("test path fits");
    let source_file = path_fork.try_intern_portable_path("root.moth", &mut string_table).expect("test path fits");
    let method_a = path_fork.try_intern_portable_path("method", &mut string_table).expect("test path fits");
    let method_name = path_fork
        .component(method_a)
        .expect("single-component path has a name");

    // The primary is filed under StructA, and by_receiver_and_name stores the entry under the
    // matching (StructA, name) key, but the entry itself claims receiver StructB. The primary
    // validation passes (it finds the entry by function path), and the secondary loop must
    // reject the wrong receiver key.
    let primary = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(1),
    );
    let wrong_key_entry = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_b.clone()),
        source_file.clone(),
        marker_signature(0),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_a.clone(), primary);
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_a.clone()), method_name),
        vec![wrong_key_entry],
    );
    catalog.by_method_name.insert(
        method_name,
        vec![receiver_entry(
            method_a.clone(),
            ReceiverKey::Struct(struct_a.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );

    let result = synchronize_receiver_secondary_indexes(&mut catalog, &path_fork);

    assert!(
        result.is_err(),
        "a by_receiver_and_name entry stored under the wrong receiver key must be rejected"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_rejects_primary_path_key_mismatch() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let receiver_path = path_fork.try_intern_portable_path("Counter", &mut string_table).expect("test path fits");
    let indexed_path = path_fork.try_intern_portable_path("indexed", &mut string_table).expect("test path fits");
    let claimed_path = path_fork.try_intern_portable_path("claimed", &mut string_table).expect("test path fits");
    let source_file = path_fork.try_intern_portable_path("root.moth", &mut string_table).expect("test path fits");

    let primary = receiver_entry(
        claimed_path,
        ReceiverKey::Struct(receiver_path),
        source_file,
        marker_signature(1),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(indexed_path, primary);

    let result = synchronize_receiver_secondary_indexes(&mut catalog, &path_fork);

    assert!(
        result.is_err(),
        "a by_function_path map key that differs from its entry path must be rejected"
    );
}

#[test]
fn synchronize_receiver_secondary_indexes_rejects_extra_secondary_entry() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let struct_a = path_fork.try_intern_portable_path("StructA", &mut string_table).expect("test path fits");
    let struct_b = path_fork.try_intern_portable_path("StructB", &mut string_table).expect("test path fits");
    let source_file = path_fork.try_intern_portable_path("root.moth", &mut string_table).expect("test path fits");
    let method_a = path_fork.try_intern_portable_path("method", &mut string_table).expect("test path fits");
    let orphan = path_fork.try_intern_portable_path("orphan", &mut string_table).expect("test path fits");
    let method_name = path_fork
        .component(method_a)
        .expect("single-component path has a name");
    let orphan_name = path_fork
        .component(orphan)
        .expect("single-component path has a name");

    let primary = receiver_entry(
        method_a.clone(),
        ReceiverKey::Struct(struct_a.clone()),
        source_file.clone(),
        marker_signature(1),
    );

    let mut catalog = ReceiverMethodCatalog::default();
    catalog.by_function_path.insert(method_a.clone(), primary);
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_a.clone()), method_name),
        vec![receiver_entry(
            method_a.clone(),
            ReceiverKey::Struct(struct_a.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );
    catalog.by_method_name.insert(
        method_name,
        vec![receiver_entry(
            method_a.clone(),
            ReceiverKey::Struct(struct_a.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );
    // An extra by_receiver_and_name entry whose function path is not a primary.
    catalog.by_receiver_and_name.insert(
        (ReceiverKey::Struct(struct_b.clone()), orphan_name),
        vec![receiver_entry(
            orphan.clone(),
            ReceiverKey::Struct(struct_b.clone()),
            source_file.clone(),
            marker_signature(0),
        )],
    );

    let result = synchronize_receiver_secondary_indexes(&mut catalog, &path_fork);

    assert!(
        result.is_err(),
        "a by_receiver_and_name entry with no matching by_function_path primary must be rejected"
    );
}
