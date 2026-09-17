//! Shared loop-header parsing for AST statement and template parsers.
//!
//! WHAT: parses the three body-independent loop header forms after the `loop`
//! keyword has been consumed: conditional, numeric range, and collection iteration.
//! WHY: statement loops and template loop suffixes need the same syntax, binding,
//! and type-validation rules, while each caller owns its own body parsing.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{
    Declaration, LoopBindings, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression_until, create_expression_without_boundary_catch,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::statements::condition_validation::ensure_loop_condition;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidLoopHeaderReason, RangeOperandKind,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FilePathSyntax, FileTokens, Token, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::utilities::token_scan::{ExpressionBoundaryDepth, NestingDepth};
use crate::compiler_frontend::value_mode::ValueMode;

#[derive(Debug, Clone)]
struct ParsedBindingName {
    id: StringId,
    span: Option<SourceSpan>,
}

#[derive(Debug, Clone)]
struct ParsedBindingNames {
    item: ParsedBindingName,
    index: Option<ParsedBindingName>,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ParsedLoopHeader {
    Conditional {
        condition: Expression,
    },
    Range {
        bindings: LoopBindings,
        range: RangeLoopSpec,
    },
    Collection {
        bindings: LoopBindings,
        iterable: Expression,
    },
}

#[derive(Debug, Clone)]
struct BindingSuffixSplit {
    core_tokens: Vec<Token>,
    bindings: ParsedBindingNames,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BareLoopBindingKind {
    Single,
    Dual,
}

#[derive(Debug, Clone)]
struct BareLoopBindingSuffix {
    core_tokens: Vec<Token>,
    span: Option<SourceSpan>,
    kind: BareLoopBindingKind,
}

#[derive(Debug, Clone)]
struct CursorBindingSuffixSplit {
    core_end: usize,
    bindings: ParsedBindingNames,
}

#[derive(Debug, Clone)]
struct CursorBareLoopBindingSuffix {
    core_end: usize,
    span: Option<SourceSpan>,
    kind: BareLoopBindingKind,
}

struct LoopHeaderParser<'a, 'types> {
    scope_context: &'a mut ScopeContext,
    type_interner: &'a mut AstTypeInterner<'types>,
    warnings: &'a mut Vec<CompilerDiagnostic>,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
}

/// Loop-header parsing preserves the AST body error lane. Canonical headers use bounded cursor
/// windows; compatibility headers use temporary streams over the frozen file-owned path table.
type LoopHeaderResult<T> = Result<T, ExpressionParseError>;

fn loop_header_error<T>(
    reason: InvalidLoopHeaderReason,
    span: Option<SourceSpan>,
) -> LoopHeaderResult<T> {
    Err(CompilerDiagnostic::invalid_loop_header(reason, span).into())
}

/// Compatibility-only parser for an already-materialised loop-header token stream.
///
/// Canonical callers must use [`parse_loop_header_cursor`] so source-owned tokens stay borrowed.
pub(crate) fn parse_loop_header_tokens(
    header_tokens: &[Token],
    path_syntax: &FilePathSyntax,
    mut context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LoopHeaderResult<(ParsedLoopHeader, ScopeContext)> {
    let source_id = context.shared.declaring_file_id;
    let mut header_tokens = header_tokens.to_vec();
    trim_edge_newlines(&mut header_tokens);
    reject_removed_in_loop_syntax(&header_tokens, string_table, source_id)?;
    let loop_header = {
        let mut parser = LoopHeaderParser {
            scope_context: &mut context,
            type_interner,
            warnings,
            string_table,
            path_fork,
        };

        // Range markers are syntax-defining for loop kind, so we dispatch range parsing first.
        // Non-range headers are then resolved as either conditional (`Bool`) or collection loops.
        if has_top_level_range_marker(&header_tokens) {
            parse_range_loop_header(&header_tokens, path_syntax, &mut parser)?
        } else {
            parse_non_range_loop_header(&header_tokens, path_syntax, &mut parser)?
        }
    };

    Ok((loop_header, context))
}

/// Parse a canonical loop header directly from its bounded parser window.
///
/// WHAT: keeps all canonical header reads on the short-lived `AstCursor` view and represents
/// grammar splits as parser-position ranges rather than materialised `Token` vectors.
/// WHY: the compatibility vector parser remains only for synthetic/remapped streams; canonical
/// source headers must not cross back into a `FileTokens` grammar adapter.
pub(crate) fn parse_loop_header_cursor(
    token_stream: &mut AstCursor,
    mut context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LoopHeaderResult<(ParsedLoopHeader, ScopeContext)> {
    let header_start = trim_cursor_header_start(token_stream, token_stream.position());
    let header_end = trim_cursor_header_end(token_stream, header_start, token_stream.length());

    if header_start >= header_end {
        return loop_header_error(
            InvalidLoopHeaderReason::ExpectedHeaderExpression,
            token_stream.span_at(header_start),
        );
    }

    token_stream
        .set_position(header_start)
        .map_err(ExpressionParseError::from)?;
    reject_removed_in_loop_syntax_cursor(token_stream, header_start, header_end, string_table)?;

    let loop_header = {
        let mut parser = LoopHeaderParser {
            scope_context: &mut context,
            type_interner,
            warnings,
            string_table,
            path_fork,
        };

        if has_top_level_range_marker_cursor(token_stream, header_start, header_end) {
            parse_range_loop_header_cursor(token_stream, header_start, header_end, &mut parser)?
        } else {
            parse_non_range_loop_header_cursor(token_stream, header_start, header_end, &mut parser)?
        }
    };

    Ok((loop_header, context))
}

fn parse_range_loop_header_cursor(
    token_stream: &mut AstCursor,
    header_start: usize,
    header_end: usize,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<ParsedLoopHeader> {
    if let Some(pipe_binding_split) =
        parse_pipe_binding_suffix_cursor(token_stream, header_start, header_end)?
    {
        let range = parse_range_loop_spec_cursor(
            token_stream,
            header_start,
            pipe_binding_split.core_end,
            parser.scope_context,
            parser.type_interner,
            parser.string_table,
            parser.path_fork,
        )?;
        let binding_type = range_binding_type(&range, parser.type_interner.environment())?;
        let bindings =
            declare_loop_bindings(Some(pipe_binding_split.bindings), binding_type, parser)?;

        return Ok(ParsedLoopHeader::Range { bindings, range });
    }

    if let Some(bare_binding_suffix) =
        detect_bare_loop_binding_suffix_cursor(token_stream, header_start, header_end)
        && parses_as_range_cursor(
            token_stream,
            header_start,
            bare_binding_suffix.core_end,
            parser.scope_context,
            parser.type_interner,
            parser.string_table,
            parser.path_fork,
        )
    {
        return bare_loop_binding_syntax_error_cursor(&bare_binding_suffix);
    }

    let range = parse_range_loop_spec_cursor(
        token_stream,
        header_start,
        header_end,
        parser.scope_context,
        parser.type_interner,
        parser.string_table,
        parser.path_fork,
    )?;
    let binding_type = range_binding_type(&range, parser.type_interner.environment())?;
    let bindings = declare_loop_bindings(None, binding_type, parser)?;
    Ok(ParsedLoopHeader::Range { bindings, range })
}

fn parse_non_range_loop_header_cursor(
    token_stream: &mut AstCursor,
    header_start: usize,
    header_end: usize,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<ParsedLoopHeader> {
    if let Some(pipe_binding_split) =
        parse_pipe_binding_suffix_cursor(token_stream, header_start, header_end)?
    {
        let (iterable, item_type) = parse_collection_iterable_cursor(
            token_stream,
            header_start,
            pipe_binding_split.core_end,
            parser.scope_context,
            parser.type_interner,
            parser.string_table,
            parser.path_fork,
        )?;
        let bindings = declare_loop_bindings(Some(pipe_binding_split.bindings), item_type, parser)?;

        return Ok(ParsedLoopHeader::Collection { bindings, iterable });
    }

    if let Some(bare_binding_suffix) =
        detect_bare_loop_binding_suffix_cursor(token_stream, header_start, header_end)
        && parses_as_collection_cursor(
            token_stream,
            header_start,
            bare_binding_suffix.core_end,
            parser.scope_context,
            parser.type_interner,
            parser.string_table,
            parser.path_fork,
        )
    {
        return bare_loop_binding_syntax_error_cursor(&bare_binding_suffix);
    }

    let expression = parse_cursor_expression(
        token_stream,
        header_start,
        header_end,
        parser.scope_context,
        parser.type_interner,
        &ValueMode::ImmutableOwned,
        parser.string_table,
        parser.path_fork,
    )?;

    let item_type_id = parser
        .type_interner
        .environment()
        .collection_element_type(expression.type_id);

    if let Some(item_type_id) = item_type_id {
        let bindings = declare_loop_bindings(None, item_type_id, parser)?;
        return Ok(ParsedLoopHeader::Collection {
            bindings,
            iterable: expression,
        });
    }

    ensure_loop_condition(&expression, parser.type_interner.environment())?;
    Ok(ParsedLoopHeader::Conditional {
        condition: expression,
    })
}

fn reject_removed_in_loop_syntax_cursor(
    token_stream: &AstCursor,
    header_start: usize,
    header_end: usize,
    string_table: &StringTable,
) -> LoopHeaderResult<()> {
    if header_end.saturating_sub(header_start) < 3 {
        return Ok(());
    }

    let Some(TokenKind::Symbol(_)) = token_stream.token_kind_at(header_start) else {
        return Ok(());
    };
    let Some(TokenKind::Symbol(second_symbol)) = token_stream.token_kind_at(header_start + 1)
    else {
        return Ok(());
    };

    if string_table.resolve(second_symbol) != "in" {
        return Ok(());
    }

    loop_header_error(
        InvalidLoopHeaderReason::RemovedInSyntax,
        token_stream.span_at(header_start + 1),
    )
}

fn parse_pipe_binding_suffix_cursor(
    token_stream: &AstCursor,
    header_start: usize,
    header_end: usize,
) -> LoopHeaderResult<Option<CursorBindingSuffixSplit>> {
    let pipe_indices =
        collect_top_level_cursor_indexes(token_stream, header_start, header_end, |kind| {
            matches!(kind, TokenKind::TypeParameterBracket)
        });
    if pipe_indices.is_empty() {
        return Ok(None);
    }

    let last_index = header_end.checked_sub(1);
    if last_index
        .and_then(|index| token_stream.token_kind_at(index))
        .is_none_or(|kind| !matches!(kind, TokenKind::TypeParameterBracket))
    {
        return loop_header_error(
            InvalidLoopHeaderReason::MissingClosingPipe,
            last_index.and_then(|index| token_stream.span_at(index)),
        );
    }

    if pipe_indices.len() != 2 {
        return loop_header_error(
            InvalidLoopHeaderReason::MalformedBindingPipes,
            token_stream.span_at(pipe_indices[0]),
        );
    }

    let open_pipe_index = pipe_indices[0];
    let close_pipe_index = pipe_indices[1];
    if close_pipe_index <= open_pipe_index {
        return loop_header_error(
            InvalidLoopHeaderReason::MalformedBindingPipes,
            token_stream.span_at(open_pipe_index),
        );
    }

    if open_pipe_index == header_start {
        return loop_header_error(
            InvalidLoopHeaderReason::MissingSourceBeforeBindings,
            token_stream.span_at(open_pipe_index),
        );
    }

    let bindings = parse_binding_cursor(token_stream, open_pipe_index + 1, close_pipe_index)?;

    Ok(Some(CursorBindingSuffixSplit {
        core_end: open_pipe_index,
        bindings,
    }))
}

fn parse_binding_cursor(
    token_stream: &AstCursor,
    binding_start: usize,
    binding_end: usize,
) -> LoopHeaderResult<ParsedBindingNames> {
    let binding_indices = (binding_start..binding_end)
        .filter(|index| {
            token_stream
                .token_kind_at(*index)
                .is_some_and(|kind| kind != TokenKind::Newline)
        })
        .collect::<Vec<_>>();
    if binding_indices.is_empty() {
        return loop_header_error(
            InvalidLoopHeaderReason::EmptyBindingList,
            token_stream.span_at(binding_start),
        );
    }

    let mut binding_names = Vec::with_capacity(2);
    let mut position = 0;
    while position < binding_indices.len() {
        let token_index = binding_indices[position];
        let token_kind = token_stream
            .token_kind_at(token_index)
            .expect("binding index was collected from a readable cursor token");
        if token_kind == TokenKind::This {
            return loop_header_error(
                InvalidLoopHeaderReason::ThisBinding,
                token_stream.span_at(token_index),
            );
        }
        let TokenKind::Symbol(symbol_id) = token_kind else {
            return loop_header_error(
                InvalidLoopHeaderReason::BindingMustBeSymbol,
                token_stream.span_at(token_index),
            );
        };

        binding_names.push(ParsedBindingName {
            id: symbol_id,
            span: token_stream.span_at(token_index),
        });
        position += 1;

        if position >= binding_indices.len() {
            break;
        }
        let separator_index = binding_indices[position];
        if token_stream.token_kind_at(separator_index) != Some(TokenKind::Comma) {
            return loop_header_error(
                InvalidLoopHeaderReason::MissingBindingComma,
                token_stream.span_at(separator_index),
            );
        }
        position += 1;
        if position >= binding_indices.len() {
            return loop_header_error(
                InvalidLoopHeaderReason::TrailingBindingComma,
                token_stream.span_at(separator_index),
            );
        }
    }

    build_binding_name_pair(binding_names)
}

fn detect_bare_loop_binding_suffix_cursor(
    token_stream: &AstCursor,
    header_start: usize,
    header_end: usize,
) -> Option<CursorBareLoopBindingSuffix> {
    let non_newline_indices =
        collect_top_level_cursor_indexes(token_stream, header_start, header_end, |kind| {
            !matches!(kind, TokenKind::Newline)
        });
    if non_newline_indices.len() < 2 {
        return None;
    }

    if non_newline_indices.len() >= 3 {
        let first_index = non_newline_indices[non_newline_indices.len() - 3];
        let separator_index = non_newline_indices[non_newline_indices.len() - 2];
        let second_index = non_newline_indices[non_newline_indices.len() - 1];
        let first_kind = token_stream.token_kind_at(first_index);
        let separator_kind = token_stream.token_kind_at(separator_index);
        let second_kind = token_stream.token_kind_at(second_index);

        if matches!(first_kind, Some(TokenKind::Symbol(_)))
            && separator_kind == Some(TokenKind::Comma)
            && matches!(second_kind, Some(TokenKind::Symbol(_)))
            && first_index > header_start
        {
            return Some(CursorBareLoopBindingSuffix {
                core_end: first_index,
                span: token_stream.span_at(first_index),
                kind: BareLoopBindingKind::Dual,
            });
        }

        if matches!(first_kind, Some(TokenKind::Symbol(_)))
            && matches!(separator_kind, Some(TokenKind::Symbol(_)))
            && matches!(second_kind, Some(TokenKind::Symbol(_)))
            && separator_index > header_start
        {
            return Some(CursorBareLoopBindingSuffix {
                core_end: separator_index,
                span: token_stream.span_at(separator_index),
                kind: BareLoopBindingKind::Dual,
            });
        }
    }

    let binding_index = *non_newline_indices.last()?;
    let core_tail_index = non_newline_indices[non_newline_indices.len() - 2];
    if matches!(
        token_stream.token_kind_at(binding_index),
        Some(TokenKind::Symbol(_))
    ) && token_stream.token_kind_at(core_tail_index) != Some(TokenKind::Comma)
    {
        return Some(CursorBareLoopBindingSuffix {
            core_end: binding_index,
            span: token_stream.span_at(binding_index),
            kind: BareLoopBindingKind::Single,
        });
    }

    None
}

fn bare_loop_binding_syntax_error_cursor<T>(
    binding_suffix: &CursorBareLoopBindingSuffix,
) -> LoopHeaderResult<T> {
    match binding_suffix.kind {
        BareLoopBindingKind::Single => loop_header_error(
            InvalidLoopHeaderReason::BareSingleBinding,
            binding_suffix.span,
        ),
        BareLoopBindingKind::Dual => loop_header_error(
            InvalidLoopHeaderReason::BareDualBinding,
            binding_suffix.span,
        ),
    }
}

fn parse_range_loop_spec_cursor(
    token_stream: &mut AstCursor,
    range_start: usize,
    range_end: usize,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LoopHeaderResult<RangeLoopSpec> {
    let saved_position = token_stream.position();
    let result = (|| {
        token_stream
            .set_position(range_start)
            .map_err(ExpressionParseError::from)?;

        let start = if token_stream.current_token_kind() == &TokenKind::ExclusiveRange {
            let span = Some(token_stream.current_span());
            Expression::new(
                ExpressionKind::Int(0),
                span,
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            )
        } else {
            parse_cursor_expression_until(
                token_stream,
                range_start,
                range_end,
                context,
                type_interner,
                &ValueMode::ImmutableReference,
                string_table,
                path_fork,
                &[TokenKind::ExclusiveRange],
            )?
        };

        let end_kind = match token_stream.current_token_kind() {
            TokenKind::ExclusiveRange => {
                token_stream.advance();
                if token_stream.current_token_kind() == &TokenKind::Ampersand {
                    token_stream.advance();
                    RangeEndKind::Inclusive
                } else {
                    RangeEndKind::Exclusive
                }
            }
            TokenKind::Eof if token_stream.position() >= range_end => {
                return loop_header_error(
                    InvalidLoopHeaderReason::MissingRangeSeparator,
                    start.span,
                );
            }
            _ => {
                return loop_header_error(
                    InvalidLoopHeaderReason::MissingRangeSeparator,
                    Some(token_stream.current_span()),
                );
            }
        };

        if token_stream.position() >= range_end
            || token_stream.current_token_kind() == &TokenKind::Eof
        {
            return loop_header_error(InvalidLoopHeaderReason::MissingRangeEndBound, start.span);
        }

        let end_start = token_stream.position();
        let by_index =
            find_top_level_expression_boundary_cursor_token(token_stream, end_start, range_end);
        let end = if by_index.is_some() {
            parse_cursor_expression_until(
                token_stream,
                end_start,
                range_end,
                context,
                type_interner,
                &ValueMode::ImmutableReference,
                string_table,
                path_fork,
                &[TokenKind::By],
            )?
        } else {
            parse_cursor_expression(
                token_stream,
                end_start,
                range_end,
                context,
                type_interner,
                &ValueMode::ImmutableReference,
                string_table,
                path_fork,
            )?
        };

        let step = if by_index.is_some() {
            let by_span = Some(token_stream.current_span());
            token_stream.advance();
            if token_stream.position() >= range_end
                || token_stream.current_token_kind() == &TokenKind::Eof
            {
                return loop_header_error(InvalidLoopHeaderReason::MissingRangeStep, by_span);
            }
            Some(parse_cursor_expression(
                token_stream,
                token_stream.position(),
                range_end,
                context,
                type_interner,
                &ValueMode::ImmutableReference,
                string_table,
                path_fork,
            )?)
        } else {
            None
        };

        let type_environment = type_interner.environment();
        let is_start_numeric = is_numeric_type_id(start.type_id, type_environment);
        let is_end_numeric = is_numeric_type_id(end.type_id, type_environment);
        let is_step_numeric = step
            .as_ref()
            .map(|expression| is_numeric_type_id(expression.type_id, type_environment));

        if !is_start_numeric {
            return Err(CompilerDiagnostic::invalid_range_operand(
                RangeOperandKind::Start,
                start.type_id,
                start.span,
            )
            .into());
        }
        if !is_end_numeric {
            return Err(CompilerDiagnostic::invalid_range_operand(
                RangeOperandKind::End,
                end.type_id,
                end.span,
            )
            .into());
        }
        if let Some(step_expression) = step.as_ref().filter(|_| is_step_numeric == Some(false)) {
            return Err(CompilerDiagnostic::invalid_range_operand(
                RangeOperandKind::Step,
                step_expression.type_id,
                step_expression.span,
            )
            .into());
        }

        let uses_float = start.type_id == type_environment.builtins().float
            || end.type_id == type_environment.builtins().float
            || step
                .as_ref()
                .is_some_and(|expression| expression.type_id == type_environment.builtins().float);
        if uses_float && step.is_none() {
            return loop_header_error(InvalidLoopHeaderReason::FloatRangeMissingStep, end.span);
        }
        if let Some(step_expression) = &step
            && is_zero_numeric_literal(step_expression)
        {
            return loop_header_error(InvalidLoopHeaderReason::ZeroRangeStep, step_expression.span);
        }

        Ok(RangeLoopSpec {
            start,
            end,
            end_kind,
            step,
        })
    })();

    token_stream
        .set_position(saved_position)
        .map_err(ExpressionParseError::from)?;
    result
}

fn parse_cursor_expression(
    token_stream: &mut AstCursor,
    expression_start: usize,
    expression_end: usize,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LoopHeaderResult<Expression> {
    if expression_start >= expression_end {
        return loop_header_error(InvalidLoopHeaderReason::ExpectedHeaderExpression, None);
    }

    parse_cursor_expression_window(token_stream, expression_start, expression_end, |stream| {
        let mut expected_type = ExpectedType::Infer;
        create_expression_without_boundary_catch(
            stream,
            context,
            type_interner,
            &mut expected_type,
            value_mode,
            false,
            string_table,
            path_fork,
        )
        .map_err(ExpressionParseError::from)
    })
}

fn parse_cursor_expression_until(
    token_stream: &mut AstCursor,
    expression_start: usize,
    expression_end: usize,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    stop_tokens: &[TokenKind],
) -> LoopHeaderResult<Expression> {
    if expression_start >= expression_end {
        return loop_header_error(InvalidLoopHeaderReason::ExpectedHeaderExpression, None);
    }

    parse_cursor_expression_window(token_stream, expression_start, expression_end, |stream| {
        let mut expected_type = ExpectedType::Infer;
        let mut cast_target_context = CastTargetContext::None;
        let input = ExpressionParseInput::without_boundary_catch(
            ExpressionParseResources {
                token_stream: stream,
                scope_context: context,
                type_interner,
                expected_type: &mut expected_type,
                cast_target_context: &mut cast_target_context,
                value_mode,
                string_table,
                path_fork,
            },
            false,
        );
        create_expression_until(input, stop_tokens).map_err(ExpressionParseError::from)
    })
}

fn parse_cursor_expression_window<T>(
    token_stream: &mut AstCursor,
    expression_start: usize,
    expression_end: usize,
    parse: impl FnOnce(&mut AstCursor) -> LoopHeaderResult<T>,
) -> LoopHeaderResult<T> {
    let previous_limit = token_stream
        .set_limit(expression_end)
        .map_err(ExpressionParseError::from)?;
    let result = token_stream
        .set_position(expression_start)
        .map_err(ExpressionParseError::from)
        .and_then(|_| parse(token_stream));
    let final_position = token_stream.position();
    token_stream.restore_limit(previous_limit);
    token_stream
        .set_position(final_position)
        .map_err(ExpressionParseError::from)?;
    result
}

fn parse_collection_iterable_cursor(
    token_stream: &mut AstCursor,
    expression_start: usize,
    expression_end: usize,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LoopHeaderResult<(Expression, TypeId)> {
    let collection_expression = parse_cursor_expression(
        token_stream,
        expression_start,
        expression_end,
        context,
        type_interner,
        &ValueMode::ImmutableReference,
        string_table,
        path_fork,
    )?;
    let type_environment = type_interner.environment();
    let Some(item_type_id) =
        type_environment.collection_element_type(collection_expression.type_id)
    else {
        return loop_header_error(
            InvalidLoopHeaderReason::CollectionSourceNotCollection {
                found_type: collection_expression.type_id,
            },
            collection_expression.span,
        );
    };
    Ok((collection_expression, item_type_id))
}

fn parses_as_collection_cursor(
    token_stream: &mut AstCursor,
    expression_start: usize,
    expression_end: usize,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> bool {
    let Ok(expression) = parse_cursor_expression(
        token_stream,
        expression_start,
        expression_end,
        context,
        type_interner,
        &ValueMode::ImmutableReference,
        string_table,
        path_fork,
    ) else {
        return false;
    };
    type_interner
        .environment()
        .collection_element_type(expression.type_id)
        .is_some()
}

fn parses_as_range_cursor(
    token_stream: &mut AstCursor,
    expression_start: usize,
    expression_end: usize,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> bool {
    parse_range_loop_spec_cursor(
        token_stream,
        expression_start,
        expression_end,
        context,
        type_interner,
        string_table,
        path_fork,
    )
    .is_ok()
}

fn has_top_level_range_marker_cursor(
    token_stream: &AstCursor,
    header_start: usize,
    header_end: usize,
) -> bool {
    find_top_level_cursor_token(
        token_stream,
        header_start,
        header_end,
        TokenKind::ExclusiveRange,
    )
    .is_some()
}

fn collect_top_level_cursor_indexes(
    token_stream: &AstCursor,
    start: usize,
    end: usize,
    predicate: impl Fn(&TokenKind) -> bool,
) -> Vec<usize> {
    let mut nesting_depth = NestingDepth::default();
    let mut indexes = Vec::new();
    let mut index = start;
    while index < end {
        let Some(kind) = token_stream.token_kind_at(index) else {
            index += 1;
            continue;
        };
        if nesting_depth.is_top_level() && predicate(&kind) {
            indexes.push(index);
        }
        nesting_depth.step(&kind);
        index += 1;
    }
    indexes
}

fn find_top_level_cursor_token(
    token_stream: &AstCursor,
    start: usize,
    end: usize,
    target: TokenKind,
) -> Option<usize> {
    collect_top_level_cursor_indexes(token_stream, start, end, |kind| *kind == target)
        .into_iter()
        .next()
}

/// Find a range-step separator using the same delimiter depth as
/// [`create_expression_until`].
///
/// WHAT: mirrors the legacy expression parser's `ExpressionBoundaryDepth` scan instead of the
/// broader loop-grammar `NestingDepth` scan.
/// WHY: a `By` nested in a template expression must make the same end/step split decision on the
/// canonical cursor path as it does in the compatibility parser.
fn find_top_level_expression_boundary_cursor_token(
    token_stream: &AstCursor,
    start: usize,
    end: usize,
) -> Option<usize> {
    let mut depth = ExpressionBoundaryDepth::default();
    let mut index = start;
    while index < end {
        let kind = token_stream.token_kind_at(index)?;
        if depth.is_top_level() && kind == TokenKind::By {
            return Some(index);
        }
        depth.step(&kind);
        index += 1;
    }
    None
}

fn trim_cursor_header_start(token_stream: &AstCursor, mut start: usize) -> usize {
    while start < token_stream.length()
        && token_stream.token_kind_at(start) == Some(TokenKind::Newline)
    {
        start += 1;
    }
    start
}

fn trim_cursor_header_end(token_stream: &AstCursor, start: usize, mut end: usize) -> usize {
    while end > start && token_stream.token_kind_at(end - 1) == Some(TokenKind::Newline) {
        end -= 1;
    }
    end
}

fn parse_range_loop_header(
    header_tokens: &[Token],
    path_syntax: &FilePathSyntax,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<ParsedLoopHeader> {
    // Parse explicit `|...|` bindings first, then reject bare binding tails with a targeted
    // diagnostic before falling back to no-binding range parsing.
    if let Some(pipe_binding_split) = parse_pipe_binding_suffix(
        header_tokens,
        parser.string_table,
        parser.scope_context.shared.declaring_file_id,
    )? {
        let range = parse_range_loop_spec_from_tokens(
            &pipe_binding_split.core_tokens,
            path_syntax,
            parser.scope_context,
            parser.type_interner,
            parser.string_table,
            parser.path_fork,
        )?;
        let binding_type = range_binding_type(&range, parser.type_interner.environment())?;
        let bindings =
            declare_loop_bindings(Some(pipe_binding_split.bindings), binding_type, parser)?;

        return Ok(ParsedLoopHeader::Range { bindings, range });
    }

    if let Some(bare_binding_suffix) = detect_bare_loop_binding_suffix(
        header_tokens,
        parser.scope_context.shared.declaring_file_id,
    ) && parse_range_loop_spec_from_tokens(
        &bare_binding_suffix.core_tokens,
        path_syntax,
        parser.scope_context,
        parser.type_interner,
        parser.string_table,
        parser.path_fork,
    )
    .is_ok()
    {
        return bare_loop_binding_syntax_error(&bare_binding_suffix);
    }

    let range = parse_range_loop_spec_from_tokens(
        header_tokens,
        path_syntax,
        parser.scope_context,
        parser.type_interner,
        parser.string_table,
        parser.path_fork,
    )?;
    let binding_type = range_binding_type(&range, parser.type_interner.environment())?;
    let bindings = declare_loop_bindings(None, binding_type, parser)?;
    Ok(ParsedLoopHeader::Range { bindings, range })
}

fn parse_non_range_loop_header(
    header_tokens: &[Token],
    path_syntax: &FilePathSyntax,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<ParsedLoopHeader> {
    // Conditional loops are distinguished by a full-header boolean expression with no binding
    // suffix. Parse explicit `|...|` bindings first, then reject bare binding tails with a
    // targeted diagnostic before evaluating conditional/collection fallback.
    if let Some(pipe_binding_split) = parse_pipe_binding_suffix(
        header_tokens,
        parser.string_table,
        parser.scope_context.shared.declaring_file_id,
    )? {
        let (iterable, item_type) = parse_collection_iterable_from_tokens(
            &pipe_binding_split.core_tokens,
            path_syntax,
            parser.scope_context,
            parser.type_interner,
            parser.string_table,
            parser.path_fork,
        )?;
        let bindings = declare_loop_bindings(Some(pipe_binding_split.bindings), item_type, parser)?;

        return Ok(ParsedLoopHeader::Collection { bindings, iterable });
    }

    if let Some(bare_binding_suffix) = detect_bare_loop_binding_suffix(
        header_tokens,
        parser.scope_context.shared.declaring_file_id,
    ) && parses_as_collection_iterable(
        &bare_binding_suffix.core_tokens,
        path_syntax,
        parser.scope_context,
        parser.type_interner,
        parser.string_table,
        parser.path_fork,
    ) {
        return bare_loop_binding_syntax_error(&bare_binding_suffix);
    }

    let expression = parse_expression_from_tokens(
        header_tokens,
        path_syntax,
        parser.scope_context,
        parser.type_interner,
        &ValueMode::ImmutableOwned,
        parser.string_table,
        parser.path_fork,
    )?;

    let item_type_id = parser
        .type_interner
        .environment()
        .collection_element_type(expression.type_id);

    if let Some(item_type_id) = item_type_id {
        let bindings = declare_loop_bindings(None, item_type_id, parser)?;
        return Ok(ParsedLoopHeader::Collection {
            bindings,
            iterable: expression,
        });
    }

    ensure_loop_condition(&expression, parser.type_interner.environment())?;

    Ok(ParsedLoopHeader::Conditional {
        condition: expression,
    })
}

fn reject_removed_in_loop_syntax(
    header_tokens: &[Token],
    string_table: &StringTable,
    source_id: SourceId,
) -> LoopHeaderResult<()> {
    if header_tokens.len() < 3 {
        return Ok(());
    }

    let TokenKind::Symbol(_) = header_tokens[0].kind else {
        return Ok(());
    };

    let TokenKind::Symbol(second_symbol) = header_tokens[1].kind else {
        return Ok(());
    };

    if string_table.resolve(second_symbol) != "in" {
        return Ok(());
    }

    loop_header_error(
        InvalidLoopHeaderReason::RemovedInSyntax,
        Some(SourceSpan::new(source_id, header_tokens[1].span)),
    )
}

fn parse_pipe_binding_suffix(
    header_tokens: &[Token],
    string_table: &StringTable,
    source_id: SourceId,
) -> LoopHeaderResult<Option<BindingSuffixSplit>> {
    let pipe_indices = collect_top_level_token_indexes(header_tokens, |token| {
        matches!(token, TokenKind::TypeParameterBracket)
    });

    if pipe_indices.is_empty() {
        return Ok(None);
    }

    if header_tokens
        .last()
        .is_none_or(|token| !matches!(token.kind, TokenKind::TypeParameterBracket))
    {
        return loop_header_error(
            InvalidLoopHeaderReason::MissingClosingPipe,
            Some(SourceSpan::new(
                source_id,
                header_tokens[header_tokens.len() - 1].span,
            )),
        );
    }

    if pipe_indices.len() != 2 {
        return loop_header_error(
            InvalidLoopHeaderReason::MalformedBindingPipes,
            Some(SourceSpan::new(
                source_id,
                header_tokens[pipe_indices[0]].span,
            )),
        );
    }

    let open_pipe_index = pipe_indices[0];
    let close_pipe_index = pipe_indices[1];

    if close_pipe_index <= open_pipe_index {
        return loop_header_error(
            InvalidLoopHeaderReason::MalformedBindingPipes,
            Some(SourceSpan::new(
                source_id,
                header_tokens[open_pipe_index].span,
            )),
        );
    }

    let core_tokens = header_tokens[..open_pipe_index].to_vec();
    let binding_tokens = header_tokens[open_pipe_index + 1..close_pipe_index].to_vec();

    if core_tokens.is_empty() {
        return loop_header_error(
            InvalidLoopHeaderReason::MissingSourceBeforeBindings,
            Some(SourceSpan::new(
                source_id,
                header_tokens[open_pipe_index].span,
            )),
        );
    }

    let bindings = parse_binding_tokens(&binding_tokens, string_table, source_id)?;

    Ok(Some(BindingSuffixSplit {
        core_tokens,
        bindings,
    }))
}

fn parse_binding_tokens(
    binding_tokens: &[Token],
    _string_table: &StringTable,
    source_id: SourceId,
) -> LoopHeaderResult<ParsedBindingNames> {
    let filtered_tokens = binding_tokens
        .iter()
        .filter(|token| !matches!(token.kind, TokenKind::Newline))
        .cloned()
        .collect::<Vec<_>>();
    if filtered_tokens.is_empty() {
        return loop_header_error(
            InvalidLoopHeaderReason::EmptyBindingList,
            binding_tokens
                .first()
                .map(|token| SourceSpan::new(source_id, token.span)),
        );
    }

    let mut binding_names = Vec::with_capacity(2);
    let mut position = 0;

    while position < filtered_tokens.len() {
        let token = &filtered_tokens[position];
        if token.kind == TokenKind::This {
            return loop_header_error(
                InvalidLoopHeaderReason::ThisBinding,
                Some(SourceSpan::new(source_id, token.span)),
            );
        }
        let TokenKind::Symbol(symbol_id) = token.kind else {
            return loop_header_error(
                InvalidLoopHeaderReason::BindingMustBeSymbol,
                Some(SourceSpan::new(source_id, token.span)),
            );
        };

        binding_names.push(ParsedBindingName {
            id: symbol_id,
            span: Some(SourceSpan::new(source_id, token.span)),
        });
        position += 1;

        if position >= filtered_tokens.len() {
            break;
        }

        if !matches!(filtered_tokens[position].kind, TokenKind::Comma) {
            return loop_header_error(
                InvalidLoopHeaderReason::MissingBindingComma,
                Some(SourceSpan::new(source_id, filtered_tokens[position].span)),
            );
        }

        position += 1;
        if position >= filtered_tokens.len() {
            return loop_header_error(
                InvalidLoopHeaderReason::TrailingBindingComma,
                Some(SourceSpan::new(
                    source_id,
                    filtered_tokens[position - 1].span,
                )),
            );
        }
    }

    build_binding_name_pair(binding_names)
}

fn detect_bare_loop_binding_suffix(
    header_tokens: &[Token],
    source_id: SourceId,
) -> Option<BareLoopBindingSuffix> {
    let non_newline_indices = collect_top_level_token_indexes(header_tokens, |token| {
        !matches!(token, TokenKind::Newline)
    });
    if non_newline_indices.len() < 2 {
        return None;
    }

    // Dual binding tail: detect `item, index` or `item index` (two trailing symbols).
    if non_newline_indices.len() >= 3 {
        let first_index = non_newline_indices[non_newline_indices.len() - 3];
        let separator_index = non_newline_indices[non_newline_indices.len() - 2];
        let second_index = non_newline_indices[non_newline_indices.len() - 1];

        if matches!(header_tokens[first_index].kind, TokenKind::Symbol(_))
            && matches!(header_tokens[separator_index].kind, TokenKind::Comma)
            && matches!(header_tokens[second_index].kind, TokenKind::Symbol(_))
            && first_index > 0
        {
            return Some(BareLoopBindingSuffix {
                core_tokens: header_tokens[..first_index].to_vec(),
                span: Some(SourceSpan::new(source_id, header_tokens[first_index].span)),
                kind: BareLoopBindingKind::Dual,
            });
        }

        if matches!(header_tokens[first_index].kind, TokenKind::Symbol(_))
            && matches!(header_tokens[separator_index].kind, TokenKind::Symbol(_))
            && matches!(header_tokens[second_index].kind, TokenKind::Symbol(_))
            && separator_index > 0
        {
            return Some(BareLoopBindingSuffix {
                core_tokens: header_tokens[..separator_index].to_vec(),
                span: Some(SourceSpan::new(
                    source_id,
                    header_tokens[separator_index].span,
                )),
                kind: BareLoopBindingKind::Dual,
            });
        }
    }

    // Single binding tail: detect a trailing symbol that is not preceded by a comma.
    let binding_index = *non_newline_indices.last()?;
    let core_tail_index = non_newline_indices[non_newline_indices.len() - 2];

    if matches!(header_tokens[binding_index].kind, TokenKind::Symbol(_))
        && !matches!(header_tokens[core_tail_index].kind, TokenKind::Comma)
    {
        return Some(BareLoopBindingSuffix {
            core_tokens: header_tokens[..binding_index].to_vec(),
            span: Some(SourceSpan::new(
                source_id,
                header_tokens[binding_index].span,
            )),
            kind: BareLoopBindingKind::Single,
        });
    }

    None
}

fn parses_as_collection_iterable(
    iterable_tokens: &[Token],
    path_syntax: &FilePathSyntax,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> bool {
    let Ok(iterable_expression) = parse_expression_from_tokens(
        iterable_tokens,
        path_syntax,
        context,
        type_interner,
        &ValueMode::ImmutableReference,
        string_table,
        path_fork,
    ) else {
        return false;
    };

    type_interner
        .environment()
        .collection_element_type(iterable_expression.type_id)
        .is_some()
}

fn bare_loop_binding_syntax_error<T>(
    binding_suffix: &BareLoopBindingSuffix,
) -> LoopHeaderResult<T> {
    match binding_suffix.kind {
        BareLoopBindingKind::Single => loop_header_error(
            InvalidLoopHeaderReason::BareSingleBinding,
            binding_suffix.span,
        ),
        BareLoopBindingKind::Dual => loop_header_error(
            InvalidLoopHeaderReason::BareDualBinding,
            binding_suffix.span,
        ),
    }
}

fn build_binding_name_pair(
    binding_names: Vec<ParsedBindingName>,
) -> LoopHeaderResult<ParsedBindingNames> {
    if binding_names.len() > 2 {
        return loop_header_error(
            InvalidLoopHeaderReason::TooManyBindings,
            binding_names[2].span,
        );
    }

    let Some(item_binding) = binding_names.first().cloned() else {
        return loop_header_error(
            InvalidLoopHeaderReason::EmptyBindingList,
            Option::<SourceSpan>::default(),
        );
    };

    let index_binding = binding_names.get(1).cloned();

    if let Some(index) = &index_binding
        && index.id == item_binding.id
    {
        return loop_header_error(InvalidLoopHeaderReason::DuplicateBindingName, index.span);
    }

    Ok(ParsedBindingNames {
        item: item_binding,
        index: index_binding,
    })
}

fn parse_collection_iterable_from_tokens(
    iterable_tokens: &[Token],
    path_syntax: &FilePathSyntax,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LoopHeaderResult<(Expression, TypeId)> {
    let collection_expression = parse_expression_from_tokens(
        iterable_tokens,
        path_syntax,
        context,
        type_interner,
        &ValueMode::ImmutableReference,
        string_table,
        path_fork,
    )?;

    let type_environment = type_interner.environment();
    let Some(item_type_id) =
        type_environment.collection_element_type(collection_expression.type_id)
    else {
        return loop_header_error(
            InvalidLoopHeaderReason::CollectionSourceNotCollection {
                found_type: collection_expression.type_id,
            },
            collection_expression.span,
        );
    };
    Ok((collection_expression, item_type_id))
}

fn parse_range_loop_spec_from_tokens(
    range_tokens: &[Token],
    path_syntax: &FilePathSyntax,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LoopHeaderResult<RangeLoopSpec> {
    let mut file_tokens = token_stream_with_eof(range_tokens, path_syntax, context)?;
    let mut stream = AstCursor::from_file_tokens(&mut file_tokens)?;

    // Omitted-start sugar: `loop to 5:` desugars to `loop 0 to 5:`.
    let start = if matches!(stream.current_token_kind(), TokenKind::ExclusiveRange) {
        let span = Some(stream.current_span());
        Expression::new(
            ExpressionKind::Int(0),
            span,
            builtin_type_ids::INT,
            DataType::Int,
            ValueMode::ImmutableOwned,
        )
    } else {
        let mut start_type = ExpectedType::Infer;
        let mut cast_target_context = CastTargetContext::None;
        let input = ExpressionParseInput::without_boundary_catch(
            ExpressionParseResources {
                token_stream: &mut stream,
                scope_context: context,
                type_interner,
                expected_type: &mut start_type,
                cast_target_context: &mut cast_target_context,
                value_mode: &ValueMode::ImmutableReference,
                string_table,
                path_fork,
            },
            false,
        );
        create_expression_until(input, &[TokenKind::ExclusiveRange, TokenKind::Eof])?
    };

    // ------------------------
    //  Parse end bound
    // ------------------------

    let end_kind = match stream.current_token_kind() {
        TokenKind::ExclusiveRange => {
            stream.advance();
            // Optional inclusive marker: `to & end`
            if matches!(stream.current_token_kind(), TokenKind::Ampersand) {
                stream.advance();
                RangeEndKind::Inclusive
            } else {
                RangeEndKind::Exclusive
            }
        }
        TokenKind::Eof => {
            return loop_header_error(InvalidLoopHeaderReason::MissingRangeSeparator, start.span);
        }
        _ => {
            return loop_header_error(
                InvalidLoopHeaderReason::MissingRangeSeparator,
                Some(stream.current_span()),
            );
        }
    };

    if matches!(stream.current_token_kind(), TokenKind::Eof) {
        return loop_header_error(InvalidLoopHeaderReason::MissingRangeEndBound, start.span);
    }

    let mut end_type = ExpectedType::Infer;
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::without_boundary_catch(
        ExpressionParseResources {
            token_stream: &mut stream,
            scope_context: context,
            type_interner,
            expected_type: &mut end_type,
            cast_target_context: &mut cast_target_context,
            value_mode: &ValueMode::ImmutableReference,
            string_table,
            path_fork,
        },
        false,
    );
    let end = create_expression_until(input, &[TokenKind::By, TokenKind::Eof])?;

    // ------------------------
    //  Parse optional step
    // ------------------------

    let step = if matches!(stream.current_token_kind(), TokenKind::By) {
        let by_span = Some(stream.current_span());
        stream.advance();

        if matches!(stream.current_token_kind(), TokenKind::Eof) {
            return loop_header_error(InvalidLoopHeaderReason::MissingRangeStep, by_span);
        }

        let mut step_type = ExpectedType::Infer;
        let mut cast_target_context = CastTargetContext::None;
        let input = ExpressionParseInput::without_boundary_catch(
            ExpressionParseResources {
                token_stream: &mut stream,
                scope_context: context,
                type_interner,
                expected_type: &mut step_type,
                cast_target_context: &mut cast_target_context,
                value_mode: &ValueMode::ImmutableReference,
                string_table,
                path_fork,
            },
            false,
        );
        Some(create_expression_until(input, &[TokenKind::Eof])?)
    } else {
        None
    };

    // ------------------------
    //  Validate operand types
    // ------------------------

    let type_environment = type_interner.environment();
    let is_start_numeric = is_numeric_type_id(start.type_id, type_environment);
    let is_end_numeric = is_numeric_type_id(end.type_id, type_environment);
    let is_step_numeric = step
        .as_ref()
        .map(|s| is_numeric_type_id(s.type_id, type_environment));

    if !is_start_numeric {
        return Err(CompilerDiagnostic::invalid_range_operand(
            RangeOperandKind::Start,
            start.type_id,
            start.span,
        )
        .into());
    }
    if !is_end_numeric {
        return Err(CompilerDiagnostic::invalid_range_operand(
            RangeOperandKind::End,
            end.type_id,
            end.span,
        )
        .into());
    }
    if let Some(step_expression) = step.as_ref().filter(|_| is_step_numeric == Some(false)) {
        return Err(CompilerDiagnostic::invalid_range_operand(
            RangeOperandKind::Step,
            step_expression.type_id,
            step_expression.span,
        )
        .into());
    }

    // ------------------------
    //  Check range constraints
    // ------------------------

    let type_environment = type_interner.environment();
    let uses_float = start.type_id == type_environment.builtins().float
        || end.type_id == type_environment.builtins().float
        || step
            .as_ref()
            .is_some_and(|s| s.type_id == type_environment.builtins().float);

    if uses_float && step.is_none() {
        return loop_header_error(InvalidLoopHeaderReason::FloatRangeMissingStep, end.span);
    }

    if let Some(step_expression) = &step
        && is_zero_numeric_literal(step_expression)
    {
        return loop_header_error(InvalidLoopHeaderReason::ZeroRangeStep, step_expression.span);
    }

    Ok(RangeLoopSpec {
        start,
        end,
        end_kind,
        step,
    })
}

fn range_binding_type(
    range: &RangeLoopSpec,
    type_environment: &TypeEnvironment,
) -> LoopHeaderResult<TypeId> {
    let is_start_numeric = is_numeric_type_id(range.start.type_id, type_environment);
    let is_end_numeric = is_numeric_type_id(range.end.type_id, type_environment);
    let is_step_numeric = range
        .step
        .as_ref()
        .map(|s| is_numeric_type_id(s.type_id, type_environment));

    if !is_start_numeric {
        return Err(CompilerDiagnostic::invalid_range_operand(
            RangeOperandKind::Start,
            range.start.type_id,
            range.start.span,
        )
        .into());
    }
    if !is_end_numeric {
        return Err(CompilerDiagnostic::invalid_range_operand(
            RangeOperandKind::End,
            range.end.type_id,
            range.end.span,
        )
        .into());
    }
    if let Some(step_expression) = range
        .step
        .as_ref()
        .filter(|_| is_step_numeric == Some(false))
    {
        return Err(CompilerDiagnostic::invalid_range_operand(
            RangeOperandKind::Step,
            step_expression.type_id,
            step_expression.span,
        )
        .into());
    }

    // Determine whether the loop variable should be `Float` or `Int`.
    let uses_float = range.start.type_id == type_environment.builtins().float
        || range.end.type_id == type_environment.builtins().float
        || range
            .step
            .as_ref()
            .is_some_and(|s| s.type_id == type_environment.builtins().float);

    Ok(if uses_float {
        type_environment.builtins().float
    } else {
        type_environment.builtins().int
    })
}

fn declare_loop_bindings(
    binding_names: Option<ParsedBindingNames>,
    item_type_id: TypeId,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<LoopBindings> {
    let Some(binding_names) = binding_names else {
        return Ok(LoopBindings {
            item: None,
            index: None,
        });
    };

    let item = Some(declare_loop_binding(
        &binding_names.item,
        item_type_id,
        parser,
    )?);

    let index = binding_names
        .index
        .as_ref()
        .map(|index_name| {
            let int_type_id = parser.type_interner.environment().builtins().int;
            declare_loop_binding(index_name, int_type_id, parser)
        })
        .transpose()?;

    Ok(LoopBindings { item, index })
}

fn declare_loop_binding(
    binding_name: &ParsedBindingName,
    type_id: TypeId,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<Declaration> {
    ensure_not_keyword_shadow_identifier(binding_name.id, binding_name.span, parser.string_table)?;

    if parser
        .scope_context
        .has_visible_local_declaration(&binding_name.id)
    {
        return loop_header_error(
            InvalidLoopHeaderReason::BindingAlreadyDeclared,
            binding_name.span,
        );
    }

    if let Some(warning) = naming_warning_for_identifier(
        binding_name.id,
        binding_name.span,
        IdentifierNamingKind::ValueLike,
        parser.string_table,
    ) {
        parser.warnings.push(warning);
    }

    let data_type = diagnostic_type_spelling(type_id, parser.type_interner.environment());
    let binding_span = binding_name.span;
    let declaration = Declaration {
        id: parser
            .path_fork
            .try_intern_child(parser.scope_context.scope, binding_name.id)
            .expect("path table exhausted while declaring loop binding"),
        value: Expression::new(
            ExpressionKind::NoValue,
            binding_span,
            type_id,
            data_type,
            ValueMode::ImmutableOwned,
        ),
        binding_span,
        config_qualifier: None,
    };
    let binding_span = declaration.value.span;
    parser
        .scope_context
        .add_var(declaration.to_owned(), binding_span, parser.path_fork);

    Ok(declaration)
}
fn parse_expression_from_tokens(
    expression_tokens: &[Token],
    path_syntax: &FilePathSyntax,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LoopHeaderResult<Expression> {
    let mut file_tokens = token_stream_with_eof(expression_tokens, path_syntax, context)?;
    let mut expression_stream = AstCursor::from_file_tokens(&mut file_tokens)?;
    let mut inferred_type = ExpectedType::Infer;

    let expression = create_expression_without_boundary_catch(
        &mut expression_stream,
        context,
        type_interner,
        &mut inferred_type,
        value_mode,
        false,
        string_table,
        path_fork,
    )?;

    Ok(expression)
}

/// Wrap a loop header's tokens in a `FileTokens` stream terminated by EOF.
///
/// WHY the context: the header tokens were lexed from the file that owns this loop, so the
/// substream takes that scope's source identity instead of a caller's argument. Callers wrap
/// the result in `AstCursor` immediately; this constructor is the one allowed explicit
/// token-vector fixture/source construction site in this file.
fn token_stream_with_eof(
    tokens: &[Token],
    path_syntax: &FilePathSyntax,
    context: &ScopeContext,
) -> LoopHeaderResult<FileTokens> {
    if tokens.is_empty() {
        return loop_header_error(InvalidLoopHeaderReason::ExpectedHeaderExpression, None);
    }

    let mut tokens_with_eof = tokens.to_vec();
    let eof_anchor = &tokens[tokens.len() - 1];
    let src_path = context.scope;
    tokens_with_eof.push(Token::new(TokenKind::Eof, eof_anchor.span));

    FileTokens::new_from_slice(
        src_path,
        context.shared.declaring_file_id,
        None,
        tokens_with_eof,
        path_syntax,
    )
    .map_err(ExpressionParseError::from)
}

fn is_numeric_type_id(type_id: TypeId, type_environment: &TypeEnvironment) -> bool {
    type_id == type_environment.builtins().int || type_id == type_environment.builtins().float
}

fn has_top_level_range_marker(tokens: &[Token]) -> bool {
    let mut nesting_depth = NestingDepth::default();

    for token in tokens {
        if nesting_depth.is_top_level() && matches!(token.kind, TokenKind::ExclusiveRange) {
            return true;
        }

        nesting_depth.step(&token.kind);
    }

    false
}

fn collect_top_level_token_indexes(
    tokens: &[Token],
    predicate: impl Fn(&TokenKind) -> bool,
) -> Vec<usize> {
    let mut nesting_depth = NestingDepth::default();
    let mut indexes = Vec::new();

    for (index, token) in tokens.iter().enumerate() {
        if nesting_depth.is_top_level() && predicate(&token.kind) {
            indexes.push(index);
        }

        nesting_depth.step(&token.kind);
    }

    indexes
}

fn trim_edge_newlines(tokens: &mut Vec<Token>) {
    while tokens
        .first()
        .is_some_and(|token| matches!(token.kind, TokenKind::Newline))
    {
        tokens.remove(0);
    }

    while tokens
        .last()
        .is_some_and(|token| matches!(token.kind, TokenKind::Newline))
    {
        tokens.pop();
    }
}

// Detect literal zero so we can reject `by 0` ranges with a targeted diagnostic.
fn is_zero_numeric_literal(expression: &Expression) -> bool {
    match expression.kind {
        ExpressionKind::Int(value) => value == 0,
        ExpressionKind::Float(value) => value == 0.0,
        _ => false,
    }
}
