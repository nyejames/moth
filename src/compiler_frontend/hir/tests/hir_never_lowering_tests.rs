//! HIR lowering and validation tests for declared `-> !` contracts.
//!
//! WHAT: pins the explicit HIR Never function contract, the dedicated terminating call
//!       terminator, its argument ordering and side-table mapping, and the validation boundaries
//!       between ordinary calls and Never calls.
//! WHY: a terminating call has one semantic representation with no result local, no continuation
//!       block and no fabricated empty return shape; regressions here would silently restore a
//!       value-shaped call path.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, CallPassingMode,
};
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::never_call::{NeverCall, NeverCallTarget};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::{CallableReturnShape, FunctionReturnContract};
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionReturnContract};
use crate::compiler_frontend::hir::hir_display::HirDisplayContext;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::terminator_targets;
use crate::compiler_frontend::hir::validation::validate_hir_module;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    entry_start_node, function_node, make_test_variable, param_with_type_id,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    build_ast_with_registered_types, lower_ast,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

struct NeverFixture {
    module: HirModule,
    type_environment: TypeEnvironment,
    external_packages: super::ExternalPackageRegistry,
    string_table: StringTable,
    path_fork: super::PathInternerFork,
    fatal: FunctionId,
    sink: FunctionId,
    run: FunctionId,
    normal: FunctionId,
    run_block: BlockId,
    call_span: Option<SourceSpan>,
}

fn never_signature(
    parameters: Vec<crate::compiler_frontend::ast::statements::functions::FunctionParameter>,
) -> FunctionSignature {
    FunctionSignature {
        parameters,
        return_contract: FunctionReturnContract::Never,
    }
}

fn ordinary_signature() -> FunctionSignature {
    FunctionSignature {
        parameters: vec![],
        return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots([], None)),
    }
}

fn never_call_node(
    target: super::PathId,
    name: crate::compiler_frontend::symbols::string_interning::StringId,
    arguments: Vec<CallArgument<ExprId>>,
    span: Option<SourceSpan>,
) -> crate::compiler_frontend::ast::ast_nodes::AstNode {
    super::node(
        NodeKind::NeverCall(Box::new(NeverCall {
            target: NeverCallTarget::Source(target),
            receiver: None,
            arguments,
            name,
            span,
        })),
        span,
    )
}

fn i32_literal(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: i64,
) -> ExprId {
    builder
        .fixed_scalar(
            constants,
            moth_lexical::numeric::fixed_scalar::FixedScalarValue::signed(FixedScalar::I32, value)
                .expect("fixture literal fits I32"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar")
        .id()
}

fn build_fixture(lower_statement_after_call: bool) -> NeverFixture {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut builder = AstExpressionBuilder::new().expect("fixture builder");
    let mut constants = ConstValueStore::default();

    let fatal_name = super::symbol("fatal", &mut path_fork, &mut string_table);
    let sink_name = super::symbol("sink", &mut path_fork, &mut string_table);
    let run_name = super::symbol("run", &mut path_fork, &mut string_table);
    let normal_name = super::symbol("normal", &mut path_fork, &mut string_table);
    let first_param_name = super::symbol("first", &mut path_fork, &mut string_table);
    let second_param_name = super::symbol("second", &mut path_fork, &mut string_table);
    let after_name = super::symbol("after", &mut path_fork, &mut string_table);

    let fatal_name_id = string_table.intern("fatal");
    let sink_name_id = string_table.intern("sink");

    let mut extended = ExtendedSpanBuilder::new();
    let call_span = Some(SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(0, 5, &mut extended).expect("fixture span"),
    ));

    let mut first_extended = ExtendedSpanBuilder::new();
    let argument_span = Some(SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(20, 1, &mut first_extended).expect("fixture argument span"),
    ));

    let fatal = function_node(
        fatal_name,
        never_signature(vec![]),
        vec![never_call_node(
            fatal_name,
            fatal_name_id,
            vec![],
            call_span,
        )],
        None,
    );

    let sink = function_node(
        sink_name,
        never_signature(vec![
            param_with_type_id(
                first_param_name,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            ),
            param_with_type_id(
                second_param_name,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                true,
                None,
            ),
        ]),
        vec![never_call_node(
            fatal_name,
            fatal_name_id,
            vec![],
            call_span,
        )],
        None,
    );

    let first_argument = i32_literal(&mut builder, &mut constants, 1);
    let second_argument = i32_literal(&mut builder, &mut constants, 2);
    let mut run_body = vec![never_call_node(
        sink_name,
        sink_name_id,
        vec![
            CallArgument::positional(first_argument, CallAccessMode::Shared, argument_span),
            CallArgument::positional(second_argument, CallAccessMode::Mutable, argument_span)
                .with_passing_mode(CallPassingMode::FreshMutableValue),
        ],
        call_span,
    )];
    if lower_statement_after_call {
        let after_value = i32_literal(&mut builder, &mut constants, 9);
        run_body.push(super::node(
            NodeKind::VariableDeclaration(make_test_variable(after_name, after_value, &builder)),
            None,
        ));
    }

    let run = function_node(run_name, ordinary_signature(), run_body, None);
    let normal = function_node(normal_name, ordinary_signature(), vec![], None);
    let start = entry_start_node(ordinary_signature(), vec![]);

    let ast = build_ast_with_registered_types(
        vec![fatal, sink, run, normal, start],
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let external_packages = super::ExternalPackageRegistry::new();
    let (module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &external_packages)
            .expect("Never fixture should lower");

    let fatal = function_id_by_name(&module, &string_table, &path_fork, "fatal");
    let sink = function_id_by_name(&module, &string_table, &path_fork, "sink");
    let run = function_id_by_name(&module, &string_table, &path_fork, "run");
    let normal = function_id_by_name(&module, &string_table, &path_fork, "normal");
    let run_block = module.functions[run.0 as usize].entry;

    NeverFixture {
        module,
        type_environment,
        external_packages,
        string_table,
        path_fork,
        fatal,
        sink,
        run,
        normal,
        run_block,
        call_span,
    }
}

fn function_id_by_name(
    module: &HirModule,
    string_table: &StringTable,
    path_fork: &super::PathInternerFork,
    name: &str,
) -> FunctionId {
    module
        .functions
        .iter()
        .find_map(|function| {
            let path = module.side_table.function_name_path(function.id)?;
            (path_fork
                .component(path)
                .is_some_and(|component| string_table.resolve(component) == name))
            .then_some(function.id)
        })
        .unwrap_or_else(|| panic!("function '{name}' should exist"))
}

#[test]
fn lowers_never_function_contract_without_implicit_return() {
    let fixture = build_fixture(false);
    let fatal = &fixture.module.functions[fixture.fatal.0 as usize];
    assert_eq!(
        fatal.return_contract,
        HirFunctionReturnContract::Never,
        "the declared Never contract lowers as an explicit variant"
    );
    let block = &fixture.module.blocks[fatal.entry.0 as usize];
    assert!(
        matches!(block.terminator, HirTerminator::NeverCall { .. }),
        "a Never body ends with its terminating call, never an implicit return"
    );
    assert!(
        !block
            .statements
            .iter()
            .any(|statement| matches!(statement.kind, HirStatementKind::Call { .. })),
        "a Never call is a terminator, not a result-producing call statement"
    );
}

#[test]
fn never_call_has_no_result_local_or_successor_and_preserves_argument_order() {
    let fixture = build_fixture(false);
    let block = &fixture.module.blocks[fixture.run_block.0 as usize];
    let HirTerminator::NeverCall { target, args } = &block.terminator else {
        panic!("run should end with a NeverCall terminator");
    };
    assert_eq!(*target, CallTarget::Local(fixture.sink));
    assert!(
        terminator_targets(&block.terminator).is_empty(),
        "a NeverCall has no successor block"
    );

    let args = fixture.module.expressions.values(*args);
    assert_eq!(args.len(), 2, "receiver-less call keeps declared arguments");
    assert!(
        matches!(
            fixture.module.expressions.expression(args[0]).kind,
            HirExpressionKind::FixedScalar(_)
        ),
        "the first argument lowers in authored order without materialisation"
    );

    let second = fixture.module.expressions.expression(args[1]);
    let HirExpressionKind::Load(place) = &second.kind else {
        panic!("a fresh mutable argument loads its hidden local");
    };
    assert_eq!(second.value_kind, ValueKind::RValue);
    assert_eq!(place.projections.len(), 0);

    let fresh_local = place.root;
    assert_eq!(
        fixture.module.side_table.local_origin_kind(fresh_local),
        Some(HirLocalOriginKind::CompilerFreshMutableArg),
        "mutable fresh-rvalue materialisation is preserved"
    );
    let origin = fixture
        .module
        .side_table
        .local_origin(fresh_local)
        .expect("fresh mutable local retains its origin facts");
    assert_eq!(origin.argument_index, Some(1));
    assert_eq!(origin.call_span, fixture.call_span);

    let write = block
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } if *local == fresh_local => Some(*value),
            _ => None,
        })
        .expect("the hidden local is defined before the call");
    assert!(matches!(
        fixture.module.expressions.expression(write).kind,
        HirExpressionKind::FixedScalar(_)
    ));
    assert_eq!(
        block.statements.len(),
        1,
        "only the fresh mutable materialisation statement precedes the terminator"
    );
}

#[test]
fn never_call_maps_the_authored_location_into_the_side_table() {
    let fixture = build_fixture(false);
    let block = &fixture.module.blocks[fixture.run_block.0 as usize];
    assert!(matches!(block.terminator, HirTerminator::NeverCall { .. }));
    assert_eq!(
        fixture.module.side_table.terminator_span(fixture.run_block),
        fixture.call_span.as_ref(),
        "the authored call location is the terminator location"
    );
    assert_eq!(
        fixture.module.side_table.hir_source_span_for_hir(
            crate::compiler_frontend::hir::hir_side_table::HirLocation::Terminator(
                fixture.run_block
            )
        ),
        fixture.call_span
    );
}

#[test]
fn stops_lowering_later_statements_after_never_call() {
    let fixture = build_fixture(true);
    let block = &fixture.module.blocks[fixture.run_block.0 as usize];
    assert!(matches!(block.terminator, HirTerminator::NeverCall { .. }));
    assert_eq!(
        block.statements.len(),
        1,
        "statements after a terminating call on the current path are not lowered"
    );
}

#[test]
fn reachability_records_never_call_as_a_normal_call_edge_without_successors() {
    use crate::compiler_frontend::hir::reachability::{
        collect_module_function_link_facts, collect_reachability_from_function_link_facts,
    };

    let fixture = build_fixture(false);
    let facts = collect_module_function_link_facts(&fixture.module)
        .expect("Never fixture link facts should build");
    let reachability = collect_reachability_from_function_link_facts(&facts, &[fixture.run])
        .expect("reachability from run should build");
    assert!(
        reachability
            .backend_selection()
            .contains_function(fixture.sink),
        "the terminating call is still a normal local call edge"
    );
    assert!(
        reachability
            .backend_selection()
            .contains_function(fixture.fatal),
        "the transitively called Never target is reachable"
    );
    assert_eq!(
        reachability
            .backend_selection()
            .blocks_for_function(fixture.run),
        Some([fixture.run_block].as_slice()),
        "a NeverCall contributes no successor block"
    );
}

#[test]
fn display_renders_never_contract_and_terminating_call() {
    let fixture = build_fixture(false);
    let context = HirDisplayContext::new(&fixture.string_table, &fixture.path_fork);
    let fatal = &fixture.module.functions[fixture.fatal.0 as usize];
    let rendered_function = context.render_function(fatal);
    assert!(
        rendered_function.contains("-> !"),
        "Never functions render an explicit contract: {rendered_function}"
    );

    let block = &fixture.module.blocks[fixture.run_block.0 as usize];
    let rendered_terminator =
        context.render_terminator(&block.terminator, &fixture.module.expressions);
    assert!(
        rendered_terminator.starts_with("never_call "),
        "the terminating call has its own display form: {rendered_terminator}"
    );
    assert!(
        rendered_terminator.ends_with(')'),
        "the terminating call renders its evaluated arguments: {rendered_terminator}"
    );
}

#[test]
fn validation_rejects_never_call_to_an_ordinary_target() {
    let mut fixture = build_fixture(false);
    let block = &mut fixture.module.blocks[fixture.run_block.0 as usize];
    let HirTerminator::NeverCall { target, .. } = &mut block.terminator else {
        panic!("run should end with NeverCall");
    };
    *target = CallTarget::Local(fixture.normal);

    let error = validate_hir_module(
        &fixture.module,
        &fixture.type_environment,
        &fixture.external_packages,
    )
    .expect_err("a NeverCall must not target an ordinary function");
    assert!(
        error
            .message
            .contains("NeverCall targets an ordinary local function contract"),
        "{error:?}"
    );
}

#[test]
fn validation_rejects_an_ordinary_call_to_a_never_function() {
    let mut fixture = build_fixture(false);
    let normal_entry = fixture.module.functions[fixture.normal.0 as usize].entry;
    fixture.module.blocks[normal_entry.0 as usize]
        .statements
        .push(HirStatement {
            id: HirNodeId(9999),
            kind: HirStatementKind::Call {
                target: CallTarget::Local(fixture.fatal),
                args: HirValueRange::empty(),
                results: vec![],
            },
            span: None,
        });

    let error = validate_hir_module(
        &fixture.module,
        &fixture.type_environment,
        &fixture.external_packages,
    )
    .expect_err("an ordinary call must not target a Never function");
    assert!(
        error
            .message
            .contains("Ordinary HIR call targets a declared Never function"),
        "{error:?}"
    );
}

#[test]
fn validation_rejects_return_terminators_in_never_functions() {
    let mut fixture = build_fixture(false);
    let fatal_entry = fixture.module.functions[fixture.fatal.0 as usize].entry;
    fixture.module.blocks[fatal_entry.0 as usize].terminator =
        HirTerminator::Return(HirValueRange::empty());

    let error = validate_hir_module(
        &fixture.module,
        &fixture.type_environment,
        &fixture.external_packages,
    )
    .expect_err("a Never function must not return to its caller");
    assert!(
        error
            .message
            .contains("Return terminator appears in a declared Never function"),
        "{error:?}"
    );
}

#[test]
fn private_failure_lane_installation_leaves_a_never_contract_unchanged() {
    use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
    use crate::compiler_frontend::hir::blocks::HirBlock;
    use crate::compiler_frontend::hir::failure_facts::{
        HirBuiltinFailureBoundary, HirFunctionFailureFacts,
    };
    use crate::compiler_frontend::hir::ids::{BlockId, RegionId};
    use crate::compiler_frontend::hir::private_failure_lane::{
        PrivateFailureLaneInstallation, install_private_failure_lanes,
    };
    use crate::compiler_frontend::public_call_summary::{
        FunctionReturnAliasSummary, PublicCallSummary,
    };

    let mut module = HirModule::new();
    module.functions.push(HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::Never,
    });
    module.blocks.push(HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::NeverCall {
            target: CallTarget::Local(FunctionId(0)),
            args: HirValueRange::empty(),
        },
    });
    module.function_failure_facts.insert(
        FunctionId(0),
        HirFunctionFailureFacts {
            span: None,
            boundary: HirBuiltinFailureBoundary::Never,
            infallible_cast_evidence: None,
            contributors: vec![],
            assertion_message_calls: vec![],
            deferred_custom_catches: vec![],
        },
    );
    let mut report = BorrowCheckReport::default();
    report.analysis.public_call_summaries.insert(
        FunctionId(0),
        PublicCallSummary {
            parameters: vec![],
            return_alias: FunctionReturnAliasSummary::Fresh,
            escapes_builtin_failure: true,
        },
    );
    let mut type_environment = TypeEnvironment::new();
    let external_packages = super::ExternalPackageRegistry::new();

    let installation = install_private_failure_lanes(
        &mut module,
        &report,
        &mut type_environment,
        &external_packages,
    )
    .expect("a closed Never boundary is not a private failure lane candidate");
    assert_eq!(
        installation,
        PrivateFailureLaneInstallation::Unchanged,
        "lane installation must not rewrite a Never-only module"
    );
    assert_eq!(
        module.functions[0].return_contract,
        HirFunctionReturnContract::Never,
        "a Never contract never gains an internal error lane"
    );
    assert!(
        module.call_result_contracts.is_empty(),
        "lane installation records no nonlocal Never contract"
    );
}

#[test]
fn never_receiver_method_lowers_the_receiver_to_argument_slot_zero() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut builder = AstExpressionBuilder::new().expect("fixture builder");
    let mut constants = ConstValueStore::default();

    let fatal_name = super::symbol("fatal", &mut path_fork, &mut string_table);
    let halt_name = super::symbol("halt", &mut path_fork, &mut string_table);
    let self_name = super::symbol("self", &mut path_fork, &mut string_table);
    let argument_name = super::symbol("argument", &mut path_fork, &mut string_table);
    let run_name = super::symbol("run", &mut path_fork, &mut string_table);
    let fatal_name_id = string_table.intern("fatal");
    let halt_name_id = string_table.intern("halt");

    let mut call_extended = ExtendedSpanBuilder::new();
    let call_span = Some(SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(10, 5, &mut call_extended).expect("fixture call span"),
    ));
    let mut argument_extended = ExtendedSpanBuilder::new();
    let argument_span = Some(SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(20, 1, &mut argument_extended).expect("fixture argument span"),
    ));

    let fatal = function_node(
        fatal_name,
        never_signature(vec![]),
        vec![never_call_node(
            fatal_name,
            fatal_name_id,
            vec![],
            call_span,
        )],
        None,
    );
    // The receiver is a hidden slot-zero argument, so the method signature carries it as its
    // first parameter just like an ordinary receiver call.
    let halt = function_node(
        halt_name,
        never_signature(vec![
            param_with_type_id(
                self_name,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                false,
                None,
            ),
            param_with_type_id(
                argument_name,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                true,
                None,
            ),
        ]),
        vec![never_call_node(
            fatal_name,
            fatal_name_id,
            vec![],
            call_span,
        )],
        None,
    );

    let receiver = i32_literal(&mut builder, &mut constants, 1);
    let argument = i32_literal(&mut builder, &mut constants, 2);
    let run = function_node(
        run_name,
        ordinary_signature(),
        vec![super::node(
            NodeKind::NeverCall(Box::new(NeverCall {
                target: NeverCallTarget::Source(halt_name),
                receiver: Some(receiver),
                arguments: vec![
                    CallArgument::positional(argument, CallAccessMode::Mutable, argument_span)
                        .with_passing_mode(CallPassingMode::FreshMutableValue),
                ],
                name: halt_name_id,
                span: call_span,
            })),
            call_span,
        )],
        None,
    );
    let start = entry_start_node(ordinary_signature(), vec![]);

    let ast = build_ast_with_registered_types(
        vec![fatal, halt, run, start],
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let external_packages = super::ExternalPackageRegistry::new();
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &external_packages)
            .expect("receiver Never fixture should lower");

    let halt = function_id_by_name(&module, &string_table, &path_fork, "halt");
    let run = function_id_by_name(&module, &string_table, &path_fork, "run");
    let run_block = module.functions[run.0 as usize].entry;
    let block = &module.blocks[run_block.0 as usize];
    let HirTerminator::NeverCall { target, args } = &block.terminator else {
        panic!("run should end with a NeverCall terminator");
    };
    assert_eq!(*target, CallTarget::Local(halt));

    let args = module.expressions.values(*args);
    assert_eq!(
        args.len(),
        2,
        "the receiver occupies argument slot zero before the declared argument"
    );
    assert!(
        matches!(
            module.expressions.expression(args[0]).kind,
            HirExpressionKind::FixedScalar(_)
        ),
        "the receiver is evaluated before the declared arguments"
    );
    let HirExpressionKind::Load(place) = &module.expressions.expression(args[1]).kind else {
        panic!("the fresh mutable argument loads its hidden local");
    };
    let origin = module
        .side_table
        .local_origin(place.root)
        .expect("fresh mutable local retains its origin facts");
    assert_eq!(
        origin.argument_index,
        Some(1),
        "declared argument indices follow the hidden receiver slot"
    );
    assert_eq!(origin.call_span, call_span);
}

#[test]
fn never_external_binding_lowers_to_an_external_terminating_call() {
    use crate::compiler_frontend::external_packages::{
        ExternalFunctionDef, ExternalFunctionLowerings, ExternalPackageRegistry, external_never,
    };

    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let builder = AstExpressionBuilder::new().expect("fixture builder");
    let constants = ConstValueStore::default();

    let mut external_packages = ExternalPackageRegistry::new();
    let external_never = external_packages
        .register_function(ExternalFunctionDef {
            name: "exit".to_owned(),
            parameters: vec![],
            return_contract: external_never(),
            const_eval: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("synthetic Never binding should register");

    let run_name = super::symbol("run", &mut path_fork, &mut string_table);
    let exit_name_id = string_table.intern("exit");
    let run = function_node(
        run_name,
        ordinary_signature(),
        vec![super::node(
            NodeKind::NeverCall(Box::new(NeverCall {
                target: NeverCallTarget::External(external_never),
                receiver: None,
                arguments: vec![],
                name: exit_name_id,
                span: None,
            })),
            None,
        )],
        None,
    );
    let start = entry_start_node(ordinary_signature(), vec![]);

    let ast = build_ast_with_registered_types(
        vec![run, start],
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &external_packages)
            .expect("external Never fixture should lower");

    let run = function_id_by_name(&module, &string_table, &path_fork, "run");
    let run_block = module.functions[run.0 as usize].entry;
    let block = &module.blocks[run_block.0 as usize];
    let HirTerminator::NeverCall { target, args } = &block.terminator else {
        panic!("run should end with a NeverCall terminator");
    };
    assert_eq!(
        *target,
        CallTarget::External(external_never),
        "the binding-backed target lowers as an external terminating call"
    );
    assert!(
        module.expressions.values(*args).is_empty(),
        "a zero-argument Never binding evaluates no arguments"
    );
    assert!(
        !module
            .call_result_contracts
            .contains_key(&CallTarget::External(external_never)),
        "a binding-backed external target never needs a nonlocal contract entry"
    );
}

#[test]
fn never_call_records_the_never_contract_for_an_imported_target() {
    use crate::compiler_frontend::ast::AstImportedFunctionContract;
    use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
    use crate::compiler_frontend::public_call_summary::{
        FunctionReturnAliasSummary, PublicCallSummary,
    };
    use crate::compiler_frontend::semantic_identity::{
        ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity, StablePackageIdentity,
    };

    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let builder = AstExpressionBuilder::new().expect("fixture builder");
    let constants = ConstValueStore::default();

    let exit_path = super::symbol("exit", &mut path_fork, &mut string_table);
    let run_name = super::symbol("run", &mut path_fork, &mut string_table);
    let exit_name_id = string_table.intern("exit");
    let run = function_node(
        run_name,
        ordinary_signature(),
        vec![super::node(
            NodeKind::NeverCall(Box::new(NeverCall {
                target: NeverCallTarget::Source(exit_path),
                receiver: None,
                arguments: vec![],
                name: exit_name_id,
                span: None,
            })),
            None,
        )],
        None,
    );
    let start = entry_start_node(ordinary_signature(), vec![]);

    let origin = OriginFunctionId::new_free(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("test"),
            "provider".to_owned(),
            ModuleRootRole::Normal,
        ),
        "exit".to_owned(),
    );
    let mut ast = build_ast_with_registered_types(
        vec![run, start],
        builder,
        constants,
        TypeEnvironment::new(),
    );
    ast.imported_functions_by_local_path.insert(
        exit_path,
        AstImportedFunctionContract {
            target: SourceFunctionTarget::Imported {
                origin: origin.clone(),
                local_path: exit_path,
            },
            summary: PublicCallSummary {
                parameters: vec![],
                return_alias: FunctionReturnAliasSummary::Fresh,
                escapes_builtin_failure: false,
            },
            return_contract: FunctionReturnContract::Never,
        },
    );

    let external_packages = super::ExternalPackageRegistry::new();
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &external_packages)
            .expect("imported Never fixture should lower");

    let target = CallTarget::CrossModule(origin);
    assert_eq!(
        module.call_result_contracts.get(&target),
        Some(&HirFunctionReturnContract::Never),
        "the imported target's contract is recorded without rescanning metadata"
    );
}

#[test]
fn never_call_records_the_never_contract_for_a_generated_target() {
    use crate::compiler_frontend::ast::AstImportedFunctionContract;
    use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
    use crate::compiler_frontend::public_call_summary::{
        FunctionReturnAliasSummary, PublicCallSummary,
    };
    use crate::compiler_frontend::semantic_identity::{
        GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModuleRootRole, OriginFunctionId,
        StableModuleOriginIdentity, StablePackageIdentity,
    };

    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let builder = AstExpressionBuilder::new().expect("fixture builder");
    let constants = ConstValueStore::default();

    let exit_path = super::symbol("exit", &mut path_fork, &mut string_table);
    let run_name = super::symbol("run", &mut path_fork, &mut string_table);
    let exit_name_id = string_table.intern("exit");
    let run = function_node(
        run_name,
        ordinary_signature(),
        vec![super::node(
            NodeKind::NeverCall(Box::new(NeverCall {
                target: NeverCallTarget::Source(exit_path),
                receiver: None,
                arguments: vec![],
                name: exit_name_id,
                span: None,
            })),
            None,
        )],
        None,
    );
    let start = entry_start_node(ordinary_signature(), vec![]);

    let origin = OriginFunctionId::new_free(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("test"),
            "provider".to_owned(),
            ModuleRootRole::Normal,
        ),
        "exit".to_owned(),
    );
    let identity = GeneratedFunctionIdentity::new(
        GeneratedDeclarationIdentity::Public(origin),
        Box::new([]),
        Box::new([]),
    );
    let mut ast = build_ast_with_registered_types(
        vec![run, start],
        builder,
        constants,
        TypeEnvironment::new(),
    );
    ast.imported_functions_by_local_path.insert(
        exit_path,
        AstImportedFunctionContract {
            target: SourceFunctionTarget::Generated {
                identity: identity.clone(),
                local_path: exit_path,
            },
            summary: PublicCallSummary {
                parameters: vec![],
                return_alias: FunctionReturnAliasSummary::Fresh,
                escapes_builtin_failure: false,
            },
            return_contract: FunctionReturnContract::Never,
        },
    );

    let external_packages = super::ExternalPackageRegistry::new();
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &external_packages)
            .expect("generated Never fixture should lower");

    let target = CallTarget::Generated(Box::new(identity));
    assert_eq!(
        module.call_result_contracts.get(&target),
        Some(&HirFunctionReturnContract::Never),
        "the generated target's contract is recorded without rescanning metadata"
    );
}

#[test]
fn surviving_never_fallthrough_is_an_internal_hir_error() {
    use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
    use crate::compiler_frontend::compiler_messages::PreparedFailure;

    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let builder = AstExpressionBuilder::new().expect("fixture builder");
    let constants = ConstValueStore::default();

    let fatal_name = super::symbol("fatal", &mut path_fork, &mut string_table);
    let fatal = function_node(fatal_name, never_signature(vec![]), vec![], None);
    let start = entry_start_node(ordinary_signature(), vec![]);
    let ast = build_ast_with_registered_types(
        vec![fatal, start],
        builder,
        constants,
        TypeEnvironment::new(),
    );

    let error = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect_err("a Never body that can fall through is an internal invariant failure");
    let error = match error {
        PreparedFailure::Infrastructure(errors) => errors.primary,
        PreparedFailure::Diagnosed(_) => {
            panic!("a surviving Never fallthrough must not be a user diagnostic")
        }
    };
    assert_eq!(error.kind, InfrastructureFailureKind::HirTransformation);
    assert!(
        error.message.contains("Never function body"),
        "the internal error names the surviving Never fallthrough: {}",
        error.message
    );
}

#[test]
fn lane_split_moves_an_authored_terminator_location_to_the_success_block() {
    use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
    use crate::compiler_frontend::canonical_type_identity::{
        CanonicalBuiltinType, CanonicalTypeIdentity,
    };
    use crate::compiler_frontend::hir::blocks::HirBlock;
    use crate::compiler_frontend::hir::failure_facts::{
        HirBuiltinFailureBoundary, HirFunctionFailureFacts,
    };
    use crate::compiler_frontend::hir::functions::HirFunctionOrigin;
    use crate::compiler_frontend::hir::hir_side_table::HirLocation;
    use crate::compiler_frontend::hir::ids::RegionId;
    use crate::compiler_frontend::hir::private_failure_lane::{
        PrivateFailureLaneInstallation, install_private_failure_lanes,
    };
    use crate::compiler_frontend::hir::regions::HirRegion;
    use crate::compiler_frontend::public_call_summary::{
        FunctionReturnAliasSummary, PublicCallSummary,
    };
    use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;

    let failure_facts = |boundary| HirFunctionFailureFacts {
        span: None,
        boundary,
        infallible_cast_evidence: None,
        contributors: vec![],
        assertion_message_calls: vec![],
        deferred_custom_catches: vec![],
    };

    let mut module = HirModule::new();
    // A private producer call before a terminating call: the lane split moves the NeverCall
    // terminator into the success continuation.
    module.functions.push(HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_contract: HirFunctionReturnContract::returns(CallableReturnShape::from_slots(
            [],
            None,
        )),
    });
    module.functions.push(HirFunction {
        id: FunctionId(1),
        entry: BlockId(1),
        params: vec![],
        return_contract: HirFunctionReturnContract::Never,
    });
    module
        .function_origins
        .insert(FunctionId(0), HirFunctionOrigin::Normal);
    module
        .function_origins
        .insert(FunctionId(1), HirFunctionOrigin::Normal);
    module
        .function_provenance
        .insert(FunctionId(0), SyntheticInterfaceProvenance::empty());
    module
        .function_provenance
        .insert(FunctionId(1), SyntheticInterfaceProvenance::empty());
    module.regions.push(HirRegion::lexical(RegionId(0), None));
    module.blocks.push(HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![HirStatement {
            id: HirNodeId(0),
            kind: HirStatementKind::Call {
                target: CallTarget::Local(FunctionId(0)),
                args: HirValueRange::empty(),
                results: vec![],
            },
            span: None,
        }],
        terminator: HirTerminator::NeverCall {
            target: CallTarget::Local(FunctionId(1)),
            args: HirValueRange::empty(),
        },
    });
    module.blocks.push(HirBlock {
        id: BlockId(1),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::NeverCall {
            target: CallTarget::Local(FunctionId(1)),
            args: HirValueRange::empty(),
        },
    });

    let mut extended = ExtendedSpanBuilder::new();
    let terminator_span = SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(40, 7, &mut extended).expect("fixture terminator span"),
    );
    module
        .side_table
        .map_terminator(Some(terminator_span), BlockId(0));
    module
        .side_table
        .map_terminator_span(BlockId(0), terminator_span);

    module.function_failure_facts.insert(
        FunctionId(0),
        failure_facts(HirBuiltinFailureBoundary::InferPrivate),
    );
    module.function_failure_facts.insert(
        FunctionId(1),
        failure_facts(HirBuiltinFailureBoundary::Never),
    );
    let mut report = BorrowCheckReport::default();
    report.analysis.public_call_summaries.insert(
        FunctionId(0),
        PublicCallSummary {
            parameters: vec![],
            return_alias: FunctionReturnAliasSummary::Fresh,
            escapes_builtin_failure: true,
        },
    );

    let mut type_environment = TypeEnvironment::new();
    type_environment
        .register_canonical_identity(
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error),
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
        )
        .expect("fixture builtin Error identity should register");
    let external_packages = super::ExternalPackageRegistry::new();

    let installation = install_private_failure_lanes(
        &mut module,
        &report,
        &mut type_environment,
        &external_packages,
    )
    .expect("lane split should produce valid HIR");
    assert_eq!(installation, PrivateFailureLaneInstallation::Rewritten);

    let HirTerminator::Invoke { success, .. } = &module.blocks[0].terminator else {
        panic!("the private producer should be split into an Invoke");
    };
    let success = *success;
    assert_eq!(
        module.side_table.terminator_span(success),
        Some(&terminator_span),
        "the authored NeverCall location follows its terminator into the success block"
    );
    assert_eq!(
        module
            .side_table
            .ast_span_for_hir(HirLocation::Terminator(success)),
        Some(terminator_span)
    );
    assert_eq!(
        module
            .side_table
            .hir_source_span_for_hir(HirLocation::Terminator(success)),
        Some(terminator_span)
    );
    assert_eq!(
        module.side_table.terminator_span(BlockId(0)),
        None,
        "the generated replacement Invoke carries no authored terminator location"
    );
    assert_eq!(
        module
            .side_table
            .ast_span_for_hir(HirLocation::Terminator(BlockId(0))),
        None
    );
    assert_eq!(
        module
            .side_table
            .hir_source_span_for_hir(HirLocation::Terminator(BlockId(0))),
        None
    );
    let reverse = module.side_table.hir_locations_for_ast(terminator_span);
    assert!(reverse.contains(&HirLocation::Terminator(success)));
    assert!(!reverse.contains(&HirLocation::Terminator(BlockId(0))));
}

#[test]
fn moving_a_spanless_terminator_location_rekeys_its_mapping_entries() {
    use crate::compiler_frontend::hir::hir_side_table::{HirLocation, HirSideTable};

    let mut extended = ExtendedSpanBuilder::new();
    let span = SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(12, 4, &mut extended).expect("fixture terminator span"),
    );
    let (from, to) = (BlockId(0), BlockId(1));
    let mut side_table = HirSideTable::default();
    side_table.map_terminator(Some(span), from);

    side_table.move_terminator_location(from, to);

    assert_eq!(side_table.terminator_span(to), None);
    assert_eq!(
        side_table.ast_span_for_hir(HirLocation::Terminator(from)),
        None
    );
    assert_eq!(
        side_table.hir_source_span_for_hir(HirLocation::Terminator(from)),
        None
    );
    assert_eq!(
        side_table.ast_span_for_hir(HirLocation::Terminator(to)),
        Some(span)
    );
    assert_eq!(
        side_table.hir_source_span_for_hir(HirLocation::Terminator(to)),
        Some(span)
    );
    assert_eq!(
        side_table.hir_locations_for_ast(span),
        [HirLocation::Terminator(to)]
    );
}
