//! Anonymous const-record expression construction.
//!
//! WHAT: converts shared parenthesised named arguments into an ordinary folded record graph.
//! WHY: shared argument parsing owns syntax and receiving rules; this module owns field identity
//! and compile-time member-group classification. No runtime anonymous-record operation is created.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::binding::resolve_const_expression_content;
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueField, ConstValueMetadata, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::call_argument::CallAccessMode;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentDiagnosticContext, CallArgumentNamingPolicy, CallArgumentReceivingContext,
    CallArgumentSyntax, CallArgumentSyntaxContext, CallArgumentValuePolicy,
    parse_call_arguments_with_receiving_context,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, ConstValueKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::DiagnosticToken;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::identifier_policy::ensure_not_keyword_shadow_identifier;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;

/// Parse a parenthesised anonymous const record through the shared argument owner.
///
/// The shared parser owns the opening delimiter, separators, named-entry lookahead and recursive
/// value expressions. This helper commits field content directly to the constant graph.
pub(super) fn parse_parenthesized_anonymous_const_record_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<OwnedExpr, ExpressionParseError> {
    let record_span = current_span(token_stream);

    let receiving_context = CallArgumentReceivingContext::with_policies(
        CallArgumentDiagnosticContext::from_syntax(CallArgumentSyntax::AnonymousConstRecord)
            .with_opening_span(record_span),
        CallArgumentNamingPolicy::NamedOnly,
        CallArgumentValuePolicy::ConstRequired,
    );
    let arguments = parse_call_arguments_with_receiving_context(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        receiving_context,
        None,
        CallArgumentSyntaxContext::Ordinary,
        path_fork,
    )?;

    let mut fields = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let field_name = argument.target_param.ok_or_else(|| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "named-only anonymous record argument had no field name",
            )
        })?;
        let binding_span = argument.target_span;
        ensure_not_keyword_shadow_identifier(field_name, binding_span, string_table)
            .map_err(ExpressionParseError::from)?;

        if argument.access_mode != CallAccessMode::Shared {
            return Err(crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::unexpected_token_from_tag(DiagnosticToken::from_static_tag(TokenTag::MUTABLE),
            argument.marker_span.or(argument.span),)
            .into());
        }

        let field_id = path_fork
            .try_intern_child(PathId::ROOT, field_name)
            .expect("anonymous record field path table exhausted");
        let value = resolve_const_expression_content(
            argument.value.id(),
            expression_builder,
            const_values,
            type_interner.environment(),
            context,
            string_table,
        )?;
        fields.push(ConstValueField {
            name: field_id,
            value,
        });
    }
    finish_record(
        fields,
        record_span,
        type_interner,
        expression_builder,
        const_values,
    )
}

fn finish_record(
    fields: Vec<ConstValueField>,
    span: Option<SourceSpan>,
    type_interner: &AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> Result<OwnedExpr, ExpressionParseError> {
    let record_type_id = type_interner.environment().anonymous_const_record_type();
    let mut metadata =
        ConstValueMetadata::resolved(record_type_id, span, ValueMode::ImmutableOwned);
    metadata.const_record_state = ConstRecordState::ConstRecord;
    metadata.value_kind = ConstValueKind::Composite;
    metadata.hir_visible = false;
    let value = const_values.insert_value(metadata, ConstValuePayload::Record(fields))?;
    let expression =
        expression_builder.constant(value, record_type_id, span, ValueMode::ImmutableOwned)?;
    let mut input = expression_builder
        .view_owned(expression)?
        .to_completed_input(expression_builder)?;
    input.classification.const_record_state = ConstRecordState::ConstRecord;
    Ok(expression_builder.commit_replacement(input)?)
}

fn current_span(token_stream: &AstCursor) -> Option<SourceSpan> {
    // Record spans are token-local facts on the canonical cursor view.
    Some(token_stream.current_span())
}
