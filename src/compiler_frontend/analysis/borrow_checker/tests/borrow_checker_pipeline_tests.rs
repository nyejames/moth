//! Borrow-checker frontend pipeline regression tests.
//!
//! WHAT: runs full frontend entrypoints and asserts borrow-check failures surface through them.
//! WHY: the borrow checker is only useful if orchestration preserves and reports its diagnostics.

use crate::build_system::build::{
    Module, ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts, ModuleRootActivity,
};
use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_messages::{BorrowDiagnosticKind, DiagnosticKind};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, function_node, make_test_variable, node, reference_expr, symbol,
    test_location,
};
use crate::compiler_frontend::tests::borrow_fixture_support::run_borrow_checker;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::{build_ast, entry_and_start, lower_hir};
use crate::compiler_frontend::validated_generic_template_metadata::ValidatedGenericTemplateStore;
use std::sync::Arc;

use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings::Config;

#[test]
fn frontend_check_borrows_propagates_failures() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);

    let x = symbol("x", &mut string_table);
    let y = symbol("y", &mut string_table);
    let z = symbol("z", &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_location(1), ValueMode::MutableOwned),
                )),
                test_location(1),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    Expression::reference(
                        x.clone(),
                        DataType::Int,
                        test_location(2),
                        ValueMode::MutableReference,
                    ),
                )),
                test_location(2),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    z,
                    reference_expr(x, DataType::Int, builtin_type_ids::INT, test_location(3)),
                )),
                test_location(3),
            ),
        ],
        test_location(1),
    );

    let hir = lower_hir(build_ast(vec![start_fn], entry_path), &mut string_table);

    let config = Config::default();
    let frontend = CompilerFrontend::new(
        &config,
        string_table,
        StyleDirectiveRegistry::built_ins(),
        Arc::new(crate::compiler_frontend::external_packages::ExternalPackageRegistry::new()),
        None,
    );
    let messages = frontend
        .check_borrows(&hir)
        .expect_err("borrow checking should fail");

    assert!(
        messages
            .error_diagnostics()
            .any(|diagnostic| diagnostic.kind
                == DiagnosticKind::Borrow(BorrowDiagnosticKind::SharedMutableConflict))
    );
}

#[test]
fn successful_borrow_report_can_be_stored_on_module() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let counter = symbol("counter", &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    counter.clone(),
                    Expression::int(0, test_location(1), ValueMode::MutableOwned),
                )),
                test_location(1),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        counter.clone(),
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_location(2),
                    ),
                    value: Expression::int(1, test_location(2), ValueMode::ImmutableOwned),
                },
                test_location(2),
            ),
        ],
        test_location(1),
    );

    let hir = lower_hir(build_ast(vec![start_fn], entry_path), &mut string_table);
    let borrow_analysis = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("borrow checking should pass");
    let function_link_facts =
        crate::compiler_frontend::hir::reachability::collect_module_function_link_facts(&hir)
            .expect("validated test HIR should produce function link facts");

    let module = Module {
        executable: ModuleExecutable {
            hir,
            type_environment:
                crate::compiler_frontend::datatypes::environment::TypeEnvironment::new(),
            borrow_analysis,
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::clone(&external_package_registry),
            external_import_candidates: Vec::new(),
            functions: function_link_facts,
        },
        metadata: ModuleCompilerMetadata {
            entry_point: std::path::PathBuf::from("main.moth"),
            warnings: Vec::new(),
            const_top_level_fragments: Vec::new(),
            root_activity: ModuleRootActivity::default(),
            doc_fragments: Vec::new(),
            rendered_path_usages: Vec::new(),
            validated_generic_templates: ValidatedGenericTemplateStore::default(),
        },
    };

    assert!(module.executable.borrow_analysis.stats.functions_analyzed >= 1);
    assert!(
        module
            .executable
            .borrow_analysis
            .analysis
            .total_state_snapshots()
            >= 1
    );
    assert!(
        !module
            .executable
            .borrow_analysis
            .analysis
            .statement_facts
            .is_empty()
    );
    assert!(
        !module
            .executable
            .borrow_analysis
            .analysis
            .terminator_facts
            .is_empty()
    );
    assert!(
        !module
            .executable
            .borrow_analysis
            .analysis
            .value_facts
            .is_empty()
    );
}
