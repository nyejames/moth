//! Template body sentinel handling.
//!
//! WHAT: owns direct child-template markers that split or reject template body
//! regions, such as standalone `[else]` and template loop-control sentinels.
//! WHY: the body parser should stay focused on consuming body tokens and
//! nesting, while this support module keeps marker policy, boundary trimming,
//! and marker diagnostics together.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::TemplateConstructionContext;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Selects how a direct `[else]` marker is interpreted in the current body.
#[derive(Clone, Copy)]
pub(super) enum ElseSentinelPolicy {
    Orphan,
    SplitIf,
    Duplicate,
    LoopBody,
}

/// Selects how direct `[break]` / `[continue]` markers are interpreted.
#[derive(Clone, Copy)]
pub(super) enum LoopControlSentinelPolicy {
    Ignore,
    Structural,
}

/// Template-local control marker state carried through recursive body parsing.
///
/// WHAT: keeps template `[else]` handling, active loop depth, and loop-control
/// sentinel handling as separate decisions.
/// WHY: `[else]` belongs to the nearest template `if`, while `[break]` and
/// `[continue]` target the nearest active template loop and must remain visible
/// through nested template control-flow bodies.
#[derive(Clone, Copy)]
pub(crate) struct TemplateBodyControlContext {
    pub(crate) active_template_loop_depth: usize,
    pub(super) else_policy: ElseSentinelPolicy,
    pub(super) loop_control_policy: LoopControlSentinelPolicy,
}

impl TemplateBodyControlContext {
    pub(crate) fn normal() -> Self {
        Self {
            active_template_loop_depth: 0,
            else_policy: ElseSentinelPolicy::Orphan,
            loop_control_policy: LoopControlSentinelPolicy::Ignore,
        }
    }

    pub(super) fn with_else_policy(self, else_policy: ElseSentinelPolicy) -> Self {
        Self {
            else_policy,
            ..self
        }
    }

    pub(super) fn enter_template_loop(self) -> Self {
        Self {
            active_template_loop_depth: self.active_template_loop_depth + 1,
            else_policy: ElseSentinelPolicy::LoopBody,
            loop_control_policy: LoopControlSentinelPolicy::Structural,
        }
    }

    pub(super) fn accepts_loop_control(self) -> bool {
        matches!(
            self.loop_control_policy,
            LoopControlSentinelPolicy::Structural
        ) && self.active_template_loop_depth > 0
    }
}

pub(super) enum TemplateBodyBoundary {
    TemplateClose,
    Else {
        span: Option<SourceSpan>,
    },
    ElseIf {
        if_index: usize,
        close_index: usize,
        span: Option<SourceSpan>,
    },
}

pub(super) enum DirectElseMarker {
    Sentinel {
        close_index: usize,
        span: Option<SourceSpan>,
    },
    ElseIf {
        if_index: usize,
        close_index: usize,
        span: Option<SourceSpan>,
    },
    MalformedElseIf {
        span: Option<SourceSpan>,
    },
    Malformed {
        span: Option<SourceSpan>,
    },
}

pub(super) enum DirectLoopControlMarker {
    Break {
        close_index: Option<usize>,
        span: Option<SourceSpan>,
    },
    Continue {
        close_index: Option<usize>,
        span: Option<SourceSpan>,
    },
}

pub(super) struct BodySentinelTarget<'a> {
    pub(super) construction_context: &'a mut TemplateConstructionContext,
    pub(super) suppress_child_templates: bool,
}

impl BodySentinelTarget<'_> {
    fn suppress_child_templates(&self) -> bool {
        self.suppress_child_templates
    }

    fn trim_trailing_whitespace(&mut self, string_table: &StringTable) {
        self.construction_context
            .trim_trailing_whitespace(string_table);
    }
}

/// Classify the bracket the body parser is sitting on as a direct `else` sentinel.
///
/// WHAT: reports the sentinel shape with the `if` and `]` positions the caller needs, or `None`
/// when the bracket opens an ordinary nested template.
/// WHY: the decision needs several tokens of lookahead, so the walk borrows the live cursor and
/// hands it back at its entry position.
pub(super) fn classify_direct_else_marker(
    token_stream: &mut AstCursor,
) -> Option<DirectElseMarker> {
    let resume = token_stream.position();
    let marker = classify_else_marker_after_bracket(token_stream);
    token_stream
        .set_position(resume)
        .expect("else marker resume stays inside the active parser view");
    marker
}

fn classify_else_marker_after_bracket(token_stream: &mut AstCursor) -> Option<DirectElseMarker> {
    token_stream.advance();
    token_stream.skip_newlines();
    if token_stream.is_at_end() || token_stream.current_tag() != TokenTag::ELSE {
        return None;
    }
    let span = Some(token_stream.current_span());
    token_stream.advance();
    token_stream.skip_newlines();
    if token_stream.is_at_end() {
        return Some(DirectElseMarker::Malformed { span });
    }
    match token_stream.current_tag() {
        TokenTag::IF => {}
        TokenTag::TEMPLATE_CLOSE => {
            return Some(DirectElseMarker::Sentinel {
                close_index: token_stream.position(),
                span,
            });
        }
        _ => return Some(DirectElseMarker::Malformed { span }),
    }
    let if_index = token_stream.position();
    token_stream.advance();

    let mut nested_templates = 0usize;
    while !token_stream.is_at_end() {
        let tag = token_stream.current_tag();
        match tag {
            TokenTag::TEMPLATE_HEAD => nested_templates += 1,
            TokenTag::TEMPLATE_CLOSE if nested_templates == 0 => {
                return Some(DirectElseMarker::ElseIf {
                    if_index,
                    close_index: token_stream.position(),
                    span,
                });
            }
            TokenTag::TEMPLATE_CLOSE => nested_templates = nested_templates.saturating_sub(1),
            TokenTag::START_TEMPLATE_BODY | TokenTag::COLON if nested_templates == 0 => {
                return Some(DirectElseMarker::MalformedElseIf { span });
            }
            TokenTag::EOF => return Some(DirectElseMarker::MalformedElseIf { span }),
            _ => {}
        }
        token_stream.advance();
    }
    Some(DirectElseMarker::MalformedElseIf { span })
}

pub(super) fn handle_direct_else_marker(
    token_stream: &mut AstCursor,
    else_marker: DirectElseMarker,
    policy: ElseSentinelPolicy,
    mut target: BodySentinelTarget<'_>,
    string_table: &mut StringTable,
) -> Result<TemplateBodyBoundary, TemplateError> {
    if target.suppress_child_templates()
        && matches!(
            policy,
            ElseSentinelPolicy::SplitIf | ElseSentinelPolicy::Duplicate
        )
        && let DirectElseMarker::Sentinel { span, .. } = &else_marker
    {
        return Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseInLiteralBody,
            *span,
        )
        .into());
    }

    let (close_index, span) = match else_marker {
        DirectElseMarker::Sentinel { close_index, span } => (close_index, span),
        DirectElseMarker::ElseIf {
            if_index,
            close_index,
            span,
        } => {
            return handle_direct_else_if_marker(
                token_stream,
                if_index,
                close_index,
                span,
                policy,
                target,
                string_table,
            );
        }
        DirectElseMarker::MalformedElseIf { span } => {
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MalformedTemplateElseIf,
                span,
            )
            .into());
        }
        DirectElseMarker::Malformed { span } => {
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MalformedTemplateElse,
                span,
            )
            .into());
        }
    };

    match policy {
        ElseSentinelPolicy::SplitIf => {
            ensure_body_boundary_before_sentinel(
                token_stream,
                span,
                string_table,
                InvalidTemplateStructureReason::InlineTemplateElse,
            )
            .map_err(|error| {
                error.map_diagnostic(|diagnostic| with_direct_else_marker_span(diagnostic, span))
            })?;
            target.trim_trailing_whitespace(string_table);
            token_stream
                .set_position(close_index)
                .expect("else sentinel close index stays inside the body cursor");
            token_stream.advance();
            Ok(TemplateBodyBoundary::Else { span })
        }
        ElseSentinelPolicy::Orphan => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::OrphanTemplateElse,
            span,
        )
        .into()),
        ElseSentinelPolicy::Duplicate => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::DuplicateTemplateElse,
            span,
        )
        .into()),
        ElseSentinelPolicy::LoopBody => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseInLoopBody,
            span,
        )
        .into()),
    }
}

pub(super) fn handle_direct_else_if_marker(
    token_stream: &mut AstCursor,
    if_index: usize,
    close_index: usize,
    span: Option<SourceSpan>,
    policy: ElseSentinelPolicy,
    mut target: BodySentinelTarget<'_>,
    string_table: &mut StringTable,
) -> Result<TemplateBodyBoundary, TemplateError> {
    if target.suppress_child_templates()
        && matches!(
            policy,
            ElseSentinelPolicy::SplitIf | ElseSentinelPolicy::Duplicate
        )
    {
        return Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseIfInLiteralBody,
            span,
        )
        .into());
    }

    match policy {
        ElseSentinelPolicy::SplitIf => {
            ensure_body_boundary_before_sentinel(
                token_stream,
                span,
                string_table,
                InvalidTemplateStructureReason::InlineTemplateElse,
            )
            .map_err(|error| {
                error
                    .map_diagnostic(|diagnostic| adjust_else_if_inline_diagnostic(diagnostic, span))
            })?;
            target.trim_trailing_whitespace(string_table);

            Ok(TemplateBodyBoundary::ElseIf {
                if_index,
                close_index,
                span,
            })
        }

        ElseSentinelPolicy::Orphan => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::OrphanTemplateElseIf,
            span,
        )
        .into()),

        ElseSentinelPolicy::Duplicate => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseIfAfterElse,
            span,
        )
        .into()),

        ElseSentinelPolicy::LoopBody => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseIfInLoopBody,
            span,
        )
        .into()),
    }
}

/// Attach the exact source span for a direct `[else]` marker diagnostic.
pub(super) fn with_direct_else_marker_span(
    mut diagnostic: CompilerDiagnostic,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    diagnostic.primary_span = span;
    diagnostic
}

/// Classify the bracket the body parser is sitting on as a direct `break`/`continue` sentinel.
///
/// WHAT: reports which control marker follows the bracket and where its `]` sits, or `None` when
/// the bracket opens an ordinary nested template.
/// WHY: the decision needs several tokens of lookahead, so the walk borrows the live cursor and
/// hands it back at its entry position.
pub(super) fn classify_direct_loop_control_marker(
    token_stream: &mut AstCursor,
) -> Option<DirectLoopControlMarker> {
    let resume = token_stream.position();
    let marker = classify_loop_control_marker_after_bracket(token_stream);
    token_stream
        .set_position(resume)
        .expect("loop control marker resume stays inside the active parser view");
    marker
}

fn classify_loop_control_marker_after_bracket(
    token_stream: &mut AstCursor,
) -> Option<DirectLoopControlMarker> {
    token_stream.advance();
    token_stream.skip_newlines();
    if token_stream.is_at_end() {
        return None;
    }
    let breaks = match token_stream.current_tag() {
        TokenTag::BREAK => true,
        TokenTag::CONTINUE => false,
        _ => return None,
    };
    let span = Some(token_stream.current_span());
    token_stream.advance();
    token_stream.skip_newlines();
    let close_index = (!token_stream.is_at_end()
        && token_stream.current_tag() == TokenTag::TEMPLATE_CLOSE)
        .then(|| token_stream.position());
    if breaks {
        Some(DirectLoopControlMarker::Break { close_index, span })
    } else {
        Some(DirectLoopControlMarker::Continue { close_index, span })
    }
}

pub(super) fn loop_control_marker_source_span(
    marker: &DirectLoopControlMarker,
) -> Option<SourceSpan> {
    match marker {
        DirectLoopControlMarker::Break { span, .. }
        | DirectLoopControlMarker::Continue { span, .. } => *span,
    }
}

pub(super) fn loop_control_marker_close_index(marker: &DirectLoopControlMarker) -> Option<usize> {
    match marker {
        DirectLoopControlMarker::Break { close_index, .. }
        | DirectLoopControlMarker::Continue { close_index, .. } => *close_index,
    }
}

pub(super) fn malformed_loop_control_reason(
    marker: &DirectLoopControlMarker,
) -> CompilerDiagnostic {
    match marker {
        DirectLoopControlMarker::Break { span, .. } => {
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MalformedTemplateBreak,
                *span,
            )
        }

        DirectLoopControlMarker::Continue { span, .. } => {
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MalformedTemplateContinue,
                *span,
            )
        }
    }
}

pub(super) fn orphan_loop_control_diagnostic(
    marker: &DirectLoopControlMarker,
) -> CompilerDiagnostic {
    match marker {
        DirectLoopControlMarker::Break { span, .. } => {
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::OrphanTemplateBreak,
                *span,
            )
        }
        DirectLoopControlMarker::Continue { span, .. } => {
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::OrphanTemplateContinue,
                *span,
            )
        }
    }
}

fn token_tag_at(token_stream: &AstCursor, index: usize) -> Option<TokenTag> {
    token_stream.token_ref_at(index).map(|token| token.tag())
}

fn token_string_id_at(
    token_stream: &AstCursor,
    index: usize,
    string_table: &mut StringTable,
) -> Result<Option<StringId>, crate::compiler_frontend::compiler_errors::CompilerError> {
    let Some(token) = token_stream.token_ref_at(index) else {
        return Ok(None);
    };
    if !matches!(
        token.tag(),
        TokenTag::STRING_SLICE_LITERAL
            | TokenTag::RAW_STRING_LITERAL
            | TokenTag::SYMBOL
            | TokenTag::STYLE_DIRECTIVE
    ) {
        return Ok(None);
    }
    if token.string_id().is_none() {
        return Err(
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "canonical string-shaped token is missing its payload",
            ),
        );
    }
    token_stream.token_string_id_at_in(index, string_table)
}

pub(super) fn ensure_loop_control_boundary_before_sentinel(
    token_stream: &AstCursor,
    marker: &DirectLoopControlMarker,
    string_table: &mut StringTable,
) -> Result<(), TemplateError> {
    ensure_body_boundary_before_sentinel(
        token_stream,
        loop_control_marker_source_span(marker),
        string_table,
        inline_loop_control_reason(marker),
    )
}

pub(super) fn ensure_loop_control_boundary_after_sentinel(
    token_stream: &AstCursor,
    marker: &DirectLoopControlMarker,
    string_table: &mut StringTable,
) -> Result<(), TemplateError> {
    if token_stream.position() >= token_stream.length() {
        return Ok(());
    }

    let next_index = token_stream.position();
    let Some(next_tag) = token_tag_at(token_stream, next_index) else {
        return Ok(());
    };
    match next_tag {
        TokenTag::STRING_SLICE_LITERAL | TokenTag::RAW_STRING_LITERAL => {
            let Some(text_id) = token_string_id_at(token_stream, next_index, string_table)? else {
                return Ok(());
            };
            if first_line_has_meaningful_text(string_table.resolve(text_id)) {
                Err(inline_sentinel_diagnostic(
                    loop_control_marker_source_span(marker),
                    inline_loop_control_reason(marker),
                )
                .into())
            } else {
                Ok(())
            }
        }
        TokenTag::TEMPLATE_HEAD => Err(inline_sentinel_diagnostic(
            loop_control_marker_source_span(marker),
            inline_loop_control_reason(marker),
        )
        .into()),
        _ => Ok(()),
    }
}
fn inline_loop_control_reason(marker: &DirectLoopControlMarker) -> InvalidTemplateStructureReason {
    match marker {
        DirectLoopControlMarker::Break { .. } => {
            InvalidTemplateStructureReason::InlineTemplateBreak
        }
        DirectLoopControlMarker::Continue { .. } => {
            InvalidTemplateStructureReason::InlineTemplateContinue
        }
    }
}

pub(super) fn ensure_else_boundary_after_sentinel(
    token_stream: &AstCursor,
    sentinel_span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> Result<(), TemplateError> {
    if token_stream.position() >= token_stream.length() {
        return Ok(());
    }

    let next_index = token_stream.position();
    let Some(next_tag) = token_tag_at(token_stream, next_index) else {
        return Ok(());
    };
    match next_tag {
        TokenTag::STRING_SLICE_LITERAL | TokenTag::RAW_STRING_LITERAL => {
            let Some(text_id) = token_string_id_at(token_stream, next_index, string_table)? else {
                return Ok(());
            };
            if first_line_has_meaningful_text(string_table.resolve(text_id)) {
                Err(with_direct_else_marker_span(
                    inline_else_diagnostic(sentinel_span),
                    sentinel_span,
                )
                .into())
            } else {
                Ok(())
            }
        }
        TokenTag::TEMPLATE_HEAD => Err(with_direct_else_marker_span(
            inline_else_diagnostic(sentinel_span),
            sentinel_span,
        )
        .into()),
        _ => Ok(()),
    }
}

fn ensure_body_boundary_before_sentinel(
    token_stream: &AstCursor,
    sentinel_span: Option<SourceSpan>,
    string_table: &mut StringTable,
    inline_reason: InvalidTemplateStructureReason,
) -> Result<(), TemplateError> {
    let Some(previous_index) = token_stream.position().checked_sub(1) else {
        return Ok(());
    };
    let Some(previous_tag) = token_tag_at(token_stream, previous_index) else {
        return Ok(());
    };
    match previous_tag {
        TokenTag::NEWLINE => Ok(()),
        TokenTag::STRING_SLICE_LITERAL | TokenTag::RAW_STRING_LITERAL => {
            let Some(text_id) = token_string_id_at(token_stream, previous_index, string_table)?
            else {
                return Ok(());
            };
            if last_line_has_meaningful_text(string_table.resolve(text_id)) {
                return Err(inline_sentinel_diagnostic(sentinel_span, inline_reason).into());
            }
            Ok(())
        }
        _ => Err(inline_sentinel_diagnostic(sentinel_span, inline_reason).into()),
    }
}

pub(super) fn first_line_has_meaningful_text(text: &str) -> bool {
    let first_line = text.split('\n').next().unwrap_or(text);
    !first_line.trim().is_empty()
}

fn last_line_has_meaningful_text(text: &str) -> bool {
    let last_line = text.rsplit('\n').next().unwrap_or(text);
    !last_line.trim().is_empty()
}

pub(super) fn inline_else_diagnostic(span: Option<SourceSpan>) -> CompilerDiagnostic {
    inline_sentinel_diagnostic(span, InvalidTemplateStructureReason::InlineTemplateElse)
}

fn inline_sentinel_diagnostic(
    span: Option<SourceSpan>,
    reason: InvalidTemplateStructureReason,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_template_structure(reason, span)
}

pub(super) fn adjust_else_if_inline_diagnostic(
    diagnostic: CompilerDiagnostic,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    if matches!(
        diagnostic.payload,
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::InlineTemplateElse
        }
    ) {
        return CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::InlineTemplateElseIf,
            span.or(diagnostic.primary_span),
        );
    }

    diagnostic
}
