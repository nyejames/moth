//! Namespace leaf resolution for value-position access.
//!
//! WHAT: turns the final member of a dotted namespace path into an expression node.
//! WHY: terminal members dispatch to the same source-callable, source-reference,
//! external-function-call, and external-constant paths used by bare identifiers,
//! so namespace access does not duplicate call/constant lowering semantics.
//! BOUNDARY: this module only resolves value leaves; type leaves are reported as
//! misuse by the orchestration layer and never reach here.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::external_namespace_members::{
    ExternalNamespaceConstantMemberInput, ExternalNamespaceFunctionMemberInput,
    parse_external_namespace_constant_member, parse_external_namespace_function_member,
};
use crate::compiler_frontend::ast::expressions::parse_expression_dispatch::{
    ExpressionOperandInput, push_expression_operand_at_location,
};
use crate::compiler_frontend::ast::expressions::source_function_calls::{
    SourceCallableMemberInput, parse_source_callable_member,
};
use crate::compiler_frontend::ast::field_access::reference_expression_from_declaration;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::binding_environment::NamespaceValueMember;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};

/// Shared mutable state passed to leaf resolution helpers.
///
/// WHAT: bundles the token stream, scope context, type interner, expression buffer,
/// boundary-catch flag, and string table so each helper signature stays small.
/// WHY: every leaf resolver needs the same parser state; a context struct avoids
/// repeating the same long argument list in three functions.
pub(super) struct LeafDispatchContext<'a, 'env> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'env>,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    pub(super) allow_boundary_catch: bool,
    pub(super) string_table: &'a mut StringTable,
}

/// Resolve the final value member of a dotted namespace path.
///
/// WHAT: dispatches source declarations to the shared source callable/reference path
/// and external symbols to the shared external function/constant path.
/// WHY: namespace access must produce the same AST expression shapes as bare
/// identifiers so later stages see a single, consistent representation.
/// BOUNDARY: the caller has already verified the path ends at this value member;
/// this function only handles expression production for the leaf.
pub(super) fn resolve_namespace_value_member(
    context: &mut LeafDispatchContext<'_, '_>,
    value_member: &NamespaceValueMember,
    member_name: StringId,
    member_location: SourceLocation,
    expected_result_evidence_allowed: bool,
) -> Result<(), ExpressionParseError> {
    match value_member {
        NamespaceValueMember::SourceDeclaration(symbol_path) => resolve_source_value_member(
            context,
            symbol_path,
            member_name,
            member_location,
            expected_result_evidence_allowed,
        ),

        NamespaceValueMember::ExternalSymbol(symbol_id) => {
            resolve_external_value_member(context, *symbol_id, member_name, member_location)
        }
    }
}

/// Resolve a terminal source value member.
///
/// WHAT: emits either a source function call (if the declaration is callable) or a
/// declaration reference for the final dotted name.
/// WHY: source namespace members share the same call/reference logic as bare source
/// identifiers, so generic functions, non-generic calls, and value references behave
/// consistently.
/// BOUNDARY: the caller has already positioned `token_stream` on the member name and
/// has verified that no further dot follows; this function only handles the leaf.
fn resolve_source_value_member(
    context: &mut LeafDispatchContext<'_, '_>,
    symbol_path: &InternedPath,
    member_name: StringId,
    member_location: SourceLocation,
    expected_result_evidence_allowed: bool,
) -> Result<(), ExpressionParseError> {
    let LeafDispatchContext {
        token_stream,
        context,
        type_interner,
        expression,
        allow_boundary_catch,
        string_table,
    } = context;

    let Some(declaration) = context
        .shared
        .lookups
        .declaration_table
        .get_by_path(symbol_path)
    else {
        return Err(CompilerDiagnostic::unknown_value_name(member_name, member_location).into());
    };

    // Namespace fields are not first-class function values. A function member must
    // continue through the normal call parser so missing `(...)` reports at the call
    // boundary instead of lowering a `NoValue` declaration reference.
    if let Some(signature) = context.source_callable_signature(declaration) {
        let generic_template = context.lookup_generic_function_template(symbol_path);

        parse_source_callable_member(SourceCallableMemberInput {
            token_stream,
            function_path: symbol_path,
            signature,
            generic_template,
            visible_name: member_name,
            call_location: member_location.clone(),
            context,
            expression,
            allow_boundary_catch: *allow_boundary_catch,
            expected_result_evidence_allowed,
            type_interner,
            string_table,
        })
    } else {
        let reference_expression = reference_expression_from_declaration(
            declaration,
            context,
            type_interner,
            member_location.clone(),
        );
        token_stream.advance();

        push_expression_operand_at_location(
            token_stream,
            context,
            type_interner,
            string_table,
            expression,
            *allow_boundary_catch,
            ExpressionOperandInput {
                operand: reference_expression,
                wrapper_location: member_location,
            },
        )
    }
}

/// Resolve a terminal external value member.
///
/// WHAT: dispatches to the existing external function or external constant parsers
/// based on the symbol id stored in the namespace record.
/// WHY: external calls and constants are registered by id and share argument parsing,
/// boundary-catch handling, and constant-context restrictions with bare external
/// identifiers; reusing those parsers avoids duplicating that logic.
/// BOUNDARY: the caller has already verified the path ends at this member; this
/// function only handles the leaf expression production.
fn resolve_external_value_member(
    context: &mut LeafDispatchContext<'_, '_>,
    symbol_id: ExternalSymbolId,
    member_name: StringId,
    member_location: SourceLocation,
) -> Result<(), ExpressionParseError> {
    let LeafDispatchContext {
        token_stream,
        context,
        type_interner,
        expression,
        allow_boundary_catch,
        string_table,
    } = context;

    match symbol_id {
        ExternalSymbolId::Function(function_id) => {
            parse_external_namespace_function_member(ExternalNamespaceFunctionMemberInput {
                function_id,
                member_name,
                member_location,
                token_stream,
                context,
                type_interner,
                expression,
                allow_boundary_catch: *allow_boundary_catch,
                string_table,
            })
        }

        ExternalSymbolId::Constant(constant_id) => {
            parse_external_namespace_constant_member(ExternalNamespaceConstantMemberInput {
                constant_id,
                member_name,
                member_location,
                token_stream,
                context,
                type_interner,
                expression,
                allow_boundary_catch: *allow_boundary_catch,
                string_table,
            })
        }

        // The orchestration layer filters type symbols before calling the value leaf
        // resolver, so this branch is a proven internal invariant violation.
        ExternalSymbolId::Type(_) => {
            Err(CompilerDiagnostic::unknown_value_name(member_name, member_location).into())
        }
    }
}
