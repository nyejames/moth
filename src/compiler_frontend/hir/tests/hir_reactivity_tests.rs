//! HIR Reactivity V1 metadata regression tests.
//!
//! WHAT: verifies that AST-resolved reactive source and template metadata survives HIR lowering.
//! WHY: backend validation and future HTML-JS lowering consume these side-table facts instead of
//! reparsing template directives or treating `$T` as a wrapper type.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ReactiveSource, ReactiveSourceKind, ReactiveTemplateMetadata,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::templates::template::ReactiveSubscription;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::hir::hir_builder::{build_ast, lower_ast};
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::reachability::{
    HirReachability, ReachableReactiveSinkKind, collect_module_function_link_facts,
    collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::hir::reactivity::HirReactiveSourceKind;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node, test_location,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    param_with_type_id, reference_expr,
};
use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn reactive_declaration_metadata_is_bound_to_local() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let count_path = super::symbol("count", &mut string_table);
    let source = reactive_source(count_path.clone(), ReactiveSourceKind::Declaration);

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                count_path.clone(),
                Expression::int(1, test_location(2), ValueMode::MutableOwned)
                    .with_reactive_source(source),
            )),
            test_location(2),
        )],
        test_location(1),
    );

    let ast = build_ast(vec![start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should preserve source metadata");

    let count_local = local_named(&module, &string_table, "count");
    let source_id = module
        .side_table
        .reactive_source_id_for_local(count_local)
        .expect("reactive declaration local should have a source id");
    let source = module
        .side_table
        .reactive_source(source_id)
        .expect("source id should resolve");

    assert_eq!(source.local_id, count_local);
    assert_eq!(source.path, count_path);
    assert_eq!(source.kind, HirReactiveSourceKind::Declaration);
    assert_eq!(source.type_id, builtin_type_ids::INT);
}

#[test]
fn reactive_parameter_metadata_is_bound_to_function_param() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let count_path = super::symbol("count", &mut string_table);
    let mut parameter = param_with_type_id(
        count_path.clone(),
        builtin_type_ids::INT,
        false,
        test_location(2),
    );
    parameter.value.reactive_source = Some(reactive_source(
        count_path.clone(),
        ReactiveSourceKind::Parameter,
    ));

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![parameter],
            returns: vec![],
        },
        vec![],
        test_location(1),
    );

    let ast = build_ast(vec![start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should preserve parameter metadata");

    let start_function = super::start_function(&module);
    let parameter_local = start_function.params[0];
    let source_id = module
        .side_table
        .reactive_source_id_for_local(parameter_local)
        .expect("reactive parameter local should have a source id");
    let source = module
        .side_table
        .reactive_source(source_id)
        .expect("source id should resolve");

    assert_eq!(source.kind, HirReactiveSourceKind::Parameter);
    assert_eq!(source.path, count_path);
    assert_eq!(source.type_id, builtin_type_ids::INT);
}

#[test]
fn reactive_template_dependency_metadata_is_bound_to_hir_value() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let count_path = super::symbol("count", &mut string_table);
    let view_path = super::symbol("view", &mut string_table);
    let count_source = reactive_source(count_path.clone(), ReactiveSourceKind::Declaration);
    let template_metadata = metadata_with_subscription(count_source.clone(), test_location(4));

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    count_path.clone(),
                    Expression::int(1, test_location(2), ValueMode::MutableOwned)
                        .with_reactive_source(count_source),
                )),
                test_location(2),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    view_path.clone(),
                    Expression::string_slice(
                        string_table.intern("<p>count</p>"),
                        test_location(3),
                        ValueMode::ImmutableOwned,
                    ),
                )),
                test_location(3),
            ),
            node(
                NodeKind::PushStartRuntimeFragment(
                    reference_expr(
                        view_path,
                        builtin_type_ids::STRING,
                        test_location(4),
                        ValueMode::ImmutableReference,
                    )
                    .with_reactive_template_metadata(template_metadata),
                ),
                test_location(4),
            ),
        ],
        test_location(1),
    );

    let ast = build_ast(vec![start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should preserve template metadata");

    let fragment_value = pushed_fragment_value(&module);
    let template = module
        .side_table
        .reactive_template_for_value(fragment_value.id)
        .expect("pushed fragment value should have reactive template metadata");

    assert!(template.template_backed);
    assert_eq!(template.dependencies.len(), 1);
    let dependency = &template.dependencies[0];
    let count_source = module
        .side_table
        .reactive_source(dependency.source)
        .expect("dependency source should resolve");
    assert_eq!(count_source.path, count_path);
    assert_eq!(dependency.type_id, builtin_type_ids::INT);
}

#[test]
fn reachability_records_reactive_runtime_fragment_and_external_sinks() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let count_path = super::symbol("count", &mut string_table);
    let view_path = super::symbol("view", &mut string_table);
    let count_source = reactive_source(count_path.clone(), ReactiveSourceKind::Declaration);
    let template_metadata = metadata_with_subscription(count_source.clone(), test_location(5));

    let reactive_view_reference = || {
        reference_expr(
            view_path.clone(),
            builtin_type_ids::STRING,
            test_location(5),
            ValueMode::ImmutableReference,
        )
        .with_reactive_template_metadata(template_metadata.clone())
    };

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    count_path,
                    Expression::int(1, test_location(2), ValueMode::MutableOwned)
                        .with_reactive_source(count_source),
                )),
                test_location(2),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    view_path.clone(),
                    Expression::string_slice(
                        string_table.intern("<p>count</p>"),
                        test_location(3),
                        ValueMode::ImmutableOwned,
                    ),
                )),
                test_location(3),
            ),
            node(
                NodeKind::PushStartRuntimeFragment(reactive_view_reference()),
                test_location(5),
            ),
            node(
                NodeKind::ExpressionStatement(Expression::host_function_call_with_arguments(
                    ExternalFunctionId::IoLine,
                    vec![CallArgument::positional(
                        reactive_view_reference(),
                        CallAccessMode::Shared,
                        test_location(6),
                    )],
                    vec![],
                    test_location(6),
                )),
                test_location(6),
            ),
        ],
        test_location(1),
    );

    let ast = build_ast(vec![start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should preserve sink metadata");
    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("reachability should collect sinks");

    assert_eq!(
        reachability.reachable_reactive_templates.len(),
        2,
        "runtime fragment and io argument values should both be reachable reactive templates"
    );
    assert!(
        reachability
            .reachable_reactive_sinks
            .iter()
            .any(|sink| sink.kind == ReachableReactiveSinkKind::RuntimeFragment),
        "top-level runtime fragment sink should be recorded"
    );
    assert!(
        reachability
            .reachable_reactive_sinks
            .iter()
            .any(|sink| matches!(
                sink.kind,
                ReachableReactiveSinkKind::ExternalCallArgument {
                    function_id: ExternalFunctionId::IoLine,
                    argument_index: 0
                }
            )),
        "external io argument sink should be recorded"
    );
}

fn reactive_source(path: InternedPath, kind: ReactiveSourceKind) -> ReactiveSource {
    ReactiveSource { path, kind }
}

fn collect_test_reachability(
    module: &crate::compiler_frontend::hir::module::HirModule,
    roots: &[crate::compiler_frontend::hir::ids::FunctionId],
) -> Result<HirReachability, crate::compiler_frontend::compiler_errors::CompilerError> {
    let function_facts = collect_module_function_link_facts(module)?;
    collect_reachability_from_function_link_facts(&function_facts, roots)
}

fn metadata_with_subscription(
    source: ReactiveSource,
    location: crate::compiler_frontend::ast::ast_nodes::SourceLocation,
) -> ReactiveTemplateMetadata {
    let mut metadata = ReactiveTemplateMetadata::template_backed();
    metadata.push_subscription(ReactiveSubscription {
        source,
        type_id: builtin_type_ids::INT,
        location,
    });
    metadata
}

fn local_named(
    module: &crate::compiler_frontend::hir::module::HirModule,
    string_table: &StringTable,
    name: &str,
) -> LocalId {
    module
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| module.side_table.resolve_local_name(local.id, string_table) == Some(name))
        .map(|local| local.id)
        .expect("expected named local")
}

fn pushed_fragment_value(
    module: &crate::compiler_frontend::hir::module::HirModule,
) -> &crate::compiler_frontend::hir::expressions::HirExpression {
    module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::PushRuntimeFragment { value, .. } => Some(value),
            _ => None,
        })
        .expect("expected pushed runtime fragment")
}
