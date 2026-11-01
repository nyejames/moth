//! Shared loop-header parsing for AST statement and template parsers.
//!
//! WHAT: parses the three body-independent loop header forms after the `loop`
//! keyword has been consumed: conditional, numeric range, and collection iteration.
//! WHY: statement loops and template loop suffixes need the same syntax, binding,
//! and type-validation rules, while each caller owns its own body parsing. Headers
//! parse from the canonical cursor window only.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{
    Declaration, DeclarationValue, LoopBindings, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::const_values::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::{ExpressionParseError, NumericDeferral};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, single_operand,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView, OwnedExpr,
};
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
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    calls_data, choices_control_flow, token_lexical,
};
use crate::compiler_frontend::compiler_messages::{
    InvalidLoopHeaderReason, InvalidLoopOperandReason, LoopOperandKind, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::range_iteration_domain;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::{LocalSpan, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
    reject_reserved_word_token,
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
        /// The optional authored step, evaluated once after the source and its length.
        ///
        /// WHAT: a built signed-integer expression, or a retained fragment whose numeric origin the
        ///       owning body root selects.
        /// WHY: the step is authored after the source and is independent of the item type, so the
        ///      header keeps its own position instead of folding it into the source window.
        step: Option<StatementExpression>,
        /// The authored `by` marker span, present exactly when the header parsed a step.
        ///
        /// WHY: a retained source classifies itself at the body root, and a completed `Bool` there
        ///      is the conditional loop whose step diagnostic belongs at the marker rather than at
        ///      the source.
        step_marker_span: Option<SourceSpan>,
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
/// WHY: a range header's exact domain is a forward result of its bounds' selected types, and a
///      retained bound has no selected type at parse time; the authored slots therefore stay in
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
    ///      the bounds' selected types, so no binding type exists yet.
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
                CompletedRangeOperands {
                    start: start.map(|id| parser.expression_builder.own(id)),
                    end: parser.expression_builder.own(end),
                    end_kind,
                    step: step.map(|id| parser.expression_builder.own(id)),
                    omitted_start_span: span,
                },
                parser.type_interner,
                parser.expression_builder,
                parser.const_values,
            )?;
            // The counter's domain is the spec's selected domain: one policy owns both.
            let binding_type = range_binding_type(&range, parser.type_interner.environment());
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
fn built_range_slot(slot: StatementExpression, position: &str) -> LoopHeaderResult<ExprId> {
    match slot {
        StatementExpression::Built(expression) => Ok(expression),
        StatementExpression::Retained(_) => Err(InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
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
    /// The authored `|...|` pipe range, used when a conditional loop rejects bindings.
    span: Option<SourceSpan>,
}

/// One collection header split at its top-level `by` step marker.
///
/// WHAT: the index of the header's single top-level `by` token.
/// WHY: the source window ends where the step begins, so the split must be decided once and then
///      shared by the source parse, the bare-binding probe and the step parse.
#[derive(Debug, Clone, Copy)]
struct CursorCollectionStepSplit {
    by_index: usize,
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
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
    warnings: &'a mut Vec<DiagnosticDraft>,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
}

struct CursorExpressionInput<'a, 'types, 'cursor, 'tokens> {
    token_stream: &'cursor mut AstCursor<'tokens>,
    expression_start: usize,
    expression_end: usize,
    context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'types>,
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
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
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
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
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
}

/// Loop-header parsing preserves the AST body error lane for canonical header windows.
type LoopHeaderResult<T> = Result<T, ExpressionParseError>;

fn loop_header_error<T>(
    reason: InvalidLoopHeaderReason,
    span: Option<SourceSpan>,
) -> LoopHeaderResult<T> {
    Err(choices_control_flow::invalid_loop_header(reason, span).into())
}

/// Parse the sole loop-header grammar directly from its bounded cursor window.
///
/// WHAT: keeps all header reads on the short-lived `AstCursor` view and represents
/// grammar splits as parser-position ranges rather than materialised payload vectors.
/// WHY: this is the only loop-header parser and consumes canonical source-token views.
#[allow(
    clippy::too_many_arguments,
    reason = "loop header parsing keeps the cursor, scope, warnings and mutable state as separate inputs"
)]
pub(crate) fn parse_loop_header_cursor(
    token_stream: &mut AstCursor,
    mut context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut Vec<DiagnosticDraft>,
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

    let loop_header = {
        let mut parser = LoopHeaderParser {
            scope_context: &mut context,
            type_interner,
            expression_builder,
            const_values,
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
            expression_builder: parser.expression_builder,
            const_values: parser.const_values,
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
            expression_builder: parser.expression_builder,
            const_values: parser.const_values,
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
        expression_builder: parser.expression_builder,
        const_values: parser.const_values,
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
    let pipe_binding_split = parse_pipe_binding_suffix_cursor(
        token_stream,
        header_start,
        header_end,
        parser.string_table,
    )?;
    // A binding suffix ends the header's expression window, and the top-level step separator then
    // splits the remaining source and step windows. The source is parsed first because it is
    // evaluated first and its element metadata declares the loop bindings.
    let (core_end, binding_names, bindings_span) = match pipe_binding_split {
        Some(split) => (split.core_end, Some(split.bindings), split.span),
        None => (header_end, None, None),
    };
    let step_split = collection_step_split_cursor(token_stream, header_start, core_end)?;
    let source_end = step_split.as_ref().map_or(core_end, |split| split.by_index);
    let step_marker_span = match &step_split {
        Some(split) => token_stream.span_at(split.by_index),
        None => None,
    };

    if let Some(binding_names) = binding_names {
        let source = parse_collection_iterable_cursor(
            CursorExpressionInput {
                token_stream,
                expression_start: header_start,
                expression_end: source_end,
                context: parser.scope_context,
                type_interner: parser.type_interner,
                expression_builder: parser.expression_builder,
                const_values: parser.const_values,
                value_mode: &ValueMode::ImmutableReference,
                string_table: parser.string_table,
                path_fork: parser.path_fork,
            },
            bindings_span,
        )?;
        let step = parse_optional_collection_step(
            token_stream,
            step_split.as_ref(),
            core_end,
            parser,
            CollectionStepValidation::AtHeader,
        )?;
        return match source {
            CollectionIterable::Known {
                iterable,
                item_type,
            } => {
                let bindings = declare_loop_bindings(Some(binding_names), item_type, parser)?;
                Ok(ParsedLoopHeader::Collection {
                    bindings: Box::new(bindings),
                    iterable,
                    step,
                    step_marker_span,
                    pending_item: None,
                })
            }
            CollectionIterable::Pending { iterable, source } => {
                let (item, bindings) =
                    declare_pending_collection_bindings(binding_names, source, parser)?;
                Ok(ParsedLoopHeader::Collection {
                    bindings: Box::new(bindings),
                    iterable,
                    step,
                    step_marker_span,
                    pending_item: Some(item),
                })
            }
        };
    }

    let bare_binding_suffix =
        detect_bare_loop_binding_suffix_cursor(token_stream, header_start, source_end)?;
    if let Some(bare_binding_suffix) = bare_binding_suffix
        && parses_as_collection_cursor(CursorExpressionInput {
            token_stream,
            expression_start: header_start,
            expression_end: bare_binding_suffix.core_end,
            context: parser.scope_context,
            type_interner: parser.type_interner,
            expression_builder: parser.expression_builder,
            const_values: parser.const_values,
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
            expression_end: source_end,
            context: parser.scope_context,
            type_interner: parser.type_interner,
            expression_builder: parser.expression_builder,
            const_values: parser.const_values,
            value_mode: &ValueMode::ImmutableOwned,
            string_table: parser.string_table,
            path_fork: parser.path_fork,
        },
        ExpectedType::Infer,
        false,
    ) {
        Ok(expression) => expression,
        Err(error) => {
            let deferral = error.take_numeric_deferral("a loop condition")?;
            let pending_collection = pending_collection_source(
                &deferral.items,
                parser.scope_context,
                parser.type_interner.environment(),
                parser.expression_builder,
            )?
            .is_some();
            // An explicit `by` marks a collection loop, so a source whose finished type this parse
            // cannot classify keeps the collection header and completes at the body root, which
            // reports a `Bool` source at the step marker and any other non-collection at the
            // source. A bare pending collection binding is the same shape with no item binding.
            if !pending_collection && step_split.is_none() {
                // A fragment that still references a pending numeric origin is numeric by
                // construction, so it cannot be a collection source. It completes at the body
                // root, where `ensure_loop_condition` validates it as a `Bool` condition with the
                // condition context rather than as a numeric receiver.
                let condition = retain_loop_condition(deferral);
                return Ok(ParsedLoopHeader::Conditional { condition });
            }
            // Completion reports the source diagnostic before the step's, so the step waits for
            // the source check even when its own expression already built here.
            let step = parse_optional_collection_step(
                token_stream,
                step_split.as_ref(),
                core_end,
                parser,
                CollectionStepValidation::AtCompletion,
            )?;
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
                step,
                step_marker_span,
                pending_item: None,
            });
        }
    };
    let source_view = parser.expression_builder.view(expression)?;
    let source_type = source_view.require_single(ValueReceiverContext::General)?;

    // A `by` step marks an intended collection loop, so the completed-source contract owns this
    // rejection: a `Bool` source points at the step marker and any other non-collection type at
    // the source. A collection source passes and takes the collection header below.
    if step_split.is_some() {
        validate_completed_collection_source(
            source_view,
            parser.type_interner.environment(),
            step_marker_span,
        )?;
    }

    let item_type_id = parser
        .type_interner
        .environment()
        .collection_element_type(source_type);

    if let Some(item_type_id) = item_type_id {
        let bindings = declare_loop_bindings(None, item_type_id, parser)?;
        let step = parse_optional_collection_step(
            token_stream,
            step_split.as_ref(),
            core_end,
            parser,
            CollectionStepValidation::AtHeader,
        )?;
        return Ok(ParsedLoopHeader::Collection {
            bindings: Box::new(bindings),
            iterable: StatementExpression::Built(expression),
            step,
            step_marker_span,
            pending_item: None,
        });
    }

    ensure_loop_condition(source_view, parser.type_interner.environment())?;
    Ok(ParsedLoopHeader::Conditional {
        condition: StatementExpression::Built(expression),
    })
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
        // A complete binding pair followed by a step is the authored step in the wrong place,
        // not a missing closing pipe. The step marker names the construct to move.
        if let [_, close_pipe_index] = pipe_indices.as_slice()
            && let Some(by_index) = find_top_level_cursor_token(
                token_stream,
                close_pipe_index.saturating_add(1),
                header_end,
                TokenTag::BY,
            )?
        {
            return loop_header_error(
                InvalidLoopHeaderReason::MisplacedLoopStep,
                token_stream.span_at(by_index),
            );
        }
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
        span: binding_suffix_span(token_stream, open_pipe_index, close_pipe_index),
    }))
}
/// The authored span covering one binding suffix from its opening to closing pipe.
///
/// WHAT: joins two short inline pipe spans without borrowing an extended-span builder.
/// WHY: the diagnostic underlines the bindings rather than the condition. Pipe tokens are single
///      characters, so their combined range stays inline except for pathological headers, which
///      return `None` so the caller falls back to the condition span.
fn binding_suffix_span(
    token_stream: &AstCursor,
    open_pipe_index: usize,
    close_pipe_index: usize,
) -> Option<SourceSpan> {
    let open_span = token_stream.span_at(open_pipe_index)?;
    let close_span = token_stream.span_at(close_pipe_index)?;
    let start = open_span.local().inline_range()?.start();
    let end = close_span.local().inline_range()?.end();
    let local = LocalSpan::exact_inline(start, end.checked_sub(start)?)?;
    Some(SourceSpan::new(open_span.source(), local))
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
            // A reserved word in a binding slot is an authored name attempt. Name the collision
            // instead of reporting that the slot is not a symbol.
            if let Some(diagnostic) = reject_reserved_word_token(
                token_tag,
                token_stream.span_at(token_index),
                string_table,
            ) {
                return Err(diagnostic.into());
            }

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
) -> Result<Option<CursorBareLoopBindingSuffix>, InfrastructureFailure> {
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
        expression_builder,
        const_values,
        string_table,
        path_fork,
    } = input;
    let saved_position = token_stream.position();
    let result = (|| {
        validate_range_step_markers(token_stream, range_start, range_end)?;
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
                            expression_builder,
                            const_values,
                            value_mode: &ValueMode::ImmutableReference,
                            string_table,
                            path_fork,
                            stop_tokens: &[TokenTag::EXCLUSIVE_RANGE],
                        },
                    ))?),
                    None,
                )
            };

        let start_span = match omitted_start_span {
            Some(span) => Some(span),
            None => match &start_operand {
                Some(slot) => range_slot_span(slot, expression_builder)?,
                None => None,
            },
        };

        let range_marker_span;
        let end_kind = match token_stream.current_tag() {
            TokenTag::EXCLUSIVE_RANGE => {
                range_marker_span = token_stream.current_span();
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
                    start_span,
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
            // The range marker is the authored point where the missing bound belonged.
            return loop_header_error(
                InvalidLoopHeaderReason::MissingRangeEndBound,
                Some(range_marker_span),
            );
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
                expression_builder,
                const_values,
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
                    expression_builder,
                    const_values,
                    value_mode: &ValueMode::ImmutableReference,
                    string_table,
                    path_fork,
                },
                ExpectedType::Infer,
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
            // Bounds that already built select the step's literal context here: a float counter
            // supplies its exact precision, while an integer counter leaves the ordinary `I32`
            // default. Pending bounds keep the literal until body completion, where the resolved
            // bounds supply the same context. The step never joins the bounds' numeric domain.
            let (step_expected, retain_step) = match (&start_operand, &end_operand) {
                (None, StatementExpression::Built(end)) => (
                    range_step_literal_destination(
                        None,
                        *end,
                        expression_builder,
                        type_interner.environment(),
                    )?,
                    false,
                ),
                (Some(StatementExpression::Built(start)), StatementExpression::Built(end)) => (
                    range_step_literal_destination(
                        Some(*start),
                        *end,
                        expression_builder,
                        type_interner.environment(),
                    )?,
                    false,
                ),
                _ => (ExpectedType::Infer, true),
            };
            Some(range_operand_from_result(parse_cursor_expression(
                CursorExpressionInput {
                    token_stream,
                    expression_start: step_start,
                    expression_end: range_end,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    value_mode: &ValueMode::ImmutableReference,
                    string_table,
                    path_fork,
                },
                step_expected,
                retain_step,
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
///      the selected bound domains, never a constraint that selects them.
fn range_operand_from_result(
    result: LoopHeaderResult<ExprId>,
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
/// WHY: the counter's type is a forward result of the range's completed bound domains, so no
///      binding type can be selected while a bound is still retained. The row keeps the
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
        .ok_or_else(token_lexical::logical_path_capacity)?;
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
fn range_slot_span(
    slot: &StatementExpression,
    expression_builder: &AstExpressionBuilder,
) -> LoopHeaderResult<Option<SourceSpan>> {
    match slot {
        StatementExpression::Built(expression) => Ok(expression_builder.view(*expression)?.span()),
        StatementExpression::Retained(fragment) => Ok(fragment.span),
    }
}

/// The authored range operand slots once every one of them is complete.
///
/// WHAT: the completed start, end and step expressions, the authored end kind and the span of an
///       omitted start.
/// WHY: the shared range policy consumes exactly these authored positions, so grouping them keeps
///      the policy's inputs and the parser services it borrows separate without a long signature.
pub(crate) struct CompletedRangeOperands {
    pub(crate) start: Option<OwnedExpr>,
    pub(crate) end: OwnedExpr,
    pub(crate) end_kind: RangeEndKind,
    pub(crate) step: Option<OwnedExpr>,
    pub(crate) omitted_start_span: Option<SourceSpan>,
}

/// Build one range loop spec from operands that are already complete.
///
/// WHAT: the single shared range policy — derive the counter domain from the completed bounds,
///       validate the independent step against that domain, synthesise an omitted start at the
///       counter domain, then apply the float-step and zero-step rules.
/// WHY: a range header whose operands referenced pending numeric origins completes at the body
///      root, and the concrete parse path needs exactly the same domain selection and checks, so
///      both callers share one implementation instead of a second policy in the completion driver.
pub(crate) fn range_loop_spec_from_completed_operands(
    operands: CompletedRangeOperands,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> LoopHeaderResult<RangeLoopSpec> {
    let CompletedRangeOperands {
        start,
        end,
        end_kind,
        step,
        omitted_start_span,
    } = operands;
    let type_environment = type_interner.environment();
    let selection = range_numeric_domain(
        start
            .map(|start| expression_builder.view_owned(start))
            .transpose()?,
        expression_builder.view_owned(end)?,
        step.map(|step| expression_builder.view_owned(step))
            .transpose()?,
        type_environment,
    )?;
    let domain = selection.domain;
    if domain.is_binary_float() && step.is_none() {
        return loop_header_error(
            InvalidLoopHeaderReason::FloatRangeMissingStep,
            expression_builder.view_owned(end)?.span(),
        );
    }
    // The zero check runs before any float-step conversion so a folded `-0.0` remains a constant.
    let step = match step {
        Some(step_id) => {
            if is_zero_numeric_literal(expression_builder.view_owned(step_id)?, const_values) {
                return loop_header_error(
                    InvalidLoopHeaderReason::ZeroLoopStep,
                    expression_builder.view_owned(step_id)?.span(),
                );
            }
            Some(match selection.step_destination {
                Some(destination) => expression_builder.coerced(step_id, destination)?,
                None => step_id,
            })
        }
        None => None,
    };

    let start = match start {
        Some(start) => start,
        None => {
            let end_domain = NumericScalar::from_type_id(
                expression_builder.view_owned(end)?.value_type_id()?,
                type_environment,
            )
            .expect("range numeric validation ensures the end has a numeric domain");
            contextual_range_zero(
                end_domain,
                omitted_start_span,
                expression_builder,
                const_values,
            )?
        }
    };

    Ok(RangeLoopSpec {
        start: start.id(),
        end: end.id(),
        end_kind,
        step: step.map(OwnedExpr::id),
        domain,
    })
}

fn range_operand_domain(
    operand: ExpressionView<'_>,
    kind: LoopOperandKind,
    type_environment: &TypeEnvironment,
) -> LoopHeaderResult<NumericScalar> {
    let operand_type = operand.require_single(ValueReceiverContext::OperatorOperand)?;
    match range_bound_domain(operand, type_environment) {
        Some(domain) => Ok(domain),
        None => Err(calls_data::invalid_loop_operand(
            kind,
            operand_type,
            InvalidLoopOperandReason::BoundNotNumeric,
            operand.span(),
        )
        .into()),
    }
}

/// The counter domain selected by a range's bounds and the conversion its step needs.
///
/// WHAT: the domain comes from start and end alone; the step is then validated against that
///       domain and either keeps its own type (integer counters) or converts to the counter type
///       (binary-float counters).
/// WHY: a step never selects or widens the counter. Keeping both facts in one selection record
///      lets the shared policy apply the step conversion before the finished spec leaves AST.
struct RangeNumericSelection {
    domain: NumericScalar,
    /// The counter type a binary-float step must convert to, when the step's own precision is
    /// narrower. `None` for integer counters and already-matching float steps.
    step_destination: Option<TypeId>,
}

fn range_numeric_domain(
    start: Option<ExpressionView<'_>>,
    end: ExpressionView<'_>,
    step: Option<ExpressionView<'_>>,
    type_environment: &TypeEnvironment,
) -> LoopHeaderResult<RangeNumericSelection> {
    let end_domain = range_operand_domain(end, LoopOperandKind::End, type_environment)?;
    let start_domain = match start {
        Some(operand) => range_operand_domain(operand, LoopOperandKind::Start, type_environment)?,
        None => end_domain,
    };
    let Some(domain) = range_iteration_domain(start_domain, end_domain) else {
        return Err(calls_data::invalid_loop_operand(
            LoopOperandKind::End,
            end.value_type_id()?,
            InvalidLoopOperandReason::BoundIncompatibleDomain,
            end.span(),
        )
        .into());
    };
    let step_destination = step
        .map(|step_operand| range_step_destination(step_operand, domain, type_environment))
        .transpose()?
        .flatten();

    Ok(RangeNumericSelection {
        domain,
        step_destination,
    })
}

/// Validate one range step against the counter domain selected by the bounds.
///
/// WHAT: an integer counter accepts only an independent signed fixed-width integer step, which
///       keeps its own type. A binary-float counter accepts a binary float at the same or narrower
///       precision and reports the counter type the step must convert to.
/// WHY: step direction and magnitude are independent of the counter, so an unsigned, float,
///       `Byte`, `Dec` or non-numeric integer-range step is an authored type error at the step
///       position, never a reason to retype the counter.
fn range_step_destination(
    step: ExpressionView<'_>,
    counter: NumericScalar,
    type_environment: &TypeEnvironment,
) -> LoopHeaderResult<Option<TypeId>> {
    let NumericScalar::Fixed(counter_scalar) = counter else {
        crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "a range counter domain is always a fixed scalar"
        )
    };

    match counter_scalar.class() {
        FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger => {
            require_signed_integer_step(step, type_environment)?;
            Ok(None)
        }

        FixedScalarClass::BinaryFloat => {
            let step_type = step.require_single(ValueReceiverContext::OperatorOperand)?;
            let step_domain = NumericScalar::from_type_id(step_type, type_environment);
            let step_scalar = match step_domain {
                Some(NumericScalar::Fixed(scalar))
                    if scalar.class() == FixedScalarClass::BinaryFloat
                        && scalar.bit_width() <= counter_scalar.bit_width() =>
                {
                    scalar
                }
                _ => {
                    return Err(calls_data::invalid_loop_operand(
                        LoopOperandKind::Step,
                        step_type,
                        InvalidLoopOperandReason::StepNotConvertibleToFloatCounter,
                        step.span(),
                    )
                    .into());
                }
            };
            Ok((step_scalar != counter_scalar).then(|| counter.type_id(type_environment)))
        }

        FixedScalarClass::Octet => crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "a Byte operand has no range counter domain"
        ),
    }
}

/// Require one loop step to be a signed fixed-width integer at its authored position.
///
/// WHAT: returns the step's single type when it is a signed integer, otherwise the shared
///       step-not-signed-integer diagnostic at the step span.
/// WHY: range steps against integer counters and collection steps share one typing rule, so both
///      owners call this predicate instead of restating it.
fn require_signed_integer_step(
    step: ExpressionView<'_>,
    type_environment: &TypeEnvironment,
) -> Result<TypeId, DiagnosticDraft> {
    let step_type = step.require_single(ValueReceiverContext::OperatorOperand)?;
    let is_signed_integer = matches!(
        NumericScalar::from_type_id(step_type, type_environment),
        Some(NumericScalar::Fixed(scalar))
            if scalar.class() == FixedScalarClass::SignedInteger
    );
    if is_signed_integer {
        return Ok(step_type);
    }
    Err(calls_data::invalid_loop_operand(
        LoopOperandKind::Step,
        step_type,
        InvalidLoopOperandReason::StepNotSignedInteger,
        step.span(),
    ))
}

/// The numeric literal context one range step takes from its bounds alone.
///
/// WHAT: a binary-float counter supplies that exact counter type as the step literal's
///       destination; an integer counter supplies no destination, so an untyped whole step
///       literal keeps its independent `I32` default rather than the counter type.
/// WHY: completion drivers complete a retained step after the bounds but before the shared range
///      policy runs. This hint makes an untyped float step materialise at the counter precision
///      without ever joining the bounds' domain; the policy still owns final validation and any
///      conversion. An invalid bound pair yields no hint and is reported by that policy.
pub(crate) fn range_step_literal_destination(
    start: Option<ExprId>,
    end: ExprId,
    expression_builder: &AstExpressionBuilder,
    type_environment: &TypeEnvironment,
) -> LoopHeaderResult<ExpectedType> {
    let end_domain = range_bound_domain(expression_builder.view(end)?, type_environment);
    let start_domain = match start {
        Some(start) => range_bound_domain(expression_builder.view(start)?, type_environment),
        None => end_domain,
    };
    let Some(domain) = start_domain
        .zip(end_domain)
        .and_then(|(start, end)| range_iteration_domain(start, end))
    else {
        return Ok(ExpectedType::Infer);
    };

    if domain.is_binary_float() {
        Ok(ExpectedType::NumericLiteral(
            domain.type_id(type_environment),
        ))
    } else {
        Ok(ExpectedType::Infer)
    }
}

/// One completed range bound's contribution to the counter domain.
///
/// WHAT: returns the bound's integer or binary-float domain, or `None` when the completed value
///       is not a legal bound type.
/// WHY: the shared policy and the literal-step hint must classify bounds identically, so one
///      classifier owns the numeric filter instead of each caller repeating it.
fn range_bound_domain(
    operand: ExpressionView<'_>,
    type_environment: &TypeEnvironment,
) -> Option<NumericScalar> {
    let [type_id] = operand.success_types() else {
        return None;
    };
    let domain = NumericScalar::from_type_id(*type_id, type_environment)?;
    (domain.is_integer() || domain.is_binary_float()).then_some(domain)
}

fn contextual_range_zero(
    domain: NumericScalar,
    span: Option<SourceSpan>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> LoopHeaderResult<OwnedExpr> {
    match domain {
        NumericScalar::Fixed(scalar) => {
            let zero = match scalar.class() {
                FixedScalarClass::SignedInteger => FixedScalarValue::signed(scalar, 0),
                FixedScalarClass::UnsignedInteger => FixedScalarValue::unsigned(scalar, 0),
                FixedScalarClass::BinaryFloat => FixedScalarValue::binary_float(scalar, 0.0),
                FixedScalarClass::Octet => None,
            }
            .expect("numeric range domain has a representable zero");
            Ok(expression_builder.fixed_scalar(
                const_values,
                zero,
                span,
                ValueMode::ImmutableOwned,
            )?)
        }
        NumericScalar::Number(_) => {
            crate::compiler_bug!(
                CompilerStage::AstConstruction,
                "Number range domains are rejected before contextual zero materialisation"
            )
        }
    }
}

/// Parse one loop-header expression position that owns its whole authored syntax.
///
/// WHAT: every header position (condition, range operand, collection source) is the complete
///       expression at that position, so it owns a numeric deferral raised by the shared parser.
///       `retain_pending_numeric_fragment` keeps a pending literal origin unresolved past this
///       position: a collection source passes true so its item's element class types the literal
///       before body-root defaults run, and a range step passes true only when its bounds are
///       still pending, so the resolved bounds can type its literal at body completion. A step
///       whose bounds already built receives that context directly. Conditions and range bounds
///       pass false.
/// WHY: the retained fragment must be spliceable verbatim at the body root, which is only legal
///      when the position owned the whole fragment instead of swallowing enclosing structure.
fn parse_cursor_expression(
    input: CursorExpressionInput<'_, '_, '_, '_>,
    expected_type: ExpectedType,
    retain_pending_numeric_fragment: bool,
) -> LoopHeaderResult<ExprId> {
    let CursorExpressionInput {
        token_stream,
        expression_start,
        expression_end,
        context,
        type_interner,
        expression_builder,
        const_values,
        value_mode,
        string_table,
        path_fork,
    } = input;
    if expression_start >= expression_end {
        return loop_header_error(InvalidLoopHeaderReason::ExpectedHeaderExpression, None);
    }

    parse_cursor_expression_window(token_stream, expression_start, expression_end, |stream| {
        let mut expected_type = expected_type;
        let mut cast_target_context = CastTargetContext::None;
        let input = ExpressionParseInput::without_boundary_catch(
            ExpressionParseResources {
                token_stream: stream,
                scope_context: context,
                type_interner,
                expression_builder,
                const_values,
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
        create_expression_with_trailing_newline_policy(input).map(|expression| expression.id())
    })
}

fn parse_cursor_expression_until(
    input: CursorExpressionUntilInput<'_, '_, '_, '_, '_>,
) -> LoopHeaderResult<ExprId> {
    let CursorExpressionUntilInput {
        token_stream,
        expression_start,
        expression_end,
        context,
        type_interner,
        expression_builder,
        const_values,
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
                expression_builder,
                const_values,
                expected_type: &mut expected_type,
                cast_target_context: &mut cast_target_context,
                value_mode,
                string_table,
                path_fork,
            },
            false,
        )
        .with_numeric_deferral_owner();
        create_expression_until(input, stop_tokens).map(|expression| expression.id())
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
/// WHAT: keeps the fragment's own inferred completion without registering a `Bool` receiver.
/// WHY: a resolved non-`Bool` condition must be reported as a condition mismatch at the body
///      root, not as a numeric receiver conflict, so the loop validates the completed type through
///      the shared condition owner exactly like the concrete path.
fn retain_loop_condition(deferral: NumericDeferral) -> StatementExpression {
    StatementExpression::Retained(deferred_numeric_expression(
        deferral.items,
        ExpectedType::Infer,
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
    match single_operand(items) {
        Some(ExpressionRpnItem::PendingNumericBinding { binding, .. }) => Some(*binding),
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
    expression_builder: &AstExpressionBuilder,
) -> LoopHeaderResult<Option<KnownShapeId>> {
    if let Some(binding) = pending_source_binding(items) {
        let state = context.numeric_state();
        return Ok(pending_collection_shape(&state, binding));
    }
    let node = match retained_known_shape(items) {
        Some(shape) => Some(shape),
        None => match single_operand(items) {
            Some(single) => retained_operand_result_shape(
                single,
                context,
                environment,
                false,
                expression_builder,
            )?,
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
        .ok_or_else(token_lexical::logical_path_capacity)?;
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
    expression_builder: &AstExpressionBuilder,
) -> LoopHeaderResult<Option<TypeId>> {
    let shape = match single_operand(items) {
        Some(single) => {
            retained_operand_result_shape(single, context, environment, false, expression_builder)?
        }
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
    bindings_span: Option<SourceSpan>,
    context: &ScopeContext,
    environment: &TypeEnvironment,
    expression_builder: &AstExpressionBuilder,
) -> LoopHeaderResult<DiagnosticDraft> {
    let found_type =
        retained_known_result_type(&deferral.items, context, environment, expression_builder)?;
    let (reason, span) = if found_type == Some(environment.builtins().bool) {
        (
            InvalidLoopHeaderReason::BindingsOnConditionalLoop,
            bindings_span.or(deferral.span),
        )
    } else {
        (
            InvalidLoopHeaderReason::CollectionSourceNotCollection { found_type },
            deferral.span,
        )
    };
    Ok(choices_control_flow::invalid_loop_header(reason, span))
}

/// Which owner reports one built collection step's type and static-zero violations.
///
/// WHY: a header that classified its source at parse time owns its step diagnostic immediately,
///      while a header whose source completes at the body root must report the source diagnostic
///      first, so completion owns the step check and its source-then-step order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollectionStepValidation {
    AtHeader,
    AtCompletion,
}

/// Parse one collection header's optional `by` step.
///
/// WHAT: the authored step expression when the header carries a `by` marker, otherwise `None`.
/// WHY: every collection header path parses the step after its source and under the same
///      deferral rules, so one helper keeps that position and those rules in a single place.
fn parse_optional_collection_step(
    token_stream: &mut AstCursor,
    step_split: Option<&CursorCollectionStepSplit>,
    core_end: usize,
    parser: &mut LoopHeaderParser<'_, '_>,
    validation: CollectionStepValidation,
) -> LoopHeaderResult<Option<StatementExpression>> {
    match step_split {
        Some(split) => Ok(Some(parse_collection_step_cursor(
            token_stream,
            split.by_index,
            core_end,
            parser,
            validation,
        )?)),
        None => Ok(None),
    }
}

/// Parse one collection loop step after its `by` marker.
///
/// WHAT: the whole authored expression between `by` and the binding suffix or header end.
/// WHY: the step is an independent signed integer expression evaluated once after the source and
///      its length, so it takes the ordinary `I32` numeric default rather than any item context.
///      A fragment that still references a pending numeric origin stays retained for body-root
///      completion, exactly like a range step.
fn parse_collection_step_cursor(
    token_stream: &mut AstCursor,
    by_index: usize,
    core_end: usize,
    parser: &mut LoopHeaderParser<'_, '_>,
    validation: CollectionStepValidation,
) -> LoopHeaderResult<StatementExpression> {
    let by_span = token_stream.span_at(by_index);
    if by_index + 1 >= core_end {
        return loop_header_error(InvalidLoopHeaderReason::MissingCollectionStep, by_span);
    }

    let slot = match parse_cursor_expression(
        CursorExpressionInput {
            token_stream,
            expression_start: by_index + 1,
            expression_end: core_end,
            context: parser.scope_context,
            type_interner: parser.type_interner,
            expression_builder: parser.expression_builder,
            const_values: parser.const_values,
            value_mode: &ValueMode::ImmutableReference,
            string_table: parser.string_table,
            path_fork: parser.path_fork,
        },
        ExpectedType::Infer,
        false,
    ) {
        Ok(expression) => StatementExpression::Built(expression),
        Err(error) => {
            let deferral = error.take_numeric_deferral("a loop collection step")?;
            StatementExpression::Retained(deferred_numeric_expression(
                deferral.items,
                ExpectedType::Infer,
                deferral.value_mode,
                deferral.span,
                deferral.suffix,
            ))
        }
    };

    if validation == CollectionStepValidation::AtHeader
        && let StatementExpression::Built(expression) = &slot
    {
        validate_collection_step(
            parser.expression_builder.view(*expression)?,
            parser.type_interner.environment(),
            parser.const_values,
        )?;
    }
    Ok(slot)
}

/// Validate one built collection loop step.
///
/// WHAT: the step must be a signed fixed-width integer (`I8` to `I64`), and a statically known
///       zero, including `-0`, is rejected at the step's authored span.
/// WHY: the step is independent of the item type and of any counter domain, so one shared check
///      owns the type and static-zero rules for the parse-time and deferred completion paths.
///      The diagnostic family is the same one range steps use, at the authored step position.
pub(crate) fn validate_collection_step(
    step: ExpressionView<'_>,
    type_environment: &TypeEnvironment,
    const_values: &ConstValueStore,
) -> Result<(), DiagnosticDraft> {
    require_signed_integer_step(step, type_environment)?;
    if is_zero_numeric_literal(step, const_values) {
        return Err(choices_control_flow::invalid_loop_header(
            InvalidLoopHeaderReason::ZeroLoopStep,
            step.span(),
        ));
    }
    Ok(())
}

/// Validate one completed collection loop source.
///
/// WHAT: succeeds for a collection, reports `StepOnConditionalLoop` at the authored `by` marker
///       when the source completed to `Bool`, and otherwise reports the ordinary invalid-source
///       diagnostic at the completed expression's span.
/// WHY: a header whose source still references pending numeric origins defers the classification to
///      the body root, so both completion owners — statement AST and template TIR — must apply the
///      same contract the concrete parse path applies, including where each diagnostic points.
pub(crate) fn validate_completed_collection_source(
    source: ExpressionView<'_>,
    type_environment: &TypeEnvironment,
    step_marker_span: Option<SourceSpan>,
) -> Result<(), DiagnosticDraft> {
    let source_type = source.require_single(ValueReceiverContext::General)?;
    if type_environment
        .collection_element_type(source_type)
        .is_some()
    {
        return Ok(());
    }
    if let Some(step_marker_span) = step_marker_span
        && source_type == type_environment.builtins().bool
    {
        return Err(choices_control_flow::invalid_loop_header(
            InvalidLoopHeaderReason::StepOnConditionalLoop,
            Some(step_marker_span),
        ));
    }
    Err(choices_control_flow::invalid_loop_header(
        InvalidLoopHeaderReason::CollectionSourceNotCollection {
            found_type: Some(source_type),
        },
        source.span(),
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
    bindings_span: Option<SourceSpan>,
) -> LoopHeaderResult<CollectionIterable> {
    let CursorExpressionInput {
        token_stream,
        expression_start,
        expression_end,
        context,
        type_interner,
        expression_builder,
        const_values,
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
            expression_builder,
            const_values,
            value_mode,
            string_table,
            path_fork,
        },
        ExpectedType::Infer,
        true,
    ) {
        Ok(expression) => expression,
        Err(error) => {
            let deferral = error.take_numeric_deferral("a loop collection source")?;
            if let Some(source) = pending_collection_source(
                &deferral.items,
                context,
                type_interner.environment(),
                expression_builder,
            )? {
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
            let retained_element_type = retained_known_result_type(
                &deferral.items,
                context,
                type_interner.environment(),
                expression_builder,
            )?
            .and_then(|return_type| {
                type_interner
                    .environment()
                    .collection_element_type(return_type)
            });
            let Some(item_type_id) = retained_element_type else {
                return Err(collection_source_diagnostic(
                    deferral,
                    bindings_span,
                    context,
                    type_interner.environment(),
                    expression_builder,
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
    let collection_view = expression_builder.view(collection_expression)?;
    let source_type = collection_view.require_single(ValueReceiverContext::General)?;
    let Some(item_type_id) = type_environment.collection_element_type(source_type) else {
        if source_type == type_environment.builtins().bool {
            return loop_header_error(
                InvalidLoopHeaderReason::BindingsOnConditionalLoop,
                bindings_span.or_else(|| collection_view.span()),
            );
        }
        return loop_header_error(
            InvalidLoopHeaderReason::CollectionSourceNotCollection {
                found_type: Some(source_type),
            },
            collection_view.span(),
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
        expression_builder,
        const_values,
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
            expression_builder,
            const_values,
            value_mode,
            string_table,
            path_fork,
        },
        ExpectedType::Infer,
        true,
    ) {
        Ok(expression) => expression,
        // Capacity exhaustion is not a rejected cursor shape: it must reach the module build.
        Err(ExpressionParseError::Diagnostic(diagnostic))
            if diagnostic.code() != "MOTH-SYNTAX-0037" =>
        {
            return Ok(false);
        }
        // A retained source is a collection only when the fragment's own parsed shape already
        // names a collection result; every other retained fragment reports its source constraint
        // from the real header parse instead of this syntax probe.
        Err(ExpressionParseError::NumericDeferral(deferral)) => {
            // A pending collection source and a retained operand whose known result is a collection
            // are the retained shapes that really are collection sources, so the probe must answer
            // like the real header parse.
            let pending_source = pending_collection_source(
                &deferral.items,
                context,
                type_interner.environment(),
                expression_builder,
            )?;
            return Ok(pending_source.is_some()
                || retained_known_result_type(
                    &deferral.items,
                    context,
                    type_interner.environment(),
                    expression_builder,
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
    Ok(expression_builder
        .view(expression)?
        .success_shape()
        .single()
        .is_some_and(|source_type| {
            type_interner
                .environment()
                .collection_element_type(source_type)
                .is_some()
        }))
}

fn parses_as_range_cursor(input: RangeLoopSpecInput<'_, '_, '_, '_>) -> LoopHeaderResult<bool> {
    match parse_range_loop_spec_cursor(input) {
        Ok(_) => Ok(true),
        // Capacity exhaustion is not a rejected cursor shape: it must reach the module build.
        Err(ExpressionParseError::Diagnostic(diagnostic))
            if diagnostic.code() != "MOTH-SYNTAX-0037" =>
        {
            Ok(false)
        }
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
) -> Result<bool, InfrastructureFailure> {
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
) -> Result<AstCursor<'a>, InfrastructureFailure> {
    token_stream.subcursor_window(start, end)
}

fn collect_top_level_cursor_indexes(
    token_stream: &AstCursor,
    start: usize,
    end: usize,
    predicate: impl Fn(TokenTag) -> bool,
) -> Result<Vec<usize>, InfrastructureFailure> {
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
) -> Result<Option<usize>, InfrastructureFailure> {
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

/// Every top-level separator index of one header window, using the expression parser's depth.
///
/// WHAT: a collection header needs the first `by` and proof that no second one is authored.
/// WHY: the source and step windows must follow the same delimiter depth as
///      `create_expression_until`, so a `by` nested in a template or call argument never splits
///      the header.
fn collect_top_level_expression_boundary_cursor_indexes(
    token_stream: &AstCursor,
    start: usize,
    end: usize,
    target: TokenTag,
) -> Result<Vec<usize>, InfrastructureFailure> {
    let mut walk = header_probe_walk(token_stream, start, end)?;
    let mut depth = ExpressionBoundaryDepth::default();
    let mut indexes = Vec::new();
    while !walk.is_at_end() {
        let tag = walk.current_tag();
        if depth.is_top_level() && tag == target {
            indexes.push(walk.position());
        }
        let is_eof = tag == TokenTag::EOF;
        depth.step_tag(tag);
        if is_eof {
            break;
        }
        walk.advance();
    }
    Ok(indexes)
}

/// Split one collection header at its single top-level `by`.
///
/// WHAT: no `by` leaves the whole window as the source; one `by` yields the step split; a second
///      top-level `by` is reported at that authored token.
/// WHY: the collection step is optional, so the header must distinguish an omitted step from a
///      repeated marker instead of letting the expression parser blame the step expression.
fn collection_step_split_cursor(
    token_stream: &AstCursor,
    header_start: usize,
    core_end: usize,
) -> LoopHeaderResult<Option<CursorCollectionStepSplit>> {
    let by_indices = collect_top_level_expression_boundary_cursor_indexes(
        token_stream,
        header_start,
        core_end,
        TokenTag::BY,
    )?;
    match by_indices.as_slice() {
        [] => Ok(None),
        [by_index] => Ok(Some(CursorCollectionStepSplit {
            by_index: *by_index,
        })),
        [_, repeated, ..] => loop_header_error(
            InvalidLoopHeaderReason::RepeatedLoopStep,
            token_stream.span_at(*repeated),
        ),
    }
}

/// Reject a misplaced or repeated range step marker before any operand parses.
///
/// WHAT: a top-level `by` before the range marker is misplaced, and a second top-level `by` is a
///       repeated step. The diagnostic points at the offending authored marker.
/// WHY: the start and end operand parsers stop at `to` and `by` only through their own windows, so
///       an early or repeated marker would otherwise surface as a generic expression error from
///       whichever operand swallowed it.
fn validate_range_step_markers(
    token_stream: &AstCursor,
    range_start: usize,
    range_end: usize,
) -> LoopHeaderResult<()> {
    let by_indices = collect_top_level_expression_boundary_cursor_indexes(
        token_stream,
        range_start,
        range_end,
        TokenTag::BY,
    )?;
    let Some(first_by) = by_indices.first().copied() else {
        return Ok(());
    };
    let first_range_marker = collect_top_level_expression_boundary_cursor_indexes(
        token_stream,
        range_start,
        range_end,
        TokenTag::EXCLUSIVE_RANGE,
    )?
    .first()
    .copied();
    let Some(first_range_marker) = first_range_marker else {
        return Ok(());
    };
    if first_by < first_range_marker {
        return loop_header_error(
            InvalidLoopHeaderReason::MisplacedLoopStep,
            token_stream.span_at(first_by),
        );
    }
    if let Some(repeated_by) = by_indices.get(1) {
        return loop_header_error(
            InvalidLoopHeaderReason::RepeatedLoopStep,
            token_stream.span_at(*repeated_by),
        );
    }
    Ok(())
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
) -> Result<Option<usize>, InfrastructureFailure> {
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
) -> Result<Option<StringId>, InfrastructureFailure> {
    if token_tag_at(token_stream, index) != Some(TokenTag::SYMBOL) {
        return Ok(None);
    }
    token_stream.token_string_id_at_in(index, string_table)
}

/// The exact counter domain of one already-built range loop spec.
///
/// WHAT: returns the counter's canonical type from the spec's bounds-selected domain.
/// WHY: a retained range header's counter is resolved at the body root, after the shared policy
///      has built and checked the spec, so both the concrete and the resumed path read that one
///      selected domain instead of selecting a second time from the spec's bounds.
pub(crate) fn range_binding_type(
    range: &RangeLoopSpec,
    type_environment: &TypeEnvironment,
) -> TypeId {
    range.domain.type_id(type_environment)
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
/// WHAT: applies keyword-shadow rejection, visible-name redeclaration rejection and the
///       value-like naming warning to one loop binding name.
/// WHY: a concrete loop binding and a retained pending counter are the same authored name in the
///      same frame, so both declaration paths must enforce exactly these rules.
fn validate_loop_binding_name(
    binding_name: &ParsedBindingName,
    parser: &mut LoopHeaderParser<'_, '_>,
) -> LoopHeaderResult<()> {
    ensure_not_keyword_shadow_identifier(binding_name.id, binding_name.span, parser.string_table)?;

    if let Some(existing) = parser.scope_context.get_reference(&binding_name.id) {
        return Err(parser
            .scope_context
            .duplicate_declaration(binding_name.id, binding_name.span, existing.binding_span())
            .into());
    }

    if let Some(diagnostic) = parser.scope_context.check_body_binding_visibility(
        binding_name.id,
        binding_name.span,
        parser.string_table,
    ) {
        return Err(diagnostic.into());
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

    let binding_span = binding_name.span;
    let declaration = Declaration {
        id: parser
            .path_fork
            .try_intern_child(parser.scope_context.scope, binding_name.id)
            .ok_or_else(token_lexical::logical_path_capacity)?,
        type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span,
        config_qualifier: None,
    };
    let binding_span = declaration.binding_span;
    parser
        .scope_context
        .add_var(declaration.to_owned(), binding_span, parser.path_fork);

    Ok(declaration)
}

// Detect a static zero so `by 0` ranges and collection steps get a targeted diagnostic.
pub(crate) fn is_zero_numeric_literal(
    expression: ExpressionView<'_>,
    const_values: &ConstValueStore,
) -> bool {
    let ExpressionOperationView::Constant(value) = expression.operation() else {
        return false;
    };
    match const_values.payload(value) {
        Some(ConstValuePayload::FixedScalar(scalar)) => {
            scalar.as_i64() == Some(0) || scalar.as_u64() == Some(0) || scalar.as_f64() == Some(0.0)
        }
        Some(ConstValuePayload::Number(number)) => number.to_integer_exact() == Ok(0),
        _ => false,
    }
}
