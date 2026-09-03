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
use crate::compiler_frontend::declaration_syntax::build_config_contract::{
    parse_build_config_qualifier, starts_build_config_qualifier,
};
use crate::compiler_frontend::symbols::identifier_policy::ensure_not_keyword_shadow_identifier;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

/// True when `|` at `pipe_index` opens a value record rather than a struct shell.
///
/// A compile-time receiving context (`#=`) treats every `|...|` initializer as a const
/// record, including empty and malformed lists. Ordinary `=` keeps empty `| |` and
/// `| name Type |` as struct shells, and only `| name =` / `| name ,` as records.
pub(crate) fn pipe_opens_value_record(
    tokens: &[Token],
    pipe_index: usize,
    compile_time: bool,
) -> bool {
    if compile_time {
        return true;
    }

    let kind_at = |cursor: usize| tokens.get(cursor).map(|token| &token.kind);
    let skip_newlines = |mut cursor: usize| {
        while matches!(kind_at(cursor), Some(TokenKind::Newline)) {
            cursor += 1;
        }
        cursor
    };

    let mut cursor = skip_newlines(pipe_index + 1);
    if matches!(kind_at(cursor), Some(TokenKind::TypeParameterBracket)) {
        return false;
    }

    if !matches!(kind_at(cursor), Some(TokenKind::Symbol(_))) {
        return false;
    }

    cursor = skip_newlines(cursor + 1);
    matches!(
        kind_at(cursor),
        Some(TokenKind::Assign) | Some(TokenKind::Comma)
    )
}

fn looks_like_nested_record_literal(tokens: &[Token], pipe_index: usize) -> bool {
    let kind_at = |cursor: usize| tokens.get(cursor).map(|token| &token.kind);
    let skip_newlines = |mut cursor: usize| {
        while matches!(kind_at(cursor), Some(TokenKind::Newline)) {
            cursor += 1;
        }
        cursor
    };

    let cursor = skip_newlines(pipe_index + 1);
    if matches!(kind_at(cursor), Some(TokenKind::TypeParameterBracket)) {
        return true;
    }

    pipe_opens_value_record(tokens, pipe_index, false)
}

/// Parse one anonymous const record from `| name = value, ... |` syntax.
///
/// ENTRY INVARIANT: the stream is positioned on the opening `|` and the receiving context is
/// compile-time (`Constant` or `ConstantHeader`).
/// EXIT INVARIANT: the stream is positioned on the token after the closing `|`.
///
/// WHAT: parses named, ordered, unique `field = expression` entries with an optional trailing
/// comma and returns the record expression. Nested `|...|` field values are rejected; declare
/// the child first and name it.
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

/// Parse one `name = value` or `name #Config of T = value` record field.
///
/// `#Config` is retained as declaration metadata on the field. It is deliberately not represented
/// as a type constructor or expression property: the compiler config service validates placement
/// and resolves the field to an ordinary primitive or optional expression.
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
    ensure_not_keyword_shadow_identifier(field_name, name_location.clone(), string_table)
        .map_err(ExpressionParseError::from)?;

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

    let mut qualifier = if starts_build_config_qualifier(token_stream, string_table) {
        Some(parse_build_config_qualifier(token_stream, string_table)?)
    } else {
        None
    };
    let has_initializer = token_stream.current_token_kind() == &TokenKind::Assign;
    if !has_initializer && qualifier.is_none() {
        // A field not followed by `=` is positional (`| a, b = 2 |`); report it through
        // the dedicated record-field reason instead of a generic `=` expectation.
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::AnonymousRecordFieldNotNamed,
            token_stream.current_location(),
        )
        .into());
    }

    let value = if has_initializer {
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
            if looks_like_nested_record_literal(&token_stream.tokens, token_stream.index) {
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

        // A bare `none` has no inferred option type. The qualifier carries the option contract,
        // so retain a sentinel and let the config resolver construct the typed OptionNone value.
        if token_stream.current_token_kind() == &TokenKind::NoneLiteral && qualifier.is_some() {
            let location = token_stream.current_location();
            token_stream.advance();
            if let Some(qualifier) = qualifier.as_mut() {
                qualifier.default_none = true;
            }
            Expression::no_value(
                location,
                crate::compiler_frontend::datatypes::DataType::Inferred,
                ValueMode::ImmutableOwned,
            )
        } else {
            parse_record_field_value(token_stream, context, type_interner, string_table)?
        }
    } else {
        // A qualified required field may omit its initializer so explicit inputs or builder
        // globals can satisfy it. Optional absence resolves to an ordinary OptionNone.
        let location = qualifier
            .as_ref()
            .map(|qualifier| qualifier.qualifier_location.clone())
            .unwrap_or_else(|| token_stream.current_location());
        Expression::no_value(
            location,
            crate::compiler_frontend::datatypes::DataType::Inferred,
            ValueMode::ImmutableOwned,
        )
    };

    fields.push(Declaration {
        id: InternedPath::from_components(vec![field_name]),
        value,
        config_qualifier: qualifier,
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
