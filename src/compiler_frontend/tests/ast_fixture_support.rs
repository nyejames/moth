//! AST fixture support for frontend unit tests.
//!
//! WHAT: builds hand-written AST nodes, source locations, and AST lookup fixtures.
//! WHY: AST and HIR tests both need small synthetic trees, but these helpers must stay free of
//!      HIR lowering and borrow-checker ownership.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, IfBranchMetadata, NodeKind, SourceLocation,
};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::datatypes::{DataType, TypeId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::CharPosition;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;

/// Creates a single-line `SourceLocation` at the given line number for use in test fixtures.
///
/// WHAT: produces a deterministic source location with an arbitrary column span.
/// WHY: many test suites construct locations for the same reason; one canonical helper prevents
///      each suite from defining its own with slightly different shapes.
pub(crate) fn test_source_location(line: i32) -> SourceLocation {
    SourceLocation {
        scope: InternedPath::new(),
        start_pos: CharPosition {
            line_number: line,
            char_column: 0,
        },
        end_pos: CharPosition {
            line_number: line,
            char_column: 120,
        },
    }
}

pub(crate) fn node(kind: NodeKind, location: SourceLocation) -> AstNode {
    AstNode {
        kind,
        location,
        scope: InternedPath::new(),
    }
}

pub(crate) fn test_if_branch_metadata(has_else: bool) -> IfBranchMetadata {
    let mut string_table = StringTable::new();
    let then_scope = InternedPath::from_single_str("test_then_branch", &mut string_table);
    let else_scope =
        has_else.then(|| InternedPath::from_single_str("test_else_branch", &mut string_table));

    IfBranchMetadata::new(IfGenericRequestRanges::default(), then_scope, else_scope)
}

pub(crate) fn make_test_variable(name: InternedPath, value: Expression) -> Declaration {
    Declaration {
        id: name,
        value,
        config_qualifier: None,
    }
}

pub(crate) fn param(
    name: InternedPath,
    data_type: DataType,
    id: TypeId,
    mutable: bool,
    location: SourceLocation,
) -> Declaration {
    let value_mode = if mutable {
        ValueMode::MutableOwned
    } else {
        ValueMode::ImmutableOwned
    };

    Declaration {
        id: name,
        value: Expression::new(ExpressionKind::NoValue, location, id, data_type, value_mode),
        config_qualifier: None,
    }
}

pub(crate) fn function_node(
    name: InternedPath,
    signature: FunctionSignature,
    body: Vec<AstNode>,
    location: SourceLocation,
) -> AstNode {
    node(NodeKind::Function(name, signature, body), location)
}

pub(crate) fn fresh_success_returns(result_type_ids: Vec<TypeId>) -> Vec<ReturnSlot> {
    result_type_ids
        .into_iter()
        .map(|type_id| ReturnSlot {
            value: DataType::Inferred,
            type_id: Some(type_id),
            reactive_template: None,
            channel: ReturnChannel::Success,
        })
        .collect()
}

pub(crate) fn symbol(name: &str, string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_single_str(name, string_table)
}

/// A reference expression whose value mode is fixed to `ImmutableReference`.
///
/// The caller supplies the diagnostic `DataType`. Named for the mode it fixes so it cannot be
/// confused with `type_id_fixture_support::inferred_type_reference_expr`, which fixes the
/// `DataType` instead and lets the caller choose the mode.
pub(crate) fn immutable_reference_expr(
    name: InternedPath,
    data_type: DataType,
    id: TypeId,
    location: SourceLocation,
) -> Expression {
    Expression::reference_with_type_id(
        name,
        data_type,
        id,
        location,
        ValueMode::ImmutableReference,
        crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
    )
}

pub(crate) fn assignment_target(
    name: InternedPath,
    data_type: DataType,
    id: TypeId,
    location: SourceLocation,
) -> PlaceExpression {
    PlaceExpression {
        kind: PlaceExpressionKind::Local(name),
        type_id: id,
        diagnostic_type: data_type,
        value_mode: ValueMode::MutableReference,
        location,
    }
}

pub(crate) fn function_node_by_name<'a>(
    ast: &'a Ast,
    string_table: &StringTable,
    name: &str,
) -> &'a AstNode {
    ast.nodes
        .iter()
        .find(|node| match &node.kind {
            NodeKind::Function(path, ..) => path.name_str(string_table) == Some(name),
            _ => false,
        })
        .unwrap_or_else(|| panic!("expected function '{name}' in AST"))
}

pub(crate) fn function_signature_by_name<'a>(
    ast: &'a Ast,
    string_table: &StringTable,
    name: &str,
) -> &'a FunctionSignature {
    let node = function_node_by_name(ast, string_table, name);
    match &node.kind {
        NodeKind::Function(_, signature, _) => signature,
        _ => unreachable!("function lookup should only return function nodes"),
    }
}

pub(crate) fn function_body_by_name<'a>(
    ast: &'a Ast,
    string_table: &StringTable,
    name: &str,
) -> &'a [AstNode] {
    let node = function_node_by_name(ast, string_table, name);
    match &node.kind {
        NodeKind::Function(_, _, body) => body,
        _ => unreachable!("function lookup should only return function nodes"),
    }
}

pub(crate) fn start_function_body<'a>(ast: &'a Ast, string_table: &StringTable) -> &'a [AstNode] {
    function_body_by_name(ast, string_table, IMPLICIT_START_FUNC_NAME)
}
