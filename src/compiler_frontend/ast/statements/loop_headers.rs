//! Shared loop-header parsing for AST statement and template parsers.
//!
//! WHAT: parses the three body-independent loop header forms after the `loop`
//! keyword has been consumed: conditional, numeric range, and collection iteration.
//! WHY: statement loops and template loop suffixes need the same syntax, binding,
//! and type-validation rules, while each caller owns its own body parsing. Headers
//! parse from the canonical cursor window only.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{
    Declaration, LoopBindings, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::{ExpressionParseError, NumericDeferral};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression_until, create_expression_with_trailing_newline_policy,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::field_access::{
    concrete_shape_type, retained_operand_result_shape,
};
use crate::compiler_frontend::ast::module_ast::scope_context::AuthoredBindingFacts;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredRangeHeader, DeferredRangeSpec, NumericBindingId, NumericRecipe, RetainedRangeCounter,
    StatementExpression, deferred_numeric_expression, pending_collection_shape,
    retained_known_shape,
};
use crate::compiler_frontend::ast::numeric_known_shape::{KnownShape, KnownShapeId};
use crate::compiler_frontend::ast::statements::condition_validation::ensure_loop_condition;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidLoopHeaderReason, InvalidRangeOperandReason, RangeOperandKind,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::range_iteration_domain;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::utilities::token_scan::{ExpressionBoundaryDepth, NestingDepth};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};

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
pub(crate) enum ParsedLoopHeader {
    /// A conditional loop. The condition is already built, or retained because it still
    /// references a pending numeric origin whose type the body root has not selected yet.
    Conditional { condition: StatementExpression },
    /// A numeric range loop: a fully built spec, or the authored operand slots whose pending
    /// numeric origins complete at the body root.
    Range(ParsedRangeHeader),
    /// A collection loop: the bindings the header's element metadata selected, plus the source.
    ///
    /// WHAT: the source is a built expression, or a retained fragment whose value completes at the
    ///       owning body root — a call whose numeric-dependent arguments defer, but whose parsed
    ///       signature already names a collection result, so the element metadata is final; or a
    ///       pending numeric collection binding, whose `pending_item` is selected at the body root
    ///       from the source's completed element type.
    /// WHY: a concrete item is final, so the loop needs no numeric completion of its own; a pending
    ///      item waits for the source's completed collection type and shares the source literal's
    ///      element class with every body use.
    Collection {
        bindings: Box<LoopBindings>,
        iterable: StatementExpression,
        pending_item: Option<NumericBindingId>,
    },
}

/// One collection loop source before its item binding is declared.
///
/// WHAT: a concrete element type, or a pending numeric collection source whose item binding the
///       header declares against the source's element class.
/// WHY: the two shapes need different binding declarations, and the header parser must not
///      fabricate an element type for the pending one.
enum CollectionIterable {
    Known {
        iterable: StatementExpression,
        item_type: TypeId,
    },
    Pending {
        iterable: StatementExpression,
        source: KnownShapeId,
    },
}

/// One parsed range loop header.
///
/// WHAT: either a spec whose operands were all concrete at parse time, or the authored operand
///       slots plus the loop bindings their domain selected.
/// WHY: the range domain and the loop binding types are facts the body consumes, so a header
///      whose operands still reference pending numeric origins keeps those exact slots and
///      completes them — with the shared range policy — at the owning body root.
#[derive(Debug, Clone)]
pub(crate) enum ParsedRangeHeader {
    /// Every operand completed at parse time: the header's exact domain and its bindings are
    /// final, and the loop needs no numeric completion.
    Concrete {
        bindings: Box<LoopBindings>,
        range: Box<RangeLoopSpec>,
    },
    /// At least one operand still references a pending numeric origin, so the header cannot select
    /// its domain (or declare its counter) at parse time.
    Deferred(DeferredRangeHeader),
}

/// One parsed range header before its domain is selected and its bindings are final.
///
/// WHY: a range header's exact domain is a forward result of its operands' selected types, and a
///      retained operand has no selected type at parse time; the authored slots therefore stay in
///      their authored form until the body root owns every permitted use of the origins they
///      reference.
#[derive(Debug)]
struct ParsedRangeSpec {
    start: Option<StatementExpression>,
    end: StatementExpression,
    end_kind: RangeEndKind,
    step: Option<StatementExpression>,
    /// The authored span of an omitted start operand, used for the implicit domain zero.
    span: Option<SourceSpan>,
}

impl ParsedRangeSpec {
    /// Build the loop header from this spec and its authored binding names.
    ///
    /// WHAT: a spec whose operands are all built selects its domain through the shared range
    ///       policy here and declares concrete bindings; a spec with a retained operand hands its
    ///       slots to the counter's construction row, or to the loop construct when the header
    ///       authored no bindings.
    /// WHY: the shared range policy is the only domain/synthesis/check owner, and a retained
    ///      header must not select a binding type: the counter's type is the forward result of
    ///      the operands' selected types, so no binding type exists yet.
    fn into_parsed_header(
        self,
        binding_names: Option<ParsedBindingNames>,
        parser: &mut LoopHeaderParser<'_, '_>,
    ) -> LoopHeaderResult<ParsedLoopHeader> {
        let ParsedRangeSpec {
            start,
            end,
            end_kind,
            step,
            span,
        } = self;

        let slots_are_built = start
            .as_ref()
            .into_iter()
            .chain(std::iter::once(&end))
            .chain(step.as_ref())
            .all(|slot| matches!(slot, StatementExpression::Built(_)));

        if slots_are_built {
            let start = match start {
                Some(slot) => Some(built_range_slot(slot, "start")?),
                None => None,
            };
            let end = built_range_slot(end, "end")?;
            let step = match step {
                Some(slot) => Some(built_range_slot(slot, "step")?),
                None => None,
            };
            let range = range_loop_spec_from_completed_operands(
                start,
                end,
                end_kind,
                step,
                span,
                parser.scope_context,
                parser.type_interner,
                parser.string_table,
                parser.path_fork,
            )?;
            // The counter's domain is the spec's selected domain: one policy owns both.
            let binding_type = range_binding_type(&range, parser.type_interner.environment())?;
            let bindings = declare_loop_bindings(binding_names, binding_type, parser)?;
            return Ok(ParsedLoopHeader::Range(ParsedRangeHeader::Concrete {
                bindings: Box::new(bindings),
                range: Box::new(range),
            }));
        }

        let range = DeferredRangeSpec {
            start,
            end,
            end_kind,
            step,
            span,
        };
        let header = match binding_names {
            Some(binding_names) => DeferredRangeHeader::Bound(Box::new(
                declare_retained_range_counter(binding_names, range, parser)?,
            )),
            None => DeferredRangeHeader::Unbound(Box::new(range)),
        };
        Ok(ParsedLoopHeader::Range(ParsedRangeHeader::Deferred(header)))
    }
}

/// The built expression of one range slot that needs no numeric completion.
fn built_range_slot(slot: StatementExpression, position: &str) -> LoopHeaderResult<Expression> {
    match slot {
        StatementExpression::Built(expression) => Ok(expression),
        StatementExpression::Retained(_) => Err(CompilerError::compiler_error(format!(
            "a range header {position} operand was retained after the header decided it is concrete"
        ))
        .into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BareLoopBindingKind {
    Single,
    Dual,
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

struct CursorExpressionInput<'a, 'types, 'cursor, 'tokens> {
    token_stream: &'cursor mut AstCursor<'tokens>,
    expression_start: usize,
    expression_end: usize,
    context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'types>,
    value_mode: &'a ValueMode,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
}

struct CursorExpressionUntilInput<'a, 'types, 'cursor, 'tokens, 'stop> {
    token_stream: &'cursor mut AstCursor<'tokens>,
    expression_start: usize,
    expression_end: usize,
    context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'types>,
    value_mode: &'a ValueMode,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
    stop_tokens: &'stop [TokenTag],
}

struct RangeLoopSpecInput<'a, 'types, 'cursor, 'tokens> {
    token_stream: &'cursor mut AstCursor<'tokens>,
    range_start: usize,
    range_end: usize,
    context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'types>,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
}

/// Loop-header parsing preserves the AST body error lane for canonical header windows.
type LoopHeaderResult<T> = Result<T, ExpressionParseError>;

fn loop_header_error<T>(
    reason: InvalidLoopHeaderReason,
    span: Option<SourceSpan>,
) -> LoopHeaderResult<T> {
    Err(CompilerDiagnostic::invalid_loop_header(reason, span).into())
}

/// Parse the sole loop-header grammar directly from its bounded cursor window.
///
/// WHAT: keeps all header reads on the short-lived `AstCursor` view and represents
/// grammar splits as parser-position ranges rather than materialised payload vectors.
/// WHY: this is the only loop-header parser and consumes canonical source-token views.
pub(crate) fn parse_loop_header_cursor(
    token_stream: &mut AstCursor,
    mut context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LoopHeaderResult<(ParsedLoopHeader, ScopeContext)> {
    let resume = token_stream.position();
    let limit = token_stream.length();
    // Direct `TokenTag` scans handle structural trimming on the canonical cursor.
    // Payloads are validated explicitly through typed accessors and invariant checks
    // rather than being inferred from the structural tag scan.
    while token_stream.position() < limit
        && !token_stream.is_at_end()
        && token_stream.current_tag() == TokenTag::NEWLINE
    {
        token_stream.advance();
    }
    let header_start = token_stream.position();
    // The trailing trim records the end after the last non-newline token. Direct
    // `TokenTag` scanning treats `TokenTag::EOF` as a non-advancing structural
    // boundary; payload validity is checked by typed accessors and invariant validation.
    let mut header_end = header_start;
    while token_stream.position() < limit && !token_stream.is_at_end() {
        let position = token_stream.position();
        let tag = token_stream.current_tag();
        if tag != TokenTag::NEWLINE {
            header_end = position + 1;
        }
        if tag == TokenTag::EOF {
            break;
        }
        token_stream.advance();
    }
    token_stream
        .set_position(resume)
        .map_err(ExpressionParseError::from)?;

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

        if has_top_level_range_marker_cursor(token_stream, header_start, header_end)? {
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
    if let Some(pipe_binding_split) = parse_pipe_binding_suffix_cursor(
        token_stream,
        header_start,
        header_end,
        parser.string_table,
    )? {
        let range = parse_range_loop_spec_cursor(RangeLoopSpecInput {
            token_stream,
            range_start: header_start,
            range_end: pipe_binding_split.core_end,
            context: parser.scope_context,
            type_interner: parser.type_interner,
            string_table: parser.string_table,
            path_fork: parser.path_fork,
        })?;
        return range.into_parsed_header(Some(pipe_binding_split.bindings), parser);
    }

    let bare_binding_suffix =
        detect_bare_loop_binding_suffix_cursor(token_stream, header_start, header_end)?;
    if let Some(bare_binding_suffix) = bare_binding_suffix
        && parses_as_range_cursor(RangeLoopSpecInput {
            token_stream,
            range_start: header_start,
            range_end: bare_binding_suffix.core_end,
            context: parser.scope_context,
            type_interner: parser.type_interner,
            string_table: parser.string_table,
            path_fork: parser.path_fork,
        })?
    {
        return bare_loop_binding_syntax_error_cursor(&bare_binding_suffix);
    }

    let range = parse_range_loop_spec_cursor(RangeLoopSpecInput {
        token_stream,
        range_start: header_start,
        range_end: header_end,
        context: parser.scope_context,
        type_interner: parser.type_interner,
        string_table: parser.string_table,
        path_fork: parser.path_fork,
    })?;
    range.into_parsed_header(None, parser)
}

fn parse_non_range_loop_header_cursor(
    token_stream: &mut AstCursor,
    header_start: usize,
    header_end: usize,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<ParsedLoopHeader> {
    if let Some(pipe_binding_split) = parse_pipe_binding_suffix_cursor(
        token_stream,
        header_start,
        header_end,
        parser.string_table,
    )? {
        let source = parse_collection_iterable_cursor(CursorExpressionInput {
            token_stream,
            expression_start: header_start,
            expression_end: pipe_binding_split.core_end,
            context: parser.scope_context,
            type_interner: parser.type_interner,
            value_mode: &ValueMode::ImmutableReference,
            string_table: parser.string_table,
            path_fork: parser.path_fork,
        })?;
        return match source {
            CollectionIterable::Known {
                iterable,
                item_type,
            } => {
                let bindings =
                    declare_loop_bindings(Some(pipe_binding_split.bindings), item_type, parser)?;
                Ok(ParsedLoopHeader::Collection {
                    bindings: Box::new(bindings),
                    iterable,
                    pending_item: None,
                })
            }
            CollectionIterable::Pending { iterable, source } => {
                let (item, bindings) = declare_pending_collection_bindings(
                    pipe_binding_split.bindings,
                    source,
                    parser,
                )?;
                Ok(ParsedLoopHeader::Collection {
                    bindings: Box::new(bindings),
                    iterable,
                    pending_item: Some(item),
                })
            }
        };
    }
    let bare_binding_suffix =
        detect_bare_loop_binding_suffix_cursor(token_stream, header_start, header_end)?;
    if let Some(bare_binding_suffix) = bare_binding_suffix
        && parses_as_collection_cursor(CursorExpressionInput {
            token_stream,
            expression_start: header_start,
            expression_end: bare_binding_suffix.core_end,
            context: parser.scope_context,
            type_interner: parser.type_interner,
            value_mode: &ValueMode::ImmutableReference,
            string_table: parser.string_table,
            path_fork: parser.path_fork,
        })?
    {
        return bare_loop_binding_syntax_error_cursor(&bare_binding_suffix);
    }

    let expression = match parse_cursor_expression(
        CursorExpressionInput {
            token_stream,
            expression_start: header_start,
            expression_end: header_end,
            context: parser.scope_context,
            type_interner: parser.type_interner,
            value_mode: &ValueMode::ImmutableOwned,
            string_table: parser.string_table,
            path_fork: parser.path_fork,
        },
        false,
    ) {
        Ok(expression) => expression,
        Err(error) => {
            let deferral = error.take_numeric_deferral("a loop condition")?;
            // A bare pending collection binding is a collection source with no item binding; the
            // body root completes its source exactly like a concrete bare collection loop.
            if pending_collection_source(
                &deferral.items,
                parser.scope_context,
                parser.type_interner.environment(),
            )?
            .is_some()
            {
                return Ok(ParsedLoopHeader::Collection {
                    bindings: Box::new(LoopBindings {
                        item: None,
                        index: None,
                    }),
                    iterable: StatementExpression::Retained(deferred_numeric_expression(
                        deferral.items,
                        ExpectedType::Infer,
                        deferral.value_mode,
                        deferral.span,
                        deferral.suffix,
                    )),
                    pending_item: None,
                });
            }
            // A fragment that still references a pending numeric origin is numeric by
            // construction, so it cannot be a collection source. It completes as this loop's
            // `Bool` condition at the body root, exactly like a concrete condition.
            let bool_type_id = parser.type_interner.environment().builtins().bool;
            let condition = retain_loop_condition(deferral, bool_type_id);
            return Ok(ParsedLoopHeader::Conditional { condition });
        }
    };

    let item_type_id = parser
        .type_interner
        .environment()
        .collection_element_type(expression.type_id);

    if let Some(item_type_id) = item_type_id {
        let bindings = declare_loop_bindings(None, item_type_id, parser)?;
        return Ok(ParsedLoopHeader::Collection {
            bindings: Box::new(bindings),
            iterable: StatementExpression::Built(expression),
            pending_item: None,
        });
    }

    ensure_loop_condition(&expression, parser.type_interner.environment())?;
    Ok(ParsedLoopHeader::Conditional {
        condition: StatementExpression::Built(expression),
    })
}

fn reject_removed_in_loop_syntax_cursor(
    token_stream: &AstCursor,
    header_start: usize,
    header_end: usize,
    string_table: &mut StringTable,
) -> LoopHeaderResult<()> {
    if header_end.saturating_sub(header_start) < 3 {
        return Ok(());
    }

    if token_tag_at(token_stream, header_start) != Some(TokenTag::SYMBOL)
        || token_tag_at(token_stream, header_start + 1) != Some(TokenTag::SYMBOL)
    {
        return Ok(());
    }
    let Some(second_symbol) = token_string_id_at(token_stream, header_start + 1, string_table)?
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
    string_table: &mut StringTable,
) -> LoopHeaderResult<Option<CursorBindingSuffixSplit>> {
    let pipe_indices =
        collect_top_level_cursor_indexes(token_stream, header_start, header_end, |tag| {
            tag == TokenTag::TYPE_PARAMETER_BRACKET
        })?;
    if pipe_indices.is_empty() {
        return Ok(None);
    }

    let last_index = header_end.checked_sub(1);
    if last_index
        .and_then(|index| token_tag_at(token_stream, index))
        .is_none_or(|tag| tag != TokenTag::TYPE_PARAMETER_BRACKET)
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
    let bindings = parse_binding_cursor(
        token_stream,
        open_pipe_index + 1,
        close_pipe_index,
        string_table,
    )?;

    Ok(Some(CursorBindingSuffixSplit {
        core_end: open_pipe_index,
        bindings,
    }))
}
fn parse_binding_cursor(
    token_stream: &AstCursor,
    binding_start: usize,
    binding_end: usize,
    string_table: &mut StringTable,
) -> LoopHeaderResult<ParsedBindingNames> {
    let binding_indices = (binding_start..binding_end)
        .filter(|index| token_tag_at(token_stream, *index) != Some(TokenTag::NEWLINE))
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
        let token_tag = token_tag_at(token_stream, token_index)
            .expect("binding index was collected from a readable cursor token");
        if token_tag == TokenTag::THIS {
            return loop_header_error(
                InvalidLoopHeaderReason::ThisBinding,
                token_stream.span_at(token_index),
            );
        }
        if token_tag != TokenTag::SYMBOL {
            return loop_header_error(
                InvalidLoopHeaderReason::BindingMustBeSymbol,
                token_stream.span_at(token_index),
            );
        }
        let Some(symbol_id) = token_string_id_at(token_stream, token_index, string_table)? else {
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
        if token_tag_at(token_stream, separator_index) != Some(TokenTag::COMMA) {
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
) -> Result<Option<CursorBareLoopBindingSuffix>, CompilerError> {
    let non_newline_indices =
        collect_top_level_cursor_indexes(token_stream, header_start, header_end, |tag| {
            tag != TokenTag::NEWLINE
        })?;
    if non_newline_indices.len() < 2 {
        return Ok(None);
    }

    if non_newline_indices.len() >= 3 {
        let first_index = non_newline_indices[non_newline_indices.len() - 3];
        let separator_index = non_newline_indices[non_newline_indices.len() - 2];
        let second_index = non_newline_indices[non_newline_indices.len() - 1];
        let first_is_symbol = token_tag_at(token_stream, first_index) == Some(TokenTag::SYMBOL);
        let separator_is_comma =
            token_tag_at(token_stream, separator_index) == Some(TokenTag::COMMA);
        let separator_is_symbol =
            token_tag_at(token_stream, separator_index) == Some(TokenTag::SYMBOL);
        let second_is_symbol = token_tag_at(token_stream, second_index) == Some(TokenTag::SYMBOL);

        if first_is_symbol && separator_is_comma && second_is_symbol && first_index > header_start {
            return Ok(Some(CursorBareLoopBindingSuffix {
                core_end: first_index,
                span: token_stream.span_at(first_index),
                kind: BareLoopBindingKind::Dual,
            }));
        }

        if first_is_symbol
            && separator_is_symbol
            && second_is_symbol
            && separator_index > header_start
        {
            return Ok(Some(CursorBareLoopBindingSuffix {
                core_end: separator_index,
                span: token_stream.span_at(separator_index),
                kind: BareLoopBindingKind::Dual,
            }));
        }
    }

    let Some(binding_index) = non_newline_indices.last().copied() else {
        return Ok(None);
    };
    let core_tail_index = non_newline_indices[non_newline_indices.len() - 2];
    if token_tag_at(token_stream, binding_index) == Some(TokenTag::SYMBOL)
        && token_tag_at(token_stream, core_tail_index) != Some(TokenTag::COMMA)
    {
        return Ok(Some(CursorBareLoopBindingSuffix {
            core_end: binding_index,
            span: token_stream.span_at(binding_index),
            kind: BareLoopBindingKind::Single,
        }));
    }

    Ok(None)
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
    input: RangeLoopSpecInput<'_, '_, '_, '_>,
) -> LoopHeaderResult<ParsedRangeSpec> {
    let RangeLoopSpecInput {
        token_stream,
        range_start,
        range_end,
        context,
        type_interner,
        string_table,
        path_fork,
    } = input;
    let saved_position = token_stream.position();
    let result = (|| {
        token_stream
            .set_position(range_start)
            .map_err(ExpressionParseError::from)?;

        let (start_operand, omitted_start_span) =
            if token_stream.current_tag() == TokenTag::EXCLUSIVE_RANGE {
                (None, Some(token_stream.current_span()))
            } else {
                (
                    Some(range_operand_from_result(parse_cursor_expression_until(
                        CursorExpressionUntilInput {
                            token_stream,
                            expression_start: range_start,
                            expression_end: range_end,
                            context,
                            type_interner,
                            value_mode: &ValueMode::ImmutableReference,
                            string_table,
                            path_fork,
                            stop_tokens: &[TokenTag::EXCLUSIVE_RANGE],
                        },
                    ))?),
                    None,
                )
            };

        let start_span =
            || omitted_start_span.or_else(|| start_operand.as_ref().and_then(range_slot_span));

        let end_kind = match token_stream.current_tag() {
            TokenTag::EXCLUSIVE_RANGE => {
                token_stream.advance();
                if token_stream.current_tag() == TokenTag::AMPERSAND {
                    token_stream.advance();
                    RangeEndKind::Inclusive
                } else {
                    RangeEndKind::Exclusive
                }
            }
            TokenTag::EOF if token_stream.position() >= range_end => {
                return loop_header_error(
                    InvalidLoopHeaderReason::MissingRangeSeparator,
                    start_span(),
                );
            }
            _ => {
                return loop_header_error(
                    InvalidLoopHeaderReason::MissingRangeSeparator,
                    Some(token_stream.current_span()),
                );
            }
        };

        if token_stream.position() >= range_end || token_stream.current_tag() == TokenTag::EOF {
            return loop_header_error(InvalidLoopHeaderReason::MissingRangeEndBound, start_span());
        }

        let end_start = token_stream.position();
        let by_index =
            find_top_level_expression_boundary_cursor_token(token_stream, end_start, range_end)?;
        let end_operand = if by_index.is_some() {
            range_operand_from_result(parse_cursor_expression_until(CursorExpressionUntilInput {
                token_stream,
                expression_start: end_start,
                expression_end: range_end,
                context,
                type_interner,
                value_mode: &ValueMode::ImmutableReference,
                string_table,
                path_fork,
                stop_tokens: &[TokenTag::BY],
            }))?
        } else {
            range_operand_from_result(parse_cursor_expression(
                CursorExpressionInput {
                    token_stream,
                    expression_start: end_start,
                    expression_end: range_end,
                    context,
                    type_interner,
                    value_mode: &ValueMode::ImmutableReference,
                    string_table,
                    path_fork,
                },
                false,
            ))?
        };

        let step_operand = if by_index.is_some() {
            let by_span = Some(token_stream.current_span());
            token_stream.advance();
            if token_stream.position() >= range_end || token_stream.current_tag() == TokenTag::EOF {
                return loop_header_error(InvalidLoopHeaderReason::MissingRangeStep, by_span);
            }
            let step_start = token_stream.position();
            Some(range_operand_from_result(parse_cursor_expression(
                CursorExpressionInput {
                    token_stream,
                    expression_start: step_start,
                    expression_end: range_end,
                    context,
                    type_interner,
                    value_mode: &ValueMode::ImmutableReference,
                    string_table,
                    path_fork,
                },
                false,
            ))?)
        } else {
            None
        };

        Ok(ParsedRangeSpec {
            start: start_operand,
            end: end_operand,
            end_kind,
            step: step_operand,
            span: omitted_start_span,
        })
    })();

    token_stream
        .set_position(saved_position)
        .map_err(ExpressionParseError::from)?;
    result
}

/// Keep one parsed range operand as its authored slot, retaining a numeric-dependent fragment.
///
/// WHAT: a built operand keeps its completed expression; a deferral keeps its exact ordered
///       fragment with the position's own authority over the whole fragment.
/// WHY: a range operand's domain is selected at the body root from every permitted use of the
///      origins it references, so the slot must survive in its authored form. No receiving
///      witness is registered on the origins here: the header's counter is a forward result of
///      the selected operand domains, never a constraint that selects them.
fn range_operand_from_result(
    result: LoopHeaderResult<Expression>,
) -> LoopHeaderResult<StatementExpression> {
    match result {
        Ok(expression) => Ok(StatementExpression::Built(expression)),
        Err(error) => {
            let deferral = error.take_numeric_deferral("a loop range operand")?;
            Ok(StatementExpression::Retained(deferred_numeric_expression(
                deferral.items,
                ExpectedType::Infer,
                deferral.value_mode,
                deferral.span,
                deferral.suffix,
            )))
        }
    }
}

/// Register one deferred range header's counter and positional index binding.
///
/// WHAT: the counter becomes a pending numeric row whose `RangeCounter` recipe owns the header's
///       authored operand slots, and its name is inserted into the loop frame as a pending slot;
///       the index binding is always the concrete `I32` declaration.
/// WHY: the counter's type is a forward result of the range's completed operand domains, so no
///      binding type can be selected while an operand is still retained. The row keeps the
///      counter visible to the body — and resolves before any binding that reads it — without a
///      fabricated `TypeId`, an early default witness on the endpoints, or a dependency on the
///      loop construct's own body resumption.
fn declare_retained_range_counter(
    binding_names: ParsedBindingNames,
    range: DeferredRangeSpec,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<RetainedRangeCounter> {
    validate_loop_binding_name(&binding_names.item, parser)?;

    let path = parser
        .path_fork
        .try_intern_child(parser.scope_context.scope, binding_names.item.id)
        .expect("path table exhausted while declaring a loop counter");
    let counter = parser
        .scope_context
        .declare_pending_numeric_binding(
            AuthoredBindingFacts {
                name: binding_names.item.id,
                path,
                span: binding_names.item.span,
                binding_span: binding_names.item.span,
                value_mode: ValueMode::ImmutableOwned,
                mutable: false,
            },
            NumericRecipe::RangeCounter {
                range: Box::new(range),
            },
        )
        .map_err(ExpressionParseError::from)?;

    let index = binding_names
        .index
        .as_ref()
        .map(|index_name| {
            declare_loop_binding(
                index_name,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                parser,
            )
        })
        .transpose()?;

    Ok(RetainedRangeCounter { counter, index })
}

/// The authored span of one range operand slot.
fn range_slot_span(slot: &StatementExpression) -> Option<SourceSpan> {
    match slot {
        StatementExpression::Built(expression) => expression.span,
        StatementExpression::Retained(fragment) => fragment.span,
    }
}

/// Build one range loop spec from operands that are already complete.
///
/// WHAT: the single shared range policy — derive the domain from the completed operand types,
///       synthesise an omitted start at that domain, then apply the start/end/step compatibility,
///       float-step and zero-step rules.
/// WHY: a range header whose operands referenced pending numeric origins completes at the body
///      root, and the concrete parse path needs exactly the same domain selection and checks, so
///      both callers share one implementation instead of a second policy in the completion driver.
#[allow(
    clippy::too_many_arguments,
    reason = "the shared range policy keeps the four authored operand slots, the omitted-start span and the parser services as separate inputs"
)]
pub(crate) fn range_loop_spec_from_completed_operands(
    start: Option<Expression>,
    end: Expression,
    end_kind: RangeEndKind,
    step: Option<Expression>,
    omitted_start_span: Option<SourceSpan>,
    _context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    _string_table: &mut StringTable,
    _path_fork: &mut PathInternerFork,
) -> LoopHeaderResult<RangeLoopSpec> {
    let type_environment = type_interner.environment();
    let domain = range_numeric_domain(start.as_ref(), &end, step.as_ref(), type_environment)?;
    if domain.is_binary_float() && step.is_none() {
        return loop_header_error(InvalidLoopHeaderReason::FloatRangeMissingStep, end.span);
    }
    if let Some(step_expression) = &step
        && is_zero_numeric_literal(step_expression)
    {
        return loop_header_error(InvalidLoopHeaderReason::ZeroRangeStep, step_expression.span);
    }

    let start = match start {
        Some(start) => start,
        None => {
            let end_domain = NumericScalar::from_type_id(end.type_id, type_environment)
                .expect("range numeric validation ensures the end has a numeric domain");
            contextual_range_zero(end_domain, omitted_start_span)
        }
    };

    Ok(RangeLoopSpec {
        start,
        end,
        end_kind,
        step,
    })
}

fn range_operand_domain(
    expression: &Expression,
    kind: RangeOperandKind,
    type_environment: &TypeEnvironment,
) -> LoopHeaderResult<NumericScalar> {
    let domain = NumericScalar::from_type_id(expression.type_id, type_environment);
    match domain {
        Some(domain) if domain.is_integer() || domain.is_binary_float() => Ok(domain),
        _ => Err(CompilerDiagnostic::invalid_range_operand(
            kind,
            expression.type_id,
            InvalidRangeOperandReason::NotNumeric,
            expression.span,
        )
        .into()),
    }
}

fn range_numeric_domain(
    start: Option<&Expression>,
    end: &Expression,
    step: Option<&Expression>,
    type_environment: &TypeEnvironment,
) -> LoopHeaderResult<NumericScalar> {
    let end_domain = range_operand_domain(end, RangeOperandKind::End, type_environment)?;
    let start_domain = match start {
        Some(expression) => {
            range_operand_domain(expression, RangeOperandKind::Start, type_environment)?
        }
        None => end_domain,
    };
    let step_domain = step
        .map(|step_expression| {
            range_operand_domain(step_expression, RangeOperandKind::Step, type_environment)
        })
        .transpose()?;
    let Some(domain) = range_iteration_domain(start_domain, end_domain, step_domain) else {
        // Applying the selector to the bounds alone tells whether the explicit step introduced
        // the incompatibility or the bounds already disagreed.
        let (operand, offender) = match step {
            Some(step_expression)
                if range_iteration_domain(start_domain, end_domain, None).is_some() =>
            {
                (RangeOperandKind::Step, step_expression)
            }
            _ => (RangeOperandKind::End, end),
        };
        return Err(CompilerDiagnostic::invalid_range_operand(
            operand,
            offender.type_id,
            InvalidRangeOperandReason::IncompatibleDomain,
            offender.span,
        )
        .into());
    };

    Ok(domain)
}

fn contextual_range_zero(domain: NumericScalar, span: Option<SourceSpan>) -> Expression {
    match domain {
        NumericScalar::Fixed(scalar) => {
            let zero = match scalar.class() {
                FixedScalarClass::SignedInteger => FixedScalarValue::signed(scalar, 0),
                FixedScalarClass::UnsignedInteger => FixedScalarValue::unsigned(scalar, 0),
                FixedScalarClass::BinaryFloat => FixedScalarValue::binary_float(scalar, 0.0),
                FixedScalarClass::Octet => None,
            }
            .expect("numeric range domain has a representable zero");
            Expression::fixed_scalar(zero, span, ValueMode::ImmutableOwned)
        }
        NumericScalar::Number(_) => {
            unreachable!("Number range domains are rejected before contextual zero materialisation")
        }
    }
}

/// Parse one loop-header expression position that owns its whole authored syntax.
///
/// WHAT: every header position (condition, range operand, collection source) is the complete
///       expression at that position, so it owns a numeric deferral raised by the shared parser.
///       `retain_pending_numeric_fragment` keeps a pending literal origin unresolved past this
///       position: a collection source passes true so its item's element class types the literal
///       before body-root defaults run. Conditions and range operands pass false.
/// WHY: the retained fragment must be spliceable verbatim at the body root, which is only legal
///      when the position owned the whole fragment instead of swallowing enclosing structure.
fn parse_cursor_expression(
    input: CursorExpressionInput<'_, '_, '_, '_>,
    retain_pending_numeric_fragment: bool,
) -> LoopHeaderResult<Expression> {
    let CursorExpressionInput {
        token_stream,
        expression_start,
        expression_end,
        context,
        type_interner,
        value_mode,
        string_table,
        path_fork,
    } = input;
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
        )
        .with_numeric_deferral_owner();
        let input = if retain_pending_numeric_fragment {
            input.with_retained_pending_numeric_fragment()
        } else {
            input
        };
        create_expression_with_trailing_newline_policy(input)
    })
}

fn parse_cursor_expression_until(
    input: CursorExpressionUntilInput<'_, '_, '_, '_, '_>,
) -> LoopHeaderResult<Expression> {
    let CursorExpressionUntilInput {
        token_stream,
        expression_start,
        expression_end,
        context,
        type_interner,
        value_mode,
        string_table,
        path_fork,
        stop_tokens,
    } = input;
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
        )
        .with_numeric_deferral_owner();
        create_expression_until(input, stop_tokens)
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

/// Retain one conditional loop condition whose fragment still references a pending origin.
///
/// WHAT: takes the complete deferral and records the exact `Bool` receiving expectation.
/// WHY: a conditional loop's condition is a `Bool` predicate; the body root completes the
///      fragment and validates the resolved expression against `Bool`, exactly like the
///      concrete path validates a built condition.
fn retain_loop_condition(deferral: NumericDeferral, bool_type_id: TypeId) -> StatementExpression {
    StatementExpression::Retained(deferred_numeric_expression(
        deferral.items,
        ExpectedType::Known(bool_type_id),
        deferral.value_mode,
        deferral.span,
        deferral.suffix,
    ))
}

/// The single pending binding a retained loop source names, following authored groups.
///
/// WHAT: returns the binding only when the whole source is one pending numeric binding read.
/// WHY: a collection item is a fact about one named source; a larger expression has no single
///      element class to share.
fn pending_source_binding(items: &[ExpressionRpnItem]) -> Option<NumericBindingId> {
    match items {
        [ExpressionRpnItem::PendingNumericBinding { binding, .. }] => Some(*binding),
        [ExpressionRpnItem::PendingGroup { nodes, .. }] => pending_source_binding(nodes),
        _ => None,
    }
}

/// The pending collection node a retained loop source names, when it has one.
///
/// WHAT: a named pending binding reads the collection node of its own recipe. An inline literal
///       fragment reads its own node. Any other single operand reads the known result shape its
///       retained operand carries, so a projected element such as `grid.get(0)!` names its
///       collection node and that node's element class. Other sources answer `None`.
/// WHY: a pending collection source shares its element class with the loop item, so only a
///      collection node can be one; any other pending source keeps the ordinary retained path.
///      The operand's result is derived before the state borrow, because a concrete result
///      interns into that state.
fn pending_collection_source(
    items: &[ExpressionRpnItem],
    context: &ScopeContext,
    environment: &TypeEnvironment,
) -> LoopHeaderResult<Option<KnownShapeId>> {
    if let Some(binding) = pending_source_binding(items) {
        let state = context.numeric_state();
        return Ok(pending_collection_shape(&state, binding));
    }
    let node = match retained_known_shape(items) {
        Some(shape) => Some(shape),
        None => match items {
            [single] => retained_operand_result_shape(single, context, environment, false)?,
            _ => None,
        },
    };
    let state = context.numeric_state();
    Ok(node.filter(|shape| matches!(state.shape(*shape), KnownShape::Collection { .. })))
}

/// Declare the item of a pending collection loop and its concrete positional index.
///
/// WHAT: the item becomes a pending row whose element class is the source literal's, and its name
///       enters the loop frame as a pending slot; the index is the concrete `I32` declaration.
/// WHY: the item's type is the source's completed element type, which no parse-time fact selects;
///      sharing the source's element class lets body uses and the source's completion agree on
///      one leaf without a fabricated type or default.
fn declare_pending_collection_bindings(
    binding_names: ParsedBindingNames,
    source: KnownShapeId,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<(NumericBindingId, LoopBindings)> {
    validate_loop_binding_name(&binding_names.item, parser)?;
    let path = parser
        .path_fork
        .try_intern_child(parser.scope_context.scope, binding_names.item.id)
        .expect("path table exhausted while declaring a collection item");
    let item = parser.scope_context.declare_pending_collection_element(
        source,
        AuthoredBindingFacts {
            name: binding_names.item.id,
            path,
            span: binding_names.item.span,
            binding_span: binding_names.item.span,
            value_mode: ValueMode::ImmutableOwned,
            mutable: false,
        },
    )?;

    let index = binding_names
        .index
        .as_ref()
        .map(|index_name| {
            let int_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
            declare_loop_binding(index_name, int_type_id, parser)
        })
        .transpose()?;

    Ok((item, LoopBindings { item: None, index }))
}

/// The concrete result type of one retained collection-source fragment, when it is known.
///
/// WHAT: delegates to the retained-operand postfix owner for the fragment's known result shape,
///       and keeps only a concrete leaf; a construction-only shape names no canonical type yet.
/// WHY: the collection-source decision and its diagnostic must use the same known-shape owner as
///      every other retained operand, so a known collection result keeps its authored source
///      instead of being refused, and no second derivation of call or step result types lives
///      here.
fn retained_known_result_type(
    items: &[ExpressionRpnItem],
    context: &ScopeContext,
    environment: &TypeEnvironment,
) -> LoopHeaderResult<Option<TypeId>> {
    let shape = match items {
        [single] => retained_operand_result_shape(single, context, environment, false)?,
        _ => None,
    };
    Ok(shape.and_then(|shape| concrete_shape_type(context, shape)))
}

/// Report one retained collection source with the ordinary invalid-source diagnostic.
///
/// WHAT: a fragment whose own parsed facts already name a non-collection result is reported with
///       that type; anything else names no type.
/// WHY: an unresolved expression has no completed type and a registered receiving witness is only
///      a target, never the origin's selected type, so the diagnostic never invents one.
fn collection_source_diagnostic(
    deferral: NumericDeferral,
    context: &ScopeContext,
    environment: &TypeEnvironment,
) -> LoopHeaderResult<CompilerDiagnostic> {
    let found_type = retained_known_result_type(&deferral.items, context, environment)?;
    Ok(CompilerDiagnostic::invalid_loop_header(
        InvalidLoopHeaderReason::CollectionSourceNotCollection { found_type },
        deferral.span,
    ))
}

/// Parse one collection loop source into its authored slot plus the element selection it makes.
///
/// WHAT: a source that completes at parse time keeps its built expression; a retained source
///       keeps its exact fragment. A retained call whose parsed signature already names a
///       collection result provides a concrete element type; a pending numeric collection source
///       provides its element descriptor, so the item shares that source's element class.
/// WHY: the loop's item binding is a fact the body consumes, so it must exist before the body
///      parses. A concrete element comes from the declared result, a pending element from the
///      source literal's element class; any other retained fragment reports the source constraint.
fn parse_collection_iterable_cursor(
    input: CursorExpressionInput<'_, '_, '_, '_>,
) -> LoopHeaderResult<CollectionIterable> {
    let CursorExpressionInput {
        token_stream,
        expression_start,
        expression_end,
        context,
        type_interner,
        value_mode,
        string_table,
        path_fork,
    } = input;
    debug_assert_eq!(
        value_mode,
        &ValueMode::ImmutableReference,
        "loop collection headers parse as immutable references",
    );
    let collection_expression = match parse_cursor_expression(
        CursorExpressionInput {
            token_stream,
            expression_start,
            expression_end,
            context,
            type_interner,
            value_mode,
            string_table,
            path_fork,
        },
        true,
    ) {
        Ok(expression) => expression,
        Err(error) => {
            let deferral = error.take_numeric_deferral("a loop collection source")?;
            if let Some(source) =
                pending_collection_source(&deferral.items, context, type_interner.environment())?
            {
                return Ok(CollectionIterable::Pending {
                    iterable: StatementExpression::Retained(deferred_numeric_expression(
                        deferral.items,
                        ExpectedType::Infer,
                        deferral.value_mode,
                        deferral.span,
                        deferral.suffix,
                    )),
                    source,
                });
            }
            let retained_element_type =
                retained_known_result_type(&deferral.items, context, type_interner.environment())?
                    .and_then(|return_type| {
                        type_interner
                            .environment()
                            .collection_element_type(return_type)
                    });
            let Some(item_type_id) = retained_element_type else {
                return Err(collection_source_diagnostic(
                    deferral,
                    context,
                    type_interner.environment(),
                )?
                .into());
            };
            return Ok(CollectionIterable::Known {
                iterable: StatementExpression::Retained(deferred_numeric_expression(
                    deferral.items,
                    ExpectedType::Infer,
                    deferral.value_mode,
                    deferral.span,
                    deferral.suffix,
                )),
                item_type: item_type_id,
            });
        }
    };
    let type_environment = type_interner.environment();
    let Some(item_type_id) =
        type_environment.collection_element_type(collection_expression.type_id)
    else {
        return loop_header_error(
            InvalidLoopHeaderReason::CollectionSourceNotCollection {
                found_type: Some(collection_expression.type_id),
            },
            collection_expression.span,
        );
    };
    Ok(CollectionIterable::Known {
        iterable: StatementExpression::Built(collection_expression),
        item_type: item_type_id,
    })
}

fn parses_as_collection_cursor(
    input: CursorExpressionInput<'_, '_, '_, '_>,
) -> LoopHeaderResult<bool> {
    let CursorExpressionInput {
        token_stream,
        expression_start,
        expression_end,
        context,
        type_interner,
        value_mode,
        string_table,
        path_fork,
    } = input;
    debug_assert_eq!(
        value_mode,
        &ValueMode::ImmutableReference,
        "loop collection probes parse as immutable references",
    );
    let expression = match parse_cursor_expression(
        CursorExpressionInput {
            token_stream,
            expression_start,
            expression_end,
            context,
            type_interner,
            value_mode,
            string_table,
            path_fork,
        },
        true,
    ) {
        Ok(expression) => expression,
        Err(ExpressionParseError::Diagnostic(_)) => return Ok(false),
        // A retained source is a collection only when the fragment's own parsed shape already
        // names a collection result; every other retained fragment reports its source constraint
        // from the real header parse instead of this syntax probe.
        Err(ExpressionParseError::NumericDeferral(deferral)) => {
            // A pending collection source and a retained operand whose known result is a collection
            // are the retained shapes that really are collection sources, so the probe must answer
            // like the real header parse.
            let pending_source =
                pending_collection_source(&deferral.items, context, type_interner.environment())?;
            return Ok(pending_source.is_some()
                || retained_known_result_type(
                    &deferral.items,
                    context,
                    type_interner.environment(),
                )?
                .is_some_and(|return_type| {
                    type_interner
                        .environment()
                        .collection_element_type(return_type)
                        .is_some()
                }));
        }
        Err(error) => return Err(error),
    };
    Ok(type_interner
        .environment()
        .collection_element_type(expression.type_id)
        .is_some())
}

fn parses_as_range_cursor(input: RangeLoopSpecInput<'_, '_, '_, '_>) -> LoopHeaderResult<bool> {
    match parse_range_loop_spec_cursor(input) {
        Ok(_) => Ok(true),
        Err(ExpressionParseError::Diagnostic(_)) => Ok(false),
        // A retained numeric operand still leaves the header range-shaped; the real range parse
        // resolves the operand from a registered witness or reports its operand diagnostic.
        Err(ExpressionParseError::NumericDeferral(_)) => Ok(true),
        Err(error) => Err(error),
    }
}

fn has_top_level_range_marker_cursor(
    token_stream: &AstCursor,
    header_start: usize,
    header_end: usize,
) -> Result<bool, CompilerError> {
    Ok(find_top_level_cursor_token(
        token_stream,
        header_start,
        header_end,
        TokenTag::EXCLUSIVE_RANGE,
    )?
    .is_some())
}

/// Short-lived canonical child over the header window `[start, end)`.
///
/// WHAT: positions a walk cursor at `start` and bounds it at `end`.
/// WHY: every loop-header probe below runs inside a canonical header window that already
/// validated these bounds.
fn header_probe_walk<'a>(
    token_stream: &'a AstCursor<'a>,
    start: usize,
    end: usize,
) -> Result<AstCursor<'a>, CompilerError> {
    token_stream.subcursor_window(start, end)
}

fn collect_top_level_cursor_indexes(
    token_stream: &AstCursor,
    start: usize,
    end: usize,
    predicate: impl Fn(TokenTag) -> bool,
) -> Result<Vec<usize>, CompilerError> {
    let mut walk = header_probe_walk(token_stream, start, end)?;
    let mut nesting_depth = NestingDepth::default();
    let mut indexes = Vec::new();
    while !walk.is_at_end() {
        let tag = walk.current_tag();
        if nesting_depth.is_top_level() && predicate(tag) {
            indexes.push(walk.position());
        }
        let is_eof = tag == TokenTag::EOF;
        nesting_depth.step_tag(tag);
        if is_eof {
            break;
        }
        walk.advance();
    }
    Ok(indexes)
}

fn find_top_level_cursor_token(
    token_stream: &AstCursor,
    start: usize,
    end: usize,
    target: TokenTag,
) -> Result<Option<usize>, CompilerError> {
    let mut walk = header_probe_walk(token_stream, start, end)?;
    let mut nesting_depth = NestingDepth::default();
    while !walk.is_at_end() {
        let tag = walk.current_tag();
        if nesting_depth.is_top_level() && tag == target {
            return Ok(Some(walk.position()));
        }
        let is_eof = tag == TokenTag::EOF;
        nesting_depth.step_tag(tag);
        if is_eof {
            break;
        }
        walk.advance();
    }
    Ok(None)
}

/// Find a range-step separator using the same delimiter depth as
/// [`create_expression_until`].
///
/// WHAT: mirrors the expression parser's `ExpressionBoundaryDepth` scan instead of the
/// broader loop-grammar `NestingDepth` scan.
/// WHY: a `By` nested in a template expression must make the same end/step split decision on the
/// canonical cursor path as it does anywhere else in the header.
fn find_top_level_expression_boundary_cursor_token(
    token_stream: &AstCursor,
    start: usize,
    end: usize,
) -> Result<Option<usize>, CompilerError> {
    let mut walk = header_probe_walk(token_stream, start, end)?;
    let mut depth = ExpressionBoundaryDepth::default();
    while !walk.is_at_end() {
        let tag = walk.current_tag();
        if depth.is_top_level() && tag == TokenTag::BY {
            return Ok(Some(walk.position()));
        }
        let is_eof = tag == TokenTag::EOF;
        depth.step_tag(tag);
        if is_eof {
            break;
        }
        walk.advance();
    }
    Ok(None)
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

fn token_tag_at(token_stream: &AstCursor, index: usize) -> Option<TokenTag> {
    token_stream.token_ref_at(index).map(|token| token.tag())
}

fn token_string_id_at(
    token_stream: &AstCursor,
    index: usize,
    string_table: &mut StringTable,
) -> Result<Option<StringId>, CompilerError> {
    if token_tag_at(token_stream, index) != Some(TokenTag::SYMBOL) {
        return Ok(None);
    }
    token_stream.token_string_id_at_in(index, string_table)
}

/// The exact counter domain of one already-built range loop spec.
///
/// WHAT: applies the shared range domain selection to the spec's completed operands.
/// WHY: a retained range header's counter is resolved at the body root, after the shared policy
///      has built and checked the spec, so both the concrete and the resumed path derive the
///      counter's type from that one spec instead of selecting a second time.
pub(crate) fn range_binding_type(
    range: &RangeLoopSpec,
    type_environment: &TypeEnvironment,
) -> LoopHeaderResult<TypeId> {
    Ok(range_numeric_domain(
        Some(&range.start),
        &range.end,
        range.step.as_ref(),
        type_environment,
    )?
    .type_id(type_environment))
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
            let int_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
            declare_loop_binding(index_name, int_type_id, parser)
        })
        .transpose()?;

    Ok(LoopBindings { item, index })
}

/// Apply the ordinary loop-binding name rules to one authored binding name.
///
/// WHAT: rejects reserved/removed keyword shadowing and names already visible in the loop's
///       enclosing scopes, and emits the ordinary value-like naming warning.
/// WHY: a concrete loop binding and a retained pending counter are the same authored name in the
///      same frame, so both declaration paths must enforce exactly these rules.
fn validate_loop_binding_name(
    binding_name: &ParsedBindingName,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<()> {
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

    Ok(())
}

fn declare_loop_binding(
    binding_name: &ParsedBindingName,
    type_id: TypeId,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<Declaration> {
    validate_loop_binding_name(binding_name, parser)?;

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

// Detect literal zero so we can reject `by 0` ranges with a targeted diagnostic.
fn is_zero_numeric_literal(expression: &Expression) -> bool {
    match &expression.kind {
        ExpressionKind::FixedScalar(value) => {
            value.as_i64() == Some(0) || value.as_u64() == Some(0) || value.as_f64() == Some(0.0)
        }
        ExpressionKind::Number(value) => value.to_integer_exact() == Ok(0),
        _ => false,
    }
}
