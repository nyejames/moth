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

use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use leaf_resolution::{LeafDispatchContext, resolve_namespace_value_member};
use traversal::{NamespaceMemberLookup, lookup_namespace_member};

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    InvalidFieldAccessReason, NamespaceMemberRole, NamespaceTypeValueMisuseKind,
};
use crate::compiler_frontend::headers::binding_environment::{
    NamespaceRecord, NamespaceRecordSource,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Input bundle for namespace access parsing.
///
/// WHAT: carries everything needed to resolve a dotted path starting at a visible namespace
/// dependency namespace.
/// WHY: avoids threading a long argument list through the identifier dispatch path.
pub(super) struct NamespaceAccessInput<'a, 'env, 'tokens> {
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'env>,
    pub(super) expression_builder: &'a mut AstExpressionBuilder,
    pub(super) const_values: &'a mut ConstValueStore,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    pub(super) allow_boundary_catch: bool,
    pub(super) expected_result_evidence_allowed: bool,
    pub(super) root_name: StringId,
    pub(super) root_record: &'a NamespaceRecord,
    pub(super) string_table: &'a mut StringTable,
    pub(super) path_fork: &'a mut PathInternerFork,
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
    input: NamespaceAccessInput<'_, '_, '_>,
) -> Result<(), ExpressionParseError> {
    let NamespaceAccessInput {
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        expression,
        allow_boundary_catch,
        expected_result_evidence_allowed,
        root_name,
        root_record,
        string_table,
        path_fork,
    } = input;

    token_stream.advance(); // move from namespace name to '.'
    let mut dot_span = Some(token_stream.current_span());
    token_stream.advance(); // move from '.' to first member name

    let mut current_record = root_record;
    let mut child_segments = Vec::new();

    loop {
        let member_span = if token_stream.current_tag() == TokenTag::EOF {
            dot_span
        } else {
            Some(token_stream.current_span())
        };
        if token_stream.current_tag() != TokenTag::SYMBOL {
            return Err(calls_data::invalid_field_access(
                InvalidFieldAccessReason::ExpectedNameAfterDot,
                None,
                None,
                Vec::new(),
                member_span,
            )
            .into());
        }
        let member_name = token_stream
            .current_string_id_in(string_table)?
            .ok_or_else(|| {
                crate::compiler_frontend::compiler_errors::InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "namespace member symbol had no string payload",
                )
            })?;

        let lookup = lookup_namespace_member(current_record, member_name);
        let has_following_dot = token_stream.peek_next_tag() == Some(TokenTag::DOT);

        // Source and module public-surface records are shallow. Any attempt to descend further
        // than one member must keep using the existing `nested_traversal` diagnostic, which the
        // existing integration fixture asserts.
        if has_following_dot
            && matches!(
                current_record.record_source,
                NamespaceRecordSource::SourceFile(_)
            )
        {
            return Err(project_config::nested_dependency_traversal(root_name, member_span).into());
        }

        match lookup {
            NamespaceMemberLookup::ChildNamespace(child_record) => {
                if has_following_dot {
                    token_stream.advance(); // to '.'
                    dot_span = Some(token_stream.current_span());
                    token_stream.advance(); // to next member name
                    current_record = child_record;
                    child_segments.push(member_name);
                    continue;
                }

                return Err(project_config::namespace_type_value_misuse(
                    member_name,
                    NamespaceTypeValueMisuseKind::Value,
                    NamespaceTypeValueMisuseKind::Namespace,
                    member_span,
                )
                .into());
            }

            NamespaceMemberLookup::Value(value_member) => {
                if has_following_dot {
                    return Err(project_config::namespace_type_value_misuse(
                        member_name,
                        NamespaceTypeValueMisuseKind::Namespace,
                        NamespaceTypeValueMisuseKind::Value,
                        member_span,
                    )
                    .into());
                }

                let mut leaf_context = LeafDispatchContext {
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    expression,
                    allow_boundary_catch,
                    string_table,
                    path_fork,
                };

                return resolve_namespace_value_member(
                    &mut leaf_context,
                    value_member,
                    member_name,
                    member_span,
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

                return Err(project_config::namespace_type_value_misuse(
                    member_name,
                    expected,
                    found,
                    member_span,
                )
                .into());
            }

            NamespaceMemberLookup::Missing => {
                let member_role = if has_following_dot {
                    NamespaceMemberRole::ChildNamespace
                } else {
                    NamespaceMemberRole::Value
                };
                return Err(names_declarations::unknown_namespace_member(
                    names_declarations::intern_dotted_namespace_path(
                        root_name,
                        &child_segments,
                        string_table,
                    ),
                    member_name,
                    current_record.kind(),
                    member_role,
                    current_record.member_names_for_role(member_role),
                    member_span,
                )
                .into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::external_packages::{
        ExternalConstantId, ExternalSymbolId, ExternalTypeId,
    };
    use crate::compiler_frontend::headers::binding_environment::{
        NamespaceTypeMember, NamespaceValueMember,
    };

    #[test]
    fn value_position_leaf_excludes_type_and_child_namespace_candidates() {
        let mut string_table = StringTable::new();
        let package = string_table.intern("@test");
        let value_name = string_table.intern("PI");
        let type_name = string_table.intern("Point");
        let child_name = string_table.intern("tools");
        let mut record = NamespaceRecord::empty(NamespaceRecordSource::ExternalPackage(package));
        record.value_members.insert(
            value_name,
            NamespaceValueMember::ExternalSymbol(ExternalSymbolId::Constant(ExternalConstantId(1))),
        );
        record.type_members.insert(
            type_name,
            NamespaceTypeMember::ExternalSymbol(ExternalSymbolId::Type(ExternalTypeId(2))),
        );
        record.child_namespaces.insert(
            child_name,
            NamespaceRecord::empty(NamespaceRecordSource::ExternalPackage(package)),
        );

        let candidates = record.member_names_for_role(NamespaceMemberRole::Value);
        assert!(candidates.contains(&value_name));
        assert!(
            !candidates.contains(&type_name),
            "final value members must not suggest types"
        );
        assert!(
            !candidates.contains(&child_name),
            "final value members must not suggest child namespaces"
        );
    }
}
