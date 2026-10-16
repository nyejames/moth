//! AST fixture support for frontend unit tests.
//!
//! WHAT: builds hand-written AST nodes, source locations, and AST lookup fixtures.
//! WHY: AST and HIR tests both need small synthetic trees, but these helpers must stay free of
//!      HIR lowering and borrow-checker ownership.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, IfBranchMetadata, NodeKind};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOp, PlaceId, PlaceRecord, SuccessShape,
};
use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionParameter, FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;

/// Creates a span-less synthetic source position for use in test fixtures.
///
/// Test ASTs model generated values rather than authored source, so their provenance is absent.
/// Keep the line argument for call-site readability while avoiding fabricated line/column data.
pub(crate) fn test_source_location(_line: i32) -> Option<SourceSpan> {
    None
}

pub(crate) fn node(kind: NodeKind, span: Option<SourceSpan>) -> AstNode {
    AstNode {
        kind,
        span,
        scope: PathId::ROOT,
    }
}

pub(crate) fn test_if_branch_metadata(has_else: bool) -> IfBranchMetadata {
    let then_scope = PathId::ROOT;
    let else_scope = has_else.then_some(PathId::ROOT);
    IfBranchMetadata::new(IfGenericRequestRanges::default(), then_scope, else_scope)
}

pub(crate) fn make_test_variable(
    name: PathId,
    value: ExprId,
    builder: &AstExpressionBuilder,
) -> Declaration {
    let expression = builder
        .view(value)
        .expect("fixture initializer belongs to builder");
    Declaration {
        id: name,
        type_id: expression
            .value_type_id()
            .expect("fixture initializer is single-valued"),
        type_syntax: None,
        value_mode: expression.value_mode(),
        value: DeclarationValue::Initializer(value),
        binding_span: None,
        config_qualifier: None,
    }
}

fn parameter(
    name: PathId,
    type_id: TypeId,
    mutable: bool,
    binding_span: Option<SourceSpan>,
) -> FunctionParameter {
    let value_mode = if mutable {
        ValueMode::MutableOwned
    } else {
        ValueMode::ImmutableOwned
    };

    FunctionParameter {
        id: name,
        type_id,
        value_mode,
        default: None,
        binding_span,
    }
}

/// Creates a parameter fixture with its resolved semantic type identity.
pub(crate) fn param_with_type_id(
    name: PathId,
    type_id: TypeId,
    mutable: bool,
    span: Option<SourceSpan>,
) -> FunctionParameter {
    parameter(name, type_id, mutable, span)
}

pub(crate) fn function_node(
    name: PathId,
    signature: FunctionSignature,
    body: Vec<AstNode>,
    span: Option<SourceSpan>,
) -> AstNode {
    node(NodeKind::Function(name, signature, body), span)
}

pub(crate) fn success_return_slot(type_id: TypeId) -> ReturnSlot {
    ReturnSlot {
        type_id: Some(type_id),
        channel: ReturnChannel::Success,
    }
}

pub(crate) fn fresh_success_returns(result_type_ids: Vec<TypeId>) -> Vec<ReturnSlot> {
    result_type_ids
        .into_iter()
        .map(success_return_slot)
        .collect()
}

pub(crate) fn symbol(
    name: &str,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> PathId {
    path_fork
        .try_intern_portable_path(name, string_table)
        .expect("test path fits")
}

fn reference_expr(
    builder: &mut AstExpressionBuilder,
    name: PathId,
    type_id: TypeId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    builder.reference_with_type_id(
        name, type_id, span, value_mode,
        crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
    ).expect("fixture reference fits expression store").id()
}

/// Builds an immutable reference expression with its resolved semantic type identity.
pub(crate) fn immutable_reference_expr(
    builder: &mut AstExpressionBuilder,
    name: PathId,
    type_id: TypeId,
    span: Option<SourceSpan>,
) -> ExprId {
    reference_expr(builder, name, type_id, span, ValueMode::ImmutableReference)
}

pub(crate) fn reference_expr_with_type_id(
    builder: &mut AstExpressionBuilder,
    name: PathId,
    type_id: TypeId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    reference_expr(builder, name, type_id, span, value_mode)
}

pub(crate) fn assignment_target(
    builder: &mut AstExpressionBuilder,
    name: PathId,
    type_id: TypeId,
    span: Option<SourceSpan>,
) -> PlaceId {
    let projections = builder
        .append_place_projections(vec![], span)
        .expect("fixture place projections fit store");
    builder
        .append_place(
            PlaceRecord {
                root: name,
                root_type_id: type_id,
                projections,
                type_id,
                value_mode: ValueMode::MutableReference,
                span,
            },
            span,
        )
        .expect("fixture place fits expression store")
}

pub(crate) fn function_node_by_name<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> &'a AstNode {
    ast.nodes
        .iter()
        .find(|node| match &node.kind {
            NodeKind::Function(path, ..) => path_fork
                .component(*path)
                .is_some_and(|component| string_table.resolve(component) == name),
            _ => false,
        })
        .unwrap_or_else(|| panic!("expected function '{name}' in AST"))
}

pub(crate) fn function_signature_by_name<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> &'a FunctionSignature {
    let node = function_node_by_name(ast, path_fork, string_table, name);
    match &node.kind {
        NodeKind::Function(_, signature, _) => signature,
        _ => unreachable!("function lookup should only return function nodes"),
    }
}

pub(crate) fn function_body_by_name<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> &'a [AstNode] {
    let node = function_node_by_name(ast, path_fork, string_table, name);
    match &node.kind {
        NodeKind::Function(_, _, body) => body,
        _ => unreachable!("function lookup should only return function nodes"),
    }
}

pub(crate) fn start_function_body<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> &'a [AstNode] {
    function_body_by_name(ast, path_fork, string_table, IMPLICIT_START_FUNC_NAME)
}

/// Builds a runtime template handoff expression through the live append and success-shape path.
///
/// The handoff row is appended to the builder's side table and wrapped as a single-`STRING`
/// expression carrying the caller-supplied `span`. Test fixtures use this instead of a
/// handoff constructor.
pub(crate) fn runtime_template_handoff_expr(
    builder: &mut AstExpressionBuilder,
    handoff: OwnedRuntimeTemplateHandoff,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    let operation = builder
        .append_runtime_template(handoff, span)
        .map(ExpressionOp::RuntimeTemplate)
        .expect("fixture runtime template handoff must append to the expression store");
    builder
        .append_with_success_shape(
            operation,
            span,
            SuccessShape::One(builtin_type_ids::STRING),
            value_mode,
        )
        .expect("fixture runtime template handoff must be a valid expression")
        .id()
}

/// Builds a runtime slot application handoff expression through the live append and
/// success-shape path. Slot-application counterpart of [`runtime_template_handoff_expr`].
pub(crate) fn runtime_slot_application_handoff_expr(
    builder: &mut AstExpressionBuilder,
    handoff: OwnedRuntimeSlotApplicationHandoff,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    let operation = builder
        .append_runtime_slot_application(handoff, span)
        .map(ExpressionOp::RuntimeSlotApplication)
        .expect("fixture runtime slot handoff must append to the expression store");
    builder
        .append_with_success_shape(
            operation,
            span,
            SuccessShape::One(builtin_type_ids::STRING),
            value_mode,
        )
        .expect("fixture runtime slot handoff must be a valid expression")
        .id()
}
