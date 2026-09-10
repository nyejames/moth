//! Shared diagnostic render context and syntax-name helpers.
//!
//! WHAT: owns the render-boundary lookup context plus token/name spelling helpers used
//! by terminal, terse, and dev-server renderers.
//! WHY: diagnostic facts stay structured until this boundary, where stable IDs and token
//! kinds become user-facing prose.

use super::*;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage, DiagnosticLabelStyle,
};
use crate::compiler_frontend::source::FrozenIdentityContext;
use crate::compiler_frontend::source::line_index::{LineIndex, LinePosition};
use crate::compiler_frontend::source::{SourceDatabase, SourceId, SourceSpan};
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthStr;

/// A secondary diagnostic label resolved into renderer-independent display facts.
///
/// A label with a source span carries a root-relative path and one-based display coordinates.
/// A spanless message carries no location. Labels with an unresolved source span are omitted by
/// [`resolve_label_render_facts_from_root`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedDiagnosticLabel {
    pub(crate) style: DiagnosticLabelStyle,
    pub(crate) path: Option<String>,
    pub(crate) line: Option<i32>,
    pub(crate) column: Option<i32>,
    pub(crate) message: String,
}

/// Exact primary source position resolved from an attached source identity context.
///
/// A diagnostic source span is meaningful only while its owning frozen identity context or source
/// database remains attached to the message set. The render boundary resolves the span's byte
/// range through that owner and borrows the retained source line; no producer-side line/column
/// representation is consulted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiagnosticPrimaryPosition<'a> {
    pub(crate) source: SourceId,
    pub(crate) path: PathBuf,
    pub(crate) host_path: Option<&'a Path>,
    pub(crate) start: LinePosition,
    pub(crate) end: LinePosition,
    pub(crate) line: &'a str,
}

/// Render-boundary data needed to turn diagnostic facts into user-facing text.
///
/// WHAT: carries shared lookup tables by reference while diagnostics keep only stable IDs.
/// WHY: type diagnostics should store semantic `TypeId`s, not rendered strings or owned
/// `TypeEnvironment` snapshots. The render boundary decides how those IDs become names.
/// Frozen identity rows are authoritative for retained source spans; the attached `SourceDatabase`
/// is consulted only when no frozen identity context is attached.
#[derive(Clone, Copy)]
pub(crate) struct DiagnosticRenderContext<'a> {
    pub(crate) string_table: &'a dyn StringTableResolver,
    pub(crate) type_environment: Option<&'a TypeEnvironment>,
    pub(crate) source_database: Option<&'a SourceDatabase>,
    pub(crate) frozen_identity: Option<&'a FrozenIdentityContext>,
}

impl<'a> DiagnosticRenderContext<'a> {
    pub(crate) fn new(string_table: &'a dyn StringTableResolver) -> Self {
        Self {
            string_table,
            type_environment: None,
            source_database: None,
            frozen_identity: None,
        }
    }

    pub(crate) fn with_frozen_identity(
        mut self,
        frozen_identity: &'a FrozenIdentityContext,
    ) -> Self {
        self.string_table = frozen_identity.strings();
        self.frozen_identity = Some(frozen_identity);
        self
    }

    pub(crate) fn with_optional_frozen_identity(
        self,
        frozen_identity: Option<&'a FrozenIdentityContext>,
    ) -> Self {
        match frozen_identity {
            Some(frozen_identity) => self.with_frozen_identity(frozen_identity),
            None => self,
        }
    }

    pub(crate) fn with_optional_type_environment(
        mut self,
        type_environment: Option<&'a TypeEnvironment>,
    ) -> Self {
        self.type_environment = type_environment;
        self
    }

    pub(crate) fn with_optional_source_database(
        mut self,
        source_database: Option<&'a SourceDatabase>,
    ) -> Self {
        self.source_database = source_database;
        self
    }

    /// Resolve a diagnostic's primary span through an attached source identity owner.
    ///
    /// SourceSpan ranges are half-open byte offsets. The owner supplies both the extended-span
    /// table and the retained source snapshot, so line/column conversion and source text always
    /// come from one identity context. Compilation-root spans and diagnostics without an
    /// attached owner intentionally have no physical source position.
    pub(crate) fn primary_position(
        self,
        diagnostic: &'a CompilerDiagnostic,
    ) -> Option<DiagnosticPrimaryPosition<'a>> {
        let span = diagnostic.primary_span?;
        if let Some(frozen_identity_handle) = diagnostic.primary_frozen_identity_handle.as_ref() {
            if let Some(frozen_identity) = frozen_identity_handle.get() {
                return self.resolve_span_in_frozen(frozen_identity, span);
            }

            // A legacy handle without a stable domain may use the message's transitional source
            // row. A donor-domain handle never authorizes requester-database fallback.
            if self.frozen_identity.is_none() && frozen_identity_handle.domain().is_none() {
                return self.resolve_span_in_database(span);
            }
            return None;
        }

        self.resolve_span(span)
    }

    /// Resolve one secondary label span through its explicit owner when present.
    ///
    /// Ordinary labels remain compact and use the diagnostic context. A donor-owned label is
    /// different: its source IDs belong to the handle's frozen context, so an uninstalled handle
    /// returns no position rather than silently falling back to the requester context.
    pub(crate) fn label_position<'label>(
        self,
        label: &'label DiagnosticLabel,
    ) -> Option<DiagnosticPrimaryPosition<'label>>
    where
        'a: 'label,
    {
        let span = label.span?;
        if let Some(frozen_identity_handle) = label.frozen_identity_handle.as_ref() {
            if let Some(frozen_identity) = frozen_identity_handle.get() {
                return self.resolve_span_in_frozen(frozen_identity, span);
            }

            // A legacy handle without a stable domain may use the message's transitional source
            // row. A donor-domain handle never authorizes requester-database fallback.
            if self.frozen_identity.is_none() && frozen_identity_handle.domain().is_none() {
                return self.resolve_span_in_database(span);
            }
            return None;
        }

        self.resolve_span(span)
    }

    fn resolve_span(self, span: SourceSpan) -> Option<DiagnosticPrimaryPosition<'a>> {
        if span.source() == SourceId::COMPILATION_ROOT {
            return None;
        }

        if let Some(frozen) = self.frozen_identity {
            return self.resolve_span_in_frozen(frozen, span);
        }

        self.resolve_span_in_database(span)
    }

    fn resolve_span_in_frozen<'identity>(
        self,
        frozen: &'identity FrozenIdentityContext,
        span: SourceSpan,
    ) -> Option<DiagnosticPrimaryPosition<'identity>> {
        let source = span.source();
        let slot = frozen.get(source)?;
        let line_index = frozen.line_index(source)?;
        let range = span.byte_range(frozen);
        let start = line_index.position(range.start())?;
        let end = line_index.position(range.end())?;
        let line = line_text_or_empty_file_eof(line_index, start)?;
        let path_id = frozen.source_logical_path(source)?;
        let mut scratch = Vec::new();
        let path = PathBuf::from(frozen.render_path(path_id, &mut scratch));

        Some(DiagnosticPrimaryPosition {
            source,
            path,
            host_path: slot.canonical_os_path.as_deref(),
            start,
            end,
            line,
        })
    }

    fn resolve_span_in_database(self, span: SourceSpan) -> Option<DiagnosticPrimaryPosition<'a>> {
        let source_database = self.source_database?;
        let source = span.source();
        let slot = source_database.get(source)?;
        let line_index = source_database.line_index(source)?;
        let range = span.byte_range(source_database);
        let start = line_index.position(range.start())?;
        let end = line_index.position(range.end())?;
        let line = line_text_or_empty_file_eof(line_index, start)?;
        let path = source_database
            .legacy_logical_path(source)
            .to_path_buf(self.string_table);

        Some(DiagnosticPrimaryPosition {
            source,
            path,
            host_path: slot.canonical_os_path.as_deref(),
            start,
            end,
            line,
        })
    }
}
/// Resolve secondary labels into facts consumed independently by terminal and HTML renderers.
///
/// The path is made relative to `root`, while line and column remain one-based display
/// coordinates. A spanless message is retained with no location; a label whose source span cannot
/// be resolved is omitted.
pub(crate) fn resolve_label_render_facts_from_root(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
    root: &Path,
) -> Vec<ResolvedDiagnosticLabel> {
    let mut labels = Vec::new();

    for label in &diagnostic.labels {
        let Some(message) = label.message.as_ref() else {
            continue;
        };
        let message = diagnostic_label_message_text(message, context);
        let Some(position) = context.label_position(label) else {
            if label.span.is_none() {
                labels.push(ResolvedDiagnosticLabel {
                    style: label.style,
                    path: None,
                    line: None,
                    column: None,
                    message,
                });
            }
            continue;
        };

        labels.push(ResolvedDiagnosticLabel {
            style: label.style,
            path: Some(relative_display_path_from_root(
                position.path.as_path(),
                root,
            )),
            line: Some(display_line_number(
                i32::try_from(position.start.line).unwrap_or(i32::MAX),
            )),
            column: Some(display_column_number(
                i32::try_from(position.start.column).unwrap_or(i32::MAX),
            )),
            message,
        });
    }

    labels
}

fn diagnostic_label_message_text(
    message: &DiagnosticLabelMessage,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let string_table = context.string_table;

    match message {
        DiagnosticLabelMessage::PreviousDeclaration => "previous declaration here".to_owned(),
        DiagnosticLabelMessage::ConflictingAccess => "earlier conflicting access here".to_owned(),
        DiagnosticLabelMessage::ExpectedTypeDeclaredHere => {
            "expected type declared here".to_owned()
        }
        DiagnosticLabelMessage::ValueMovedHere => "value moved here".to_owned(),
        DiagnosticLabelMessage::RenderedText(text) => string_table.resolve(*text).to_owned(),
        DiagnosticLabelMessage::GenericInstantiationCallSite => {
            "while instantiating this generic call".to_owned()
        }
        DiagnosticLabelMessage::GenericInstantiationBodySite => {
            "generic body operation failed here".to_owned()
        }
        DiagnosticLabelMessage::GenericInstantiationDeclarationSite => {
            "generic function declared here".to_owned()
        }
        DiagnosticLabelMessage::GenericInstantiationSubstitutions { substitutions } => {
            let substitution_text = substitutions
                .iter()
                .map(|substitution| {
                    let parameter_name = string_table.resolve(substitution.parameter_name);
                    let concrete_type =
                        diagnostic_type_name(substitution.concrete_type_id, context);
                    format!("{parameter_name} = {concrete_type}")
                })
                .collect::<Vec<_>>()
                .join(", ");

            format!("generic substitution: {substitution_text}")
        }
        DiagnosticLabelMessage::GenericInferencePreviousEvidence => {
            "previous generic inference evidence here".to_owned()
        }
        DiagnosticLabelMessage::ImmutableBindingDeclaration => {
            "immutable binding declared here".to_owned()
        }
    }
}

/// Return a validated retained line, allowing only the exact EOF position of an empty snapshot to
/// use an empty excerpt. Ordinary out-of-range positions must remain unresolved.
fn line_text_or_empty_file_eof<'a>(
    line_index: LineIndex<'a>,
    position: LinePosition,
) -> Option<&'a str> {
    line_index.line_text(position.line).or_else(|| {
        (line_index.line_count() == 0 && position.line == 0 && position.column == 0).then_some("")
    })
}

/// Display-cell tab stop for caret geometry. A tab advances the caret to the next multiple of
/// this width, matching common terminal emulators and browsers.
pub(crate) const RENDER_TAB_STOP_CELLS: usize = 8;

/// Expand tabs using the same source-column display policy used by caret geometry.
///
/// The source excerpt is printed after the diagnostic gutter, so raw terminal tabs would use the
/// gutter's absolute column as their origin while [`caret_cells`] starts at the source excerpt's
/// first cell. Replacing tabs with spaces makes both views use the same source-relative tab stops.
pub(crate) fn expand_tabs_for_display(line: &str) -> String {
    let mut expanded = String::with_capacity(line.len());
    let mut cells = 0usize;
    let mut chunks = line.split('\t').peekable();

    while let Some(chunk) = chunks.next() {
        expanded.push_str(chunk);
        cells += UnicodeWidthStr::width(chunk);

        if chunks.peek().is_some() {
            let next_cells = advance_tab_stop(cells);
            expanded.push_str(&" ".repeat(next_cells.saturating_sub(cells)));
            cells = next_cells;
        }
    }

    expanded
}

/// Return the shared minimum-width gutter used by source and caret rows.
pub(crate) fn display_gutter_width(display_line: i32) -> usize {
    display_line.to_string().len().max(3)
}

/// Convert one retained source line plus scalar-column span bounds into caret geometry.
///
/// WHAT: walks non-tab runs as strings so contextual Unicode display sequences use their
/// repository-defined width; a tab advances to the next [`RENDER_TAB_STOP_CELLS`] multiple.
/// Returns the padding cells before `start_column` and the underline cells covering
/// `start_column..end_column`.
/// WHY: scalar columns misplace carets behind tabs and wide characters, while tooling columns
/// (UTF-16) and scalar source offsets stay separate concerns owned by `LineIndex`. Only this
/// render boundary knows display cells.
pub(crate) fn caret_cells(line: &str, start_column: u32, end_column: u32) -> (usize, usize) {
    let padding = display_cells_at_scalar_column(line, start_column);
    let underline_end = display_cells_at_scalar_column(line, end_column.max(start_column));
    (padding, underline_end.saturating_sub(padding).max(1))
}

fn display_cells_at_scalar_column(line: &str, scalar_column: u32) -> usize {
    let target = scalar_column as usize;
    let mut scalar_index = 0usize;
    let mut cells = 0usize;
    let mut chunks = line.split('\t').peekable();

    while let Some(chunk) = chunks.next() {
        let chunk_scalar_count = chunk.chars().count();
        let chunk_end = scalar_index + chunk_scalar_count;
        if target <= chunk_end {
            let prefix_scalar_count = target.saturating_sub(scalar_index);
            let prefix_end = chunk
                .char_indices()
                .nth(prefix_scalar_count)
                .map_or(chunk.len(), |(byte_index, _)| byte_index);
            return cells + UnicodeWidthStr::width(&chunk[..prefix_end]);
        }

        cells += UnicodeWidthStr::width(chunk);
        scalar_index = chunk_end;

        if chunks.peek().is_some() {
            cells = advance_tab_stop(cells);
            scalar_index += 1;
            if target <= scalar_index {
                return cells;
            }
        }
    }

    cells
}

fn advance_tab_stop(cells: usize) -> usize {
    cells + RENDER_TAB_STOP_CELLS - cells % RENDER_TAB_STOP_CELLS
}

/// Count the display cells before a primary span's start column on its retained line.
pub(crate) fn primary_caret_padding(position: &DiagnosticPrimaryPosition<'_>, line: &str) -> usize {
    caret_cells(line, position.start.column, position.start.column).0
}

pub(crate) fn primary_underline_length(
    position: &DiagnosticPrimaryPosition<'_>,
    line: &str,
) -> usize {
    let end_column = if position.start.line == position.end.line {
        position.end.column
    } else {
        u32::MAX
    };
    caret_cells(line, position.start.column, end_column).1
}

pub(crate) fn diagnostic_type_name(
    type_id: TypeId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    match context.type_environment {
        Some(type_environment) if type_environment.get(type_id).is_some() => {
            crate::compiler_frontend::datatypes::display::display_type_with_resolver(
                type_id,
                type_environment,
                context.string_table,
            )
        }
        _ => format!("TypeId({})", type_id.0),
    }
}

pub(crate) fn type_mismatch_context_name(
    context: crate::compiler_frontend::compiler_messages::TypeMismatchContext,
) -> &'static str {
    match context {
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::Assignment => {
            "assignment"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::Declaration => {
            "declaration"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::ReturnValue => {
            "return value"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::FunctionArgument => {
            "function argument"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::AssertionArgument => {
            "assertion argument"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::ConstructorArgument => {
            "constructor argument"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::ReceiverArgument => {
            "receiver argument"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::Operator => "operator",
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::Condition => "condition",
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::CollectionElement => {
            "collection element"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::StructFieldDefault => {
            "struct field default"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::TemplateInterpolation => {
            "template interpolation"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::MatchScrutinee => {
            "match scrutinee"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::MatchPattern => {
            "match pattern"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::ErrorReturn => {
            "error return"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::Pattern => "pattern",
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::General => "general",
    }
}

/// Render a compact diagnostic token as source-facing syntax rather than Rust enum debug output.
///
/// WHAT: gives syntax diagnostics one spelling source across terminal, terse, dev-server, and
/// contextless compiler-error fallback.
/// WHY: durable diagnostics retain only a stable token tag plus one compact payload, so rendering
/// must not depend on the live tokenizer enum or a source token side store.
pub(crate) fn token_kind_name(
    token: &DiagnosticToken,
    string_table: &dyn StringTableResolver,
) -> String {
    let descriptor = token.tag().descriptor();

    match descriptor.payload() {
        TokenDescriptorPayload::Static => descriptor.text().to_owned(),
        TokenDescriptorPayload::Symbol => {
            format!(
                "{} `{}`",
                descriptor.text(),
                string_table.resolve(token.string_id())
            )
        }
        TokenDescriptorPayload::StyleDirective => {
            format!(
                "{} `${}`",
                descriptor.text(),
                string_table.resolve(token.string_id())
            )
        }
        TokenDescriptorPayload::StringLiteral => {
            format!(
                "{} \"{}\"",
                descriptor.text(),
                string_table.resolve(token.string_id())
            )
        }
        TokenDescriptorPayload::NumericLiteral => {
            let text = string_table.resolve(token.string_id());
            if token.is_whole_number() {
                format!("integer literal `{text}`")
            } else {
                format!("float literal `{text}`")
            }
        }
        TokenDescriptorPayload::CharLiteral => {
            format!("{} `{}`", descriptor.text(), token.char_value())
        }
        TokenDescriptorPayload::RawStringLiteral => {
            format!(
                "{} `{}`",
                descriptor.text(),
                string_table.resolve(token.string_id())
            )
        }
        TokenDescriptorPayload::BoolLiteral => {
            format!("{} `{}`", descriptor.text(), token.bool_value())
        }
    }
}

pub(crate) fn expected_token_message(
    expected: &DiagnosticToken,
    found: Option<&DiagnosticToken>,
    string_table: &dyn StringTableResolver,
) -> String {
    let expected = token_kind_name(expected, string_table);

    if let Some(found) = found {
        let found = token_kind_name(found, string_table);
        format!("Expected {expected}, but found {found}.")
    } else {
        format!("Expected {expected}.")
    }
}

pub(crate) fn unexpected_token_message(
    found: &DiagnosticToken,
    string_table: &dyn StringTableResolver,
) -> String {
    let found = token_kind_name(found, string_table);
    format!("Unexpected token {found}.")
}

pub(crate) fn unknown_name_message(
    name: StringId,
    namespace: NameNamespace,
    string_table: &dyn StringTableResolver,
) -> String {
    let name = string_table.resolve(name);
    let namespace = namespace_name(namespace);

    format!("Unknown {namespace} name '{name}'.")
}

pub(crate) fn duplicate_declaration_message(
    name: StringId,
    string_table: &dyn StringTableResolver,
) -> String {
    let name_str = string_table.resolve(name);

    format!(
        "Cannot declare '{name_str}' because that name is already visible in this scope. Moth does not allow duplicate names or shadowing."
    )
}
