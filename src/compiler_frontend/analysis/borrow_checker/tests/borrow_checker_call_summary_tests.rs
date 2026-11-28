//! Borrow-checker call-summary regression tests.
//!
//! WHAT: verifies how call return aliases and host-call access summaries affect borrow facts.
//! WHY: call boundaries are where alias metadata is easiest to get wrong and hardest to debug.

use crate::compiler_frontend::analysis::borrow_checker::problem::{EventKind, from_hir};
use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckError, check_borrows, refresh_call_allocations, seed_call_summaries,
};
use crate::compiler_frontend::analysis::borrow_checker::{LocalMode, OptionalTransferStatus};
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, CallPassingMode,
};
use crate::compiler_frontend::ast::expressions::expression::{
    FallibleExpressionHandling, HandledFallibleHostFunctionCallInput,
};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, OwnedExpr,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    MultipleMutableBorrowsView, SharedMutableConflictView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, DiagnosticBag, InvalidMutableAccessReason,
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::{CallableReturnShape, FunctionReturnContract};
use crate::compiler_frontend::external_packages::external_returns;
use crate::compiler_frontend::external_packages::test_support::{
    TestExternalAccessKind as ExternalAccessKind, TestExternalReturnAlias as ExternalReturnAlias,
    TestExternalReturnType as ExternalReturnType, set_test_external_function_return_alias,
};
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAbiType as RegistryAbiType, ExternalFunctionDef, ExternalFunctionId,
    ExternalFunctionLowerings, ExternalNumericContract, ExternalReturnSlot, ExternalSignatureType,
};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes;
use crate::compiler_frontend::hir::reachability::collect_module_function_link_facts;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::entry_start_node;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, fresh_success_returns, function_node, immutable_reference_expr,
    make_test_variable, node, param_with_type_id, symbol, test_source_location,
};
use crate::compiler_frontend::tests::borrow_fixture_support::{
    assert_infrastructure_error_contains, assert_invalid_mutable_access_reason, run_borrow_checker,
    run_borrow_checker_with_types, run_prepared_borrow_checker,
    summarize_prepared_call_allocations,
};
use crate::compiler_frontend::tests::external_package_support::{
    default_external_package_registry, register_external_function,
};
use crate::compiler_frontend::tests::hir_fixture_support::{PreparedHirFixture, lower_hir};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::tests::type_id_fixture_support::{
    build_ast_with_registered_types, lower_ast,
};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

use crate::compiler_frontend::value_mode::ValueMode;
use std::sync::Arc;

use crate::compiler_frontend::public_call_summary::typed::{
    EntryPort, Lane, ObjectRef, ScalarSource, TypedCallAllocationFacts, ValueSource,
};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallTransferEffect,
};

fn allocation_facts(summary: &FunctionReturnAliasSummary) -> &TypedCallAllocationFacts {
    let FunctionReturnAliasSummary::Known(facts) = summary else {
        panic!("expected completed call allocation facts, got {summary:?}");
    };
    facts
}

pub(super) fn result_sources(lane: &Lane, slot: usize) -> Vec<ValueSource> {
    let mut sources = Vec::new();
    lane.visit_compatible(
        |_| Ok::<_, std::convert::Infallible>(None),
        |_, product| {
            sources.extend(
                product
                    .components()
                    .iter()
                    .flat_map(|component| &component.alternatives)
                    .flat_map(|post| &post.results)
                    .filter(|(index, _)| *index == slot)
                    .map(|(_, value)| value.source.clone()),
            );
            Ok(())
        },
    )
    .unwrap();
    sources.sort_unstable();
    sources.dedup();
    sources
}

fn root_source_arguments(facts: &TypedCallAllocationFacts) -> Vec<usize> {
    let mut arguments = Vec::new();
    for slot in 0..facts.success.result_extent() {
        for source in result_sources(&facts.success, slot) {
            let port = match source {
                ValueSource::Allocation(ObjectRef::Entry(port))
                | ValueSource::Scalar(ScalarSource::EntryValue(port)) => port,
                _ => continue,
            };
            if let EntryPort::Value { parameter, path } = &facts.ports[port.0 as usize]
                && path.is_empty()
            {
                arguments.push(*parameter);
            }
        }
    }
    arguments.sort_unstable();
    arguments.dedup();
    arguments
}

fn i32_expression(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: i64,
    span: Option<SourceSpan>,
    mode: ValueMode,
) -> ExprId {
    builder
        .fixed_scalar(
            constants,
            FixedScalarValue::signed(FixedScalar::I32, value).expect("I32 literal in range"),
            span,
            mode,
        )
        .expect("valid I32 fixture")
        .id()
}

fn function_call_node(
    builder: &mut AstExpressionBuilder,
    name: PathId,
    args: Vec<CallArgument<OwnedExpr>>,
    result_type_ids: Vec<TypeId>,
    span: Option<SourceSpan>,
) -> NodeKind {
    NodeKind::ExpressionStatement(
        builder
            .function_call_with_typed_arguments(name, args, result_type_ids, span)
            .expect("valid function call fixture")
            .id(),
    )
}

fn host_function_call_node(
    builder: &mut AstExpressionBuilder,
    id: ExternalFunctionId,
    args: Vec<CallArgument<OwnedExpr>>,
    result_type_ids: Vec<TypeId>,
    span: Option<SourceSpan>,
) -> NodeKind {
    NodeKind::ExpressionStatement(
        builder
            .host_function_call_with_typed_arguments(id, args, result_type_ids, false, span)
            .expect("valid host call fixture")
            .id(),
    )
}

#[test]
fn public_call_summaries_cover_zero_parameter_and_parameter_effects() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let summary_target = symbol("summary_target", &mut path_fork, &mut string_table);
    let summary_target_name = summary_target;
    let shared_parameter = symbol("shared_parameter", &mut path_fork, &mut string_table);
    let mutable_parameter = symbol("mutable_parameter", &mut path_fork, &mut string_table);
    let untouched_mutable_parameter = symbol(
        "untouched_mutable_parameter",
        &mut path_fork,
        &mut string_table,
    );

    let target = function_node(
        summary_target,
        FunctionSignature {
            parameters: vec![
                param_with_type_id(
                    shared_parameter,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    false,
                    test_source_location(1),
                ),
                param_with_type_id(
                    mutable_parameter,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    true,
                    test_source_location(1),
                ),
                param_with_type_id(
                    untouched_mutable_parameter,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    true,
                    test_source_location(1),
                ),
            ],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![node(
            NodeKind::Assignment {
                target: assignment_target(
                    &mut expression_builder,
                    mutable_parameter,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    test_source_location(2),
                ),
                value: i32_expression(
                    &mut expression_builder,
                    &mut const_values,
                    2,
                    test_source_location(2),
                    ValueMode::ImmutableOwned,
                ),
            },
            test_source_location(2),
        )],
        None,
    );

    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![target, start],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("public call summary construction should succeed");

    assert_eq!(
        report.analysis.public_call_summaries.len(),
        hir.functions.len()
    );

    let target_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == summary_target_name)
        })
        .expect("summary target should lower to HIR")
        .id;
    let target_summary = report
        .analysis
        .public_call_summaries
        .get(&target_id)
        .expect("every local function should have one retained summary");

    assert_eq!(target_summary.parameters.len(), 3);
    assert_eq!(
        target_summary.parameters[0].access,
        PublicCallParameterAccess::Shared
    );
    assert_eq!(
        target_summary.parameters[0].mutation,
        PublicCallMutationEffect::NoWrite
    );
    assert_eq!(
        target_summary.parameters[0].transfer_effect,
        PublicCallTransferEffect::MayConsume
    );

    assert_eq!(
        target_summary.parameters[1].access,
        PublicCallParameterAccess::Mutable
    );
    assert_eq!(
        target_summary.parameters[1].mutation,
        PublicCallMutationEffect::Writes
    );
    assert_eq!(
        target_summary.parameters[1].transfer_effect,
        PublicCallTransferEffect::MayConsume
    );

    assert_eq!(
        target_summary.parameters[2].access,
        PublicCallParameterAccess::Mutable
    );
    assert_eq!(
        target_summary.parameters[2].mutation,
        PublicCallMutationEffect::NoWrite
    );

    let start_id = hir
        .start_function
        .expect("normal test module should have start");
    let start_summary = report
        .analysis
        .public_call_summaries
        .get(&start_id)
        .expect("the zero-parameter start function should have a summary");
    assert!(start_summary.parameters.is_empty());
    let start_facts = allocation_facts(&start_summary.return_alias);
    assert!(start_facts.success.can_return());
    assert_eq!(start_facts.success.result_extent(), 0);

    let rendered = format!("{target_summary:?}");
    assert!(!rendered.contains("LocalId"));
    assert!(!rendered.contains("BlockId"));
    assert!(!rendered.contains("SourceSpan"));
    assert!(!rendered.contains("PathId"));
}

#[test]
fn incomplete_public_call_summary_metadata_uses_infrastructure_lane() {
    let expression_builder = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);
    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
    );

    let mut hir = lower_hir(
        build_ast_with_registered_types(vec![start], expression_builder, const_values, types),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    hir.functions[0].params.push(LocalId(999_999));

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("partial parameter metadata should fail summary construction");
    assert_infrastructure_error_contains(
        &error,
        InfrastructureFailureKind::Internal,
        &["could not resolve mutability for parameter local"],
    );
}

#[test]
fn public_call_summary_keeps_mutable_but_unwritten_call_path_read_only() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);
    let mutator_name = symbol("mutator", &mut path_fork, &mut string_table);
    let wrapper_name = symbol("wrapper", &mut path_fork, &mut string_table);
    let wrapper_name_for_lookup = wrapper_name;
    let parameter_name = symbol("value", &mut path_fork, &mut string_table);

    let mutator = function_node(
        mutator_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                symbol("input", &mut path_fork, &mut string_table),
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                true,
                test_source_location(1),
            )],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
        None,
    );
    let wrapper = function_node(
        wrapper_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                parameter_name,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                true,
                test_source_location(2),
            )],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![node(
            {
                let arguments = vec![CallArgument::positional(
                    {
                        let value = immutable_reference_expr(
                            &mut expression_builder,
                            parameter_name,
                            builtin_type_ids::fixed_scalar(FixedScalar::I32),
                            test_source_location(3),
                        );
                        expression_builder.own(value)
                    },
                    CallAccessMode::Mutable,
                    test_source_location(3),
                )];
                function_call_node(
                    &mut expression_builder,
                    mutator_name,
                    arguments,
                    vec![],
                    test_source_location(3),
                )
            },
            test_source_location(3),
        )],
        None,
    );
    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![mutator, wrapper, start],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("mutable parameter call path should validate");
    let wrapper_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == wrapper_name_for_lookup)
        })
        .expect("wrapper function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&wrapper_id)
        .expect("wrapper should have a retained call summary");
    assert_eq!(
        summary.parameters[0].mutation,
        PublicCallMutationEffect::NoWrite
    );
}

#[test]
fn public_call_summary_tracks_mixed_argument_access_by_position() {
    let source = r#"mutator |shared I32, mutable ~I32|:
;

wrapper |shared I32, mutable ~I32|:
mutator(shared, ~mutable)
;"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let entry_path = path_fork
        .try_intern_portable_path("@entry.moth", &mut string_table)
        .expect("test entry path fits");
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let wrapper_name = path_fork
        .try_intern_child(entry_path, string_table.intern("wrapper"))
        .expect("test path fits");
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("mixed shared and mutable call arguments should validate");

    let wrapper_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == wrapper_name)
        })
        .expect("wrapper function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&wrapper_id)
        .expect("wrapper should have a retained call summary");

    assert_eq!(
        summary.parameters[0].mutation,
        PublicCallMutationEffect::NoWrite
    );
    assert_eq!(
        summary.parameters[1].mutation,
        PublicCallMutationEffect::NoWrite
    );
}

#[test]
fn parameter_alias_definition_does_not_publish_a_parameter_write() {
    // Issuing a mutation-capable alias records an exclusive access on the source parameter.
    // A definition cannot write the parameter, so the summary must stay read-only even when the
    // parameter is mutable and the alias is the only thing that touches it.
    let source = r#"reader |divisor ~I32| -> I32:
    n ~I32 = divisor
    return n
;"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let entry_path = path_fork
        .try_intern_portable_path("@entry.moth", &mut string_table)
        .expect("test entry path fits");
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("reading through a mutable alias of a mutable parameter should validate");

    let reader_name = path_fork
        .try_intern_child(entry_path, string_table.intern("reader"))
        .expect("test path fits");
    let reader_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == reader_name)
        })
        .expect("reader function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&reader_id)
        .expect("reader should have a retained call summary");

    assert_eq!(
        summary.parameters[0].access,
        PublicCallParameterAccess::Mutable
    );
    assert_eq!(
        summary.parameters[0].mutation,
        PublicCallMutationEffect::NoWrite
    );
    assert_eq!(
        summary.parameters[0].transfer_effect,
        PublicCallTransferEffect::MayConsume
    );
}

#[test]
fn mutable_alias_acquisition_rejects_read_only_source_places() {
    for source in [
        r#"reader |divisor I32| -> I32:
    n ~I32 = divisor
    return n
;"#,
        r#"reader |divisor ~I32| -> I32:
    shared = divisor
    n ~= shared
    return n
;"#,
        r#"reader || -> I32:
    total ~= 0
    loop 1 to 3 |value, ordinal|:
        ~total = ordinal
    ;
    return total
;"#,
    ] {
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
        let external_package_registry = default_external_package_registry(&mut string_table);
        let hir = lower_hir(
            ast,
            &mut string_table,
            &mut path_fork,
            &external_package_registry,
        );
        let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
            .expect_err("a read-only source place cannot grant mutable access");
        assert_invalid_mutable_access_reason(error, InvalidMutableAccessReason::ImmutablePlace);
    }
}

#[test]
fn mutable_scalar_reborrow_distinguishes_overlap_from_immutable_provenance() {
    let source = r#"identity |value I32| -> I32:
    return value
;

read_join |condition Bool| -> I32:
    original = 5
    slot ~= 0
    candidate ~= 0
    ~candidate = identity(original)
    if condition:
        ~candidate = slot
    ;
    writer ~= candidate
    return writer
;"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork, &registry);
    run_borrow_checker(&hir, &registry, &path_fork, &string_table)
        .expect("both reaching source cells are mutable");

    let source = r#"identity |value I32| -> I32:
    return value
;

read_view || -> I32:
    original = 5
    slot ~= 0
    ~slot = identity(original)
    view = slot
    writer ~= slot
    snapshot = copy writer
    return snapshot + view
;"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork, &registry);
    let error = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
        .expect_err("the shared view still overlaps acquisition of the writer");
    assert_invalid_mutable_access_reason(
        error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

/// Borrow-check `source` and return each named function's published parameter mutation effects,
/// in the order the names appear.
///
/// WHY: the summary tests repeat the same parse, lower and look-up pipeline, and an inline
/// look-up can silently assert against the wrong function.
fn named_function_parameter_mutations(
    source: &str,
    function_names: &[&str],
) -> Vec<Vec<PublicCallMutationEffect>> {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let entry_path = path_fork
        .try_intern_portable_path("@entry.moth", &mut string_table)
        .expect("test entry path fits");
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("named function summary fixture should validate");

    function_names
        .iter()
        .map(|name| {
            let name_path = path_fork
                .try_intern_child(entry_path, string_table.intern(name))
                .expect("test path fits");
            let function_id = hir
                .functions
                .iter()
                .find(|function| {
                    hir.side_table
                        .function_name_path(function.id)
                        .is_some_and(|path| path == name_path)
                })
                .unwrap_or_else(|| panic!("{name} should lower to HIR"))
                .id;
            report
                .analysis
                .public_call_summaries
                .get(&function_id)
                .unwrap_or_else(|| panic!("{name} should have a retained call summary"))
                .parameters
                .iter()
                .map(|parameter| parameter.mutation)
                .collect()
        })
        .collect()
}

fn local_call_mutation_summary_in_order(
    callee_body: &str,
    callee_declared_first: bool,
) -> (PublicCallMutationEffect, PublicCallMutationEffect) {
    let callee = format!("mutator |input ~I32|:\n{callee_body}\n;\n");
    let wrapper = "wrapper |value ~I32|:\n    mutator(~value)\n;\n";
    let source = if callee_declared_first {
        format!("{callee}\n{wrapper}")
    } else {
        format!("{wrapper}\n{callee}")
    };
    let mut mutations = named_function_parameter_mutations(&source, &["mutator", "wrapper"]);
    let wrapper = mutations.pop().expect("wrapper summary exists");
    let mutator = mutations.pop().expect("mutator summary exists");

    (mutator[0], wrapper[0])
}

#[test]
fn local_mutation_summary_keeps_unwritten_mutable_callee_order_independent() {
    for callee_declared_first in [true, false] {
        let (mutator_mutation, wrapper_mutation) =
            local_call_mutation_summary_in_order("", callee_declared_first);
        assert_eq!(mutator_mutation, PublicCallMutationEffect::NoWrite);
        assert_eq!(wrapper_mutation, PublicCallMutationEffect::NoWrite);
    }
}

#[test]
fn local_mutation_summary_propagates_writes_order_independently() {
    for callee_declared_first in [true, false] {
        let (mutator_mutation, wrapper_mutation) =
            local_call_mutation_summary_in_order("    ~input = 1", callee_declared_first);
        assert_eq!(mutator_mutation, PublicCallMutationEffect::Writes);
        assert_eq!(wrapper_mutation, PublicCallMutationEffect::Writes);
    }
}

#[test]
fn slot_rebind_of_mutable_parameter_publishes_no_write() {
    // The slot rebind reads `input` to acquire a mutation-capable alias of it. That exclusive
    // access must not be published as a write, nor inherited by callers through the wrapper.
    for callee_declared_first in [true, false] {
        let (mutator_mutation, wrapper_mutation) = local_call_mutation_summary_in_order(
            "    slot ~I32 = 0\n    ~slot = input",
            callee_declared_first,
        );
        assert_eq!(mutator_mutation, PublicCallMutationEffect::NoWrite);
        assert_eq!(wrapper_mutation, PublicCallMutationEffect::NoWrite);
    }
}

// WHAT: a write through a mutable binding that may alias a projected parameter field on one
//       control-flow path publishes a write to that parameter.
// WHY: the alias targets the containing allocation `pair` observed when the assignment stored
//      the projected value. Losing that target at the join would publish NoWrite and hide the
//      parameter mutation from summary propagation.
#[test]
fn projected_alias_write_at_join_publishes_parameter_mutation() {
    let source = r#"Pair = | first I32, second I32 |

shape |flag Bool, pair ~Pair|:
    cell ~= 0
    writer ~= 0
    if flag:
        ~writer = cell
    else
        ~writer = pair.first
    ;
    ~writer = 6
;

control |flag Bool, pair ~Pair| -> I32:
    cell ~= 0
    writer ~= 0
    if flag:
        ~writer = cell
    else
        ~writer = cell
    ;
    ~writer = 6
    return pair.first
;"#;
    let mutations = named_function_parameter_mutations(source, &["shape", "control"]);
    let (shape, control) = (mutations[0].as_slice(), mutations[1].as_slice());

    assert_eq!(
        shape,
        [
            PublicCallMutationEffect::NoWrite,
            PublicCallMutationEffect::Writes
        ]
    );
    assert_eq!(
        control,
        [
            PublicCallMutationEffect::NoWrite,
            PublicCallMutationEffect::NoWrite
        ]
    );
}

// WHAT: a write through a projected alias of a call-returned holder publishes a write to the
//       parameter whose allocation the projection is rooted in, and a forwarding wrapper
//       inherits it.
// WHY: the holder owns only its binding cell; the containing allocation is the parameter's.
//      Recording the holder cell publishes NoWrite and loses the mutation in summary
//      propagation. A whole-cell replacement of the holder stores a new value in that cell and
//      must stay NoWrite: it does not mutate the value it replaces.
#[test]
fn projected_write_through_call_returned_holder_publishes_parameter_mutation() {
    let source = r#"Pair = | first {I32}, second {I32} |

identity |value Pair| -> Pair:
    return value
;

mutator |input ~Pair|:
    holder ~= identity(input)
    writer ~= holder.first
    ~writer = {2, 3}
;

rebind |input ~Pair|:
    holder ~= identity(input)
    ~holder = Pair({2}, {3})
;

wrapper |value ~Pair|:
    mutator(~value)
;"#;
    let mutations = named_function_parameter_mutations(source, &["mutator", "rebind", "wrapper"]);

    assert_eq!(mutations[0], [PublicCallMutationEffect::Writes]);
    assert_eq!(mutations[1], [PublicCallMutationEffect::NoWrite]);
    assert_eq!(mutations[2], [PublicCallMutationEffect::Writes]);
}

#[test]
fn public_call_summary_map_set_only_writes_receiver_parameter() {
    let source = r#"mutate |scores ~{String = String}, key String, value String|:
~scores.set(key, value) catch:
;
;"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let entry_path = path_fork
        .try_intern_portable_path("@entry.moth", &mut string_table)
        .expect("test entry path fits");
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let mutate_name = path_fork
        .try_intern_child(entry_path, string_table.intern("mutate"))
        .expect("test path fits");
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("map set with mutable receiver should validate");

    let function_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == mutate_name)
        })
        .expect("map mutator function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&function_id)
        .expect("map mutator should have a retained call summary");

    assert_eq!(
        summary.parameters[0].mutation,
        PublicCallMutationEffect::Writes
    );
    assert_eq!(
        summary.parameters[1].mutation,
        PublicCallMutationEffect::NoWrite
    );
    assert_eq!(
        summary.parameters[2].mutation,
        PublicCallMutationEffect::NoWrite
    );
}

#[test]
fn immutable_shared_parameter_optional_transfer_remains_legal() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);
    let reader_name = symbol("reader", &mut path_fork, &mut string_table);
    let value_name = symbol("value", &mut path_fork, &mut string_table);

    let reader = function_node(
        reader_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                symbol("input", &mut path_fork, &mut string_table),
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                test_source_location(1),
            )],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
        None,
    );
    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    value_name,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(2),
                        ValueMode::ImmutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(2),
            ),
            node(
                {
                    let arguments = vec![CallArgument::positional(
                        {
                            let value = immutable_reference_expr(
                                &mut expression_builder,
                                value_name,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                test_source_location(3),
                            );
                            expression_builder.own(value)
                        },
                        CallAccessMode::Shared,
                        test_source_location(3),
                    )];
                    function_call_node(
                        &mut expression_builder,
                        reader_name,
                        arguments,
                        vec![],
                        test_source_location(3),
                    )
                },
                test_source_location(3),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![reader, start],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("optional transfer of an immutable shared parameter should fall back to borrowing");
}

#[test]
fn path_dependent_optional_call_records_transfer_or_borrow_without_invalidating_source() {
    let source = r#"inspect |value String|:
;

optional_transfer |flag Bool, op_name String|:
if flag:
    inspect(op_name)
else
    inspect(op_name)
    retained = op_name
;
sentinel = 0
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("path-dependent optional transfer should fall back to borrowing");

    let op_name_local = hir
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| {
            hir.side_table
                .resolve_local_name(local.id, &path_fork, &string_table)
                == Some("op_name")
        })
        .map(|local| local.id)
        .expect("should locate the path-dependent parameter");

    assert!(
        report.analysis.value_facts.values().any(|fact| {
            fact.optional_transfer == OptionalTransferStatus::Transfer
                && fact.roots.contains(&op_name_local)
        }),
        "the final-use branch should record advisory transfer"
    );
    assert!(
        report.analysis.value_facts.values().any(|fact| {
            fact.optional_transfer == OptionalTransferStatus::Borrow
                && fact.roots.contains(&op_name_local)
        }),
        "the retaining branch should record a borrow fallback"
    );

    let sentinel_statement = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(local),
                    ..
                } if hir.side_table.resolve_local_name(*local, &path_fork, &string_table)
                    == Some("sentinel")
            )
        })
        .expect("should locate the post-join sentinel assignment");
    let op_name_snapshot = report
        .analysis
        .statement_entry_states
        .get(&sentinel_statement.id)
        .and_then(|state| {
            state
                .locals
                .iter()
                .find(|local| local.local == op_name_local)
        })
        .expect("post-join state should include the parameter");
    assert!(
        !op_name_snapshot.mode.is_definitely_uninit(),
        "optional transfer must not make the parameter unavailable at the join"
    );
    assert!(
        op_name_snapshot.mode.contains(LocalMode::SLOT),
        "post-join source state should retain its initialized slot"
    );
}

#[test]
fn unused_alias_before_final_shared_call_falls_back_to_borrow() {
    let source = r#"
inspect |value String|:
;

check |source String|:
alias = source
inspect(source)
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("an unused alias must not make a final shared call invalid");

    let source_local = hir
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| {
            hir.side_table
                .resolve_local_name(local.id, &path_fork, &string_table)
                == Some("source")
        })
        .map(|local| local.id)
        .expect("should locate the source parameter");
    let call_argument = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| {
            let HirStatementKind::Call { args, .. } = &statement.kind else {
                return None;
            };
            let argument_id = *hir.expressions.values(*args).first()?;
            let argument = hir.expressions.expression(argument_id);
            matches!(
                &argument.kind,
                crate::compiler_frontend::hir::expressions::HirExpressionKind::Load(place)
                    if place.root == source_local
                        && hir.expressions.projections(place.projections).is_empty()
            )
            .then_some(argument_id)
        })
        .expect("should locate the final shared call argument");

    assert_eq!(
        report
            .analysis
            .value_fact(call_argument)
            .expect("the call argument should have a borrow fact")
            .optional_transfer,
        OptionalTransferStatus::Borrow,
        "failed transfer exclusivity must fall back to an ordinary shared borrow"
    );
}

#[test]
fn user_function_returning_param_alias_allows_caller_rebinding() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let alias_fn = symbol("alias_fn", &mut path_fork, &mut string_table);
    let p = symbol("p", &mut path_fork, &mut string_table);
    let x = symbol("x", &mut path_fork, &mut string_table);
    let y = symbol("y", &mut path_fork, &mut string_table);

    let callee = function_node(
        alias_fn,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                p,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                test_source_location(1),
            )],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [builtin_type_ids::fixed_scalar(FixedScalar::I32)],
                None,
            )),
        },
        vec![node(
            NodeKind::Return(vec![immutable_reference_expr(
                &mut expression_builder,
                p,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                test_source_location(2),
            )]),
            test_source_location(2),
        )],
        None,
    );

    let caller = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    {
                        let arguments: Vec<_> = vec![immutable_reference_expr(
                            &mut expression_builder,
                            x,
                            builtin_type_ids::fixed_scalar(FixedScalar::I32),
                            test_source_location(11),
                        )]
                        .into_iter()
                        .map(|value| {
                            CallArgument::positional(
                                expression_builder.own(value),
                                CallAccessMode::Shared,
                                None,
                            )
                        })
                        .collect();
                        expression_builder.function_call_with_typed_arguments(
                            alias_fn,
                            arguments,
                            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
                            test_source_location(11),
                        )
                    }
                    .expect("valid call fixture")
                    .id(),
                    &expression_builder,
                )),
                test_source_location(11),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        &mut expression_builder,
                        x,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(12),
                    ),
                    value: i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        2,
                        test_source_location(12),
                        ValueMode::ImmutableOwned,
                    ),
                },
                test_source_location(12),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![callee, caller],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("reassigning the source binding should detach it from the returned alias");
}

#[test]
fn slot_backed_alias_result_rebinding_to_another_alias_detaches_old_root() {
    let source = r#"Point = |
x I32,
|

identity |value Point| -> Point:
return value
;

original ~= Point(1)
returned ~= identity(original)
other ~= Point(2)
other_returned ~= identity(other)

~returned = other_returned
~original.x = 4
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );

    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("slot-backed result rebinding should detach its previous allocation root");
}

#[test]
fn fallible_alias_return_propagation_validates_success_alias_metadata() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let source_fn = symbol("source_fn", &mut path_fork, &mut string_table);
    let source_param = symbol("source_param", &mut path_fork, &mut string_table);
    let source_unused = symbol("source_unused", &mut path_fork, &mut string_table);
    let forward_fn = symbol("forward_fn", &mut path_fork, &mut string_table);
    let forward_param = symbol("forward_param", &mut path_fork, &mut string_table);
    let source_fn_name_for_lookup = source_fn;
    let forward_fn_name_for_lookup = forward_fn;

    let source = function_node(
        source_fn,
        FunctionSignature {
            parameters: vec![
                param_with_type_id(
                    source_param,
                    builtin_type_ids::STRING,
                    false,
                    test_source_location(20),
                ),
                param_with_type_id(
                    source_unused,
                    builtin_type_ids::STRING,
                    false,
                    test_source_location(20),
                ),
            ],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [builtin_type_ids::STRING],
                Some(builtin_type_ids::STRING),
            )),
        },
        vec![node(
            NodeKind::Return(vec![immutable_reference_expr(
                &mut expression_builder,
                source_unused,
                builtin_type_ids::STRING,
                test_source_location(21),
            )]),
            test_source_location(21),
        )],
        None,
    );

    let propagated_call = {
        let arguments = vec![
            CallArgument::positional(
                expression_builder
                    .string_slice(
                        &mut const_values,
                        string_table.intern("unused"),
                        test_source_location(30),
                        ValueMode::ImmutableOwned,
                    )
                    .expect("valid expression fixture"),
                CallAccessMode::Shared,
                test_source_location(30),
            ),
            CallArgument::positional(
                {
                    let value = immutable_reference_expr(
                        &mut expression_builder,
                        forward_param,
                        builtin_type_ids::STRING,
                        test_source_location(30),
                    );
                    expression_builder.own(value)
                },
                CallAccessMode::Shared,
                test_source_location(30),
            ),
        ];
        expression_builder.handled_fallible_function_call_with_typed_arguments(
            source_fn,
            arguments,
            vec![builtin_type_ids::STRING],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Propagate,
            None,
        )
    }
    .expect("valid handled call fixture")
    .id();

    let forward = function_node(
        forward_fn,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                forward_param,
                builtin_type_ids::STRING,
                false,
                test_source_location(29),
            )],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [builtin_type_ids::STRING],
                Some(builtin_type_ids::STRING),
            )),
        },
        vec![node(
            NodeKind::Return(vec![propagated_call]),
            test_source_location(30),
        )],
        None,
    );

    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(40))],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![source, forward, start],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("fallible alias forwarding should validate declared success alias metadata");

    for (function_name, argument) in [
        (&source_fn_name_for_lookup, 1),
        (&forward_fn_name_for_lookup, 0),
    ] {
        let function_id = hir
            .functions
            .iter()
            .find(|function| {
                hir.side_table
                    .function_name_path(function.id)
                    .is_some_and(|path| path == *function_name)
            })
            .expect("fallible alias function should lower to HIR")
            .id;
        let summary = report
            .analysis
            .public_call_summaries
            .get(&function_id)
            .expect("fallible alias function should have a retained call summary");
        let facts = allocation_facts(&summary.return_alias);
        assert_eq!(root_source_arguments(facts), [argument]);
        assert_eq!(result_sources(&facts.success, 0).len(), 1);
        assert_eq!(facts.error, Lane::Bottom);
    }
}

#[test]
fn fresh_user_return_does_not_alias_caller_roots() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let fresh_fn = symbol("fresh_fn", &mut path_fork, &mut string_table);
    let fresh_fn_name_for_lookup = fresh_fn;
    let p = symbol("p", &mut path_fork, &mut string_table);
    let x = symbol("x", &mut path_fork, &mut string_table);
    let y = symbol("y", &mut path_fork, &mut string_table);

    let callee = function_node(
        fresh_fn,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                p,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                test_source_location(1),
            )],
            return_contract: FunctionReturnContract::Returns(fresh_success_returns(vec![
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ])),
        },
        vec![node(
            NodeKind::Return(vec![i32_expression(
                &mut expression_builder,
                &mut const_values,
                42,
                test_source_location(2),
                ValueMode::ImmutableOwned,
            )]),
            test_source_location(2),
        )],
        None,
    );

    let caller = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    {
                        let arguments: Vec<_> = vec![immutable_reference_expr(
                            &mut expression_builder,
                            x,
                            builtin_type_ids::fixed_scalar(FixedScalar::I32),
                            test_source_location(11),
                        )]
                        .into_iter()
                        .map(|value| {
                            CallArgument::positional(
                                expression_builder.own(value),
                                CallAccessMode::Shared,
                                None,
                            )
                        })
                        .collect();
                        expression_builder.function_call_with_typed_arguments(
                            fresh_fn,
                            arguments,
                            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
                            test_source_location(11),
                        )
                    }
                    .expect("valid call fixture")
                    .id(),
                    &expression_builder,
                )),
                test_source_location(11),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        &mut expression_builder,
                        x,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(12),
                    ),
                    value: i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        2,
                        test_source_location(12),
                        ValueMode::ImmutableOwned,
                    ),
                },
                test_source_location(12),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![callee, caller],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("fresh callee returns should not alias caller roots");
    let fresh_function_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == fresh_fn_name_for_lookup)
        })
        .expect("fresh function should lower to HIR")
        .id;
    let fresh_summary = report
        .analysis
        .public_call_summaries
        .get(&fresh_function_id)
        .expect("fresh function should have a retained call summary");
    let facts = allocation_facts(&fresh_summary.return_alias);
    assert_eq!(
        result_sources(&facts.success, 0),
        [ValueSource::Scalar(ScalarSource::Independent)]
    );
}

#[test]
fn multi_return_fallible_external_preserves_ordered_contract() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let external_id = Arc::make_mut(&mut external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "imprecise_external".to_owned(),
            parameters: vec![],
            return_contract: external_returns(
                vec![
                    ExternalReturnSlot::fresh(RegistryAbiType::Fixed(FixedScalar::I32)),
                    ExternalReturnSlot::fresh(RegistryAbiType::Fixed(FixedScalar::I32)),
                ],
                Some(ExternalSignatureType::Abi(RegistryAbiType::Fixed(
                    FixedScalar::I32,
                ))),
            ),
            const_eval: None,
            numeric_contract: ExternalNumericContract::Unknown,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("fallible external fixture registration should succeed");
    let external = external_package_registry
        .get_function_by_id(external_id)
        .expect("registered external definition");
    assert_eq!(external.hir_return_sources().len(), 2);

    let imprecise_return_name = symbol("imprecise_return", &mut path_fork, &mut string_table);
    let fallible_external_call = {
        let arguments = HandledFallibleHostFunctionCallInput {
            id: external_id,
            args: vec![],
            result_type_ids: vec![
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ],
            requires_external_float_validation: false,
            error_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            handling: FallibleExpressionHandling::Propagate,
            span: None,
        };
        expression_builder.handled_fallible_host_function_call_with_typed_arguments(arguments)
    }
    .expect("valid handled host call fixture")
    .id();
    let imprecise_return = function_node(
        imprecise_return_name,
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                ],
                Some(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
            )),
        },
        vec![node(
            NodeKind::Return(vec![fallible_external_call]),
            test_source_location(2),
        )],
        None,
    );
    let forward_name = symbol(
        "forward_imprecise_return",
        &mut path_fork,
        &mut string_table,
    );
    let forwarded_call = {
        let arguments = vec![];
        expression_builder.handled_fallible_function_call_with_typed_arguments(
            imprecise_return_name,
            arguments,
            vec![
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ],
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            FallibleExpressionHandling::Propagate,
            None,
        )
    }
    .expect("valid handled call fixture")
    .id();
    let forward = function_node(
        forward_name,
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                ],
                Some(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
            )),
        },
        vec![node(
            NodeKind::Return(vec![forwarded_call]),
            test_source_location(3),
        )],
        None,
    );
    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![imprecise_return, forward, start],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("fallible external return should validate");
    let mut reversed_hir = hir.clone();
    reversed_hir.functions.reverse();
    let reversed_report = run_borrow_checker(
        &reversed_hir,
        &external_package_registry,
        &path_fork,
        &string_table,
    )
    .expect("reversed fallible external return should validate");

    for analysis in [&report.analysis, &reversed_report.analysis] {
        for function_name in [&imprecise_return_name, &forward_name] {
            let function_id = hir
                .functions
                .iter()
                .find(|function| {
                    hir.side_table
                        .function_name_path(function.id)
                        .is_some_and(|path| path == *function_name)
                })
                .expect("imprecise return function should lower to HIR")
                .id;
            let summary = analysis
                .public_call_summaries
                .get(&function_id)
                .expect("forwarded imprecise function should have a retained call summary");
            assert_eq!(
                summary, &report.analysis.public_call_summaries[&function_id],
                "declaration order must not affect the forwarded native contract"
            );
            let facts = allocation_facts(&summary.return_alias);
            assert!(facts.success.can_return() && facts.error.can_return());
            let function = hir
                .functions
                .iter()
                .find(|function| function.id == function_id)
                .expect("summary function should be present");
            let problem = from_hir(
                &hir,
                function,
                Some(&analysis.public_call_summaries),
                Some(&external_package_registry),
                None,
            )
            .expect("external and wrapper provenance should normalize");
            let native_results = problem
                .events()
                .iter()
                .find_map(|event| {
                    let EventKind::CallEffect(effect) = &event.kind else {
                        return None;
                    };
                    (effect.results.len() == 2).then_some(&effect.results)
                })
                .expect("the normalized success edge has both native destinations");
            assert_eq!(
                native_results[0].origin, native_results[1].origin,
                "the Boracle adapter must not invent distinct origins from ABI slot positions"
            );
        }
    }
}

#[test]
fn checked_numeric_return_local_retains_fresh_alias_summary() {
    let source = r#"
increment |input I32| -> I32, Error!:
return input + 1
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let entry_path = path_fork
        .try_intern_portable_path("@entry.moth", &mut string_table)
        .expect("test entry path fits");
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let increment_name = path_fork
        .try_intern_child(entry_path, string_table.intern("increment"))
        .expect("test path fits");
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("checked numeric return should validate");

    let function_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == increment_name)
        })
        .expect("numeric return function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&function_id)
        .expect("numeric return function should have a call summary");

    let facts = allocation_facts(&summary.return_alias);
    assert_eq!(
        result_sources(&facts.success, 0),
        [ValueSource::Scalar(ScalarSource::Independent)]
    );
}

#[test]
fn map_remove_return_local_retains_fresh_alias_summary() {
    let source = r#"
remove_value |scores ~{String = String}| -> String, Error!:
return ~scores.remove("key")!
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let entry_path = path_fork
        .try_intern_portable_path("@entry.moth", &mut string_table)
        .expect("test entry path fits");
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let remove_value_name = path_fork
        .try_intern_child(entry_path, string_table.intern("remove_value"))
        .expect("test path fits");
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("map remove return should validate");

    let function_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == remove_value_name)
        })
        .expect("map return function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&function_id)
        .expect("map return function should have a call summary");

    let facts = allocation_facts(&summary.return_alias);
    assert!(facts.success.can_return() && facts.error.can_return());
    assert_eq!(facts.success.result_extent(), 1);
    assert_eq!(facts.error.result_extent(), 1);
}

#[test]
fn multi_return_summary_retains_slot_sources_and_sharing_separately() {
    let source = r#"
Point = |
x I32,
|

pair |left Point, right Point| -> Point, Point:
return left, right
;

duplicate |value Point| -> Point, Point:
return value, value
;

copy_one |value Point| -> Point, Point:
return value, copy value
;
"#;
    let fixture = PreparedHirFixture::new(source);
    let hir = &fixture.hir;
    let report = run_prepared_borrow_checker(&fixture)
        .expect("multi-return alias summaries should validate");

    let summary_for = |name: &str| {
        let function = hir
            .functions
            .iter()
            .find(|function| {
                hir.side_table
                    .function_name_path(function.id)
                    .is_some_and(|path| {
                        fixture
                            .paths
                            .component(path)
                            .is_some_and(|component| fixture.strings.resolve(component) == name)
                    })
            })
            .expect("multi-return function should lower to HIR");
        report
            .analysis
            .public_call_summaries
            .get(&function.id)
            .expect("multi-return function should have a call summary")
            .return_alias
            .clone()
    };

    for (name, first, second) in [
        ("pair", 0, Some(1)),
        ("duplicate", 0, Some(0)),
        ("copy_one", 0, None),
    ] {
        let summary = summary_for(name);
        let facts = allocation_facts(&summary);
        for (slot, parameter) in [Some(first), second].into_iter().enumerate() {
            match (result_sources(&facts.success, slot).as_slice(), parameter) {
                ([ValueSource::Allocation(ObjectRef::Entry(port))], Some(parameter)) => {
                    assert_eq!(
                        facts.ports[port.0 as usize],
                        EntryPort::Value {
                            parameter,
                            path: Box::new([])
                        },
                        "{name}, slot {slot}",
                    );
                }
                ([ValueSource::Allocation(ObjectRef::Exact(_))], None) => {}
                (sources, _) => panic!("{name}, slot {slot}: {sources:?}"),
            }
        }
    }
}

#[test]
fn nonreturning_recursive_cycle_does_not_create_a_fresh_result() {
    let source = r#"
recursive |value I32| -> I32:
return recursive(value)
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("recursive summary cycle should remain a valid conservative analysis");
    let recursive_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| {
                    path_fork
                        .component(path)
                        .is_some_and(|component| string_table.resolve(component) == "recursive")
                })
        })
        .expect("recursive function should lower to HIR")
        .id;

    let summary = &report.analysis.public_call_summaries[&recursive_id];
    let facts = allocation_facts(&summary.return_alias);
    assert_eq!(facts.success, Lane::Bottom);
}

#[test]
fn inferred_alias_return_from_parameter_reference_allows_caller_rebinding() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let unknown_fn = symbol("unknown_fn", &mut path_fork, &mut string_table);
    let p = symbol("p", &mut path_fork, &mut string_table);
    let q = symbol("q", &mut path_fork, &mut string_table);
    let x = symbol("x", &mut path_fork, &mut string_table);
    let y = symbol("y", &mut path_fork, &mut string_table);

    let callee = function_node(
        unknown_fn,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                p,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                test_source_location(1),
            )],
            return_contract: FunctionReturnContract::Returns(fresh_success_returns(vec![
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ])),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    q,
                    immutable_reference_expr(
                        &mut expression_builder,
                        p,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(2),
                    ),
                    &expression_builder,
                )),
                test_source_location(2),
            ),
            node(
                NodeKind::Return(vec![immutable_reference_expr(
                    &mut expression_builder,
                    q,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    test_source_location(3),
                )]),
                test_source_location(3),
            ),
        ],
        None,
    );

    let caller = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    {
                        let arguments: Vec<_> = vec![immutable_reference_expr(
                            &mut expression_builder,
                            x,
                            builtin_type_ids::fixed_scalar(FixedScalar::I32),
                            test_source_location(11),
                        )]
                        .into_iter()
                        .map(|value| {
                            CallArgument::positional(
                                expression_builder.own(value),
                                CallAccessMode::Shared,
                                None,
                            )
                        })
                        .collect();
                        expression_builder.function_call_with_typed_arguments(
                            unknown_fn,
                            arguments,
                            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
                            test_source_location(11),
                        )
                    }
                    .expect("valid call fixture")
                    .id(),
                    &expression_builder,
                )),
                test_source_location(11),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        &mut expression_builder,
                        x,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(12),
                    ),
                    value: i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        2,
                        test_source_location(12),
                        ValueMode::ImmutableOwned,
                    ),
                },
                test_source_location(12),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![callee, caller],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("reassigning the source binding should preserve the returned allocation alias");
}

#[test]
fn mutable_user_argument_is_accepted_without_false_shared_conflict() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let mut_sink = symbol("mut_sink", &mut path_fork, &mut string_table);
    let p = symbol("p", &mut path_fork, &mut string_table);
    let x = symbol("x", &mut path_fork, &mut string_table);

    let callee = function_node(
        mut_sink,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                p,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                true,
                test_source_location(1),
            )],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
        None,
    );

    let caller = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                {
                    let arguments = vec![CallArgument::positional(
                        {
                            let value = immutable_reference_expr(
                                &mut expression_builder,
                                x,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                test_source_location(11),
                            );
                            expression_builder.own(value)
                        },
                        CallAccessMode::Shared,
                        test_source_location(11),
                    )];
                    function_call_node(
                        &mut expression_builder,
                        mut_sink,
                        arguments,
                        vec![],
                        test_source_location(11),
                    )
                },
                test_source_location(11),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![callee, caller],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("single mutable argument call should be accepted");
}

#[test]
fn mutable_call_uses_rvalue_backed_binding_as_its_access_actor() {
    let source = r#"
make_values || -> {I32}:
return {1}
;

mutate |values ~{I32}|:
~values.push(2)
;

items ~= make_values()
mutate(~items)
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );

    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("a mutable borrow through its own RValue-backed binding should pass");
}

#[test]
fn checked_decimal_result_can_initialize_a_fresh_mutable_argument() {
    let source = r#"
produce |divisor Dec2| -> Dec2:
return 1.00 / divisor
;

replace |value ~Dec2|:
~value = 4.00
;

replace(produce(2.00))
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("a checked Decimal result is a value-backed fresh mutable argument");
}

pub(super) fn assert_borrow_code_at(
    error: BorrowCheckError,
    source: &str,
    access: &str,
    code: &str,
) {
    let draft = error
        .into_draft_or_infrastructure()
        .expect("borrow diagnostic");
    let bag = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let diagnostic = bag.first().expect("borrow diagnostic");
    assert_eq!(diagnostic.descriptor().code, code, "{access}");
    let start = source.find(access).expect("authored access") as u32;
    let range = start..start + access.len() as u32;
    assert!(
        matches!(
            diagnostic.primary_site(),
            SiteRef::Local(span) if span.local().inline_range().is_some_and(|site| range.contains(&site.start()))
        ),
        "diagnostic should point inside {access}",
    );
}

#[test]
fn call_reservations_distinguish_projected_cells_and_fresh_values() {
    for (call, expected_code) in [
        ("update(~pair.left, ~pair.right)", None),
        ("update(~pair.left, ~pair.left)", Some("MOTH-BORROW-0002")),
        ("update_holder(~pair.left, ~pair)", Some("MOTH-BORROW-0002")),
        ("update(~pair.left, 8)", None),
        ("update(copy pair.left, ~pair.left)", None),
        ("update_and_read(~pair.left, pair.right)", None),
        (
            "update_and_read(~pair.left, pair.left)",
            Some("MOTH-BORROW-0003"),
        ),
    ] {
        let source = format!(
            "Pair = |left I32, right I32|\n\
             update |first ~I32, second ~I32|:\n\
             ~first = 3\n\
             ~second = 4\n\
             ;\n\
             update_holder |field ~I32, holder ~Pair|:\n\
             ~field = 3\n\
             ~holder = Pair(4, 5)\n\
             ;\n\
             update_and_read |target ~I32, observed I32|:\n\
             ~target = copy observed\n\
             ;\n\
             probe || -> I32:\n\
             pair ~= Pair(1, 2)\n\
             {call}\n\
             return pair.left\n\
             ;"
        );
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(&source);
        let registry = default_external_package_registry(&mut string_table);
        let (mut hir, mut types) = lower_ast(ast, &mut string_table, &mut path_fork, &registry)
            .expect("the reservation fixture should lower");
        let mut summaries = seed_call_summaries(&hir, &path_fork, &string_table)
            .expect("the fixture's declarations should seed call summaries");
        infer_builtin_failure_summaries(&hir, &mut summaries);
        install_private_failure_lanes(&mut hir, &summaries, &mut types, &registry)
            .expect("the fixture's private call lanes should be finalized");
        let links = collect_module_function_link_facts(&hir).expect("the fixture's call links");
        let result =
            crate::compiler_frontend::ast::generic_functions::with_registered_type_projection(
                &types,
                &registry,
                |projection| {
                    refresh_call_allocations(
                        &hir,
                        &types,
                        &registry,
                        &path_fork,
                        &string_table,
                        (&links, &Default::default(), projection),
                        &mut summaries,
                    )
                    .expect("call allocation summaries should converge before validation");
                    check_borrows(
                        &hir,
                        &types,
                        &registry,
                        &path_fork,
                        &string_table,
                        Some((summaries.analysis.public_call_summaries, projection)),
                    )
                },
            );
        if let Some(code) = expected_code {
            let error = result.expect_err("overlapping call operands must conflict");
            assert_borrow_code_at(error, &source, call, code);
        } else {
            result.unwrap_or_else(|error| panic!("{call} should be legal: {error:?}"));
        }
    }
}

#[test]
fn builtin_receiver_edit_checks_a_live_lookup_on_a_sibling_receiver() {
    let edit = "~pair.right.push(7)";
    for (right, shared) in [("{2}", false), ("items", true)] {
        let source = format!(
            "Pair = |left {{I32}}, right {{I32}}|\n\
             probe || -> I32, Error!:\n\
             items ~= {{1}}\n\
             pair ~= Pair(items, {right})\n\
             observed = pair.left.get(0)!\n\
             {edit}\n\
             return observed\n\
             ;"
        );
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(&source);
        let registry = default_external_package_registry(&mut string_table);
        let hir = lower_hir(ast, &mut string_table, &mut path_fork, &registry);
        let result = run_borrow_checker(&hir, &registry, &path_fork, &string_table);
        if shared {
            let error = result.expect_err("the edit invalidates the shared receiver's live lookup");
            assert_borrow_code_at(error, &source, edit, "MOTH-BORROW-0007");
        } else {
            result.expect("editing a distinct receiver does not invalidate the lookup");
        }
    }
}

#[test]
fn mutable_user_call_with_fresh_mutable_arg_does_not_alias_existing_place_argument() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let mut2 = symbol("mut2", &mut path_fork, &mut string_table);
    let a = symbol("a", &mut path_fork, &mut string_table);
    let b = symbol("b", &mut path_fork, &mut string_table);
    let x = symbol("x", &mut path_fork, &mut string_table);

    let callee = function_node(
        mut2,
        FunctionSignature {
            parameters: vec![
                param_with_type_id(
                    a,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    true,
                    test_source_location(1),
                ),
                param_with_type_id(
                    b,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    true,
                    test_source_location(1),
                ),
            ],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
        None,
    );

    let caller = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                {
                    let arguments = vec![
                        CallArgument::positional(
                            {
                                let value = immutable_reference_expr(
                                    &mut expression_builder,
                                    x,
                                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                    test_source_location(11),
                                );
                                expression_builder.own(value)
                            },
                            CallAccessMode::Shared,
                            test_source_location(11),
                        ),
                        CallArgument::positional(
                            {
                                let value = i32_expression(
                                    &mut expression_builder,
                                    &mut const_values,
                                    2,
                                    test_source_location(11),
                                    ValueMode::ImmutableOwned,
                                );
                                expression_builder.own(value)
                            },
                            CallAccessMode::Shared,
                            test_source_location(11),
                        )
                        .with_passing_mode(CallPassingMode::FreshMutableValue),
                    ];
                    function_call_node(
                        &mut expression_builder,
                        mut2,
                        arguments,
                        vec![],
                        test_source_location(11),
                    )
                },
                test_source_location(11),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![callee, caller],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table).expect(
    "fresh mutable call args should be treated as independent locals, not aliases of other args",
);
}

#[test]
fn host_mutable_parameter_requires_mutable_access() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let host_fn = register_external_function(
        &mut external_package_registry,
        "host_mut",
        vec![ExternalAccessKind::Mutable],
        ExternalReturnAlias::Fresh,
        ExternalReturnType::Void,
    );
    let x = symbol("x", &mut path_fork, &mut string_table);

    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(1),
                        ValueMode::ImmutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(1),
            ),
            node(
                {
                    let arguments = vec![CallArgument::positional(
                        {
                            let value = immutable_reference_expr(
                                &mut expression_builder,
                                x,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                test_source_location(2),
                            );
                            expression_builder.own(value)
                        },
                        CallAccessMode::Shared,
                        test_source_location(2),
                    )];
                    host_function_call_node(
                        &mut expression_builder,
                        host_fn,
                        arguments,
                        vec![],
                        test_source_location(2),
                    )
                },
                test_source_location(2),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start], expression_builder, const_values, types),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("mutable host parameter should enforce mutable access");
    assert_invalid_mutable_access_reason(error, InvalidMutableAccessReason::ImmutablePlace);
}

#[test]
fn host_mutable_parameter_accepts_mutable_local_argument() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let host_fn = register_external_function(
        &mut external_package_registry,
        "host_mut_ok",
        vec![ExternalAccessKind::Mutable],
        ExternalReturnAlias::Fresh,
        ExternalReturnType::Void,
    );
    let x = symbol("x", &mut path_fork, &mut string_table);

    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(1),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(1),
            ),
            node(
                {
                    let arguments = vec![CallArgument::positional(
                        {
                            let value = immutable_reference_expr(
                                &mut expression_builder,
                                x,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                test_source_location(2),
                            );
                            expression_builder.own(value)
                        },
                        CallAccessMode::Shared,
                        test_source_location(2),
                    )];
                    host_function_call_node(
                        &mut expression_builder,
                        host_fn,
                        arguments,
                        vec![],
                        test_source_location(2),
                    )
                },
                test_source_location(2),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start], expression_builder, const_values, types),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("mutable host parameter should accept mutable local argument");
}

#[test]
fn host_shared_parameter_is_shared_only() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let host_fn = register_external_function(
        &mut external_package_registry,
        "host_shared",
        vec![ExternalAccessKind::Shared],
        ExternalReturnAlias::Fresh,
        ExternalReturnType::Void,
    );
    let x = symbol("x", &mut path_fork, &mut string_table);

    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(1),
                        ValueMode::ImmutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(1),
            ),
            node(
                {
                    let arguments = vec![CallArgument::positional(
                        {
                            let value = immutable_reference_expr(
                                &mut expression_builder,
                                x,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                test_source_location(2),
                            );
                            expression_builder.own(value)
                        },
                        CallAccessMode::Shared,
                        test_source_location(2),
                    )];
                    host_function_call_node(
                        &mut expression_builder,
                        host_fn,
                        arguments,
                        vec![],
                        test_source_location(2),
                    )
                },
                test_source_location(2),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start], expression_builder, const_values, types),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("shared host parameter should not force mutable access");
}

#[test]
fn two_mutable_args_to_same_root_are_rejected() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let mut2 = symbol("mut2", &mut path_fork, &mut string_table);
    let a = symbol("a", &mut path_fork, &mut string_table);
    let b = symbol("b", &mut path_fork, &mut string_table);
    let x = symbol("x", &mut path_fork, &mut string_table);
    let y = symbol("y", &mut path_fork, &mut string_table);

    let callee = function_node(
        mut2,
        FunctionSignature {
            parameters: vec![
                param_with_type_id(
                    a,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    true,
                    test_source_location(1),
                ),
                param_with_type_id(
                    b,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    true,
                    test_source_location(1),
                ),
            ],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
        None,
    );

    let caller = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                {
                    let arguments = vec![
                        CallArgument::positional(
                            {
                                let value = immutable_reference_expr(
                                    &mut expression_builder,
                                    x,
                                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                    test_source_location(11),
                                );
                                expression_builder.own(value)
                            },
                            CallAccessMode::Shared,
                            test_source_location(11),
                        ),
                        CallArgument::positional(
                            {
                                let value = immutable_reference_expr(
                                    &mut expression_builder,
                                    x,
                                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                    test_source_location(11),
                                );
                                expression_builder.own(value)
                            },
                            CallAccessMode::Shared,
                            test_source_location(11),
                        ),
                    ];
                    function_call_node(
                        &mut expression_builder,
                        mut2,
                        arguments,
                        vec![],
                        test_source_location(11),
                    )
                },
                test_source_location(11),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    immutable_reference_expr(
                        &mut expression_builder,
                        x,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(12),
                    ),
                    &expression_builder,
                )),
                test_source_location(12),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![callee, caller],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("two mutable args to the same root should fail");
    let draft = error
        .into_draft_or_infrastructure()
        .expect("borrow failure should retain its typed diagnostic");
    let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let diagnostic = compact
        .first()
        .expect("multiple-mutable-borrows diagnostic should compact");
    assert_eq!(diagnostic.descriptor().code, "MOTH-BORROW-0002");
    diagnostic
        .family_view::<MultipleMutableBorrowsView>()
        .expect("multiple-mutable-borrows diagnostic should retain typed facts");
}

#[test]
fn shared_then_mutable_args_to_same_root_are_rejected() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let read_then_mut = symbol("read_then_mut", &mut path_fork, &mut string_table);
    let read = symbol("read", &mut path_fork, &mut string_table);
    let mutate = symbol("mutate", &mut path_fork, &mut string_table);
    let x = symbol("x", &mut path_fork, &mut string_table);

    let callee = function_node(
        read_then_mut,
        FunctionSignature {
            parameters: vec![
                param_with_type_id(
                    read,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    false,
                    test_source_location(1),
                ),
                param_with_type_id(
                    mutate,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    true,
                    test_source_location(1),
                ),
            ],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
        None,
    );

    let caller = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                {
                    let arguments = vec![
                        CallArgument::positional(
                            {
                                let value = immutable_reference_expr(
                                    &mut expression_builder,
                                    x,
                                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                    test_source_location(11),
                                );
                                expression_builder.own(value)
                            },
                            CallAccessMode::Shared,
                            test_source_location(11),
                        ),
                        CallArgument::positional(
                            {
                                let value = immutable_reference_expr(
                                    &mut expression_builder,
                                    x,
                                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                    test_source_location(11),
                                );
                                expression_builder.own(value)
                            },
                            CallAccessMode::Shared,
                            test_source_location(11),
                        ),
                    ];
                    function_call_node(
                        &mut expression_builder,
                        read_then_mut,
                        arguments,
                        vec![],
                        test_source_location(11),
                    )
                },
                test_source_location(11),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![callee, caller],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("shared then mutable access to same root in a call must fail");
    let draft = error
        .into_draft_or_infrastructure()
        .expect("borrow failure should retain its typed diagnostic");
    let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let diagnostic = compact
        .first()
        .expect("shared/mutable conflict diagnostic should compact");
    assert_eq!(diagnostic.descriptor().code, "MOTH-BORROW-0003");
    let conflict = diagnostic
        .family_view::<SharedMutableConflictView>()
        .expect("shared/mutable conflict should retain typed access facts");
    assert_eq!(
        (conflict.existing_access, conflict.requested_access),
        (BorrowAccessKind::Shared, BorrowAccessKind::Mutable)
    );
}

#[test]
fn external_alias_args_result_stays_slot_backed_after_rebinding() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let host_alias = register_external_function(
        &mut external_package_registry,
        "alias_arg_result",
        vec![ExternalAccessKind::Shared],
        ExternalReturnAlias::AliasArgs(vec![0]),
        ExternalReturnType::Fixed(FixedScalar::I32),
    );
    let original = symbol("original", &mut path_fork, &mut string_table);
    let returned = symbol("returned", &mut path_fork, &mut string_table);

    let returned_call = {
        let arguments = vec![CallArgument::positional(
            {
                let value = immutable_reference_expr(
                    &mut expression_builder,
                    original,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    test_source_location(11),
                );
                expression_builder.own(value)
            },
            CallAccessMode::Shared,
            test_source_location(11),
        )];
        expression_builder.host_function_call_with_typed_arguments(
            host_alias,
            arguments,
            vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
            false,
            None,
        )
    }
    .expect("valid call fixture")
    .id();

    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    original,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    returned,
                    returned_call,
                    &expression_builder,
                )),
                test_source_location(11),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        &mut expression_builder,
                        returned,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(12),
                    ),
                    value: i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        2,
                        test_source_location(12),
                        ValueMode::ImmutableOwned,
                    ),
                },
                test_source_location(12),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        &mut expression_builder,
                        original,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(13),
                    ),
                    value: i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        3,
                        test_source_location(13),
                        ValueMode::ImmutableOwned,
                    ),
                },
                test_source_location(13),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start], expression_builder, const_values, types),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("external AliasArgs result should remain rebindable as a caller slot");
    let original_local = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                ..
            } if hir
                .side_table
                .resolve_local_name(*local, &path_fork, &string_table)
                == Some("original") =>
            {
                Some(*local)
            }
            _ => None,
        })
        .expect("should locate the original binding");
    let returned_local = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                ..
            } if hir
                .side_table
                .resolve_local_name(*local, &path_fork, &string_table)
                == Some("returned") =>
            {
                Some(*local)
            }
            _ => None,
        })
        .expect("should locate the returned binding");
    let returned_initializer = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } if *local == returned_local => Some(*value),
            _ => None,
        })
        .expect("the returned caller binding must have a definition");
    let initializer = hir.expressions.expression(returned_initializer);
    assert_eq!(
        initializer.value_kind,
        ValueKind::RValue,
        "a call result load remains an rvalue at the caller binding boundary"
    );
    let HirExpressionKind::Load(call_result_place) = &initializer.kind else {
        panic!("the external call result should be loaded into its caller binding");
    };
    assert!(
        hir.expressions
            .projections(call_result_place.projections)
            .is_empty()
    );
    let call_result_local = call_result_place.root;
    assert_ne!(
        call_result_local, returned_local,
        "the external call result scratch must be distinct from the caller binding"
    );
    assert!(
        hir.blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Call {
                        results,
                        ..
                    } if matches!(
                        results.as_slice(),
                        [HirLocalDestination::Define(result_local)] if *result_local == call_result_local
                    )
                )
            }),
        "the internal call result is a defined compiler temporary"
    );
    let returned_rebinding = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .rfind(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Write {
                    target: HirWriteTarget::AssignPlace(place),
                    ..
                } if place.root == returned_local
                    && hir.expressions.projections(place.projections).is_empty()
            )
        })
        .expect("should locate the result rebinding statement");
    let returned_before_rebinding = report
        .analysis
        .statement_entry_states
        .get(&returned_rebinding.id)
        .expect("result rebinding should have an entry snapshot")
        .locals
        .iter()
        .find(|local| local.local == returned_local)
        .expect("result rebinding snapshot should include the external result");

    assert!(
        returned_before_rebinding.mode.contains(LocalMode::SLOT),
        "external AliasArgs result should use a caller slot before rebinding, got mode {:?} with roots {:?}",
        returned_before_rebinding.mode,
        returned_before_rebinding.alias_roots
    );
    assert!(!returned_before_rebinding.mode.contains(LocalMode::ALIAS));
    assert!(
        returned_before_rebinding
            .alias_roots
            .contains(&original_local),
        "external AliasArgs result should retain the argument root before rebinding, got mode {:?} with roots {:?}",
        returned_before_rebinding.mode,
        returned_before_rebinding.alias_roots
    );

    let original_assignment = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .rfind(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Write {
                    target: HirWriteTarget::AssignPlace(place),
                    ..
                } if place.root == original_local
                    && hir.expressions.projections(place.projections).is_empty()
            )
        })
        .expect("should locate the source rebinding statement");
    let snapshot = report
        .analysis
        .statement_entry_states
        .get(&original_assignment.id)
        .expect("source rebinding should have an entry snapshot");
    let returned_snapshot = snapshot
        .locals
        .iter()
        .find(|local| local.local == returned_local)
        .expect("source rebinding snapshot should include the external result");

    assert!(returned_snapshot.mode.contains(LocalMode::SLOT));
    assert!(!returned_snapshot.mode.contains(LocalMode::ALIAS));
    assert!(returned_snapshot.alias_roots.is_empty());
}

#[test]
fn unresolved_or_mismatched_host_signature_errors() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let one_arg = register_external_function(
        &mut external_package_registry,
        "one_arg",
        vec![ExternalAccessKind::Shared],
        ExternalReturnAlias::Fresh,
        ExternalReturnType::Void,
    );
    let no_args = register_external_function(
        &mut external_package_registry,
        "no_args",
        vec![],
        ExternalReturnAlias::Fresh,
        ExternalReturnType::Void,
    );

    let missing_host =
        crate::compiler_frontend::external_packages::ExternalFunctionId::Synthetic(9999);

    let start_missing = function_node(
        symbol("start_missing", &mut path_fork, &mut string_table),
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![node(
            {
                let arguments = vec![];
                host_function_call_node(
                    &mut expression_builder,
                    no_args,
                    arguments,
                    vec![],
                    test_source_location(1),
                )
            },
            test_source_location(1),
        )],
        None,
    );

    let start_mismatch = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![node(
            {
                let arguments = vec![CallArgument::positional(
                    {
                        let value = i32_expression(
                            &mut expression_builder,
                            &mut const_values,
                            1,
                            test_source_location(2),
                            ValueMode::ImmutableOwned,
                        );
                        expression_builder.own(value)
                    },
                    CallAccessMode::Shared,
                    test_source_location(2),
                )];
                host_function_call_node(
                    &mut expression_builder,
                    one_arg,
                    arguments,
                    vec![],
                    test_source_location(2),
                )
            },
            test_source_location(2),
        )],
    );

    let mut hir = lower_hir(
        build_ast_with_registered_types(
            vec![start_missing, start_mismatch],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );

    // Start with validated HIR, then corrupt one contract at a time to test
    // the borrow checker's trusted-metadata boundary rather than lowering.
    for missing_target in [true, false] {
        let call = hir
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Call { target: CallTarget::External(target), .. }
                        if *target == if missing_target { no_args } else { one_arg }
                )
            })
            .expect("the validated host call must be present");
        let HirStatementKind::Call { target, args, .. } = &mut call.kind else {
            unreachable!("the selected statement is a host call");
        };
        let original_target = CallTarget::External(if missing_target { no_args } else { one_arg });
        if missing_target {
            *target = CallTarget::External(missing_host);
        } else {
            *args = crate::compiler_frontend::hir::expression_store::HirValueRange::empty();
        }
        let call_id = call.id;
        let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
            .expect_err("missing host or mismatched signature should fail");
        let expected_message = if missing_target {
            "host call target"
        } else {
            "argument count mismatch"
        };
        assert_infrastructure_error_contains(
            &error,
            InfrastructureFailureKind::Internal,
            &[expected_message],
        );
        let call = hir
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find(|statement| statement.id == call_id)
            .expect("the corrupted call must remain present");
        if let HirStatementKind::Call { target, .. } = &mut call.kind {
            *target = original_target;
        }
    }
}

#[test]
fn mutable_user_parameter_rejects_immutable_argument_reused_after_call() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let mut_user = symbol("mut_user", &mut path_fork, &mut string_table);
    let p = symbol("p", &mut path_fork, &mut string_table);
    let x = symbol("x", &mut path_fork, &mut string_table);
    let y = symbol("y", &mut path_fork, &mut string_table);

    let callee = function_node(
        mut_user,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                p,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                true,
                test_source_location(1),
            )],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
        None,
    );

    let caller = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::ImmutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                {
                    let arguments = vec![CallArgument::positional(
                        {
                            let value = immutable_reference_expr(
                                &mut expression_builder,
                                x,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                test_source_location(11),
                            );
                            expression_builder.own(value)
                        },
                        CallAccessMode::Shared,
                        test_source_location(11),
                    )];
                    function_call_node(
                        &mut expression_builder,
                        mut_user,
                        arguments,
                        vec![],
                        test_source_location(11),
                    )
                },
                test_source_location(11),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    immutable_reference_expr(
                        &mut expression_builder,
                        x,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(12),
                    ),
                    &expression_builder,
                )),
                test_source_location(12),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![callee, caller],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("immutable argument passed to mutable user param and reused must fail");
    assert_invalid_mutable_access_reason(error, InvalidMutableAccessReason::ImmutablePlace);
}

#[test]
fn out_of_range_return_alias_metadata_is_reported_at_call_site() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let bad_alias_host = register_external_function(
        &mut external_package_registry,
        "bad_alias_host",
        vec![ExternalAccessKind::Shared],
        ExternalReturnAlias::AliasArgs(vec![0]),
        ExternalReturnType::Fixed(FixedScalar::I32),
    );
    // Admit a valid descriptor first, then corrupt only the stored metadata to reach
    // the borrow checker's independent caller guard.
    set_test_external_function_return_alias(
        Arc::make_mut(&mut external_package_registry),
        bad_alias_host,
        ExternalReturnAlias::AliasArgs(vec![1]),
    );
    let x = symbol("x", &mut path_fork, &mut string_table);

    let start = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                {
                    let arguments = vec![CallArgument::positional(
                        {
                            let value = immutable_reference_expr(
                                &mut expression_builder,
                                x,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                test_source_location(11),
                            );
                            expression_builder.own(value)
                        },
                        CallAccessMode::Shared,
                        test_source_location(11),
                    )];
                    host_function_call_node(
                        &mut expression_builder,
                        bad_alias_host,
                        arguments,
                        vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
                        test_source_location(11),
                    )
                },
                test_source_location(11),
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start], expression_builder, const_values, types),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );

    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("out-of-range return alias metadata should fail at call site");
    assert_infrastructure_error_contains(
        &error,
        InfrastructureFailureKind::Internal,
        &["out-of-range return-alias index"],
    );
}

#[test]
fn same_line_mutable_call_then_reuse_uses_order_keys() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let mut_user = symbol("mut_user", &mut path_fork, &mut string_table);
    let p = symbol("p", &mut path_fork, &mut string_table);
    let x = symbol("x", &mut path_fork, &mut string_table);
    let y = symbol("y", &mut path_fork, &mut string_table);

    let callee = function_node(
        mut_user,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                p,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                true,
                test_source_location(1),
            )],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![],
        None,
    );

    // WHAT: both statements intentionally share one source line.
    // WHY: validates that borrow/move classification uses statement order keys, not line numbers.
    let same_line = None;
    let caller = entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    i32_expression(
                        &mut expression_builder,
                        &mut const_values,
                        1,
                        test_source_location(10),
                        ValueMode::MutableOwned,
                    ),
                    &expression_builder,
                )),
                test_source_location(10),
            ),
            node(
                {
                    let arguments = vec![CallArgument::positional(
                        {
                            let value = immutable_reference_expr(
                                &mut expression_builder,
                                x,
                                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                                same_line,
                            );
                            expression_builder.own(value)
                        },
                        CallAccessMode::Shared,
                        same_line,
                    )];
                    function_call_node(
                        &mut expression_builder,
                        mut_user,
                        arguments,
                        vec![],
                        same_line,
                    )
                },
                same_line,
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    immutable_reference_expr(
                        &mut expression_builder,
                        x,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        same_line,
                    ),
                    &expression_builder,
                )),
                same_line,
            ),
        ],
    );

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![callee, caller],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table).expect(
        "same-line mutable call + later reuse should borrow (not move) when ordered by statement",
    );
}

#[test]
fn short_circuit_rhs_mutable_call_with_later_merge_use_borrows_instead_of_moving() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let types = TypeEnvironment::new();
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let external_package_registry = default_external_package_registry(&mut string_table);

    let rhs_name = symbol("rhs_short", &mut path_fork, &mut string_table);
    let calls = symbol("calls", &mut path_fork, &mut string_table);
    let lhs = symbol("lhs", &mut path_fork, &mut string_table);
    let value = symbol("value", &mut path_fork, &mut string_table);
    let sink = symbol("sink", &mut path_fork, &mut string_table);
    let param_calls = symbol("param_calls", &mut path_fork, &mut string_table);

    let rhs_function = function_node(
        rhs_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                param_calls,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                true,
                test_source_location(1),
            )],
            return_contract: FunctionReturnContract::Returns(fresh_success_returns(vec![
                builtin_type_ids::BOOL,
            ])),
        },
        vec![
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        &mut expression_builder,
                        param_calls,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        test_source_location(2),
                    ),
                    value: {
                        let items = vec![
                            ExpressionRpnItem::Operand(expression_builder.reference_with_type_id(param_calls, builtin_type_ids::fixed_scalar(FixedScalar::I32), test_source_location(2), ValueMode::MutableOwned, crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue).expect("valid expression fixture").id()),
                            ExpressionRpnItem::Operand(i32_expression(&mut expression_builder, &mut const_values, 1, test_source_location(2), ValueMode::ImmutableOwned)),
                            ExpressionRpnItem::Operator {
                                operator: Operator::Add,
                                span: None,
                                preceded_by_operand: true,
                            },
                        ];
                        crate::compiler_frontend::tests::type_id_fixture_support::runtime_expr(
                            &mut expression_builder,
                            &mut const_values,
                            items,
                            builtin_type_ids::fixed_scalar(FixedScalar::I32),
                            test_source_location(2),
                            ValueMode::MutableOwned,
                            &types,
                        )
                    },
                },
                test_source_location(2),
            ),
            node(
                NodeKind::Return(vec![
                    expression_builder
                        .bool(
                            &mut const_values,
                            true,
                            test_source_location(3),
                            ValueMode::ImmutableOwned,
                        )
                        .expect("valid expression fixture")
                        .id(),
                ]),
                test_source_location(3),
            ),
        ],
        None,
    );

    let short_circuit_value = {
        let items = vec![
        ExpressionRpnItem::Operand(expression_builder.reference_with_type_id(lhs, builtin_type_ids::BOOL, test_source_location(11), ValueMode::ImmutableOwned, crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue).expect("valid expression fixture").id()),
        ExpressionRpnItem::Operand({ let arguments: Vec<_> = vec![expression_builder.reference_with_type_id(calls, builtin_type_ids::fixed_scalar(FixedScalar::I32), test_source_location(11), ValueMode::MutableOwned, crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue).expect("valid expression fixture").id()].into_iter().map(|value| CallArgument::positional(expression_builder.own(value), CallAccessMode::Shared, None)).collect(); expression_builder.function_call_with_typed_arguments(rhs_name, arguments, vec![builtin_type_ids::BOOL], test_source_location(11)) }.expect("valid call fixture").id()),
        ExpressionRpnItem::Operator {
            operator: Operator::And,
            span: None,
            preceded_by_operand: true,
        },
    ];
        crate::compiler_frontend::tests::type_id_fixture_support::runtime_expr(
            &mut expression_builder,
            &mut const_values,
            items,
            builtin_type_ids::BOOL,
            None,
            ValueMode::ImmutableOwned,
            &types,
        )
    };

    let start_function = entry_start_node(FunctionSignature { parameters: vec![], return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots([], None)),  }, vec![
        node(
            NodeKind::VariableDeclaration(make_test_variable(lhs, expression_builder.bool(&mut const_values, false, test_source_location(10), ValueMode::ImmutableOwned).expect("valid expression fixture").id(), &expression_builder)),
            test_source_location(10),
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(calls, i32_expression(&mut expression_builder, &mut const_values, 0, test_source_location(10), ValueMode::MutableOwned), &expression_builder)),
            test_source_location(10),
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(value, short_circuit_value, &expression_builder)),
            test_source_location(11),
        ),
        node(
            NodeKind::VariableDeclaration(make_test_variable(sink, expression_builder.reference_with_type_id(calls, builtin_type_ids::fixed_scalar(FixedScalar::I32), test_source_location(12), ValueMode::ImmutableReference, crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue).expect("valid expression fixture").id(), &expression_builder)),
            test_source_location(12),
        ),
    ]);

    let hir = lower_hir(
        build_ast_with_registered_types(
            vec![rhs_function, start_function],
            expression_builder,
            const_values,
            types,
        ),
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("rhs-only mutable short-circuit call with later merge use should stay borrowed");
}

#[test]
fn catch_private_call_return_alias_unions_success_and_parameter_derived_handler() {
    let source = "load |value String, trigger I32| -> String:\n\
                  numeric_result = trigger + 2147483647\n\
                  return value\n;\n\
                  recover |fallback String, success String, trigger I32| -> String:\n\
                  return load(success, trigger) catch:\n\
                      return fallback\n\
                  ;\n;\n";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let entry_path = path_fork
        .try_intern_portable_path("@entry.moth", &mut string_table)
        .expect("test entry path fits");
    let registry = default_external_package_registry(&mut string_table);
    let (mut hir, mut type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &registry)
            .expect("a direct private catch must lower");
    let mut initial = run_borrow_checker_with_types(
        &hir,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    )
    .expect("the pre-installation HIR must be legal");
    infer_builtin_failure_summaries(&hir, &mut initial);
    install_private_failure_lanes(&mut hir, &initial, &mut type_environment, &registry)
        .expect("the inferred call must install a real handler edge");
    assert!(hir.catch_protected_calls.is_empty());
    let report = run_borrow_checker_with_types(
        &hir,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    )
    .expect("return provenance must remain legal after error edges are installed");
    let recover_name = path_fork
        .try_intern_child(entry_path, string_table.intern("recover"))
        .expect("the test path fits");
    let recover = hir
        .functions
        .iter()
        .find(|function| hir.side_table.function_name_path(function.id) == Some(recover_name))
        .expect("the recovering function must exist");
    let summary = &report.analysis.public_call_summaries[&recover.id];
    let facts = allocation_facts(&summary.return_alias);
    assert_eq!(root_source_arguments(facts), [0, 1]);

    let return_facts = hir
        .blocks
        .iter()
        .filter(|block| matches!(block.terminator, HirTerminator::Return(_)))
        .map(|block| &report.analysis.terminator_facts[&block.id])
        .collect::<Vec<_>>();
    assert!(
        return_facts
            .iter()
            .any(|fact| fact.shared_roots.contains(&recover.params[0])),
        "the handler return must retain the fallback parameter"
    );
    assert!(
        return_facts
            .iter()
            .any(|fact| fact.shared_roots.contains(&recover.params[1])),
        "the success return must retain the successful call's parameter provenance"
    );
}

#[test]
fn catch_join_preserves_fresh_scalar_results_and_reference_parameter_union() {
    let source = r#"
fresh_numeric |value ~I32| -> I32:
    return value + 1 catch then 0
;

fresh_cast |value ~I32| -> U8:
    return cast value catch then 0
;

maybe_reference |value String, trigger I32| -> String, Error!:
    if trigger is 0:
        return! Error("missing")
    ;
    return value
;

retained_reference |value ~String, fallback String, trigger I32| -> String:
    return maybe_reference(value, trigger) catch then fallback
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork, &registry);
    let report = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
        .expect("catch joins must preserve native success provenance");

    for (name, expected_arguments) in [
        ("fresh_numeric", &[][..]),
        ("fresh_cast", &[][..]),
        ("retained_reference", &[0, 1][..]),
    ] {
        let function = hir
            .functions
            .iter()
            .find(|function| {
                hir.side_table
                    .function_name_path(function.id)
                    .and_then(|path| path_fork.component(path))
                    .is_some_and(|component| string_table.resolve(component) == name)
            })
            .expect("each catch fixture must lower to a function");
        let summary = &report.analysis.public_call_summaries[&function.id];
        let facts = allocation_facts(&summary.return_alias);
        assert!(facts.success.can_return(), "{name}");
        assert_eq!(root_source_arguments(facts), expected_arguments, "{name}");
        assert_eq!(
            summary.parameters[0].access,
            PublicCallParameterAccess::Mutable,
            "{name} must preserve its authored mutable parameter contract"
        );
        assert_eq!(
            summary.parameters[0].mutation,
            PublicCallMutationEffect::NoWrite,
            "{name} only reads its mutable parameter"
        );
        if name == "retained_reference" {
            assert_eq!(
                summary.parameters[1].access,
                PublicCallParameterAccess::Shared
            );
            assert_eq!(
                summary.parameters[1].mutation,
                PublicCallMutationEffect::NoWrite
            );
        }
    }
}

#[test]
fn call_clear_effect_live() {
    let source = r#"
Scores as {String = I32}

edit |scores ~Scores|:
    ~scores.clear()
;

scores ~Scores = {"k" = 7}
score = scores.get("k")!
edit(~scores)
~[:[score]]
"#;
    super::borrow_checker_fact_tests::assert_lookup_conflict(source, "edit(~scores)");
}

#[test]
fn call_mutable_actual_observes() {
    let source = r#"
show |value ~I32|:
    io.line([:[value]])
;

scores ~{String = I32} = {"k" = 7}
score ~= scores.get("k")!
~scores.clear()
show(~score)
"#;
    super::borrow_checker_fact_tests::assert_lookup_conflict(source, "~scores.clear()");
}

#[test]
fn lookup_via_writer_preserves_directional_derivation() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};

    let fixture = |operation| {
        format!(
            "items ~= {{1, 2}}\nwriter ~= items\nvalue = writer.get(0)!\n\
         {operation}\n~[:[value],[length],[writer.length()]]\n"
        )
    };
    check_place_access_source(&fixture("length = writer.length()"))
        .expect("the lookup derives from the writer");

    let parent = fixture("length = items.length()");
    let draft = check_place_access_source(&parent)
        .expect_err("the parent bypasses the active writer")
        .into_draft_or_infrastructure()
        .expect("typed borrow diagnostic");
    let bag = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let diagnostic = bag.first().expect("borrow diagnostic");
    assert!(
        diagnostic
            .family_view::<SharedMutableConflictView>()
            .is_some()
    );
    let start = parent.find("length = items.length()").unwrap();
    let end = start + "length = items.length()".len();
    assert!(matches!(
        diagnostic.primary_site(),
        SiteRef::Local(span) if span.local().inline_range()
            .is_some_and(|range| (start..end).contains(&(range.start() as usize)))
    ));

    assert_lookup_conflict(
        &fixture("~writer.push(3)\nlength = writer.length()"),
        "~writer.push",
    );
}

#[test]
fn identity_call_transports_both_scalar_and_heap_lookup_values() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};
    for (ty, payload, read) in [
        ("I32", "7", "forwarded + 0"),
        ("{I32}", "{7}", "forwarded.length()"),
    ] {
        let fixture = |before, after| {
            format!(
                r#"
identity |value {ty}| -> {ty}:
    return value
;
scores ~{{String = {ty}}} = {{"k" = {payload}}}
score = scores.get("k")!
forwarded = identity(score)
{before}
seen = {read}
{after}
~[:[seen]]
"#
            )
        };
        assert_lookup_conflict(&fixture("~scores.clear()", ""), "~scores.clear()");
        check_place_access_source(&fixture("", "~scores.clear()"))
            .expect("the forwarded value was already observed");
    }
}

#[test]
fn summary_scalar_descriptors_limit_dependencies_to_selected_source() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};
    use crate::compiler_frontend::public_call_summary::{CallField, typed::CellPost};

    let clear = "~scores.clear()";
    let cases = [
        (
            include_str!("../../../../../tests/cases/borrow_call_scalar_field/input/@entry.moth"),
            "pack",
            Some(0),
            Some(0),
            &[][..],
            true,
        ),
        (
            include_str!("../../../../../tests/cases/borrow_call_scalar_store/input/@entry.moth"),
            "store",
            Some(0),
            Some(1),
            &[][..],
            true,
        ),
        (
            include_str!("../../../../../tests/cases/borrow_pack_then_extract/input/@entry.moth"),
            "item_of",
            None,
            Some(0),
            &[CallField::Field(0)][..],
            true,
        ),
        (
            r#"fresh |value I32| -> I32:
    return 3
;
scores ~{String = I32} = {"k" = 7}
score = scores.get("k") catch then 0
forwarded = fresh(score)
~scores.clear()
~[:[forwarded]]"#,
            "fresh",
            None,
            None,
            &[][..],
            false,
        ),
        (
            r#"count_of |value I32| -> I32:
    return 0
;
scores ~{String = I32} = {"k" = 7}
score = scores.get("k") catch then 0
n = count_of(score)
~scores.clear()
~[:[n]]"#,
            "count_of",
            None,
            None,
            &[][..],
            false,
        ),
        (
            r#"Saved = |item I32, count I32|
pack |item I32| -> Saved:
    return Saved(item, 3)
;
scores ~{String = I32} = {"k" = 7}
score = scores.get("k")!
saved = pack(score)
~scores.clear()
~[:[saved.count]]"#,
            "pack",
            Some(1),
            None,
            &[][..],
            false,
        ),
        (
            r#"Saved = |item I32, count I32|
pack |item I32, count I32| -> Saved:
    return Saved(item, count)
;
scores ~{String = I32} = {"k" = 7}
score = scores.get("k") catch then 0
saved = pack(score, 3)
~scores.clear()
~[:[saved.count]]"#,
            "pack",
            Some(1),
            Some(1),
            &[][..],
            false,
        ),
    ];
    for (source, name, field, parameter, entry_path, conflict) in cases {
        let fixture = PreparedHirFixture::new(source);
        let facts = summarize_prepared_call_allocations(&fixture)
            .expect("the typed producer must describe the selected scalar");
        let callee = fixture
            .hir
            .functions
            .iter()
            .find(|function| {
                fixture
                    .hir
                    .side_table
                    .function_name_path(function.id)
                    .and_then(|path| fixture.paths.component(path))
                    .is_some_and(|component| fixture.strings.resolve(component) == name)
            })
            .unwrap();
        let facts =
            allocation_facts(&facts.analysis.public_call_summaries[&callee.id].return_alias);
        let mut sources = Vec::new();
        if let Some(field) = field {
            facts
                .success
                .visit_compatible(
                    |_| Ok::<_, std::convert::Infallible>(None),
                    |_, product| {
                        use crate::compiler_frontend::public_call_summary::typed::{
                            CellPort, ExportPort,
                        };
                        for post in product
                            .components()
                            .iter()
                            .flat_map(|component| &component.alternatives)
                        {
                            for (cell, edit) in &post.cells {
                                let CellPort::Edge(holder, CallField::Field(ordinal)) = cell else {
                                    continue;
                                };
                                let selected_holder = match holder {
                                    ObjectRef::Entry(port) => {
                                        matches!(&facts.ports[port.0 as usize],
                                    EntryPort::Value { parameter: 0, path } if path.is_empty())
                                    }
                                    ObjectRef::Exact(ExportPort::Result(path)) => {
                                        path.slot == 0 && path.path.is_empty()
                                    }
                                    _ => false,
                                };
                                if selected_holder && *ordinal == field {
                                    let (CellPost::Replace(values) | CellPost::WeakJoin(values)) =
                                        edit;
                                    sources.extend(
                                        values.iter().flatten().map(|value| value.source.clone()),
                                    );
                                }
                            }
                        }
                        Ok(())
                    },
                )
                .unwrap();
            sources.sort_unstable();
            sources.dedup();
        } else {
            sources = result_sources(&facts.success, 0);
        }
        match (sources.as_slice(), parameter) {
            ([ValueSource::Scalar(ScalarSource::EntryValue(port))], Some(parameter)) => {
                assert_eq!(
                    facts.ports[port.0 as usize],
                    EntryPort::Value {
                        parameter,
                        path: entry_path.into(),
                    },
                    "{source}"
                );
            }
            ([ValueSource::Scalar(ScalarSource::Independent)], None) => {}
            _ => panic!("{source}: unexpected scalar sources {sources:?}"),
        }
        if conflict {
            assert_lookup_conflict(source, clear);
        } else {
            check_place_access_source(source).expect("the other scalar has no receiver dependency");
        }
        let (before, last_use) = source.split_once("~scores.clear()\n").unwrap();
        check_place_access_source(&format!("{before}{last_use}\n{clear}\n"))
            .expect("the same value was last used before the clear");

        let (declarations, body) = source.split_once("scores ~").unwrap();
        let (initial, rest) = body.split_once('\n').unwrap();
        let (_, continuation) = rest.split_once('\n').unwrap();
        let (commands, observation) = continuation.trim_end().rsplit_once('\n').unwrap();
        let commands = commands
            .replace("(score", "(fallback")
            .replace(", score", ", fallback")
            .replace('\n', "\n        ");
        let observation = observation
            .strip_prefix("~[:[")
            .unwrap()
            .strip_suffix("]]")
            .unwrap();
        let selected_error = format!(
            r#"{declarations}
probe || -> I32:
    scores ~{initial}
    score = scores.get("k") catch:
        fallback = 0
        {commands}
        return {observation}
    ;
    return score
;
"#
        );
        check_place_access_source(&selected_error)
            .expect("the selected lookup error loaned no receiver, even with a live observation");
    }

    let replacement = |write| {
        format!(
            r#"
Saved = |item I32|
store |dst ~Saved, item I32|:
    ~dst.item = item
;
probe |replace Bool| -> I32:
    scores ~{{String = I32}} = {{"k" = 7}}
    score = scores.get("k") catch then 0
    saved ~= Saved(3)
    store(~saved, score)
    {write}
    ~scores.clear()
    return saved.item
;
"#
        )
    };
    check_place_access_source(&replacement("store(~saved, 0)"))
        .expect("a definite replacement kills the old scalar carrier");
    assert_lookup_conflict(
        &replacement("if replace:\n        store(~saved, 0)\n    ;"),
        clear,
    );
}

#[test]
fn exported_fresh_receiver_loans_do_not_depend_on_field_order() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};

    for (fields, values) in [
        ("item I32, box {I32}", "score, box"),
        ("box {I32}, item I32", "box, score"),
    ] {
        let fixture = |score| {
            format!(
                r#"
Saved = |{fields}|
pack |seed {{I32}}| -> Saved:
    box ~= copy seed
    score = {score}
    return Saved({values})
;
seed ~= {{7}}
saved ~= pack(seed)
seen = saved.item
~saved.box.push(1)
~[:[seen]]
"#
            )
        };
        assert_lookup_conflict(&fixture("box.get(0) catch then 0"), "~saved.box.push(1)");
        check_place_access_source(&fixture("7"))
            .expect("an independent scalar does not borrow the fresh receiver");
    }
}

#[test]
fn summary_lookup_dependencies_survive_scalar_forwarding_and_catch() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};
    let fixture = |success_before, success_after| {
        format!(
            r#"
lookup |scores {{String = I32}}| -> I32, Error!:
    return scores.get("k")!
;
forward |scores {{String = I32}}| -> I32, Error!:
    return lookup(scores)!
;
probe |scores ~{{String = I32}}| -> I32:
    value = forward(scores) catch:
        ~scores.clear()
        return 0
    ;
    {success_before}
    seen = value
    io.line([:[seen]])
    {success_after}
    return 1
;
"#
        )
    };
    assert_lookup_conflict(&fixture("~scores.clear()", ""), "~scores.clear()\n    seen");
    check_place_access_source(&fixture("", "~scores.clear()"))
        .expect("the error has no loan and the successful value was last used before clear");
}

#[test]
fn summary_import_only_scalar_lookup_activates_observation() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};
    let source = r#"
Scores as {String = I32}
lookup |scores Scores| -> I32:
    return scores.get("k") catch then 0
;
scores ~Scores = {"k" = 7}
score = lookup(scores)
~scores.clear()
~[:[score]]
"#;
    assert_lookup_conflict(source, "~scores.clear()");
    let (before, use_score) = source.split_once("~scores.clear()\n").unwrap();
    check_place_access_source(&format!("{before}{use_score}~scores.clear()\n"))
        .expect("the imported observation ends at the last use");
}

#[test]
fn call_missing_scalar_paths_do_not_taint_explicit_parameter_subtree() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};
    let fixture = |field| {
        format!(
            r#"
Saved = |item I32|
Wrapped = |plain Saved, dependent I32|
pack |plain Saved, value I32| -> Wrapped:
    return Wrapped(plain, value)
;
scores ~{{String = I32}} = {{"k" = 7}}
score = scores.get("k")!
plain = Saved(3)
wrapped = pack(plain, score)
~scores.clear()
~[:[wrapped.{field}]]
"#
        )
    };
    assert_lookup_conflict(&fixture("dependent"), "~scores.clear()");
    check_place_access_source(&fixture("plain.item"))
        .expect("the known parameter subtree keeps its own independent contents");
}

#[test]
fn summary_transport_keeps_raw_permission_but_restricts_new_shared_edge() {
    use super::borrow_checker_fact_tests::check_place_access_source;
    let declarations = r#"
Link = |items {I32}|
raw |items {I32}| -> {I32}:
    return items
;
wrap |items {I32}| -> Link:
    return Link(items)
;
"#;
    let raw = format!("{declarations}\nitems ~= {{1}}\nwriter ~= raw(items)\n~writer.push(2)\n");
    check_place_access_source(&raw)
        .expect("a raw return preserves its actual's writable permission");

    let wrapped =
        format!("{declarations}\nitems ~= {{1}}\nholder ~= wrap(items)\n~holder.items.push(2)\n");
    let error = check_place_access_source(&wrapped)
        .expect_err("the new field may not acquire a writable edge through a shared formal");
    assert_invalid_mutable_access_reason(error, InvalidMutableAccessReason::ImmutablePlace);

    let copied = wrapped.replace("return Link(items)", "return Link(copy items)");
    check_place_access_source(&copied).expect("the callee can create a writable field by copying");
}

#[test]
fn summary_clear_refill_transports_new_members_and_kills_old_members() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};
    for inline in [false, true] {
        for retained in [false, true] {
            let initial = if retained { "0" } else { "score" };
            let incoming = if retained { "score" } else { "0" };
            let operation = if inline {
                format!("~dst.clear()\n~dst.set(\"k\", {incoming})!")
            } else {
                format!("refill(~dst, {incoming})!")
            };
            let source = format!(
                r#"
Scores as {{String = I32}}
refill |dst ~Scores, value I32| -> Error!:
    ~dst.clear()
    ~dst.set("k", value)!
;
src ~Scores = {{"k" = 7}}
score = src.get("k")!
dst ~Scores = {{"old" = {initial}}}
{operation}
~src.clear()
removed = ~dst.remove("k")!
~[:[removed]]
"#
            );
            if retained {
                assert_lookup_conflict(&source, "~src.clear()");
            } else {
                check_place_access_source(&source)
                    .expect("refill with an independent value kills the old member's loan");
            }
        }
    }
}

#[test]
fn summary_refill_preserves_duplicate_child_occurrences() {
    use super::borrow_checker_fact_tests::check_place_access_source;
    for duplicate in [false, true] {
        let second = if duplicate {
            "~dst.set(\"second\", child)!"
        } else {
            ""
        };
        let observe = if duplicate {
            "remaining = entries.get(\"second\") catch then {0}\n    return remaining.length()"
        } else {
            "return entries.length"
        };
        let source = format!(
            r#"
refill |dst ~{{String = {{I32}}}}| -> Error!:
    ~dst.clear()
    child ~= {{1}}
    ~dst.set("first", child)!
    {second}
;
probe || -> I32, Error!:
    entries ~{{String = {{I32}}}} = {{"old" = {{0}}}}
    refill(~entries)!
    detached ~= ~entries.remove("first") catch then {{0}}
    ~detached.push(2)
    {observe}
;
"#
        );
        if duplicate {
            let error = check_place_access_source(&source)
                .expect_err("removing one occurrence cannot detach another occurrence");
            assert_borrow_code_at(error, &source, "detached.push", "MOTH-BORROW-0007");
        } else {
            check_place_access_source(&source).expect("the single inserted child detaches");
        }
    }
}

#[test]
fn a_refill_failure_erases_old_members_without_importing_success_members() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};
    let fixture = |body, success_clear| {
        format!(
            r#"
Scores as {{String = I32}}
refill |dst ~Scores, item I32, ready Bool| -> I32, Error!:
    ~dst.clear()
    {body}
;
probe |ready Bool| -> I32:
    src ~Scores = {{"k" = 7}}
    score = src.get("k") catch then 0
    dst ~Scores = {{"old" = score}}
    status = refill(~dst, score, ready) catch:
        ~src.clear()
        return dst.get("old") catch then 0
    ;
    {success_clear}
    return dst.get("k") catch then status
;
"#
        )
    };
    for body in [
        "if ready:\n        ~dst.set(\"k\", item)!\n        return 1\n    ;\n    return! Error(\"skipped refill\")",
        "if ready is false:\n        return! Error(\"skipped refill\")\n    ;\n    ~dst.set(\"k\", item)!\n    return 1",
    ] {
        assert_lookup_conflict(
            &fixture(body, "~src.clear()"),
            "~src.clear()\n    return dst",
        );
        check_place_access_source(&fixture(body, ""))
            .expect("the clear-only error outcome cannot carry either old or new score");
    }
}
