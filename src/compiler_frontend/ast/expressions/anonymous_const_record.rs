//! Anonymous const-record literal parsing.
//!
//! WHAT: parses `| name = value, ... |` record literals in expression position when the
//! receiving context requires a compile-time value, and produces one
//! [`ExpressionKind::AnonymousConstRecord`] operand.
//! WHY: this grammar owns `name = value` record fields only. Struct shells, choice payloads,
//! receiver signatures and function parameters keep their `field Type` owners in
//! `declaration_syntax`; runtime-position pipes report a deferred-feature diagnostic through
//! the expression dispatcher instead of entering this parser.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidExpressionReason};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::utilities::token_scan::pipe_opens_anonymous_record;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

/// Parse one anonymous const record from `| name = value, ... |` syntax.
///
/// ENTRY INVARIANT: the stream is positioned on the opening `|` and the receiving context is
/// compile-time (`Constant` or `ConstantHeader`).
/// EXIT INVARIANT: the stream is positioned on the token after the closing `|`.
///
/// WHAT: parses named, ordered, unique `field = expression` entries with an optional trailing
/// comma and returns the record expression. Nested `|...|` field values parse through the
/// ordinary expression parser in the same constant context.
/// WHY: this is the single anonymous-record grammar owner. Struct shells (`field Type`),
/// choice payloads and signature member lists keep their own parsers.
pub(super) fn parse_anonymous_const_record_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let record_location = token_stream.current_location();
    token_stream.advance(); // past the opening `|`

    let mut fields: Vec<Declaration> = Vec::new();
    let mut seen_field_names: FxHashMap<StringId, SourceLocation> = FxHashMap::default();

    // Empty record: `| |` (with optional authored newlines) is allowed.
    token_stream.skip_newlines();
    if token_stream.current_token_kind() == &TokenKind::TypeParameterBracket {
        token_stream.advance();
        return Ok(finish_record(Vec::new(), record_location, type_interner));
    }

    loop {
        // Record regions span authored lines, so blank lines between fields are layout.
        token_stream.skip_newlines();

        match token_stream.current_token_kind() {
            TokenKind::TypeParameterBracket => {
                token_stream.advance();
                break;
            }

            TokenKind::Eof => {
                return Err(unexpected_record_end(string_table, token_stream));
            }

            TokenKind::Symbol(field_name) => {
                parse_record_field(
                    *field_name,
                    token_stream,
                    context,
                    type_interner,
                    &mut fields,
                    &mut seen_field_names,
                    string_table,
                )?;
            }

            _ => {
                return Err(CompilerDiagnostic::invalid_expression(
                    InvalidExpressionReason::AnonymousRecordFieldNotNamed,
                    token_stream.current_location(),
                )
                .into());
            }
        }

        // ------------------------
        //  Field separator
        // ------------------------
        match token_stream.current_token_kind() {
            TokenKind::Comma => {
                token_stream.advance();
                token_stream.skip_newlines();

                // A trailing comma before the closing pipe is allowed.
                if token_stream.current_token_kind() == &TokenKind::TypeParameterBracket {
                    token_stream.advance();
                    break;
                }
            }

            TokenKind::TypeParameterBracket => {
                token_stream.advance();
                break;
            }

            TokenKind::Eof
            | TokenKind::CloseParenthesis
            | TokenKind::CloseCurly
            | TokenKind::TemplateClose => {
                return Err(unexpected_record_end(string_table, token_stream));
            }

            _ => {
                return Err(CompilerDiagnostic::expected_token(
                    TokenKind::Comma,
                    Some(token_stream.current_token_kind().to_owned()),
                    token_stream.current_location(),
                )
                .into());
            }
        }
    }

    Ok(finish_record(fields, record_location, type_interner))
}

/// Parse one `name = value` record field.
///
/// WHAT: reads the field name, requires the `=` separator, parses the value with ordinary
/// expression semantics, and rejects duplicate names through the shared duplicate
/// declaration diagnostic.
fn parse_record_field(
    field_name: StringId,
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    fields: &mut Vec<Declaration>,
    seen_field_names: &mut FxHashMap<StringId, SourceLocation>,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    let name_location = token_stream.current_location();

    if let Some(first_location) = seen_field_names.get(&field_name) {
        return Err(CompilerDiagnostic::duplicate_declaration(
            field_name,
            Some(first_location.to_owned()),
            name_location,
        )
        .into());
    }
    seen_field_names.insert(field_name, name_location);

    token_stream.advance(); // past the field name

    if token_stream.current_token_kind() != &TokenKind::Assign {
        // A field not followed by `=` is positional (`| a, b = 2 |`); report it through
        // the dedicated record-field reason instead of a generic `=` expectation.
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::AnonymousRecordFieldNotNamed,
            token_stream.current_location(),
        )
        .into());
    }
    token_stream.advance(); // past `=`
    token_stream.skip_newlines();

    if matches!(
        token_stream.current_token_kind(),
        TokenKind::Comma | TokenKind::Eof
    ) {
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::AnonymousRecordFieldNotNamed,
            token_stream.current_location(),
        )
        .into());
    }

    if token_stream.current_token_kind() == &TokenKind::TypeParameterBracket {
        if pipe_opens_anonymous_record(&token_stream.tokens, token_stream.index) {
            return Err(CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::NestedAnonymousConstRecord,
                token_stream.current_location(),
            )
            .into());
        }

        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::AnonymousRecordFieldNotNamed,
            token_stream.current_location(),
        )
        .into());
    }

    let value = parse_record_field_value(token_stream, context, type_interner, string_table)?;

    fields.push(Declaration {
        id: InternedPath::from_components(vec![field_name]),
        value,
    });

    Ok(())
}

/// Parse one field value expression in the surrounding constant context.
///
/// WHAT: parses the field initializer with ordinary expression semantics. Nested
/// `|...|` literals are rejected so each record is a single pipe-delimited region.
/// WHY: inner records are separate declarations; the field then names that binding.
fn parse_record_field_value(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let mut expected_type = ExpectedType::Infer;
    let mut field_context = context.clone();
    field_context.inside_anonymous_const_record = true;
    create_expression(
        token_stream,
        &field_context,
        type_interner,
        &mut expected_type,
        &ValueMode::ImmutableOwned,
        false,
        string_table,
    )
}

fn finish_record(
    fields: Vec<Declaration>,
    record_location: SourceLocation,
    type_interner: &AstTypeInterner<'_>,
) -> Expression {
    let record_type_id = type_interner.environment().anonymous_const_record_type();
    Expression::anonymous_const_record(
        fields,
        record_location,
        ValueMode::ImmutableOwned,
        record_type_id,
    )
}

fn unexpected_record_end(
    string_table: &mut StringTable,
    token_stream: &FileTokens,
) -> ExpressionParseError {
    CompilerDiagnostic::unexpected_end_of_file(
        Some(string_table.intern("|")),
        token_stream.current_location(),
    )
    .into()
}
