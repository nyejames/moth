//! AST place-shape helpers shared by parser and call validation.
//!
//! WHAT: classifies AST nodes as readable/writable places and describes the
//! receiver source state used by shared receiver-access validation.
//! WHY: receiver-method parsing, builtin member parsing, assignment and call
//! validation all enforce the same place rules, so one helper module keeps
//! diagnostics and semantics aligned.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::PlaceExpression;
use crate::compiler_frontend::ast::expressions::expression_rpn::PlaceExpressionKind;
use crate::compiler_frontend::ast::expressions::parse_expression_places::{
    place_expression_from_expression, place_expression_is_mutable,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// The receiver source state shared by source methods, collection builtins and map builtins.
///
/// WHAT: distinguishes a temporary/non-place receiver from an existing place, and for an
///       existing place carries its mutability and the simple root binding name when the
///       place has a namable root.
/// WHY: mutable receiver validation needs all three facts together to choose the right
///      diagnostic, and computing them in one traversal keeps receiver-access ownership in
///      one place instead of three overlapping receiver walks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReceiverSourceState {
    /// A temporary or non-place receiver: a literal, constructor, call result or computed
    /// expression. A mutable place is required to mutate through it, so it cannot be repaired
    /// by declaring an existing binding mutable.
    Temporary,
    /// An existing immutable place. `binding_name` is the simple root binding name when one is
    /// namable, so immutable-place diagnostics can name the binding to declare mutable.
    ImmutablePlace { binding_name: Option<StringId> },
    /// An existing mutable place. `binding_name` is the simple root binding name when one is
    /// namable, so the missing-`~` diagnostic can show a concrete `~name.method(...)` example.
    MutablePlace { binding_name: Option<StringId> },
}

fn place_expression_from_node(node: &AstNode) -> Option<PlaceExpression> {
    let NodeKind::ExpressionStatement(expression) = &node.kind else {
        return None;
    };

    place_expression_from_expression(expression)
}

/// Classify a receiver node's source state in a single traversal.
///
/// WHAT: walks the receiver node once to decide whether it is a non-place value or an existing
///       place and, for an existing place, its mutability and simple root binding name.
/// WHY: shared receiver-access validation for source methods, collection builtins and map
///      builtins consumes one classification instead of asking place and mutability separately
///      and walking the same receiver twice.
pub(crate) fn classify_receiver_source_state(
    node: &AstNode,
    path_fork: &PathInternerFork,
) -> ReceiverSourceState {
    let Some(place) = place_expression_from_node(node) else {
        return ReceiverSourceState::Temporary;
    };

    let binding_name = root_binding_name(&place, path_fork);
    if place_expression_is_mutable(&place) {
        ReceiverSourceState::MutablePlace { binding_name }
    } else {
        ReceiverSourceState::ImmutablePlace { binding_name }
    }
}

/// Resolve the simple root binding name of a place, if its root is a namable local.
///
/// WHAT: follows field projections down to their root local and returns its simple name.
/// WHY: immutable-place receiver diagnostics name the binding the author must declare mutable,
///      but only when the root is a simple named binding rather than an unnamed projection.
fn root_binding_name(place: &PlaceExpression, path_fork: &PathInternerFork) -> Option<StringId> {
    match &place.kind {
        PlaceExpressionKind::Local(path) => path_fork.component(*path),
        PlaceExpressionKind::Field { base, .. } => root_binding_name(base, path_fork),
    }
}

