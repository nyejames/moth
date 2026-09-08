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
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};

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

/// Boundary where body parsing stopped.
pub(super) enum TemplateBodyBoundary {
    TemplateClose,
    Else {
        location: SourceLocation,
    },
    ElseIf {
        if_index: usize,
        close_index: usize,
        location: SourceLocation,
    },
}

pub(super) enum DirectElseMarker {
    Sentinel {
        close_index: usize,
        location: SourceLocation,
    },
    ElseIf {
        if_index: usize,
        close_index: usize,
        location: SourceLocation,
    },
    MalformedElseIf {
        location: SourceLocation,
    },
    Malformed {
        location: SourceLocation,
    },
}

pub(super) enum DirectLoopControlMarker {
    Break {
        close_index: Option<usize>,
        location: SourceLocation,
        span: LocalSpan,
        source: Option<SourceId>,
    },
    Continue {
        close_index: Option<usize>,
        location: SourceLocation,
        span: LocalSpan,
        source: Option<SourceId>,
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

    let location = token_stream.tokens[index].location.clone();
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
                        location,
                    });
                }
                TokenKind::TemplateClose => nested_templates = nested_templates.saturating_sub(1),
                TokenKind::StartTemplateBody | TokenKind::Colon if nested_templates == 0 => {
                    return Some(DirectElseMarker::MalformedElseIf { location });
                }
                TokenKind::Eof => return Some(DirectElseMarker::MalformedElseIf { location }),
                _ => {}
            }

            scan_index += 1;
        }

        return Some(DirectElseMarker::MalformedElseIf { location });
    }

    if index < token_stream.length
        && matches!(token_stream.tokens[index].kind, TokenKind::TemplateClose)
    {
        return Some(DirectElseMarker::Sentinel {
            close_index: index,
            location,
        });
    }

    Some(DirectElseMarker::Malformed { location })
}

pub(super) fn handle_direct_else_marker(
    token_stream: &mut FileTokens,
    else_marker: DirectElseMarker,
    policy: ElseSentinelPolicy,
    mut target: BodySentinelTarget<'_>,
    string_table: &StringTable,
) -> Result<TemplateBodyBoundary, CompilerDiagnostic> {
    // Literal-body directives keep bracketed body content opaque. A standalone
    // `[else]` inside such a template `if` would otherwise be split before the
    // literal bracket consumer sees it, so reject the conflicting form directly.
    if target.suppress_child_templates()
        && matches!(
            policy,
            ElseSentinelPolicy::SplitIf | ElseSentinelPolicy::Duplicate
        )
        && let DirectElseMarker::Sentinel { location, .. } = &else_marker
    {
        return Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseInLiteralBody,
            location.clone(),
        ));
    }

    let (close_index, location) = match else_marker {
        DirectElseMarker::Sentinel {
            close_index,
            location,
        } => (close_index, location),

        DirectElseMarker::ElseIf {
            if_index,
            close_index,
            location,
        } => {
            return handle_direct_else_if_marker(
                token_stream,
                if_index,
                close_index,
                location,
                policy,
                target,
                string_table,
            );
        }

        DirectElseMarker::MalformedElseIf { location } => {
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MalformedTemplateElseIf,
                location,
            ));
        }

        DirectElseMarker::Malformed { location } => {
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MalformedTemplateElse,
                location,
            ));
        }
    };

    match policy {
        ElseSentinelPolicy::SplitIf => {
            ensure_body_boundary_before_sentinel(
                token_stream,
                &location,
                string_table,
                InvalidTemplateStructureReason::InlineTemplateElse,
            )?;
            target.trim_trailing_whitespace(string_table);
            token_stream.index = close_index;
            token_stream.advance();
            Ok(TemplateBodyBoundary::Else { location })
        }

        ElseSentinelPolicy::Orphan => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::OrphanTemplateElse,
            location,
        )),

        ElseSentinelPolicy::Duplicate => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::DuplicateTemplateElse,
            location,
        )),

        ElseSentinelPolicy::LoopBody => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseInLoopBody,
            location,
        )),
    }
}

pub(super) fn handle_direct_else_if_marker(
    token_stream: &mut FileTokens,
    if_index: usize,
    close_index: usize,
    location: SourceLocation,
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
            location,
        ));
    }

    match policy {
        ElseSentinelPolicy::SplitIf => {
            ensure_body_boundary_before_sentinel(
                token_stream,
                &location,
                string_table,
                InvalidTemplateStructureReason::InlineTemplateElse,
            )
            .map_err(|diagnostic| remap_else_if_inline_diagnostic(diagnostic, &location))?;
            target.trim_trailing_whitespace(string_table);

            Ok(TemplateBodyBoundary::ElseIf {
                if_index,
                close_index,
                location,
            })
        }

        ElseSentinelPolicy::Orphan => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::OrphanTemplateElseIf,
            location,
        )),

        ElseSentinelPolicy::Duplicate => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseIfAfterElse,
            location,
        )),

        ElseSentinelPolicy::LoopBody => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateElseIfInLoopBody,
            location,
        )),
    }
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

    let (kind_is_break, location, span, source) = match token_stream.tokens.get(index) {
        Some(token) if matches!(token.kind, TokenKind::Break) => (
            true,
            token.location.clone(),
            token.span,
            token_stream.file_id,
        ),
        Some(token) if matches!(token.kind, TokenKind::Continue) => (
            false,
            token.location.clone(),
            token.span,
            token_stream.file_id,
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
        Some(DirectLoopControlMarker::Break {
            close_index,
            location,
            span,
            source,
        })
    } else {
        Some(DirectLoopControlMarker::Continue {
            close_index,
            location,
            span,
            source,
        })
    }
}

pub(super) fn loop_control_marker_location(marker: &DirectLoopControlMarker) -> &SourceLocation {
    match marker {
        DirectLoopControlMarker::Break { location, .. }
        | DirectLoopControlMarker::Continue { location, .. } => location,
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
        DirectLoopControlMarker::Break { location, .. } => {
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MalformedTemplateBreak,
                location.clone(),
            )
        }

        DirectLoopControlMarker::Continue { location, .. } => {
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MalformedTemplateContinue,
                location.clone(),
            )
        }
    }
}

pub(super) fn orphan_loop_control_diagnostic(
    marker: &DirectLoopControlMarker,
) -> CompilerDiagnostic {
    match marker {
        DirectLoopControlMarker::Break {
            location,
            span,
            source,
            ..
        } => with_loop_control_marker_span(
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::OrphanTemplateBreak,
                location.clone(),
            ),
            *span,
            *source,
        ),
        DirectLoopControlMarker::Continue {
            location,
            span,
            source,
            ..
        } => with_loop_control_marker_span(
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::OrphanTemplateContinue,
                location.clone(),
            ),
            *span,
            *source,
        ),
    }
}

fn with_loop_control_marker_span(
    mut diagnostic: CompilerDiagnostic,
    span: LocalSpan,
    source: Option<SourceId>,
) -> CompilerDiagnostic {
    if let Some(source) = source {
        diagnostic.primary_span = Some(SourceSpan::new(source, span));
    }
    diagnostic
}

pub(super) fn ensure_loop_control_boundary_before_sentinel(
    token_stream: &FileTokens,
    marker: &DirectLoopControlMarker,
    string_table: &StringTable,
) -> Result<(), CompilerDiagnostic> {
    ensure_body_boundary_before_sentinel(
        token_stream,
        loop_control_marker_location(marker),
        string_table,
        inline_loop_control_reason(marker),
    )
}

pub(super) fn ensure_loop_control_boundary_after_sentinel(
    token_stream: &FileTokens,
    marker: &DirectLoopControlMarker,
    string_table: &StringTable,
) -> Result<(), CompilerDiagnostic> {
    let location = loop_control_marker_location(marker);
    if token_stream.index >= token_stream.length {
        return Ok(());
    }

    let next_token = &token_stream.tokens[token_stream.index];
    match &next_token.kind {
        TokenKind::StringSliceLiteral(text) | TokenKind::RawStringLiteral(text)
            if first_line_has_meaningful_text(string_table.resolve(*text)) =>
        {
            return Err(inline_sentinel_diagnostic(
                location,
                inline_loop_control_reason(marker),
            ));
        }

        TokenKind::TemplateHead
            if next_token.location.start_pos.line_number == location.start_pos.line_number =>
        {
            return Err(inline_sentinel_diagnostic(
                location,
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
    sentinel_location: &SourceLocation,
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
            return Err(inline_else_diagnostic(sentinel_location));
        }

        TokenKind::TemplateHead
            if next_token.location.start_pos.line_number
                == sentinel_location.start_pos.line_number =>
        {
            return Err(inline_else_diagnostic(sentinel_location));
        }

        _ => {}
    }

    Ok(())
}

fn ensure_body_boundary_before_sentinel(
    token_stream: &FileTokens,
    sentinel_location: &SourceLocation,
    string_table: &StringTable,
    inline_reason: InvalidTemplateStructureReason,
) -> Result<(), CompilerDiagnostic> {
    if token_stream.index == 0 {
        return Ok(());
    }

    let previous_index = token_stream.index - 1;
    let previous_token = &token_stream.tokens[previous_index];

    match &previous_token.kind {
        TokenKind::Newline => Ok(()),

        TokenKind::StringSliceLiteral(text) | TokenKind::RawStringLiteral(text) => {
            if previous_token.location.end_pos.line_number
                == sentinel_location.start_pos.line_number
                && last_line_has_meaningful_text(string_table.resolve(*text))
            {
                return Err(inline_sentinel_diagnostic(sentinel_location, inline_reason));
            }

            Ok(())
        }

        _ if previous_token.location.end_pos.line_number
            == sentinel_location.start_pos.line_number =>
        {
            Err(inline_sentinel_diagnostic(sentinel_location, inline_reason))
        }

        _ => Ok(()),
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

pub(super) fn inline_else_diagnostic(location: &SourceLocation) -> CompilerDiagnostic {
    inline_sentinel_diagnostic(location, InvalidTemplateStructureReason::InlineTemplateElse)
}

fn inline_sentinel_diagnostic(
    location: &SourceLocation,
    reason: InvalidTemplateStructureReason,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_template_structure(reason, location.clone())
}

pub(super) fn remap_else_if_inline_diagnostic(
    diagnostic: CompilerDiagnostic,
    location: &SourceLocation,
) -> CompilerDiagnostic {
    if matches!(
        diagnostic.payload,
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::InlineTemplateElse
        }
    ) {
        return CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::InlineTemplateElseIf,
            location.clone(),
        );
    }

    diagnostic
}
