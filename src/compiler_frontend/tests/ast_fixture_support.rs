//! AST fixture support for frontend unit tests.
//!
//! WHAT: builds hand-written AST nodes, source locations, and AST lookup fixtures.
//! WHY: AST and HIR tests both need small synthetic trees, but these helpers must stay free of
//!      HIR lowering and borrow-checker ownership.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, IfBranchMetadata, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::datatypes::{DataType, TypeId};
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

pub(crate) fn make_test_variable(name: PathId, value: Expression) -> Declaration {
    Declaration {
        id: name,
        value,
        binding_span: None,
        config_qualifier: None,
    }
}

fn parameter(
    name: PathId,
    data_type: DataType,
    type_id: TypeId,
    mutable: bool,
    span: Option<SourceSpan>,
) -> Declaration {
    let value_mode = if mutable {
        ValueMode::MutableOwned
    } else {
        ValueMode::ImmutableOwned
    };

    Declaration {
        id: name,
        value: Expression::new(
            ExpressionKind::NoValue,
            span,
            type_id,
            data_type,
            value_mode,
        ),
        binding_span: None,
        config_qualifier: None,
    }
}

/// Parameter fixture entry points: use `param_with_datatype` when the diagnostic
/// type is part of the fixture, and `param_with_type_id` when the canonical
/// `TypeId` is all the test needs.
pub(crate) fn param_with_datatype(
    name: PathId,
    data_type: DataType,
    type_id: TypeId,
    mutable: bool,
    span: Option<SourceSpan>,
) -> Declaration {
    parameter(name, data_type, type_id, mutable, span)
}

pub(crate) fn param_with_type_id(
    name: PathId,
    type_id: TypeId,
    mutable: bool,
    span: Option<SourceSpan>,
) -> Declaration {
    parameter(name, DataType::Inferred, type_id, mutable, span)
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
        value: DataType::Inferred,
        type_id: Some(type_id),
        reactive_template: None,
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
    name: PathId,
    data_type: DataType,
    type_id: TypeId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> Expression {
    Expression::reference_with_type_id(
        name,
        data_type,
        type_id,
        span,
        value_mode,
        crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
    )
}

/// Reference-expression fixture entry points: use `reference_expr_with_datatype`
/// when the diagnostic type is part of the fixture (and the reference is
/// immutable), or `reference_expr_with_type_id` when the canonical `TypeId` is
/// known and the value mode must be chosen by the caller.
pub(crate) fn reference_expr_with_datatype(
    name: PathId,
    data_type: DataType,
    type_id: TypeId,
    span: Option<SourceSpan>,
) -> Expression {
    reference_expr(
        name,
        data_type,
        type_id,
        span,
        ValueMode::ImmutableReference,
    )
}

pub(crate) fn reference_expr_with_type_id(
    name: PathId,
    type_id: TypeId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> Expression {
    reference_expr(name, DataType::Inferred, type_id, span, value_mode)
}

pub(crate) fn assignment_target(
    name: PathId,
    data_type: DataType,
    id: TypeId,
    span: Option<SourceSpan>,
) -> PlaceExpression {
    PlaceExpression {
        kind: PlaceExpressionKind::Local(name),
        type_id: id,
        diagnostic_type: data_type,
        value_mode: ValueMode::MutableReference,
        span,
    }
}

pub(crate) fn function_node_by_name<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> &'a AstNode {
    let mut scratch = Vec::new();
    ast.nodes
        .iter()
        .find(|node| match &node.kind {
            NodeKind::Function(path, ..) => {
                path_fork.render_portable(*path, string_table, &mut scratch) == name
            }
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
