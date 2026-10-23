//! Shared diagnostic render context and syntax-name helpers.
//!
//! WHAT: owns the render-boundary lookup context plus token/name spelling helpers used
//! by terminal, terse, and dev-server renderers.
//! WHY: diagnostic facts stay structured until this boundary, where stable IDs and token
//! kinds become user-facing prose.

#[cfg(test)]
use super::subject::DiagnosticSubject;
use super::*;
use crate::compiler_frontend::build_config::BuildConfigValueOrigin;
use crate::compiler_frontend::compiler_messages::DiagnosticLabelStyle;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    UnexpectedTokenShape, UnexpectedTokenView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
#[cfg(test)]
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::compact_render_for_test;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReport;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
#[cfg(test)]
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticLabel};
#[cfg(test)]
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::source::line_index::{LineIndex, LinePosition};
use crate::compiler_frontend::source::{FrozenIdentityContext, FrozenIdentityHandle};
use crate::compiler_frontend::source::{SourceId, SourceSpan};
#[cfg(test)]
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathTable};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTableResolver};
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthStr;

/// A secondary diagnostic label resolved into renderer-independent display facts.
///
/// A label with a source span carries a root-relative path and one-based display coordinates;
/// its message is empty when the label marks a location without prose. A spanless message carries
/// no location. A label whose source span cannot be resolved, or that has neither a location nor
/// a message, is omitted.
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
/// Source of complete-path identities used by diagnostic renderers.
///
/// Mutable frontend diagnostics use a module-local fork; diagnostics retained past a merge use
/// the frozen path table attached to their identity context. A renderer must never reconstruct a
/// path through a legacy component-vector value.
#[derive(Clone, Copy)]
pub(crate) enum DiagnosticPathContext<'a> {
    #[cfg(test)]
    Fork(&'a PathInternerFork),
    Table(&'a PathTable),
}

impl<'a> DiagnosticPathContext<'a> {
    fn render(
        self,
        path: PathId,
        string_table: &dyn StringTableResolver,
        scratch: &mut Vec<StringId>,
    ) -> String {
        let components = match self {
            #[cfg(test)]
            Self::Fork(fork) => fork.resolve_components(path, scratch),
            Self::Table(table) => table.resolve_components(path, scratch),
        };
        let mut rendered = String::new();
        for (index, component) in components.iter().enumerate() {
            if index > 0 {
                rendered.push('/');
            }
            rendered.push_str(string_table.resolve(*component));
        }
        rendered
    }
}

/// Render-boundary data needed to turn diagnostic facts into user-facing text.
///
/// WHAT: carries shared lookup tables by reference while diagnostics keep only stable IDs.
/// WHY: type spellings are frozen into snapshots at the owning boundary, while source identity
/// rows remain available to renderers after diagnostics outlive their producer.
/// Frozen identity rows are authoritative for retained source spans. A test-only
/// `SourceDatabase` is consulted only when no frozen identity context is attached.
#[derive(Clone, Copy)]
pub(crate) struct DiagnosticRenderContext<'a> {
    pub(crate) string_table: &'a dyn StringTableResolver,
    #[cfg(test)]
    pub(crate) source_database: Option<&'a SourceDatabase>,
    pub(crate) frozen_identity: Option<&'a FrozenIdentityContext>,
    pub(crate) path_context: Option<DiagnosticPathContext<'a>>,
}

impl<'a> DiagnosticRenderContext<'a> {
    pub(crate) fn new(string_table: &'a dyn StringTableResolver) -> Self {
        Self {
            string_table,
            #[cfg(test)]
            source_database: None,
            frozen_identity: None,
            path_context: None,
        }
    }

    /// The render context for one frozen report: its strings and its frozen identity rows.
    pub(crate) fn for_report(report: &'a DiagnosticReport) -> Self {
        Self::new(report.context().strings()).with_frozen_identity(report.context())
    }

    pub(crate) fn with_frozen_identity(
        mut self,
        frozen_identity: &'a FrozenIdentityContext,
    ) -> Self {
        self.string_table = frozen_identity.strings();
        self.frozen_identity = Some(frozen_identity);
        self.path_context = Some(DiagnosticPathContext::Table(frozen_identity.paths()));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_optional_source_database(
        mut self,
        source_database: Option<&'a SourceDatabase>,
    ) -> Self {
        self.source_database = source_database;
        if self.path_context.is_none() {
            self.path_context =
                source_database.map(|database| DiagnosticPathContext::Table(database.paths()));
        }
        self
    }
    #[cfg(test)]
    pub(crate) fn with_optional_path_table(mut self, path_table: Option<&'a PathTable>) -> Self {
        if let Some(path_table) = path_table {
            self.path_context = Some(DiagnosticPathContext::Table(path_table));
        }
        self
    }

    #[cfg(test)]
    pub(crate) fn with_path_fork(mut self, path_fork: &'a PathInternerFork) -> Self {
        self.path_context = Some(DiagnosticPathContext::Fork(path_fork));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_path_table(mut self, path_table: &'a PathTable) -> Self {
        self.path_context = Some(DiagnosticPathContext::Table(path_table));
        self
    }

    /// Render a complete logical path through the explicitly attached identity table.
    pub(crate) fn render_path(self, path: PathId) -> String {
        let mut scratch = Vec::new();
        self.render_path_with_scratch(path, &mut scratch)
    }

    pub(crate) fn render_path_with_scratch(
        self,
        path: PathId,
        scratch: &mut Vec<StringId>,
    ) -> String {
        self.path_context
            .expect("diagnostic path rendering requires an attached path table")
            .render(path, self.string_table, scratch)
    }

    pub(crate) fn path_table(self) -> Option<&'a PathTable> {
        match self.path_context {
            #[cfg(test)]
            Some(DiagnosticPathContext::Fork(_)) => None,
            Some(DiagnosticPathContext::Table(table)) => Some(table),
            None => None,
        }
    }

    /// Resolve a diagnostic's primary span through an attached source identity owner.
    ///
    /// SourceSpan ranges are half-open byte offsets. The owner supplies both the extended-span
    /// table and the retained source snapshot, so line/column conversion and source text always
    /// come from one identity context. Compilation-root spans and diagnostics without an
    /// attached owner intentionally have no physical source position.
    #[cfg(test)]
    pub(crate) fn primary_position(
        self,
        diagnostic: &'a CompilerDiagnostic,
    ) -> Option<DiagnosticPrimaryPosition<'a>> {
        let span = diagnostic.primary_span?;
        self.position_of(span, diagnostic.primary_frozen_identity_handle.as_ref())
    }

    /// Resolve one secondary label span through its explicit owner when present.
    ///
    /// Ordinary labels remain compact and use the diagnostic context. A donor-owned label is
    /// different: its source IDs belong to the handle's frozen context, so an uninstalled handle
    /// returns no position rather than silently falling back to the requester context.
    #[cfg(test)]
    pub(crate) fn label_position<'label>(
        self,
        label: &'label DiagnosticLabel,
    ) -> Option<DiagnosticPrimaryPosition<'label>>
    where
        'a: 'label,
    {
        let span = label.span?;
        self.position_of(span, label.frozen_identity_handle.as_ref())
    }

    /// Resolve a compact site: a foreign site through its own identity, a local site through this context, and an
    /// absent site to no position.
    pub(crate) fn site_position<'site>(
        self,
        site: SiteRef<'site>,
    ) -> Option<DiagnosticPrimaryPosition<'site>>
    where
        'a: 'site,
    {
        match site {
            SiteRef::Absent => None,
            SiteRef::Local(span) => self.position_of(span, None),
            SiteRef::Foreign { span, context } => self.position_of(span, Some(context)),
        }
    }

    /// Resolve one span through the identity that owns it. A span with no handle uses this context. A handle resolves
    /// through its installed context; an uninstalled handle falls back to the requester database only when neither
    /// side is a donor domain, and otherwise has no position.
    fn position_of<'site>(
        self,
        span: SourceSpan,
        frozen_identity_handle: Option<&'site FrozenIdentityHandle>,
    ) -> Option<DiagnosticPrimaryPosition<'site>>
    where
        'a: 'site,
    {
        let Some(frozen_identity_handle) = frozen_identity_handle else {
            return self.resolve_span(span);
        };

        if let Some(frozen_identity) = frozen_identity_handle.get() {
            return self.resolve_span_in_frozen(frozen_identity, span);
        }

        // A legacy handle without a stable domain may use the message's transitional source
        // row. A donor-domain handle never authorizes requester-database fallback.
        #[cfg(test)]
        if self.frozen_identity.is_none() && frozen_identity_handle.domain().is_none() {
            return self.resolve_span_in_database(span);
        }
        None
    }

    fn resolve_span(self, span: SourceSpan) -> Option<DiagnosticPrimaryPosition<'a>> {
        if span.source() == SourceId::COMPILATION_ROOT {
            return None;
        }

        match self.frozen_identity {
            Some(frozen) => self.resolve_span_in_frozen(frozen, span),
            #[cfg(test)]
            None => self.resolve_span_in_database(span),
            #[cfg(not(test))]
            None => None,
        }
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

    #[cfg(test)]
    fn resolve_span_in_database(self, span: SourceSpan) -> Option<DiagnosticPrimaryPosition<'a>> {
        let source_database = self.source_database?;
        let source = span.source();
        let slot = source_database.get(source)?;
        let line_index = source_database.line_index(source)?;
        let range = span.byte_range(source_database);
        let start = line_index.position(range.start())?;
        let end = line_index.position(range.end())?;
        let line = line_text_or_empty_file_eof(line_index, start)?;
        let path_id = source_database.source_logical_path(source)?;
        let mut scratch = Vec::new();
        let path = PathBuf::from(
            DiagnosticPathContext::Table(source_database.paths()).render(
                path_id,
                self.string_table,
                &mut scratch,
            ),
        );

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
/// Resolve labels through the captured compact snapshot for legacy-boundary tests.
///
/// This adapter keeps source-position tests on the same snapshot-backed label path as production.
#[cfg(test)]
pub(crate) fn resolve_label_render_facts_from_root(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
    root: &Path,
) -> Vec<ResolvedDiagnosticLabel> {
    let compact = compact_render_for_test(diagnostic);
    DiagnosticSubject::new(compact.view()).resolved_labels(context, root)
}

/// Build one resolved label from its style, its position and its message.
///
/// A positioned label keeps its location even when its message is empty. A label without a position keeps only a
/// non-empty message, and only when it has no source span to explain. Any other label has no usable fact and is
/// omitted.
pub(super) fn resolved_label_fact(
    style: DiagnosticLabelStyle,
    spanned: bool,
    position: Option<DiagnosticPrimaryPosition<'_>>,
    message: Option<String>,
    root: &Path,
) -> Option<ResolvedDiagnosticLabel> {
    let Some(position) = position else {
        let message = message.filter(|message| !message.is_empty())?;
        return (!spanned).then_some(ResolvedDiagnosticLabel {
            style,
            path: None,
            line: None,
            column: None,
            message,
        });
    };

    Some(ResolvedDiagnosticLabel {
        style,
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
        message: message.unwrap_or_default(),
    })
}

/// The message of one compact label. `None` is the message-less label, which marks a location
/// without prose.
pub(crate) fn compact_label_message_text(
    message: DurableLabelMessage<'_>,
    types: &TypeDisplaySnapshot,
    context: DiagnosticRenderContext<'_>,
) -> Option<String> {
    let string_table = context.string_table;
    let type_text = |type_id| types.display_for_render(type_id, string_table, context.path_table());

    let text = match message {
        DurableLabelMessage::None => return None,
        DurableLabelMessage::PreviousDeclaration => "previous declaration here".to_owned(),
        DurableLabelMessage::ConflictingAccess => "earlier conflicting access here".to_owned(),
        DurableLabelMessage::ValueMovedHere => "value moved here".to_owned(),
        DurableLabelMessage::ConfigInputOrigin { input_name, origin } => {
            let origin_text = match origin {
                BuildConfigValueOrigin::ExplicitInput => "an explicit input",
                BuildConfigValueOrigin::BuilderGlobal => "a builder-provided global",
                BuildConfigValueOrigin::FixedProjectField => "a fixed project field",
                BuildConfigValueOrigin::DeclarationDefault => "its declaration default",
            };
            format!(
                "configuration input `{}` resolved from {origin_text}",
                string_table.resolve(input_name)
            )
        }
        DurableLabelMessage::RenderedText(message_text) => {
            string_table.resolve(message_text).to_owned()
        }
        DurableLabelMessage::GenericInstantiationBodySite => {
            "generic body operation failed here".to_owned()
        }
        DurableLabelMessage::GenericInstantiationDeclarationSite => {
            "generic function declared here".to_owned()
        }
        DurableLabelMessage::GenericInstantiationSubstitutions(substitutions) => {
            let substitution_text = substitutions
                .iter()
                .map(|substitution| {
                    let parameter_name = string_table.resolve(substitution.parameter);
                    format!("{parameter_name} = {}", type_text(substitution.concrete))
                })
                .collect::<Vec<_>>()
                .join(", ");

            format!("generic substitution: {substitution_text}")
        }
        DurableLabelMessage::GenericInferencePreviousEvidence => {
            "previous generic inference evidence here".to_owned()
        }
        DurableLabelMessage::ImmutableBindingDeclaration => {
            "immutable binding declared here".to_owned()
        }
        DurableLabelMessage::TypedFailureProducer { error_type } => {
            format!("`{}!` failure produced here", type_text(error_type))
        }
        DurableLabelMessage::ImplicitFailureProducer => {
            "implicit built-in failure may escape here".to_owned()
        }
        DurableLabelMessage::BuiltinFailureCall => {
            "implicit built-in failure propagates through this call".to_owned()
        }
        DurableLabelMessage::BuiltinFailureOrigin => {
            "original failing numeric or range operation here".to_owned()
        }
        DurableLabelMessage::BuiltinWriteBackOrigin { target_type } => {
            format!(
                "write-back into `{}` here after the compound update",
                type_text(target_type)
            )
        }
    };

    Some(text)
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

pub(crate) fn value_receiver_context_name(
    receiver: crate::compiler_frontend::compiler_messages::ValueReceiverContext,
) -> &'static str {
    use crate::compiler_frontend::compiler_messages::ValueReceiverContext;
    match receiver {
        ValueReceiverContext::Declaration => "declaration",
        ValueReceiverContext::Assignment => "assignment",
        ValueReceiverContext::Return => "return",
        ValueReceiverContext::Then => "then",
        ValueReceiverContext::FunctionArgument => "function argument",
        ValueReceiverContext::ConstructorArgument => "constructor argument",
        ValueReceiverContext::OperatorOperand => "operator operand",
        ValueReceiverContext::Condition => "condition",
        ValueReceiverContext::ReceiverBase => "receiver base",
        ValueReceiverContext::CollectionElement => "collection element",
        ValueReceiverContext::MapKey => "map key",
        ValueReceiverContext::MapValue => "map value",
        ValueReceiverContext::TemplateInterpolation => "template interpolation",
        ValueReceiverContext::CastSource => "cast source",
        ValueReceiverContext::MatchScrutinee => "match scrutinee",
        ValueReceiverContext::AssertionArgument => "assertion argument",
        ValueReceiverContext::General => "value",
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
        TokenDescriptorPayload::Static | TokenDescriptorPayload::Path => {
            descriptor.text().to_owned()
        }
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

/// WHAT: renders an unexpected-token diagnostic from its compact shape.
/// WHY: a plain token and a multi-bind misuse share one row, so the shape selects the prose.
pub(crate) fn render_unexpected_token(
    _diagnostic: &DiagnosticView<'_>,
    view: &UnexpectedTokenView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let message = match &view.shape {
        UnexpectedTokenShape::Token(found) => unexpected_token_message(found, context.string_table),
        UnexpectedTokenShape::MultiBind {
            code,
            target_name,
            expected,
            found,
        } => invalid_multi_bind_message(
            code.to_reason(*expected, *found),
            *target_name,
            context.string_table,
        ),
    };
    RenderedPayload {
        message,
        guidance: Vec::new(),
    }
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
