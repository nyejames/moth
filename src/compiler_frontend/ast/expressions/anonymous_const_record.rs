//! Anonymous const-record expression construction.
//!
//! WHAT: converts shared parenthesised named arguments into the existing
//! [`ExpressionKind::AnonymousConstRecord`] operand.
//! WHY: shared call-argument parsing owns delimiters, separators and recursive values; this
//! module owns only record-field identity, duplicate/keyword validation and construction.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::call_argument::CallAccessMode;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentDiagnosticContext, CallArgumentNamingPolicy, CallArgumentReceivingContext,
    CallArgumentSyntax, CallArgumentSyntaxContext, CallArgumentValuePolicy,
    parse_call_arguments_with_receiving_context,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticToken};
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
/// value expressions. This helper only turns the retained named arguments into the existing
/// anonymous-record expression representation.
pub(super) fn parse_parenthesized_anonymous_const_record_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
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
            return Err(CompilerDiagnostic::unexpected_token_from_tag(
                DiagnosticToken::from_static_tag(TokenTag::MUTABLE),
                argument.marker_span.or(argument.span),
            )
            .into());
        }

        let field_id = path_fork
            .try_intern_child(PathId::ROOT, field_name)
            .expect("anonymous record field path table exhausted");
        fields.push(Declaration {
            id: field_id,
            value: argument.value,
            binding_span,
            config_qualifier: None,
        });
    }
    Ok(finish_record(fields, record_span, type_interner))
}

fn finish_record(
    fields: Vec<Declaration>,
    span: Option<SourceSpan>,
    type_interner: &AstTypeInterner<'_>,
) -> Expression {
    let record_type_id = type_interner.environment().anonymous_const_record_type();
    Expression::anonymous_const_record(fields, span, ValueMode::ImmutableOwned, record_type_id)
}

fn current_span(token_stream: &AstCursor) -> Option<SourceSpan> {
    // Record spans are token-local facts on the canonical cursor view.
    Some(token_stream.current_span())
}
