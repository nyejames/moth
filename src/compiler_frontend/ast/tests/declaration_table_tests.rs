//! Stable declaration table regression tests.
//!
//! WHAT: checks the AST environment-owned top-level declaration table independent of parser
//! setup.
//! WHY: phase 3 relies on updates preserving placeholder slots so later lookups observe resolved
//! metadata through the shared environment-owned table.

use super::environment::{DeclarationId, DeclarationPassLanes, TopLevelDeclarationTable};
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::ast_nodes::DeclarationShell;
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::headers::module_symbols::{
    OrderedSemanticDeclaration, OrderedSemanticDeclarationKind,
};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileRole, Header, HeaderExportMode, HeaderKind,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{TokenIndex, TokenRange};
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn updates_existing_declaration_slot_without_reordering() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let first_path = path_fork
        .try_intern_portable_path("first", &mut string_table)
        .expect("test path fits");
    let second_path = path_fork
        .try_intern_portable_path("second", &mut string_table)
        .expect("test path fits");

    let mut table = TopLevelDeclarationTable::new(
        vec![
            declaration(&first_path, DataType::Inferred),
            declaration(&second_path, DataType::Bool),
        ],
        &path_fork,
    );

    let first_id = table
        .declaration_id_by_path(&first_path)
        .expect("existing declaration path should have an ID");
    assert!(table.replace_by_id(first_id, declaration(&first_path, DataType::StringSlice),));

    let declarations = table.iter().collect::<Vec<_>>();
    assert_eq!(declarations[0].id, first_path);
    assert_eq!(declarations[0].diagnostic_type, DataType::StringSlice);
    assert_eq!(declarations[1].id, second_path);
    assert_eq!(declarations[1].diagnostic_type, DataType::Bool);

    let first_name = path_fork
        .component(first_path)
        .expect("test path should have a name");
    let by_name = table
        .get_visible_resolved_by_name(first_name, None)
        .expect("name lookup should see updated declaration");
    assert_eq!(by_name.diagnostic_type, DataType::StringSlice);
}

#[test]
fn stage3_order_assigns_ids_to_value_and_metadata_declarations() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let alias_path = path_fork
        .try_intern_portable_path("Alias", &mut string_table)
        .expect("test path fits");
    let constant_path = path_fork
        .try_intern_portable_path("constant", &mut string_table)
        .expect("test path fits");
    let trait_path = path_fork
        .try_intern_portable_path("Trait", &mut string_table)
        .expect("test path fits");
    let function_path = path_fork
        .try_intern_portable_path("function", &mut string_table)
        .expect("test path fits");

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
                Some(function_shell(&function_path)),
            ),
        ],
        Vec::new(),
        &path_fork,
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

fn pending_constant_state_is_independent_of_declared_type() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let constant_path = path_fork
        .try_intern_portable_path("typed_constant", &mut string_table)
        .expect("test path fits");
    let function_path = path_fork
        .try_intern_portable_path("shell_function", &mut string_table)
        .expect("test path fits");

    let table = TopLevelDeclarationTable::from_stage3_order(
        vec![
            ordered_declaration(
                0,
                0,
                &constant_path,
                OrderedSemanticDeclarationKind::Constant,
                Some({
                    let mut declaration = declaration(&constant_path, DataType::Bool);
                    declaration.value = DeclarationValue::PendingConstant;
                    declaration
                }),
            ),
            ordered_declaration(
                1,
                1,
                &function_path,
                OrderedSemanticDeclarationKind::Function,
                Some({
                    let mut declaration = declaration(&function_path, DataType::Bool);
                    declaration.value =
                        DeclarationValue::Shell(DeclarationShell::Function { receiver: None });
                    declaration
                }),
            ),
        ],
        Vec::new(),
        &path_fork,
    )
    .expect("distinct Stage 3 paths should build a declaration table");

    let constant = table
        .get_by_path(&constant_path)
        .expect("constant header should be indexed");
    let function = table
        .get_by_path(&function_path)
        .expect("function header should be indexed");
    assert!(table.is_unresolved_constant_placeholder(constant));
    assert!(!table.is_unresolved_constant_placeholder(function));
    assert!(
        table
            .get_visible_resolved_by_path(&constant_path, None)
            .is_none()
    );
    assert!(
        table
            .get_visible_resolved_by_path(&function_path, None)
            .is_some()
    );
}

#[test]
fn stage3_order_rejects_duplicate_semantic_paths() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let duplicate_path = path_fork
        .try_intern_portable_path("Duplicate", &mut string_table)
        .expect("test path fits");

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
        &path_fork,
    );

    assert!(
        result.is_err(),
        "duplicate Stage 3 semantic paths must not overwrite the first ID"
    );
}

#[test]
fn stage3_order_rejects_missing_and_unexpected_value_rows() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let constant_path = path_fork
        .try_intern_portable_path("constant", &mut string_table)
        .expect("test path fits");
    let alias_path = path_fork
        .try_intern_portable_path("Alias", &mut string_table)
        .expect("test path fits");

    let missing_value = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            0,
            0,
            &constant_path,
            OrderedSemanticDeclarationKind::Constant,
            None,
        )],
        Vec::new(),
        &path_fork,
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
        &path_fork,
    );
    assert!(
        unexpected_value.is_err(),
        "metadata-only aliases must not acquire declaration rows"
    );
}

#[test]
fn stage3_order_rejects_non_dense_ids_and_mismatched_value_paths() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let record_path = path_fork
        .try_intern_portable_path("record", &mut string_table)
        .expect("test path fits");
    let value_path = path_fork
        .try_intern_portable_path("value", &mut string_table)
        .expect("test path fits");

    let non_dense = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            1,
            0,
            &record_path,
            OrderedSemanticDeclarationKind::TypeAlias,
            None,
        )],
        Vec::new(),
        &path_fork,
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
        &path_fork,
    );
    assert!(
        mismatched_path.is_err(),
        "a Stage 3 value row must carry its record path"
    );
}

#[test]
fn compiler_owned_rows_reject_semantic_path_collisions() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork
        .try_intern_portable_path("collision", &mut string_table)
        .expect("test path fits");

    let result = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            0,
            0,
            &path,
            OrderedSemanticDeclarationKind::TypeAlias,
            None,
        )],
        vec![declaration(&path, DataType::Bool)],
        &path_fork,
    );

    assert!(
        result.is_err(),
        "compiler-owned rows must not overwrite semantic path indexes"
    );
}

#[test]
fn implicit_start_rejects_authored_start_collision() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let start_path = path_fork
        .try_intern_portable_path("start", &mut string_table)
        .expect("test path fits");
    let authored = function_shell(&start_path);
    let implicit = authored.clone();

    let authored_collision = TopLevelDeclarationTable::from_stage3_order(
        vec![ordered_declaration(
            0,
            0,
            &start_path,
            OrderedSemanticDeclarationKind::Function,
            Some(authored),
        )],
        vec![implicit],
        &path_fork,
    );
    assert!(
        authored_collision.is_err(),
        "an authored start path must not shadow the compiler-owned start"
    );

    let repeated_start = TopLevelDeclarationTable::from_stage3_order(
        Vec::new(),
        vec![function_shell(&start_path), function_shell(&start_path)],
        &path_fork,
    );
    assert!(
        repeated_start.is_err(),
        "a second compiler-owned start must not replace the first compiler-owned row"
    );
}

#[test]
fn declaration_lanes_reject_missing_semantic_records() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let semantic_kinds = [
        HeaderKind::Function {
            generic_parameters: Default::default(),
            signature: Default::default(),
        },
        HeaderKind::Constant {
            declaration: DeclarationSyntax {
                span: None,
                binding_mode: BindingMode::default(),
                type_annotation: ParsedTypeRef::Inferred,
                config_qualifier: None,
                initializer_range: None,
                initializer_references: Vec::new(),
            },
        },
        HeaderKind::Struct {
            generic_parameters: Default::default(),
            fields: Vec::new(),
        },
    ];

    for (index, kind) in semantic_kinds.into_iter().enumerate() {
        let path = path_fork
            .try_intern_portable_path(&format!("missing_{index}"), &mut string_table)
            .expect("test path fits");
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
    let mut path_fork = PathInternerFork::empty();
    let function_path = path_fork
        .try_intern_portable_path("function", &mut string_table)
        .expect("test path fits");
    let other_path = path_fork
        .try_intern_portable_path("other", &mut string_table)
        .expect("test path fits");
    let header = semantic_header(
        HeaderKind::Function {
            generic_parameters: Default::default(),
            signature: Default::default(),
        },
        function_path,
        &mut string_table,
    );

    let path_mismatch = vec![ordered_declaration(
        0,
        0,
        &other_path,
        OrderedSemanticDeclarationKind::Function,
        Some(function_shell(&other_path)),
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
            Some(function_shell(&function_path)),
        ),
        ordered_declaration(
            1,
            0,
            &function_path,
            OrderedSemanticDeclarationKind::Function,
            Some(function_shell(&function_path)),
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
    let mut path_fork = PathInternerFork::empty();
    let function_path = path_fork
        .try_intern_portable_path("function", &mut string_table)
        .expect("test path fits");
    let header = semantic_header(
        HeaderKind::Function {
            generic_parameters: Default::default(),
            signature: Default::default(),
        },
        function_path,
        &mut string_table,
    );
    let function_declaration = || function_shell(&function_path);

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
    let mut path_fork = PathInternerFork::empty();
    let alias_path = path_fork
        .try_intern_portable_path("Alias", &mut string_table)
        .expect("test path fits");
    let trait_path = path_fork
        .try_intern_portable_path("Trait", &mut string_table)
        .expect("test path fits");
    let start_path = path_fork
        .try_intern_portable_path("start", &mut string_table)
        .expect("test path fits");
    let builtin_path = path_fork
        .try_intern_portable_path("Builtin", &mut string_table)
        .expect("test path fits");
    let imported_path = path_fork
        .try_intern_portable_path("imported", &mut string_table)
        .expect("test path fits");

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
            function_shell(&start_path),
            declaration(&builtin_path, DataType::Inferred),
        ],
        &path_fork,
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
        .append_for_construction(declaration(&imported_path, DataType::Bool), &path_fork)
        .expect("import projection should append through the table owner");
    assert_eq!(imported_id.index(), 4);
}

#[test]
fn appending_declaration_updates_path_and_name_indexes() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let first_path = path_fork
        .try_intern_portable_path("first", &mut string_table)
        .expect("test path fits");
    let appended_path = path_fork
        .try_intern_portable_path("appended", &mut string_table)
        .expect("test path fits");

    let mut table =
        TopLevelDeclarationTable::new(vec![declaration(&first_path, DataType::Bool)], &path_fork);
    let appended = declaration(&appended_path, DataType::StringSlice);

    table
        .append_for_construction(appended, &path_fork)
        .expect("new declaration path should append during construction");

    assert_eq!(table.iter().count(), 2);
    assert_eq!(
        table
            .get_by_path(&appended_path)
            .expect("path index should include appended declaration")
            .diagnostic_type,
        DataType::StringSlice
    );
    assert_eq!(
        table
            .get_visible_resolved_by_name(
                path_fork
                    .component(appended_path)
                    .expect("appended test path should have a name"),
                None,
            )
            .expect("name index should include appended declaration")
            .id,
        appended_path
    );
    assert!(
        table
            .append_for_construction(declaration(&appended_path, DataType::Bool), &path_fork)
            .is_none()
    );
}

#[test]
fn flat_table_clone_keeps_replacements_and_appends_independent() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let shared_path = path_fork
        .try_intern_portable_path("shared", &mut string_table)
        .expect("test path fits");
    let appended_path = path_fork
        .try_intern_portable_path("appended", &mut string_table)
        .expect("test path fits");

    let original =
        TopLevelDeclarationTable::new(vec![declaration(&shared_path, DataType::Bool)], &path_fork);
    let mut generated = original.clone();

    let shared_id = generated
        .declaration_id_by_path(&shared_path)
        .expect("copied declaration should have an ID");
    assert!(generated.replace_by_id(shared_id, declaration(&shared_path, DataType::StringSlice),));
    generated
        .append_for_construction(declaration(&appended_path, DataType::Bool), &path_fork)
        .expect("generated copy should append a local declaration");

    assert_eq!(
        original
            .get_by_path(&shared_path)
            .expect("original declaration should remain present")
            .diagnostic_type,
        DataType::Bool
    );
    assert!(original.get_by_path(&appended_path).is_none());
    assert_eq!(
        generated
            .get_by_path(&shared_path)
            .expect("generated declaration should remain present")
            .diagnostic_type,
        DataType::StringSlice
    );
    assert!(generated.get_by_path(&appended_path).is_some());
}

#[test]
fn flat_table_clones_preserve_metadata_ids_and_isolate_mutations() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let alias_path = path_fork
        .try_intern_portable_path("Alias", &mut string_table)
        .expect("test path fits");
    let shared_path = path_fork
        .try_intern_portable_path("shared", &mut string_table)
        .expect("test path fits");
    let trait_path = path_fork
        .try_intern_portable_path("Trait", &mut string_table)
        .expect("test path fits");
    let first_path = path_fork
        .try_intern_portable_path("first_local", &mut string_table)
        .expect("test path fits");
    let nested_path = path_fork
        .try_intern_portable_path("nested_local", &mut string_table)
        .expect("test path fits");

    let root = TopLevelDeclarationTable::from_stage3_order(
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
        &path_fork,
    )
    .expect("metadata holes should coexist with one value declaration");
    let mut first = root.clone();
    let sibling = root.clone();

    let shared_id = first
        .declaration_id_by_path(&shared_path)
        .expect("copied declaration should have an ID");
    assert!(first.replace_by_id(shared_id, declaration(&shared_path, DataType::StringSlice),));
    let first_id = first
        .append_for_construction(declaration(&first_path, DataType::Bool), &path_fork)
        .expect("first copy should append one declaration");
    assert_eq!(first_id.index(), 3);

    let mut nested = first.clone();
    assert!(nested.replace_by_id(shared_id, declaration(&shared_path, DataType::Inferred),));
    let nested_id = nested
        .append_for_construction(declaration(&nested_path, DataType::Bool), &path_fork)
        .expect("nested copy should append one declaration");
    assert_eq!(nested_id.index(), 4);

    assert_eq!(
        first
            .get_by_path(&shared_path)
            .expect("first replacement should remain visible")
            .diagnostic_type,
        DataType::StringSlice
    );
    assert!(first.get_by_path(&nested_path).is_none());
    assert_eq!(
        sibling
            .get_by_path(&shared_path)
            .expect("sibling should retain the root declaration")
            .diagnostic_type,
        DataType::Bool
    );
    assert!(sibling.get_by_path(&first_path).is_none());
    assert_eq!(
        nested
            .get_by_path(&shared_path)
            .expect("nested replacement should be visible")
            .diagnostic_type,
        DataType::Inferred
    );
    assert!(nested.get_by_path(&first_path).is_some());
    assert!(nested.get_by_path(&nested_path).is_some());
    assert_eq!(nested.iter().count(), 3);
    assert_eq!(nested.len(), 5);
    for (path, index) in [(alias_path, 0), (trait_path, 2)] {
        let declaration_id = DeclarationId::from_index(index);
        assert_eq!(nested.declaration_id_by_path(&path), Some(declaration_id));
        assert!(nested.get_by_id(declaration_id).is_none());
        assert!(nested.get_mut_by_id(declaration_id).is_none());
        assert!(!nested.replace_by_id(declaration_id, declaration(&path, DataType::Bool)));
    }
}

#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
#[test]
fn declaration_copy_counter_detects_flat_row_clones() {
    use crate::compiler_frontend::instrumentation::{
        capture_counters_for_test, lock_counter_test, log_frontend_counters,
        reset_frontend_counters,
    };
    use crate::timing::start_benchmark_collection;

    let _guard = lock_counter_test();
    let _counter_capture = capture_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let first_path = path_fork
        .try_intern_portable_path("first", &mut string_table)
        .expect("test path fits");
    let second_path = path_fork
        .try_intern_portable_path("second", &mut string_table)
        .expect("test path fits");
    let root = TopLevelDeclarationTable::new(
        vec![
            declaration(&first_path, DataType::Bool),
            declaration(&second_path, DataType::StringSlice),
        ],
        &path_fork,
    );

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
fn flat_table_clones_count_existing_and_appended_slots() {
    use crate::compiler_frontend::instrumentation::{
        capture_counters_for_test, lock_counter_test, log_frontend_counters,
        reset_frontend_counters,
    };
    use crate::timing::start_benchmark_collection;

    let _guard = lock_counter_test();
    let _counter_capture = capture_counters_for_test();
    reset_frontend_counters();
    let timing_session = start_benchmark_collection(true).expect("timing session should start");

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let shared_path = path_fork
        .try_intern_portable_path("shared", &mut string_table)
        .expect("test path fits");
    let local_path = path_fork
        .try_intern_portable_path("local", &mut string_table)
        .expect("test path fits");
    let root =
        TopLevelDeclarationTable::new(vec![declaration(&shared_path, DataType::Bool)], &path_fork);
    let mut generated = root.clone();
    let shared_id = generated
        .declaration_id_by_path(&shared_path)
        .expect("copied declaration should have an ID");
    assert!(generated.replace_by_id(shared_id, declaration(&shared_path, DataType::StringSlice),));
    generated
        .append_for_construction(declaration(&local_path, DataType::Bool), &path_fork)
        .expect("flat table should append a row");

    let generated_copy = generated.clone();
    let nested = generated_copy.clone();
    let _nested_copy = nested.clone();
    log_frontend_counters();

    let observations = timing_session.finish();
    assert_counter(
        &observations.counters,
        "generated_declaration_inherited_row_copies",
        7.0,
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

fn declaration(path: &PathId, data_type: DataType) -> Declaration {
    let type_id = match data_type {
        DataType::Bool => builtin_type_ids::BOOL,
        DataType::StringSlice => builtin_type_ids::STRING,
        DataType::Inferred => builtin_type_ids::NONE,
        _ => panic!("unsupported declaration-table fixture type"),
    };
    Declaration {
        id: path.to_owned(),
        type_id,
        diagnostic_type: data_type,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    }
}

fn function_shell(path: &PathId) -> Declaration {
    Declaration {
        value: DeclarationValue::Shell(DeclarationShell::Function { receiver: None }),
        value_mode: ValueMode::ImmutableReference,
        ..declaration(path, DataType::Inferred)
    }
}

fn ordered_declaration(
    declaration_index: usize,
    header_index: usize,
    path: &PathId,
    kind: OrderedSemanticDeclarationKind,
    declaration: Option<Declaration>,
) -> OrderedSemanticDeclaration {
    OrderedSemanticDeclaration {
        declaration_id: DeclarationId::from_index(declaration_index),
        header_index,
        path: *path,
        kind,
        declaration,
    }
}

fn semantic_header(kind: HeaderKind, path: PathId, _string_table: &mut StringTable) -> Header {
    let token_index =
        TokenIndex::try_from_raw(0).expect("zero token index should be representable");
    Header {
        kind,
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints: Default::default(),
        name_span: None,
        synthetic_content_payload: None,
        tokens: TokenRange::new(SourceId::COMPILATION_ROOT, token_index, token_index)
            .expect("equal token indexes always form a valid range"),
        declaration_path: path,
        token_sequence: None,
        capacity_references: Vec::new(),
    }
}
