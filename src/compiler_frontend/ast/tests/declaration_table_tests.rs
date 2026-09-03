//! Stable declaration table regression tests.
//!
//! WHAT: checks the AST environment-owned top-level declaration table independent of parser
//! setup.
//! WHY: phase 3 relies on updates preserving placeholder slots so later lookups observe resolved
//! metadata through the shared environment-owned table.

use super::environment::{DeclarationId, DeclarationPassLanes, TopLevelDeclarationTable};
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::headers::module_symbols::{
    CompilerOwnedDeclaration, CompilerOwnedDeclarationKind, OrderedSemanticDeclaration,
    OrderedSemanticDeclarationKind,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileRole, Header, HeaderExportMode, HeaderKind,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn updates_existing_declaration_slot_without_reordering() {
    let mut string_table = StringTable::new();
    let first_path = InternedPath::from_single_str("first", &mut string_table);
    let second_path = InternedPath::from_single_str("second", &mut string_table);

    let mut table = TopLevelDeclarationTable::new(vec![
        declaration(&first_path, DataType::Inferred),
        declaration(&second_path, DataType::Bool),
    ]);

    let first_id = table
        .declaration_id_by_path(&first_path)
        .expect("existing declaration path should have an ID");
    assert!(table.replace_by_id(first_id, declaration(&first_path, DataType::StringSlice),));

    let declarations = table.iter().collect::<Vec<_>>();
    assert_eq!(declarations[0].id, first_path);
    assert_eq!(declarations[0].value.diagnostic_type, DataType::StringSlice);
    assert_eq!(declarations[1].id, second_path);
    assert_eq!(declarations[1].value.diagnostic_type, DataType::Bool);

    let first_name = first_path.name().expect("test path should have a name");
    let by_name = table
        .get_visible_resolved_by_name(first_name, None)
        .expect("name lookup should see updated declaration");
    assert_eq!(by_name.value.diagnostic_type, DataType::StringSlice);
}

#[test]
fn stage3_order_assigns_ids_to_value_and_metadata_declarations() {
    let mut string_table = StringTable::new();
    let alias_path = InternedPath::from_single_str("Alias", &mut string_table);
    let constant_path = InternedPath::from_single_str("constant", &mut string_table);
    let trait_path = InternedPath::from_single_str("Trait", &mut string_table);
    let function_path = InternedPath::from_single_str("function", &mut string_table);

    let table = TopLevelDeclarationTable::from_stage3_order(
        vec![
            ordered_declaration(
                0,
                0,
                &alias_path,
                OrderedSemanticDeclarationKind::TypeAlias,
                None,
            ),
            ordered_declaration(
                1,
                1,
                &constant_path,
                OrderedSemanticDeclarationKind::Constant,
                Some(declaration(&constant_path, DataType::Bool)),
            ),
            ordered_declaration(
                2,
                2,
                &trait_path,
                OrderedSemanticDeclarationKind::Trait,
                None,
            ),
            ordered_declaration(
                3,
                3,
                &function_path,
                OrderedSemanticDeclarationKind::Function,
                Some(declaration(
                    &function_path,
                    DataType::Function(Box::new(None), Default::default()),
                )),
            ),
        ],
        Vec::new(),
    )
    .expect("distinct Stage 3 paths should build a declaration table");

    let alias_id = table
        .declaration_id_by_path(&alias_path)
        .expect("compile-time-only declarations should receive stable IDs");
    let constant_id = table
        .declaration_id_by_path(&constant_path)
        .expect("constant should receive a stable ID");
    let trait_id = table
        .declaration_id_by_path(&trait_path)
        .expect("compile-time-only traits should receive stable IDs");
    let function_id = table
        .declaration_id_by_path(&function_path)
        .expect("function should receive a stable ID");

    assert_eq!(alias_id.index(), 0);
    assert_eq!(constant_id.index(), 1);
    assert_eq!(trait_id.index(), 2);
    assert_eq!(function_id.index(), 3);
    assert!(table.get_by_id(alias_id).is_none());
    assert!(table.get_by_id(trait_id).is_none());
    assert_eq!(
        table
            .get_by_id(constant_id)
            .expect("constant ID should own a value row")
            .id,
        constant_path
    );
}

#[test]
fn stage3_order_rejects_duplicate_semantic_paths() {
    let mut string_table = StringTable::new();
    let duplicate_path = InternedPath::from_single_str("Duplicate", &mut string_table);

    let result = TopLevelDeclarationTable::from_stage3_order(
        vec![
            ordered_declaration(
                0,
                0,
                &duplicate_path,
                OrderedSemanticDeclarationKind::TypeAlias,
                None,
            ),
            ordered_declaration(
                1,
                1,
                &duplicate_path,
                OrderedSemanticDeclarationKind::Trait,
                None,
            ),
        ],
        Vec::new(),
    );

    assert!(
        result.is_err(),
        "duplicate Stage 3 semantic paths must not overwrite the first ID"
    );
}

#[test]
fn stage3_order_rejects_missing_and_unexpected_value_rows() {
    let mut string_table = StringTable::new();
    let constant_path = InternedPath::from_single_str("constant", &mut string_table);
    let alias_path = InternedPath::from_single_str("Alias", &mut string_table);

    let missing_value = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            0,
            0,
            &constant_path,
            OrderedSemanticDeclarationKind::Constant,
            None,
        )],
        Vec::new(),
    );
    assert!(
        missing_value.is_err(),
        "constants require a Stage 3 value row"
    );

    let unexpected_value = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            0,
            0,
            &alias_path,
            OrderedSemanticDeclarationKind::TypeAlias,
            Some(declaration(&alias_path, DataType::Bool)),
        )],
        Vec::new(),
    );
    assert!(
        unexpected_value.is_err(),
        "metadata-only aliases must not acquire declaration rows"
    );
}

#[test]
fn stage3_order_rejects_non_dense_ids_and_mismatched_value_paths() {
    let mut string_table = StringTable::new();
    let record_path = InternedPath::from_single_str("record", &mut string_table);
    let value_path = InternedPath::from_single_str("value", &mut string_table);

    let non_dense = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            1,
            0,
            &record_path,
            OrderedSemanticDeclarationKind::TypeAlias,
            None,
        )],
        Vec::new(),
    );
    assert!(
        non_dense.is_err(),
        "Stage 3 IDs must start at zero without gaps"
    );

    let mismatched_path = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            0,
            0,
            &record_path,
            OrderedSemanticDeclarationKind::Constant,
            Some(declaration(&value_path, DataType::Bool)),
        )],
        Vec::new(),
    );
    assert!(
        mismatched_path.is_err(),
        "a Stage 3 value row must carry its record path"
    );
}

#[test]
fn compiler_owned_rows_reject_semantic_path_collisions() {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str("collision", &mut string_table);

    let result = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            0,
            0,
            &path,
            OrderedSemanticDeclarationKind::TypeAlias,
            None,
        )],
        vec![compiler_owned(
            CompilerOwnedDeclarationKind::Builtin,
            declaration(&path, DataType::Bool),
        )],
    );

    assert!(
        result.is_err(),
        "compiler-owned rows must not overwrite semantic path indexes"
    );
}

#[test]
fn implicit_start_preserves_authored_start_collision_for_source_diagnostics() {
    let mut string_table = StringTable::new();
    let start_path = InternedPath::from_single_str("start", &mut string_table);
    let authored = declaration(
        &start_path,
        DataType::Function(Box::new(None), Default::default()),
    );
    let implicit = authored.clone();

    let table = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            0,
            0,
            &start_path,
            OrderedSemanticDeclarationKind::Function,
            Some(authored),
        )],
        vec![compiler_owned(
            CompilerOwnedDeclarationKind::Start,
            implicit,
        )],
    )
    .expect("implicit start may share the authored start path until body diagnostics run");

    assert!(table.get_by_id(DeclarationId::from_index(0)).is_some());
    assert_eq!(
        table.declaration_id_by_path(&start_path),
        Some(DeclarationId::from_index(1))
    );

    let repeated_start = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            0,
            0,
            &start_path,
            OrderedSemanticDeclarationKind::Function,
            Some(declaration(
                &start_path,
                DataType::Function(Box::new(None), Default::default()),
            )),
        )],
        vec![
            compiler_owned(
                CompilerOwnedDeclarationKind::Start,
                declaration(
                    &start_path,
                    DataType::Function(Box::new(None), Default::default()),
                ),
            ),
            compiler_owned(
                CompilerOwnedDeclarationKind::Start,
                declaration(
                    &start_path,
                    DataType::Function(Box::new(None), Default::default()),
                ),
            ),
        ],
    );
    assert!(
        repeated_start.is_err(),
        "a second compiler-owned start must not replace the first trailing row"
    );
}

#[test]
fn declaration_lanes_reject_missing_semantic_records() {
    let mut string_table = StringTable::new();
    let semantic_kinds = [
        HeaderKind::Function {
            generic_parameters: Default::default(),
            signature: Default::default(),
        },
        HeaderKind::Constant {
            declaration: DeclarationSyntax {
                binding_mode: BindingMode::default(),
                type_annotation: ParsedTypeRef::Inferred,
                config_qualifier: None,
                initializer_tokens: Vec::new(),
                initializer_references: Vec::new(),
                location: SourceLocation::default(),
            },
        },
        HeaderKind::Struct {
            generic_parameters: Default::default(),
            fields: Vec::new(),
        },
    ];

    for (index, kind) in semantic_kinds.into_iter().enumerate() {
        let path = InternedPath::from_single_str(&format!("missing_{index}"), &mut string_table);
        let header = semantic_header(kind, path, &mut string_table);

        assert!(
            DeclarationPassLanes::from_stage3_order(&[header], &[]).is_err(),
            "every semantic header must have a Stage 3 identity record"
        );
    }
}

#[test]
fn declaration_lanes_reject_mismatched_and_duplicate_header_associations() {
    let mut string_table = StringTable::new();
    let function_path = InternedPath::from_single_str("function", &mut string_table);
    let other_path = InternedPath::from_single_str("other", &mut string_table);
    let header = semantic_header(
        HeaderKind::Function {
            generic_parameters: Default::default(),
            signature: Default::default(),
        },
        function_path.clone(),
        &mut string_table,
    );

    let path_mismatch = vec![ordered_declaration(
        0,
        0,
        &other_path,
        OrderedSemanticDeclarationKind::Function,
        Some(declaration(
            &other_path,
            DataType::Function(Box::new(None), Default::default()),
        )),
    )];
    assert!(
        DeclarationPassLanes::from_stage3_order(std::slice::from_ref(&header), &path_mismatch)
            .is_err(),
        "lane records must preserve their header path"
    );

    let kind_mismatch = vec![ordered_declaration(
        0,
        0,
        &function_path,
        OrderedSemanticDeclarationKind::Constant,
        Some(declaration(&function_path, DataType::Bool)),
    )];
    assert!(
        DeclarationPassLanes::from_stage3_order(std::slice::from_ref(&header), &kind_mismatch)
            .is_err(),
        "lane records must preserve their header kind"
    );

    let duplicate_header = vec![
        ordered_declaration(
            0,
            0,
            &function_path,
            OrderedSemanticDeclarationKind::Function,
            Some(declaration(
                &function_path,
                DataType::Function(Box::new(None), Default::default()),
            )),
        ),
        ordered_declaration(
            1,
            0,
            &function_path,
            OrderedSemanticDeclarationKind::Function,
            Some(declaration(
                &function_path,
                DataType::Function(Box::new(None), Default::default()),
            )),
        ),
    ];
    assert!(
        DeclarationPassLanes::from_stage3_order(&[header], &duplicate_header).is_err(),
        "one semantic header cannot own two Stage 3 records"
    );
}

#[test]
fn declaration_lanes_reject_non_dense_ids_and_out_of_range_headers() {
    let mut string_table = StringTable::new();
    let function_path = InternedPath::from_single_str("function", &mut string_table);
    let header = semantic_header(
        HeaderKind::Function {
            generic_parameters: Default::default(),
            signature: Default::default(),
        },
        function_path.clone(),
        &mut string_table,
    );
    let function_declaration = || {
        declaration(
            &function_path,
            DataType::Function(Box::new(None), Default::default()),
        )
    };

    let non_dense = vec![ordered_declaration(
        1,
        0,
        &function_path,
        OrderedSemanticDeclarationKind::Function,
        Some(function_declaration()),
    )];
    assert!(
        DeclarationPassLanes::from_stage3_order(std::slice::from_ref(&header), &non_dense).is_err(),
        "lane IDs must be dense"
    );

    let out_of_range = vec![ordered_declaration(
        0,
        1,
        &function_path,
        OrderedSemanticDeclarationKind::Function,
        Some(function_declaration()),
    )];
    assert!(
        DeclarationPassLanes::from_stage3_order(&[header], &out_of_range).is_err(),
        "lane header associations must stay in range"
    );
}

#[test]
fn compiler_owned_and_appended_rows_follow_semantic_metadata_holes() {
    let mut string_table = StringTable::new();
    let alias_path = InternedPath::from_single_str("Alias", &mut string_table);
    let trait_path = InternedPath::from_single_str("Trait", &mut string_table);
    let start_path = InternedPath::from_single_str("start", &mut string_table);
    let builtin_path = InternedPath::from_single_str("Builtin", &mut string_table);
    let imported_path = InternedPath::from_single_str("imported", &mut string_table);

    let mut table = TopLevelDeclarationTable::from_stage3_order(
        vec![
            ordered_declaration(
                0,
                0,
                &alias_path,
                OrderedSemanticDeclarationKind::TypeAlias,
                None,
            ),
            ordered_declaration(
                1,
                1,
                &trait_path,
                OrderedSemanticDeclarationKind::Trait,
                None,
            ),
        ],
        vec![
            compiler_owned(
                CompilerOwnedDeclarationKind::Start,
                declaration(
                    &start_path,
                    DataType::Function(Box::new(None), Default::default()),
                ),
            ),
            compiler_owned(
                CompilerOwnedDeclarationKind::Builtin,
                declaration(&builtin_path, DataType::Inferred),
            ),
        ],
    )
    .expect("compiler-owned rows should follow the semantic range");

    assert_eq!(
        table
            .declaration_id_by_path(&start_path)
            .expect("start should have an ID")
            .index(),
        2
    );
    assert_eq!(
        table
            .declaration_id_by_path(&builtin_path)
            .expect("builtin should have an ID")
            .index(),
        3
    );
    let imported_id = table
        .append_for_construction(declaration(&imported_path, DataType::Bool))
        .expect("import projection should append through the table owner");
    assert_eq!(imported_id.index(), 4);
}

#[test]
fn appending_declaration_updates_path_and_name_indexes() {
    let mut string_table = StringTable::new();
    let first_path = InternedPath::from_single_str("first", &mut string_table);
    let appended_path = InternedPath::from_single_str("appended", &mut string_table);

    let mut table = TopLevelDeclarationTable::new(vec![declaration(&first_path, DataType::Bool)]);
    let appended = declaration(&appended_path, DataType::StringSlice);

    table
        .append_for_construction(appended)
        .expect("new declaration path should append during construction");

    assert_eq!(table.iter().count(), 2);
    assert_eq!(
        table
            .get_by_path(&appended_path)
            .expect("path index should include appended declaration")
            .value
            .diagnostic_type,
        DataType::StringSlice
    );
    assert_eq!(
        table
            .get_visible_resolved_by_name(
                appended_path
                    .name()
                    .expect("appended test path should have a name"),
                None,
            )
            .expect("name index should include appended declaration")
            .id,
        appended_path
    );
    assert!(
        table
            .append_for_construction(declaration(&appended_path, DataType::Bool))
            .is_none()
    );
}

#[test]
fn generated_layer_keeps_replacements_and_appends_local() {
    let mut string_table = StringTable::new();
    let shared_path = InternedPath::from_single_str("shared", &mut string_table);
    let appended_path = InternedPath::from_single_str("appended", &mut string_table);

    let original = std::rc::Rc::new(TopLevelDeclarationTable::new(vec![declaration(
        &shared_path,
        DataType::Bool,
    )]));
    let mut generated = TopLevelDeclarationTable::fork_for_generated(std::rc::Rc::clone(&original));

    let shared_id = generated
        .declaration_id_by_path(&shared_path)
        .expect("inherited declaration should have an ID");
    assert!(generated.replace_by_id(shared_id, declaration(&shared_path, DataType::StringSlice),));
    generated
        .append_for_construction(declaration(&appended_path, DataType::Bool))
        .expect("generated copy should append a local declaration");

    assert_eq!(
        original
            .get_by_path(&shared_path)
            .expect("original declaration should remain present")
            .value
            .diagnostic_type,
        DataType::Bool
    );
    assert!(original.get_by_path(&appended_path).is_none());
    assert_eq!(
        generated
            .get_by_path(&shared_path)
            .expect("generated declaration should remain present")
            .value
            .diagnostic_type,
        DataType::StringSlice
    );
    assert!(generated.get_by_path(&appended_path).is_some());
}

#[test]
fn nested_generated_layers_inherit_prior_deltas_without_mutating_siblings() {
    let mut string_table = StringTable::new();
    let alias_path = InternedPath::from_single_str("Alias", &mut string_table);
    let shared_path = InternedPath::from_single_str("shared", &mut string_table);
    let trait_path = InternedPath::from_single_str("Trait", &mut string_table);
    let first_path = InternedPath::from_single_str("first_local", &mut string_table);
    let nested_path = InternedPath::from_single_str("nested_local", &mut string_table);

    let root = std::rc::Rc::new(
        TopLevelDeclarationTable::from_stage3_order(
            vec![
                ordered_declaration(
                    0,
                    0,
                    &alias_path,
                    OrderedSemanticDeclarationKind::TypeAlias,
                    None,
                ),
                ordered_declaration(
                    1,
                    1,
                    &shared_path,
                    OrderedSemanticDeclarationKind::Constant,
                    Some(declaration(&shared_path, DataType::Bool)),
                ),
                ordered_declaration(
                    2,
                    2,
                    &trait_path,
                    OrderedSemanticDeclarationKind::Trait,
                    None,
                ),
            ],
            Vec::new(),
        )
        .expect("metadata holes should coexist with one value declaration"),
    );
    let mut first = TopLevelDeclarationTable::fork_for_generated(std::rc::Rc::clone(&root));
    let sibling = TopLevelDeclarationTable::fork_for_generated(std::rc::Rc::clone(&root));

    let shared_id = first
        .declaration_id_by_path(&shared_path)
        .expect("inherited declaration should have an ID");
    assert!(first.replace_by_id(shared_id, declaration(&shared_path, DataType::StringSlice),));
    let first_id = first
        .append_for_construction(declaration(&first_path, DataType::Bool))
        .expect("first layer should append one local declaration");
    assert_eq!(first_id.index(), 3);

    let first = std::rc::Rc::new(first);
    let mut nested = TopLevelDeclarationTable::fork_for_generated(std::rc::Rc::clone(&first));
    assert!(nested.replace_by_id(shared_id, declaration(&shared_path, DataType::Inferred),));
    let nested_id = nested
        .append_for_construction(declaration(&nested_path, DataType::Bool))
        .expect("nested layer should append one local declaration");
    assert_eq!(nested_id.index(), 4);

    assert_eq!(
        first
            .get_by_path(&shared_path)
            .expect("first replacement should remain visible")
            .value
            .diagnostic_type,
        DataType::StringSlice
    );
    assert!(first.get_by_path(&nested_path).is_none());
    assert_eq!(
        sibling
            .get_by_path(&shared_path)
            .expect("sibling should retain the root declaration")
            .value
            .diagnostic_type,
        DataType::Bool
    );
    assert!(sibling.get_by_path(&first_path).is_none());
    assert_eq!(
        nested
            .get_by_path(&shared_path)
            .expect("nested replacement should be visible")
            .value
            .diagnostic_type,
        DataType::Inferred
    );
    assert!(nested.get_by_path(&first_path).is_some());
    assert!(nested.get_by_path(&nested_path).is_some());
    assert_eq!(nested.iter().count(), 3);
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn declaration_copy_counter_detects_flat_row_clones() {
    use crate::compiler_frontend::instrumentation::{
        capture_frontend_counters_for_test, lock_counter_test, log_frontend_counters,
        reset_frontend_counters,
    };
    use crate::timing::start_benchmark_collection;

    let _guard = lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let mut string_table = StringTable::new();
    let first_path = InternedPath::from_single_str("first", &mut string_table);
    let second_path = InternedPath::from_single_str("second", &mut string_table);
    let root = TopLevelDeclarationTable::new(vec![
        declaration(&first_path, DataType::Bool),
        declaration(&second_path, DataType::StringSlice),
    ]);

    let _prohibited_flat_copy = root.clone();
    log_frontend_counters();

    let observations = timing_session.finish();
    assert_counter(
        &observations.counters,
        "generated_declaration_inherited_row_copies",
        2.0,
    );
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn generated_layer_clones_do_not_copy_inherited_rows() {
    use crate::compiler_frontend::instrumentation::{
        capture_frontend_counters_for_test, lock_counter_test, log_frontend_counters,
        reset_frontend_counters,
    };
    use crate::timing::start_benchmark_collection;

    let _guard = lock_counter_test();
    let _counter_capture = capture_frontend_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let mut string_table = StringTable::new();
    let shared_path = InternedPath::from_single_str("shared", &mut string_table);
    let local_path = InternedPath::from_single_str("local", &mut string_table);
    let root = std::rc::Rc::new(TopLevelDeclarationTable::new(vec![declaration(
        &shared_path,
        DataType::Bool,
    )]));
    let mut generated = TopLevelDeclarationTable::fork_for_generated(root);
    let shared_id = generated
        .declaration_id_by_path(&shared_path)
        .expect("inherited declaration should have an ID");
    assert!(generated.replace_by_id(shared_id, declaration(&shared_path, DataType::StringSlice),));
    generated
        .append_for_construction(declaration(&local_path, DataType::Bool))
        .expect("generated layer should append a local row");

    let generated_copy = generated.clone();
    let nested = TopLevelDeclarationTable::fork_for_generated(std::rc::Rc::new(generated_copy));
    let _nested_copy = nested.clone();
    log_frontend_counters();

    let observations = timing_session.finish();
    assert_counter(
        &observations.counters,
        "generated_declaration_inherited_row_copies",
        0.0,
    );
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
fn assert_counter(
    counters: &[crate::timing::BenchmarkObservationMetric],
    name: &str,
    expected: f64,
) {
    let actual = counters
        .iter()
        .find(|counter| counter.name == name)
        .map(|counter| counter.value);
    assert_eq!(actual, Some(expected), "unexpected value for {name}");
}

fn declaration(path: &InternedPath, data_type: DataType) -> Declaration {
    Declaration {
        id: path.to_owned(),
        value: Expression::no_value(
            SourceLocation::default(),
            data_type,
            ValueMode::ImmutableOwned,
        ),
        config_qualifier: None,
    }
}

fn ordered_declaration(
    declaration_index: usize,
    header_index: usize,
    path: &InternedPath,
    kind: OrderedSemanticDeclarationKind,
    declaration: Option<Declaration>,
) -> OrderedSemanticDeclaration {
    OrderedSemanticDeclaration {
        declaration_id: DeclarationId::from_index(declaration_index),
        header_index,
        path: path.clone(),
        kind,
        declaration,
    }
}

fn compiler_owned(
    kind: CompilerOwnedDeclarationKind,
    declaration: Declaration,
) -> CompilerOwnedDeclaration {
    CompilerOwnedDeclaration { kind, declaration }
}

fn semantic_header(kind: HeaderKind, path: InternedPath, string_table: &mut StringTable) -> Header {
    Header {
        kind,
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints: Default::default(),
        name_location: SourceLocation::default(),
        tokens: FileTokens::new(path, Vec::new()),
        source_file: InternedPath::from_single_str("root.moth", string_table),
        capacity_references: Vec::new(),
    }
}
