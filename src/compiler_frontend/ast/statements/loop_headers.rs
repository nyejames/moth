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
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    FilePathSyntax, FileTokens, SourceLocation, Token, TokenKind,
};
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::utilities::token_scan::NestingDepth;
use crate::compiler_frontend::value_mode::ValueMode;

#[derive(Debug, Clone)]
struct ParsedBindingName {
    id: StringId,
    location: SourceLocation,
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
    location: SourceLocation,
    kind: BareLoopBindingKind,
}

struct LoopHeaderParser<'a, 'types> {
    scope_context: &'a mut ScopeContext,
    type_interner: &'a mut AstTypeInterner<'types>,
    warnings: &'a mut Vec<CompilerDiagnostic>,
    string_table: &'a mut StringTable,
}

/// Loop-header parsing preserves the AST body error lane because range and iterable expressions
/// are parsed through temporary streams over the frozen file-owned path table.
type LoopHeaderResult<T> = Result<T, ExpressionParseError>;

fn loop_header_error<T>(
    reason: InvalidLoopHeaderReason,
    location: SourceLocation,
) -> LoopHeaderResult<T> {
    Err(CompilerDiagnostic::invalid_loop_header(reason, location).into())
}

pub(crate) fn parse_loop_header_tokens(
    header_tokens: &[Token],
    path_syntax: &FilePathSyntax,
    mut context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> LoopHeaderResult<(ParsedLoopHeader, ScopeContext)> {
    let mut header_tokens = header_tokens.to_vec();
    trim_edge_newlines(&mut header_tokens);
    reject_removed_in_loop_syntax(&header_tokens, string_table)?;

    let loop_header = {
        let mut parser = LoopHeaderParser {
            scope_context: &mut context,
            type_interner,
            warnings,
            string_table,
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

fn parse_range_loop_header(
    header_tokens: &[Token],
    path_syntax: &FilePathSyntax,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<ParsedLoopHeader> {
    // Parse explicit `|...|` bindings first, then reject bare binding tails with a targeted
    // diagnostic before falling back to no-binding range parsing.
    if let Some(pipe_binding_split) = parse_pipe_binding_suffix(header_tokens, parser.string_table)?
    {
        let range = parse_range_loop_spec_from_tokens(
            &pipe_binding_split.core_tokens,
            path_syntax,
            parser.scope_context,
            parser.type_interner,
            parser.string_table,
        )?;
        let binding_type = range_binding_type(&range, parser.type_interner.environment())?;
        let bindings =
            declare_loop_bindings(Some(pipe_binding_split.bindings), binding_type, parser)?;

        return Ok(ParsedLoopHeader::Range { bindings, range });
    }

    if let Some(bare_binding_suffix) = detect_bare_loop_binding_suffix(header_tokens)
        && parse_range_loop_spec_from_tokens(
            &bare_binding_suffix.core_tokens,
            path_syntax,
            parser.scope_context,
            parser.type_interner,
            parser.string_table,
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
    if let Some(pipe_binding_split) = parse_pipe_binding_suffix(header_tokens, parser.string_table)?
    {
        let (iterable, item_type) = parse_collection_iterable_from_tokens(
            &pipe_binding_split.core_tokens,
            path_syntax,
            parser.scope_context,
            parser.type_interner,
            parser.string_table,
        )?;
        let bindings = declare_loop_bindings(Some(pipe_binding_split.bindings), item_type, parser)?;

        return Ok(ParsedLoopHeader::Collection { bindings, iterable });
    }

    if let Some(bare_binding_suffix) = detect_bare_loop_binding_suffix(header_tokens)
        && parses_as_collection_iterable(
            &bare_binding_suffix.core_tokens,
            path_syntax,
            parser.scope_context,
            parser.type_interner,
            parser.string_table,
        )
    {
        return bare_loop_binding_syntax_error(&bare_binding_suffix);
    }

    let expression = parse_expression_from_tokens(
        header_tokens,
        path_syntax,
        parser.scope_context,
        parser.type_interner,
        &ValueMode::ImmutableOwned,
        parser.string_table,
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
        header_tokens[1].location.clone(),
    )
}

fn parse_pipe_binding_suffix(
    header_tokens: &[Token],
    string_table: &StringTable,
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
            header_tokens[header_tokens.len() - 1].location.clone(),
        );
    }

    if pipe_indices.len() != 2 {
        return loop_header_error(
            InvalidLoopHeaderReason::MalformedBindingPipes,
            header_tokens[pipe_indices[0]].location.clone(),
        );
    }

    let open_pipe_index = pipe_indices[0];
    let close_pipe_index = pipe_indices[1];

    if close_pipe_index <= open_pipe_index {
        return loop_header_error(
            InvalidLoopHeaderReason::MalformedBindingPipes,
            header_tokens[open_pipe_index].location.clone(),
        );
    }

    let core_tokens = header_tokens[..open_pipe_index].to_vec();
    let binding_tokens = header_tokens[open_pipe_index + 1..close_pipe_index].to_vec();

    if core_tokens.is_empty() {
        return loop_header_error(
            InvalidLoopHeaderReason::MissingSourceBeforeBindings,
            header_tokens[open_pipe_index].location.clone(),
        );
    }

    let bindings = parse_binding_tokens(&binding_tokens, string_table)?;

    Ok(Some(BindingSuffixSplit {
        core_tokens,
        bindings,
    }))
}

fn parse_binding_tokens(
    binding_tokens: &[Token],
    _string_table: &StringTable,
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
                .map(|token| token.location.clone())
                .unwrap_or_default(),
        );
    }

    let mut binding_names = Vec::with_capacity(2);
    let mut position = 0;

    while position < filtered_tokens.len() {
        let token = &filtered_tokens[position];
        if token.kind == TokenKind::This {
            return loop_header_error(InvalidLoopHeaderReason::ThisBinding, token.location.clone());
        }
        let TokenKind::Symbol(symbol_id) = token.kind else {
            return loop_header_error(
                InvalidLoopHeaderReason::BindingMustBeSymbol,
                token.location.clone(),
            );
        };

        binding_names.push(ParsedBindingName {
            id: symbol_id,
            location: token.location.clone(),
        });
        position += 1;

        if position >= filtered_tokens.len() {
            break;
        }

        if !matches!(filtered_tokens[position].kind, TokenKind::Comma) {
            return loop_header_error(
                InvalidLoopHeaderReason::MissingBindingComma,
                filtered_tokens[position].location.clone(),
            );
        }

        position += 1;
        if position >= filtered_tokens.len() {
            return loop_header_error(
                InvalidLoopHeaderReason::TrailingBindingComma,
                filtered_tokens[position - 1].location.clone(),
            );
        }
    }

    build_binding_name_pair(binding_names)
}

fn detect_bare_loop_binding_suffix(header_tokens: &[Token]) -> Option<BareLoopBindingSuffix> {
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
                location: header_tokens[first_index].location.clone(),
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
                location: header_tokens[separator_index].location.clone(),
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
            location: header_tokens[binding_index].location.clone(),
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
) -> bool {
    let Ok(iterable_expression) = parse_expression_from_tokens(
        iterable_tokens,
        path_syntax,
        context,
        type_interner,
        &ValueMode::ImmutableReference,
        string_table,
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
            binding_suffix.location.clone(),
        ),
        BareLoopBindingKind::Dual => loop_header_error(
            InvalidLoopHeaderReason::BareDualBinding,
            binding_suffix.location.clone(),
        ),
    }
}

fn build_binding_name_pair(
    binding_names: Vec<ParsedBindingName>,
) -> LoopHeaderResult<ParsedBindingNames> {
    if binding_names.len() > 2 {
        return loop_header_error(
            InvalidLoopHeaderReason::TooManyBindings,
            binding_names[2].location.clone(),
        );
    }

    let Some(item_binding) = binding_names.first().cloned() else {
        return loop_header_error(
            InvalidLoopHeaderReason::EmptyBindingList,
            SourceLocation::default(),
        );
    };

    let index_binding = binding_names.get(1).cloned();

    if let Some(index) = &index_binding
        && index.id == item_binding.id
    {
        return loop_header_error(
            InvalidLoopHeaderReason::DuplicateBindingName,
            index.location.clone(),
        );
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
) -> LoopHeaderResult<(Expression, TypeId)> {
    let collection_expression = parse_expression_from_tokens(
        iterable_tokens,
        path_syntax,
        context,
        type_interner,
        &ValueMode::ImmutableReference,
        string_table,
    )?;

    let type_environment = type_interner.environment();
    let Some(item_type_id) =
        type_environment.collection_element_type(collection_expression.type_id)
    else {
        return loop_header_error(
            InvalidLoopHeaderReason::CollectionSourceNotCollection {
                found_type: collection_expression.type_id,
            },
            collection_expression.location.clone(),
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
) -> LoopHeaderResult<RangeLoopSpec> {
    let mut stream = token_stream_with_eof(range_tokens, path_syntax)?;

    // Omitted-start sugar: `loop to 5:` desugars to `loop 0 to 5:`.
    let start = if matches!(stream.current_token_kind(), TokenKind::ExclusiveRange) {
        let location = stream.current_location();
        Expression::new(
            ExpressionKind::Int(0),
            location,
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
            return loop_header_error(
                InvalidLoopHeaderReason::MissingRangeSeparator,
                start.location.clone(),
            );
        }
        _ => {
            return loop_header_error(
                InvalidLoopHeaderReason::MissingRangeSeparator,
                stream.current_location(),
            );
        }
    };

    if matches!(stream.current_token_kind(), TokenKind::Eof) {
        return loop_header_error(
            InvalidLoopHeaderReason::MissingRangeEndBound,
            start.location.clone(),
        );
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
        },
        false,
    );
    let end = create_expression_until(input, &[TokenKind::By, TokenKind::Eof])?;

    // ------------------------
    //  Parse optional step
    // ------------------------

    let step = if matches!(stream.current_token_kind(), TokenKind::By) {
        let by_location = stream.current_location();
        stream.advance();

        if matches!(stream.current_token_kind(), TokenKind::Eof) {
            return loop_header_error(InvalidLoopHeaderReason::MissingRangeStep, by_location);
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
            start.location.clone(),
        )
        .into());
    }
    if !is_end_numeric {
        return Err(CompilerDiagnostic::invalid_range_operand(
            RangeOperandKind::End,
            end.type_id,
            end.location.clone(),
        )
        .into());
    }
    if let Some(step_expression) = step.as_ref().filter(|_| is_step_numeric == Some(false)) {
        return Err(CompilerDiagnostic::invalid_range_operand(
            RangeOperandKind::Step,
            step_expression.type_id,
            step_expression.location.clone(),
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
        return loop_header_error(
            InvalidLoopHeaderReason::FloatRangeMissingStep,
            end.location.clone(),
        );
    }

    if let Some(step_expression) = &step
        && is_zero_numeric_literal(step_expression)
    {
        return loop_header_error(
            InvalidLoopHeaderReason::ZeroRangeStep,
            step_expression.location.clone(),
        );
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
            range.start.location.clone(),
        )
        .into());
    }
    if !is_end_numeric {
        return Err(CompilerDiagnostic::invalid_range_operand(
            RangeOperandKind::End,
            range.end.type_id,
            range.end.location.clone(),
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
            step_expression.location.clone(),
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
    ensure_not_keyword_shadow_identifier(
        binding_name.id,
        binding_name.location.clone(),
        parser.string_table,
    )?;

    if parser
        .scope_context
        .has_visible_local_declaration(&binding_name.id)
    {
        return loop_header_error(
            InvalidLoopHeaderReason::BindingAlreadyDeclared,
            binding_name.location.clone(),
        );
    }

    if let Some(warning) = naming_warning_for_identifier(
        binding_name.id,
        binding_name.location.clone(),
        IdentifierNamingKind::ValueLike,
        parser.string_table,
    ) {
        parser.warnings.push(warning);
    }

    let data_type = diagnostic_type_spelling(type_id, parser.type_interner.environment());
    let declaration = Declaration {
        id: parser.scope_context.scope.append(binding_name.id),
        value: Expression::new(
            ExpressionKind::NoValue,
            binding_name.location.clone(),
            type_id,
            data_type,
            ValueMode::ImmutableOwned,
        ),
        config_qualifier: None,
    };

    let binding_location = declaration.value.location.clone();
    parser
        .scope_context
        .add_var(declaration.to_owned(), binding_location);

    Ok(declaration)
}

fn parse_expression_from_tokens(
    expression_tokens: &[Token],
    path_syntax: &FilePathSyntax,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
) -> LoopHeaderResult<Expression> {
    let mut expression_stream = token_stream_with_eof(expression_tokens, path_syntax)?;
    let mut inferred_type = ExpectedType::Infer;

    let expression = create_expression_without_boundary_catch(
        &mut expression_stream,
        context,
        type_interner,
        &mut inferred_type,
        value_mode,
        false,
        string_table,
    )?;

    Ok(expression)
}

fn token_stream_with_eof(
    tokens: &[Token],
    path_syntax: &FilePathSyntax,
) -> LoopHeaderResult<FileTokens> {
    if tokens.is_empty() {
        return loop_header_error(
            InvalidLoopHeaderReason::ExpectedHeaderExpression,
            SourceLocation::default(),
        );
    }

    let mut tokens_with_eof = tokens.to_vec();
    let eof_location = tokens[tokens.len() - 1].location.clone();
    let src_path = tokens[0].location.scope.clone();

    tokens_with_eof.push(Token::new(TokenKind::Eof, eof_location));

    FileTokens::new_from_slice(src_path, None, None, tokens_with_eof, path_syntax)
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
