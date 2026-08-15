//! Value-position namespace traversal parsing.
//!
//! WHAT: parses dotted namespace access such as `namespace.member`, `namespace.child.member`,
//! or `namespace.external.function()` in expression position.
//! WHY: external package surfaces are recursive, while source and module public-surface namespace
//! records remain shallow; this module walks the dotted path and reuses existing leaf parsers for
//! the final source or external member.
//! BOUNDARY: this module only handles value-position namespace access. Type-position traversal
//! is owned by the type-resolution stage and is explicitly out of scope here.

mod leaf_resolution;
mod traversal;

use leaf_resolution::{LeafDispatchContext, resolve_namespace_value_member};
use traversal::{NamespaceMemberLookup, lookup_namespace_member};

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFieldAccessReason, NamespaceTypeValueMisuseKind,
};
use crate::compiler_frontend::headers::binding_environment::{
    NamespaceRecord, NamespaceRecordSource,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Input bundle for namespace access parsing.
///
/// WHAT: carries everything needed to resolve a dotted path starting at a visible namespace
/// dependency namespace.
/// WHY: avoids threading a long argument list through the identifier dispatch path.
pub(super) struct NamespaceAccessInput<'a, 'env> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'env>,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    pub(super) allow_boundary_catch: bool,
    pub(super) expected_result_evidence_allowed: bool,
    pub(super) root_name: StringId,
    pub(super) root_record: &'a NamespaceRecord,
    pub(super) string_table: &'a mut StringTable,
}

/// Parse a dotted namespace access path in value position.
///
/// WHAT: walks `root.member.member...` while each intermediate member resolves to a child
/// namespace, then dispatches the final value member to the existing source or external
/// leaf parser.
/// WHY: namespace records are the binding stage's field-access-only view of dependencies; AST
/// must resolve the whole dotted path into ordinary declaration references or stable
/// external IDs before producing HIR, so no runtime namespace value ever exists.
/// BOUNDARY: source and module public-surface namespace records remain shallow, so any second dot
/// in a source or module public-surface path reports the existing `nested_traversal` diagnostic.
pub(super) fn parse_namespace_access(
    input: NamespaceAccessInput<'_, '_>,
) -> Result<(), ExpressionParseError> {
    let NamespaceAccessInput {
        token_stream,
        context,
        type_interner,
        expression,
        allow_boundary_catch,
        expected_result_evidence_allowed,
        root_name,
        root_record,
        string_table,
    } = input;

    token_stream.advance(); // move from namespace name to '.'
    let mut dot_location = token_stream.current_location();
    token_stream.advance(); // move from '.' to first member name

    let mut current_record = root_record;

    loop {
        let member_location = if matches!(token_stream.current_token_kind(), TokenKind::Eof) {
            dot_location.clone()
        } else {
            token_stream.current_location()
        };
        let TokenKind::Symbol(member_name) = token_stream.current_token_kind().to_owned() else {
            return Err(CompilerDiagnostic::invalid_field_access(
                InvalidFieldAccessReason::ExpectedNameAfterDot,
                None,
                None,
                Vec::new(),
                member_location,
            )
            .into());
        };

        let lookup = lookup_namespace_member(current_record, member_name);
        let has_following_dot = token_stream.peek_next_token() == Some(&TokenKind::Dot);

        // Source and module public-surface records are shallow. Any attempt to descend further
        // than one member must keep using the existing `nested_traversal` diagnostic, which the
        // existing integration fixture asserts.
        if has_following_dot
            && matches!(
                current_record.record_source,
                NamespaceRecordSource::SourceFile(_)
            )
        {
            return Err(CompilerDiagnostic::nested_dependency_traversal(
                root_name,
                member_location,
            )
            .into());
        }

        match lookup {
            NamespaceMemberLookup::ChildNamespace(child_record) => {
                if has_following_dot {
                    token_stream.advance(); // to '.'
                    dot_location = token_stream.current_location();
                    token_stream.advance(); // to next member name
                    current_record = child_record;
                    continue;
                }

                return Err(CompilerDiagnostic::namespace_type_value_misuse(
                    member_name,
                    NamespaceTypeValueMisuseKind::Value,
                    NamespaceTypeValueMisuseKind::Namespace,
                    member_location,
                )
                .into());
            }

            NamespaceMemberLookup::Value(value_member) => {
                if has_following_dot {
                    return Err(CompilerDiagnostic::namespace_type_value_misuse(
                        member_name,
                        NamespaceTypeValueMisuseKind::Namespace,
                        NamespaceTypeValueMisuseKind::Value,
                        member_location,
                    )
                    .into());
                }

                let mut leaf_context = LeafDispatchContext {
                    token_stream,
                    context,
                    type_interner,
                    expression,
                    allow_boundary_catch,
                    string_table,
                };

                return resolve_namespace_value_member(
                    &mut leaf_context,
                    value_member,
                    member_name,
                    member_location,
                    expected_result_evidence_allowed,
                );
            }

            NamespaceMemberLookup::Type => {
                let (expected, found) = if has_following_dot {
                    (
                        NamespaceTypeValueMisuseKind::Namespace,
                        NamespaceTypeValueMisuseKind::Type,
                    )
                } else {
                    (
                        NamespaceTypeValueMisuseKind::Value,
                        NamespaceTypeValueMisuseKind::Type,
                    )
                };

                return Err(CompilerDiagnostic::namespace_type_value_misuse(
                    member_name,
                    expected,
                    found,
                    member_location,
                )
                .into());
            }

            NamespaceMemberLookup::Missing => {
                return Err(
                    CompilerDiagnostic::unknown_value_name(member_name, member_location).into(),
                );
            }
        }
    }
}
