//! Template body sentinel handling.
//!
//! WHAT: owns direct child-template markers that split or reject template body
//! regions, such as standalone `[else]` and template loop-control sentinels.
//! WHY: the body parser should stay focused on consuming body tokens and
//! nesting, while this support module keeps marker policy, boundary trimming,
//! and marker diagnostics together.

use crate::compiler_frontend::ast::templates::tir::TemplateConstructionContext;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

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

pub(super) fn classify_direct_else_marker(token_stream: &FileTokens) -> Option<DirectElseMarker> {
    let mut index = token_stream.index + 1;

    while index < token_stream.length
        && matches!(token_stream.tokens[index].kind, TokenKind::Newline)
    {
        index += 1;
    }

    if index >= token_stream.length || !matches!(token_stream.tokens[index].kind, TokenKind::Else) {
        return None;
    }

    let else_token = &token_stream.tokens[index];
    let span = Some(SourceSpan::new(token_stream.file_id, else_token.span));
    index += 1;

    while index < token_stream.length
        && matches!(token_stream.tokens[index].kind, TokenKind::Newline)
    {
        index += 1;
    }

    if index < token_stream.length && matches!(token_stream.tokens[index].kind, TokenKind::If) {
        let if_index = index;
        index += 1;

        let mut scan_index = index;
        let mut nested_templates = 0usize;
        while scan_index < token_stream.length {
            match token_stream.tokens[scan_index].kind {
                TokenKind::TemplateHead => nested_templates += 1,
                TokenKind::TemplateClose if nested_templates == 0 => {
                    return Some(DirectElseMarker::ElseIf {
                        if_index,
                        close_index: scan_index,
                        span,
                    });
                }
                TokenKind::TemplateClose => nested_templates = nested_templates.saturating_sub(1),
                TokenKind::StartTemplateBody | TokenKind::Colon if nested_templates == 0 => {
                    return Some(DirectElseMarker::MalformedElseIf { span });
                }
                TokenKind::Eof => {
                    return Some(DirectElseMarker::MalformedElseIf { span });
                }
                _ => {}
            }

            scan_index += 1;
        }

        return Some(DirectElseMarker::MalformedElseIf { span });
    }

    if index < token_stream.length
        && matches!(token_stream.tokens[index].kind, TokenKind::TemplateClose)
    {
        return Some(DirectElseMarker::Sentinel {
            close_index: index,
            span,
        });
    }

    Some(DirectElseMarker::Malformed { span })
}
pub(super) fn handle_direct_else_marker(
    token_stream: &mut FileTokens,
    else_marker: DirectElseMarker,
    policy: ElseSentinelPolicy,
    mut target: BodySentinelTarget<'_>,
    string_table: &StringTable,
) -> Result<TemplateBodyBoundary, CompilerDiagnostic> {
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
        ));
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
            ));
        }
        DirectElseMarker::Malformed { span } => {
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MalformedTemplateElse,
                span,
            ));
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
            .map_err(|diagnostic| with_direct_else_marker_span(diagnostic, span))?;
            target.trim_trailing_whitespace(string_table);
            token_stream.index = close_index;
            token_stream.advance();
            Ok(TemplateBodyBoundary::Else { span })
        }
        ElseSentinelPolicy::Orphan => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::OrphanTemplateElse,
            span,
        )),
        ElseSentinelPolicy::Duplicate => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::DuplicateTemplateElse,
            span,
        )),
        ElseSentinelPolicy::LoopBody => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseInLoopBody,
            span,
        )),
    }
}

pub(super) fn handle_direct_else_if_marker(
    token_stream: &mut FileTokens,
    if_index: usize,
    close_index: usize,
    span: Option<SourceSpan>,
    policy: ElseSentinelPolicy,
    mut target: BodySentinelTarget<'_>,
    string_table: &StringTable,
) -> Result<TemplateBodyBoundary, CompilerDiagnostic> {
    if target.suppress_child_templates()
        && matches!(
            policy,
            ElseSentinelPolicy::SplitIf | ElseSentinelPolicy::Duplicate
        )
    {
        return Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseIfInLiteralBody,
            span,
        ));
    }

    match policy {
        ElseSentinelPolicy::SplitIf => {
            ensure_body_boundary_before_sentinel(
                token_stream,
                span,
                string_table,
                InvalidTemplateStructureReason::InlineTemplateElse,
            )
            .map_err(|diagnostic| adjust_else_if_inline_diagnostic(diagnostic, span))?;
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
        )),

        ElseSentinelPolicy::Duplicate => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseIfAfterElse,
            span,
        )),

        ElseSentinelPolicy::LoopBody => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseIfInLoopBody,
            span,
        )),
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

pub(super) fn classify_direct_loop_control_marker(
    token_stream: &FileTokens,
) -> Option<DirectLoopControlMarker> {
    let mut index = token_stream.index + 1;

    while index < token_stream.length
        && matches!(token_stream.tokens[index].kind, TokenKind::Newline)
    {
        index += 1;
    }

    let (kind_is_break, span) = match token_stream.tokens.get(index) {
        Some(token) if matches!(token.kind, TokenKind::Break) => (
            true,
            Some(SourceSpan::new(token_stream.file_id, token.span)),
        ),
        Some(token) if matches!(token.kind, TokenKind::Continue) => (
            false,
            Some(SourceSpan::new(token_stream.file_id, token.span)),
        ),
        _ => return None,
    };
    index += 1;

    while index < token_stream.length
        && matches!(token_stream.tokens[index].kind, TokenKind::Newline)
    {
        index += 1;
    }

    let close_index = if index < token_stream.length
        && matches!(token_stream.tokens[index].kind, TokenKind::TemplateClose)
    {
        Some(index)
    } else {
        None
    };

    if kind_is_break {
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

pub(super) fn ensure_loop_control_boundary_before_sentinel(
    token_stream: &FileTokens,
    marker: &DirectLoopControlMarker,
    string_table: &StringTable,
) -> Result<(), CompilerDiagnostic> {
    ensure_body_boundary_before_sentinel(
        token_stream,
        loop_control_marker_source_span(marker),
        string_table,
        inline_loop_control_reason(marker),
    )
}

pub(super) fn ensure_loop_control_boundary_after_sentinel(
    token_stream: &FileTokens,
    marker: &DirectLoopControlMarker,
    string_table: &StringTable,
) -> Result<(), CompilerDiagnostic> {
    if token_stream.index >= token_stream.length {
        return Ok(());
    }

    let next_token = &token_stream.tokens[token_stream.index];
    match &next_token.kind {
        TokenKind::StringSliceLiteral(text) | TokenKind::RawStringLiteral(text)
            if first_line_has_meaningful_text(string_table.resolve(*text)) =>
        {
            return Err(inline_sentinel_diagnostic(
                loop_control_marker_source_span(marker),
                inline_loop_control_reason(marker),
            ));
        }

        TokenKind::TemplateHead => {
            return Err(inline_sentinel_diagnostic(
                loop_control_marker_source_span(marker),
                inline_loop_control_reason(marker),
            ));
        }

        _ => {}
    }

    Ok(())
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
    token_stream: &FileTokens,
    sentinel_span: Option<SourceSpan>,
    string_table: &StringTable,
) -> Result<(), CompilerDiagnostic> {
    if token_stream.index >= token_stream.length {
        return Ok(());
    }

    let next_token = &token_stream.tokens[token_stream.index];
    match &next_token.kind {
        TokenKind::StringSliceLiteral(text) | TokenKind::RawStringLiteral(text)
            if first_line_has_meaningful_text(string_table.resolve(*text)) =>
        {
            return Err(with_direct_else_marker_span(
                inline_else_diagnostic(sentinel_span),
                sentinel_span,
            ));
        }

        TokenKind::TemplateHead => {
            return Err(with_direct_else_marker_span(
                inline_else_diagnostic(sentinel_span),
                sentinel_span,
            ));
        }

        _ => {}
    }

    Ok(())
}

fn ensure_body_boundary_before_sentinel(
    token_stream: &FileTokens,
    sentinel_span: Option<SourceSpan>,
    string_table: &StringTable,
    inline_reason: InvalidTemplateStructureReason,
) -> Result<(), CompilerDiagnostic> {
    if token_stream.index == 0 {
        return Ok(());
    }

    let previous_token = &token_stream.tokens[token_stream.index - 1];

    match &previous_token.kind {
        TokenKind::Newline => Ok(()),

        TokenKind::StringSliceLiteral(text) | TokenKind::RawStringLiteral(text) => {
            if last_line_has_meaningful_text(string_table.resolve(*text)) {
                return Err(inline_sentinel_diagnostic(sentinel_span, inline_reason));
            }

            Ok(())
        }

        _ => Err(inline_sentinel_diagnostic(sentinel_span, inline_reason)),
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
