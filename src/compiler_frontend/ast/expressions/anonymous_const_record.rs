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
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticToken, InvalidExpressionReason,
};
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::declaration_syntax::build_config_contract::{
    parse_build_config_qualifier, starts_build_config_qualifier_at_cursor,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::identifier_policy::ensure_not_keyword_shadow_identifier;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

/// Cursor-view form of the shared struct-shell/record dispatch for expression-window scans.
///
/// WHAT: reads the same newline-tolerant `|`, `name`, `=`, `,` facts through the
/// canonical cursor view.
/// WHY: expression field-value checks need only token-local facts; keeping the slice helper
/// preserves the shared struct-shell grammar boundary owned with `declarations.rs`.
fn looks_like_nested_record_literal_at_cursor(
    cursor: &DeclarationCursor,
    pipe_index: usize,
) -> bool {
    let skip_newlines = |mut probe: usize| {
        while matches!(
            cursor.token_tag_at(probe),
            Some(TokenTag::NEWLINE)
        ) {
            probe += 1;
        }
        probe
    };

    let probe = skip_newlines(pipe_index + 1);
    if matches!(
        cursor.token_tag_at(probe),
        Some(TokenTag::TYPE_PARAMETER_BRACKET)
    ) {
        return true;
    }

    let mut probe = skip_newlines(pipe_index + 1);
    if matches!(
        cursor.token_tag_at(probe),
        Some(TokenTag::TYPE_PARAMETER_BRACKET)
    ) {
        return false;
    }
    if !matches!(
        cursor.token_tag_at(probe),
        Some(TokenTag::SYMBOL)
    ) {
        return false;
    }
    probe = skip_newlines(probe + 1);
    matches!(
        cursor.token_tag_at(probe),
        Some(TokenTag::ASSIGN) | Some(TokenTag::COMMA)
    )
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
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let record_span = current_span(token_stream);
    token_stream.advance(); // past the opening `|`

    let mut fields: Vec<Declaration> = Vec::new();
    let mut seen_field_names: FxHashMap<StringId, Option<SourceSpan>> = FxHashMap::default();

    // Empty record: `| |` (with optional authored newlines) is allowed.
    token_stream.skip_newlines();
    if token_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET {
        token_stream.advance();
        return Ok(finish_record(Vec::new(), record_span, type_interner));
    }

    loop {
        // Record regions span authored lines, so blank lines between fields are layout.
        token_stream.skip_newlines();

        match token_stream.current_tag() {
            TokenTag::TYPE_PARAMETER_BRACKET => {
                token_stream.advance();
                break;
            }

            TokenTag::EOF => {
                return Err(unexpected_record_end(string_table, token_stream));
            }

            TokenTag::SYMBOL => {
                let field_name = token_stream
                    .current_string_id_in(string_table)?
                    .ok_or_else(|| {
                        crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                            "anonymous record field symbol had no string payload",
                        )
                    })?;
                parse_record_field(
                    field_name,
                    token_stream,
                    context,
                    type_interner,
                    &mut fields,
                    &mut seen_field_names,
                    string_table,
                    path_fork,
                )?;
            }

            _ => {
                return Err(CompilerDiagnostic::invalid_expression(
                    InvalidExpressionReason::AnonymousRecordFieldNotNamed,
                    current_span(token_stream),
                )
                .into());
            }
        }

        // ------------------------
        //  Field separator
        // ------------------------
        match token_stream.current_tag() {
            TokenTag::COMMA => {
                token_stream.advance();
                token_stream.skip_newlines();

                // A trailing comma before the closing pipe is allowed.
                if token_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET {
                    token_stream.advance();
                    break;
                }
            }

            TokenTag::TYPE_PARAMETER_BRACKET => {
                token_stream.advance();
                break;
            }

            TokenTag::EOF
            | TokenTag::CLOSE_PARENTHESIS
            | TokenTag::CLOSE_CURLY
            | TokenTag::TEMPLATE_CLOSE => {
                return Err(unexpected_record_end(string_table, token_stream));
            }

            _ => {
                let found = match token_stream.current() {
                    Some(found) => Some(DiagnosticToken::try_from_token_ref(found).map_err(
                        |error| {
                            crate::compiler_frontend::compiler_messages::CompilerDiagnostic::token_view_invariant_error(
                                error,
                                "anonymous-record separator diagnostic",
                            )
                        },
                    )?),
                    None => Some(DiagnosticToken::from_static_tag(token_stream.current_tag())),
                };
                return Err(CompilerDiagnostic::expected_token_from_tags(
                    TokenTag::COMMA,
                    found,
                    current_span(token_stream),
                )
                .into());
            }
        }
    }

    Ok(finish_record(fields, record_span, type_interner))
}

/// Parse one `name = value` or `name #Config of T = value` record field.
///
/// `#Config` is retained as declaration metadata on the field. It is deliberately not represented
/// as a type constructor or expression property: the compiler config service validates placement
/// and resolves the field to an ordinary primitive or optional expression.
#[allow(
    clippy::too_many_arguments,
    reason = "record-field parsing keeps the field name, token stream, scope, mutable field/duplicate-tracking/interner/string/path state as separate borrows"
)]
fn parse_record_field(
    field_name: StringId,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    fields: &mut Vec<Declaration>,
    seen_field_names: &mut FxHashMap<StringId, Option<SourceSpan>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let binding_span = current_span(token_stream);
    ensure_not_keyword_shadow_identifier(field_name, binding_span, string_table)
        .map_err(ExpressionParseError::from)?;

    if let Some(first_span) = seen_field_names.get(&field_name) {
        return Err(CompilerDiagnostic::duplicate_declaration(
            field_name,
            *first_span,
            binding_span,
        )
        .into());
    }
    seen_field_names.insert(field_name, binding_span);
    token_stream.advance(); // past the field name

    let mut qualifier = {
        let parsed_qualifier = {
            let mut declaration_cursor = token_stream.declaration_cursor()?;
            if !starts_build_config_qualifier_at_cursor(&declaration_cursor, string_table)? {
                None
            } else {
                let qualifier =
                    parse_build_config_qualifier(&mut declaration_cursor, string_table, None)?;
                Some((qualifier, declaration_cursor.position()))
            }
        };
        if let Some((qualifier, next_index)) = parsed_qualifier {
            token_stream.set_position(next_index)?;
            Some(qualifier)
        } else {
            None
        }
    };
    let has_initializer = token_stream.current_tag() == TokenTag::ASSIGN;
    if !has_initializer && qualifier.is_none() {
        // A field not followed by `=` is positional (`| a, b = 2 |`); report it through
        // the dedicated record-field reason instead of a generic `=` expectation.
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::AnonymousRecordFieldNotNamed,
            current_span(token_stream),
        )
        .into());
    }

    let value = if has_initializer {
        token_stream.advance(); // past `=`
        token_stream.skip_newlines();

        if matches!(
            token_stream.current_tag(),
            TokenTag::COMMA | TokenTag::EOF
        ) {
            return Err(CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::AnonymousRecordFieldNotNamed,
                current_span(token_stream),
            )
            .into());
        }

        if token_stream.current_tag() == TokenTag::TYPE_PARAMETER_BRACKET {
            // Nested `|...|` values are rejected; the nested-record fact is token-local
            // lookahead through the canonical cursor position.
            let nested = token_stream
                .declaration_cursor()
                .map(|cursor| {
                    looks_like_nested_record_literal_at_cursor(&cursor, cursor.position())
                })
                .unwrap_or(false);
            if nested {
                return Err(CompilerDiagnostic::invalid_expression(
                    InvalidExpressionReason::NestedAnonymousConstRecord,
                    current_span(token_stream),
                )
                .into());
            }

            return Err(CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::AnonymousRecordFieldNotNamed,
                current_span(token_stream),
            )
            .into());
        }

        // A bare `none` has no inferred option type. The qualifier carries the option contract,
        // so retain a sentinel and let the config resolver construct the typed OptionNone value.
        if token_stream.current_tag() == TokenTag::NONE_LITERAL && qualifier.is_some() {
            let span = current_span(token_stream);
            token_stream.advance();
            if let Some(qualifier) = qualifier.as_mut() {
                qualifier.default_none = true;
            }
            Expression::no_value(
                span,
                crate::compiler_frontend::datatypes::DataType::Inferred,
                ValueMode::ImmutableOwned,
            )
        } else {
            parse_record_field_value(
                token_stream,
                context,
                type_interner,
                string_table,
                path_fork,
            )?
        }
    } else {
        // A qualified required field may omit its initializer so explicit inputs or builder
        // globals can satisfy it. Optional absence resolves to an ordinary OptionNone.
        let span = qualifier
            .as_ref()
            .and_then(|qualifier| qualifier.qualifier_span)
            .or_else(|| current_span(token_stream));
        Expression::no_value(
            span,
            crate::compiler_frontend::datatypes::DataType::Inferred,
            ValueMode::ImmutableOwned,
        )
    };

    fields.push(Declaration {
        id: path_fork
            .try_intern_child(PathId::ROOT, field_name)
            .expect("anonymous record field path table exhausted"),
        value,
        binding_span,
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
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
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
        path_fork,
    )
}

fn finish_record(
    fields: Vec<Declaration>,
    span: Option<SourceSpan>,
    type_interner: &AstTypeInterner<'_>,
) -> Expression {
    let record_type_id = type_interner.environment().anonymous_const_record_type();
    Expression::anonymous_const_record(fields, span, ValueMode::ImmutableOwned, record_type_id)
}

fn unexpected_record_end(
    string_table: &mut StringTable,
    token_stream: &AstCursor,
) -> ExpressionParseError {
    CompilerDiagnostic::unexpected_end_of_file(
        Some(string_table.intern("|")),
        current_span(token_stream),
    )
    .into()
}

fn current_span(token_stream: &AstCursor) -> Option<SourceSpan> {
    // Record spans are token-local facts on the canonical cursor view.
    Some(token_stream.current_span())
}
