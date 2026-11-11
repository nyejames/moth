//! Source-directive attachment syntax and operand folding.
//!
//! WHAT: captures one source directive's authored operand range and folds a required
//!       compile-time `String` operand through the ordinary expression/template parser.
//! WHY: header and statement attachment run at different parse stages but must share one operand
//!      contract, one depth rule and one constant-folding lane. Keeping both here prevents a
//!      second directive scanner or a documentation-specific formatter.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::binding::resolve_const_expression_content;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::templates::top_level_templates::AstDocFragment;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, InvalidTemplateDirectiveReason,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::style_directives::{
    SourceDirectiveKind, SourceModifierOperand, StyleDirectiveKind,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TokenCursor, TokenRange, TokenTag,
};
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::utilities::token_scan::NestingDepth;
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;
use std::sync::Arc;

/// One recognised source directive waiting for its target construct.
///
/// WHAT: the directive kind, authored name and span, plus the operand's source-qualified token
///       range.
/// WHY: attachment must validate each directive independently after the whole contiguous run is
///      known. Retaining the range lets the AST stage fold the operand with the same parser and
///      constant store as every other expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceDirectiveAnnotation {
    pub(crate) kind: SourceDirectiveKind,
    pub(crate) name_id: StringId,
    pub(crate) span: SourceSpan,
    pub(crate) operand: Option<TokenRange>,
}

/// Captures one source directive's operand from the token after its name.
///
/// WHAT: at outer source depth, the next `STYLE_DIRECTIVE`, newline or EOF ends the operand; a
///       newline inside balanced syntax stays part of it.
/// WHY: `$doc [$md: ...]` must not treat the nested `$md` as the next outer directive, and a
///       multi-line balanced operand must keep all of its tokens. The caller owns target-line
///       validation after the complete run is captured.
pub(crate) fn capture_source_directive_operand(
    cursor: &mut TokenCursor<'_>,
    operand_contract: SourceModifierOperand,
    name_id: StringId,
    directive_span: SourceSpan,
) -> Result<Option<TokenRange>, DiagnosticDraft> {
    match operand_contract {
        SourceModifierOperand::None => {
            let current = cursor.current();
            let has_operand = current.is_some_and(|token| {
                !matches!(
                    token.tag(),
                    TokenTag::NEWLINE | TokenTag::STYLE_DIRECTIVE | TokenTag::EOF
                )
            });
            if has_operand {
                return Err(templates::invalid_template_directive(
                    Some(name_id),
                    InvalidTemplateDirectiveReason::UnexpectedArguments,
                    current.map(|token| token.source_span()),
                ));
            }
            Ok(None)
        }
        SourceModifierOperand::RequiredSingleString => {
            let start = cursor.position();
            let first = cursor.current();
            if first.is_none_or(|token| {
                matches!(
                    token.tag(),
                    TokenTag::NEWLINE | TokenTag::STYLE_DIRECTIVE | TokenTag::EOF
                )
            }) {
                return Err(templates::invalid_template_directive(
                    Some(name_id),
                    InvalidTemplateDirectiveReason::MissingArgument,
                    Some(directive_span),
                ));
            }
            if first.is_some_and(|token| token.shape().open_parenthesis_is_attached()) {
                return Err(templates::invalid_template_directive(
                    Some(name_id),
                    InvalidTemplateDirectiveReason::UnexpectedArguments,
                    first.map(|token| token.source_span()),
                ));
            }

            let mut depth = NestingDepth::default();
            while let Some(token) = cursor.current() {
                let tag = token.tag();
                if depth.is_top_level()
                    && matches!(
                        tag,
                        TokenTag::NEWLINE | TokenTag::STYLE_DIRECTIVE | TokenTag::EOF
                    )
                {
                    break;
                }
                depth.step_tag(tag);
                cursor.advance();
            }

            let end = cursor.position();
            TokenRange::try_new_for(cursor.source_tokens(), start, end)
                .map(Some)
                .map_err(|_| {
                    templates::invalid_template_directive(
                        Some(name_id),
                        InvalidTemplateDirectiveReason::InvalidArgument { detail: None },
                        Some(directive_span),
                    )
                })
        }
    }
}

/// Resources shared by every source-directive operand fold.
pub(crate) struct SourceOperandFoldResources<'a, 'env> {
    pub(crate) type_interner: &'a mut AstTypeInterner<'env>,
    pub(crate) expression_builder: &'a mut AstExpressionBuilder,
    pub(crate) const_values: &'a mut ConstValueStore,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// Folds one required compile-time `String` source-directive operand.
///
/// WHAT: parses the retained operand through the ordinary expression/template pipeline, requires a
///       compile-time value and returns its final text.
/// WHY: `$doc` and `$todo` share one value contract. A runtime interpolation or a non-`String`
///       expression must fail with the directive's authored span instead of silently producing
///       metadata or an empty string.
pub(crate) fn fold_source_directive_operand(
    annotation: &SourceDirectiveAnnotation,
    source: &Arc<SourceTokens>,
    origin_cursor: Option<&AstCursor<'_>>,
    context: &ScopeContext,
    resources: SourceOperandFoldResources<'_, '_>,
) -> Result<(StringId, SourceSpan), ExpressionParseError> {
    let Some(operand) = annotation.operand else {
        return Err(templates::invalid_template_directive(
            Some(annotation.name_id),
            InvalidTemplateDirectiveReason::MissingArgument,
            Some(annotation.span),
        )
        .into());
    };
    let operand_span = source
        .token(operand.start())
        .map(|token| token.source_span())
        .unwrap_or(annotation.span);

    // Statement directives are parsed from a cursor that may carry a frozen donor payload
    // origin. Keep that origin for the bounded operand parse so imported generic bodies resolve
    // donor string and path handles against their issuing tables, not the requester's.
    let mut cursor = match origin_cursor {
        Some(parent) => parent.nested_cursor(operand).map_err(|error| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("source directive operand did not form a bounded cursor: {error:?}"),
            )
        })?,
        None => AstCursor::from_source_tokens(source, operand)?,
    };
    let mut expected_type = ExpectedType::Known(builtin_type_ids::STRING);
    let mut cast_target_context = CastTargetContext::None;
    let value_mode = ValueMode::ImmutableOwned;
    let mut input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream: &mut cursor,
            scope_context: context,
            type_interner: resources.type_interner,
            expression_builder: resources.expression_builder,
            const_values: resources.const_values,
            expected_type: &mut expected_type,
            cast_target_context: &mut cast_target_context,
            value_mode: &value_mode,
            string_table: resources.string_table,
            path_fork: resources.path_fork,
        },
        false,
    );
    input.stop_at_adjacent_operand = true;
    let expression = create_expression_with_trailing_newline_policy(input)?.id();

    // The captured range is exactly one operand expression. A parser stop before the range end
    // means a second adjacent expression was authored on the same directive line.
    if cursor.position() != cursor.length() {
        let trailing_span = cursor
            .current()
            .map(|token| token.source_span())
            .unwrap_or(operand_span);
        return Err(templates::invalid_template_directive(
            Some(annotation.name_id),
            InvalidTemplateDirectiveReason::UnexpectedArguments,
            Some(trailing_span),
        )
        .into());
    }

    let view = resources.expression_builder.view(expression)?;
    let kind = view.const_value_kind_with_template_classifier(resources.const_values, &mut |template| {
        crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir(
            template,
            &context.template_ir_store,
            resources.expression_builder,
            resources.const_values,
            Some(context.external_package_registry.as_ref()),
        )
    })?;
    if !kind.is_compile_time_value() {
        return Err(templates::invalid_template_directive(
            Some(annotation.name_id),
            InvalidTemplateDirectiveReason::InvalidArgument { detail: None },
            Some(operand_span),
        )
        .into());
    }

    let value = resolve_const_expression_content(
        expression,
        resources.expression_builder,
        resources.const_values,
        resources.type_interner.environment(),
        context,
        resources.string_table,
    )?;
    let text = source_directive_text(
        value,
        resources.const_values,
        resources.string_table,
        annotation.name_id,
        operand_span,
    )?;
    Ok((text, operand_span))
}

/// Extracts final text from a folded source-directive value.
///
/// WHAT: unwraps coercions, accepts a literal `String` or a fully folded template, and flattens
///       authored site-root spelling while rejecting structural resource text.
/// WHY: the operand is compile-time metadata, not builder output, so every accepted value must
///       already be final text.
fn source_directive_text(
    mut value: ConstValueId,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
    name_id: StringId,
    span: SourceSpan,
) -> Result<StringId, ExpressionParseError> {
    loop {
        match const_values.payload(value) {
            Some(ConstValuePayload::Coerced(inner)) => value = *inner,
            Some(ConstValuePayload::String(text)) => {
                return flatten_source_string(text.clone(), string_table, span);
            }
            Some(ConstValuePayload::Template {
                folded: Some(text), ..
            }) => return flatten_source_string(text.clone(), string_table, span),
            Some(ConstValuePayload::Template { folded: None, .. }) | Some(_) | None => {
                return Err(templates::invalid_template_directive(
                    Some(name_id),
                    InvalidTemplateDirectiveReason::InvalidArgument { detail: None },
                    Some(span),
                )
                .into());
            }
        }
    }
}

/// Parses one contiguous source-directive run at an executable statement boundary.
///
/// WHAT: consumes every directive in the run, requires the target construct to start on a later
///       line, validates `$doc`'s declaration target and folds `$doc`/`$todo` operands.
/// WHY: statement attachment must not rescan past directives per name or cross a scope close. The
///       caller re-enters ordinary statement dispatch at the returned target position.
pub(crate) fn parse_statement_source_directive_run(
    cursor: &mut AstCursor<'_>,
    context: &mut ScopeContext,
    resources: SourceOperandFoldResources<'_, '_>,
    target_is_scope_boundary: impl Fn(&AstCursor<'_>) -> bool,
) -> Result<(), ExpressionParseError> {
    let SourceOperandFoldResources {
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    } = resources;
    let mut annotations: Vec<SourceDirectiveAnnotation> = Vec::new();

    loop {
        let Some(name_id) = cursor.current_string_id_in(string_table)? else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "statement directive token had no string payload",
            )
            .into());
        };
        let directive_span = cursor.current_span();
        let directive_name = string_table.resolve(name_id);
        let Some(spec) = context.style_directives.find(directive_name) else {
            return Err(templates::invalid_template_directive(
                Some(name_id),
                InvalidTemplateDirectiveReason::UnknownDirective,
                Some(directive_span),
            )
            .into());
        };
        let (kind, operand_contract) = match spec.kind {
            StyleDirectiveKind::SourceModifier { target, operand }
                if spec.contexts.allows_source_modifier() =>
            {
                (target.directive_kind(), operand)
            }
            _ => {
                return Err(templates::invalid_template_directive(
                    Some(name_id),
                    InvalidTemplateDirectiveReason::WrongContext,
                    Some(directive_span),
                )
                .into());
            }
        };

        if let Some(previous) = annotations
            .iter()
            .find(|annotation| annotation.name_id == name_id)
        {
            return Err(templates::invalid_template_directive(
                Some(name_id),
                InvalidTemplateDirectiveReason::DuplicateModifier,
                Some(directive_span),
            )
            .with_label(
                Some(previous.span),
                Some(
                    crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage::EarlierDirective,
                ),
            )
            .into());
        }

        // The capture helper reads from the token after the directive name.
        cursor.advance();
        let operand_start = cursor.position();
        let operand = {
            let token_cursor = cursor.token_cursor_mut();
            capture_source_directive_operand(
                token_cursor,
                operand_contract,
                name_id,
                directive_span,
            )?
        };
        // `capture_source_directive_operand` leaves a bare directive on its own token and
        // advances a directive with an operand to the next boundary. Normalize both shapes so a
        // contiguous run advances exactly once per directive. A following directive stays in
        // place so the separator check can inspect its authored adjacency.
        if cursor.position() == operand_start && cursor.current_tag() != TokenTag::STYLE_DIRECTIVE {
            cursor.advance();
        }
        annotations.push(SourceDirectiveAnnotation {
            kind,
            name_id,
            span: directive_span,
            operand,
        });

        // Same-line stacking is one contiguous run; a line break also continues the run when
        // the next token is another directive. The target must start on a later line.
        let mut saw_line_break = false;
        while cursor.current_tag() == TokenTag::NEWLINE {
            if cursor
                .current()
                .is_some_and(|token| token.shape().newline_has_blank_line())
            {
                return Err(dangling_statement_directive(&annotations));
            }
            saw_line_break = true;
            cursor.advance();
        }
        if cursor.current_tag() == TokenTag::STYLE_DIRECTIVE {
            if !saw_line_break
                && cursor
                    .current()
                    .is_some_and(|token| token.shape().style_directive_is_attached())
            {
                let Some(incoming_name_id) = cursor.current_string_id_in(string_table)? else {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "adjacent statement directive token had no string payload",
                    )
                    .into());
                };
                return Err(missing_source_directive_separator(
                    incoming_name_id,
                    cursor.current_span(),
                ));
            }
            continue;
        }
        if matches!(
            cursor.current_tag(),
            TokenTag::EOF
                | TokenTag::END
                | TokenTag::ELSE
                | TokenTag::CATCH
                | TokenTag::CLOSE_CURLY
                | TokenTag::CLOSE_PARENTHESIS
                | TokenTag::TEMPLATE_CLOSE
        ) {
            return Err(dangling_statement_directive(&annotations));
        }
        if !saw_line_break {
            let annotation = annotations
                .first()
                .expect("a parsed source-directive run is never empty");
            return Err(templates::invalid_template_directive(
                Some(annotation.name_id),
                InvalidTemplateDirectiveReason::WrongTarget,
                Some(annotation.span),
            )
            .into());
        }
        break;
    }

    if target_is_scope_boundary(cursor) {
        return Err(dangling_statement_directive(&annotations));
    }
    let declaration_target =
        statement_declaration_target(cursor, context, &mut *string_table, &mut *path_fork)?;
    let source = cursor.canonical_owner().cloned().ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "source directive operand has no canonical source owner",
        )
    })?;
    let doc_sink = context
        .publish_source_docs
        .then(|| Rc::clone(&context.source_docs));

    for annotation in annotations {
        match annotation.kind {
            SourceDirectiveKind::Doc => {
                let Some(target) = declaration_target else {
                    return Err(templates::invalid_template_directive(
                        Some(annotation.name_id),
                        InvalidTemplateDirectiveReason::WrongTarget,
                        Some(annotation.span),
                    )
                    .into());
                };
                let (value, span) = fold_source_directive_operand(
                    &annotation,
                    &source,
                    Some(cursor),
                    context,
                    SourceOperandFoldResources {
                        type_interner: &mut *type_interner,
                        expression_builder: &mut *expression_builder,
                        const_values: &mut *const_values,
                        string_table: &mut *string_table,
                        path_fork: &mut *path_fork,
                    },
                )?;
                if let Some(doc_sink) = &doc_sink {
                    doc_sink.borrow_mut().push(AstDocFragment {
                        target,
                        value,
                        span: Some(span),
                    });
                }
            }
            SourceDirectiveKind::Todo => {
                let _ = fold_source_directive_operand(
                    &annotation,
                    &source,
                    Some(cursor),
                    context,
                    SourceOperandFoldResources {
                        type_interner: &mut *type_interner,
                        expression_builder: &mut *expression_builder,
                        const_values: &mut *const_values,
                        string_table: &mut *string_table,
                        path_fork: &mut *path_fork,
                    },
                )?;
            }
            SourceDirectiveKind::Config | SourceDirectiveKind::Export => {
                return Err(templates::invalid_template_directive(
                    Some(annotation.name_id),
                    InvalidTemplateDirectiveReason::WrongTarget,
                    Some(annotation.span),
                )
                .into());
            }
        }
    }

    Ok(())
}

fn missing_source_directive_separator(name_id: StringId, span: SourceSpan) -> ExpressionParseError {
    templates::invalid_template_directive(
        Some(name_id),
        InvalidTemplateDirectiveReason::MissingSeparator,
        Some(span),
    )
    .into()
}

fn dangling_statement_directive(annotations: &[SourceDirectiveAnnotation]) -> ExpressionParseError {
    let annotation = annotations
        .first()
        .expect("a parsed source-directive run is never empty");
    templates::invalid_template_directive(
        Some(annotation.name_id),
        InvalidTemplateDirectiveReason::DanglingModifier,
        Some(annotation.span),
    )
    .into()
}

/// Returns the local declaration path when the next statement starts a new declaration.
///
/// WHAT: checks only the syntactic declaration shape and existing-name facts the declaration
///       parser already uses.
/// WHY: `$doc` must reject an assignment or call target before parsing it as a declaration. The
///      declaration parser remains the sole semantic owner of the resulting binding.
fn statement_declaration_target(
    cursor: &AstCursor<'_>,
    context: &ScopeContext,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<PathId>, ExpressionParseError> {
    let (symbol_offset, symbol_token) = match cursor.current_tag() {
        TokenTag::SYMBOL => (0usize, cursor.current()),
        TokenTag::MUTABLE if cursor.peek_next_tag() == Some(TokenTag::SYMBOL) => {
            (1usize, cursor.token_ref_at_offset(1))
        }
        _ => return Ok(None),
    };
    if symbol_token.is_none() {
        return Ok(None);
    }
    let symbol_index = cursor.position().saturating_add(symbol_offset);
    let symbol_id = cursor
        .token_string_id_at_in(symbol_index, string_table)?
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "local declaration target symbol had no string payload",
            )
        })?;
    let follower_offset = symbol_offset + 1;
    let Some(follower) = cursor.token_ref_at_offset(follower_offset) else {
        return Ok(None);
    };
    let follower_tag = follower.tag();
    let declaration_shape = follower_tag.is_assignment_operator()
        || matches!(
            follower_tag,
            TokenTag::TYPE_PARAMETER_BRACKET | TokenTag::TYPE | TokenTag::HASH | TokenTag::SYMBOL
        );
    if !declaration_shape
        || matches!(follower_tag, TokenTag::OPEN_PARENTHESIS | TokenTag::DOT)
        || context
            .get_value_reference(&symbol_id, None)
            .map_err(ExpressionParseError::from)?
            .is_some()
        || context
            .lookup_visible_external_function(symbol_id)
            .is_some()
        || context
            .lookup_visible_receiver_method_by_name(symbol_id)
            .is_some()
    {
        return Ok(None);
    }
    Ok(path_fork.try_intern_child(context.scope, symbol_id))
}
fn flatten_source_string(
    text: ConstStringValue,
    string_table: &mut StringTable,
    span: SourceSpan,
) -> Result<StringId, ExpressionParseError> {
    match text {
        ConstStringValue::Text(value) => Ok(value),
        ConstStringValue::Pieces(pieces) => {
            let mut output = String::new();
            for piece in pieces {
                match piece {
                    ConstStringPiece::Text(value) => output.push_str(string_table.resolve(value)),
                    ConstStringPiece::SiteRoot => output.push_str("@/"),
                    ConstStringPiece::Resource(_) => {
                        let operation = string_table.intern("source directive");
                        return Err(
                            crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::compile_time_evaluation_error(
                                CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
                                Some(operation),
                                Some(span),
                            )
                            .into(),
                        );
                    }
                }
            }
            Ok(string_table.get_or_intern(output))
        }
    }
}
