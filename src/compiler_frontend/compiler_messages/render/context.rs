//! Shared diagnostic render context and syntax-name helpers.
//!
//! WHAT: owns the render-boundary lookup context plus token/name spelling helpers used
//! by terminal, terse, and dev-server renderers.
//! WHY: diagnostic facts stay structured until this boundary, where stable IDs and token
//! kinds become user-facing prose.

use super::*;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::source::FrozenIdentityContext;
use crate::compiler_frontend::source::line_index::LinePosition;
use crate::compiler_frontend::source::{SourceDatabase, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathId;
use unicode_width::UnicodeWidthChar;

/// Exact primary source position used by the renderer boundary.
///
/// A retained source span is resolved against the frozen identity context when one is present,
/// falling back to the transitional database that owns its source identity. The
/// legacy location remains the fallback for diagnostics that have no usable retained snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiagnosticPrimaryPosition {
    pub(crate) scope: InternedPath,
    pub(crate) source: Option<SourceId>,
    pub(crate) start: LinePosition,
    pub(crate) end: LinePosition,
}

/// Render-boundary data needed to turn diagnostic facts into user-facing text.
///
/// WHAT: carries shared lookup tables by reference while diagnostics keep only stable IDs.
/// WHY: type diagnostics should store semantic `TypeId`s, not rendered strings or owned
/// `TypeEnvironment` snapshots. The render boundary decides how those IDs become names.
/// Frozen identity rows are authoritative for retained source spans when present; the
/// transitional `StringTable`/`SourceDatabase` rows remain for diagnostics with no frozen row.
#[derive(Clone, Copy)]
pub(crate) struct DiagnosticRenderContext<'a> {
    pub(crate) string_table: &'a dyn StringTableResolver,
    legacy_string_table: &'a dyn StringTableResolver,
    pub(crate) type_environment: Option<&'a TypeEnvironment>,
    pub(crate) source_database: Option<&'a SourceDatabase>,
    pub(crate) frozen_identity: Option<&'a FrozenIdentityContext>,
}

impl<'a> DiagnosticRenderContext<'a> {
    pub(crate) fn new(string_table: &'a dyn StringTableResolver) -> Self {
        Self {
            string_table,
            legacy_string_table: string_table,
            type_environment: None,
            source_database: None,
            frozen_identity: None,
        }
    }

    /// Build a render context directly from an immutable identity snapshot.
    ///
    /// WHAT: selects the frozen string table and source snapshots as one authoritative owner.
    /// WHY: a frozen diagnostic context must never resolve IDs through an unrelated mutable table.
    pub(crate) fn from_frozen_identity(frozen_identity: &'a FrozenIdentityContext) -> Self {
        Self {
            string_table: frozen_identity.strings(),
            legacy_string_table: frozen_identity.strings(),
            type_environment: None,
            source_database: None,
            frozen_identity: Some(frozen_identity),
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

    /// Borrow one retained source line through the frozen snapshot when present.
    ///
    /// Frozen lookup is authoritative: a unique frozen match wins. Otherwise the transitional
    /// `SourceDatabase` behavior is preserved unchanged.
    pub(crate) fn retained_source_line(
        self,
        scope: &InternedPath,
        line_number: i32,
    ) -> Option<&'a str> {
        if let Some(frozen) = self.frozen_identity {
            if let Some(line) =
                frozen_source_line(frozen, self.legacy_string_table, scope, line_number)
            {
                return Some(line);
            }
            if let Some(line) = frozen_source_line(frozen, frozen.strings(), scope, line_number) {
                return Some(line);
            }
        }
        let source_database = self.source_database?;
        let slot = source_database.unique_record_for_logical_path(scope)?;
        let line_number = u32::try_from(line_number).ok()?;
        source_database.line_index(slot.id)?.line_text(line_number)
    }
    /// Resolve a diagnostic's primary span into renderer columns while its source snapshot is
    /// still available. SourceSpan ranges are half-open; legacy locations are converted to the
    /// same exclusive-end shape so caret lengths remain one calculation in every renderer.
    ///
    /// The frozen identity context is authoritative when present: its source records and
    /// `LineIndex` resolve the exact half-open `SourceSpan` byte range with unchanged scalar
    /// columns. Otherwise the transitional `SourceDatabase` behavior is preserved.
    ///
    /// The reserved compilation root owns no snapshot, so its spans never enter either retained
    /// branch and renderers keep omitting a physical source frame for them; the legacy location
    /// remains their fallback shape.
    pub(crate) fn primary_position(
        self,
        diagnostic: &CompilerDiagnostic,
    ) -> DiagnosticPrimaryPosition {
        if let Some(span) = diagnostic.primary_span
            && span.source() != SourceId::COMPILATION_ROOT
            && let Some(frozen) = self.frozen_identity
            && let Some(line_index) = frozen.line_index(span.source())
        {
            let range = span.byte_range(frozen);
            if let (Some(start), Some(end)) = (
                line_index.position(range.start()),
                line_index.position(range.end()),
            ) {
                return DiagnosticPrimaryPosition {
                    scope: frozen_interned_path(frozen, span.source())
                        .unwrap_or_else(|| diagnostic.primary_location.scope.clone()),
                    source: Some(span.source()),
                    start,
                    end,
                };
            }
        }
        if let Some(span) = diagnostic.primary_span
            && span.source() != SourceId::COMPILATION_ROOT
            && let Some(source_database) = self.source_database
            && let Some(line_index) = source_database.line_index(span.source())
        {
            let range = span.byte_range(source_database);
            if let (Some(start), Some(end)) = (
                line_index.position(range.start()),
                line_index.position(range.end()),
            ) {
                return DiagnosticPrimaryPosition {
                    scope: source_database.legacy_logical_path(span.source()),
                    source: Some(span.source()),
                    start,
                    end,
                };
            }
        }

        let location = &diagnostic.primary_location;
        DiagnosticPrimaryPosition {
            scope: location.scope.clone(),
            source: None,
            start: legacy_line_position(
                location.start_pos.line_number,
                location.start_pos.char_column,
            ),
            end: legacy_line_position(
                location.end_pos.line_number,
                location.end_pos.char_column.saturating_add(1),
            ),
        }
    }

    /// Borrow the retained line behind a resolved primary position, or `None` when no snapshot
    /// is available.
    ///
    /// A compilation-root primary position carries no source identity, so it looks its line up
    /// through the legacy logical path and yields `None` when the root has no frame to render.
    /// Frozen snapshots are authoritative when present; the transitional database remains the
    /// fallback.
    pub(crate) fn retained_source_line_for_primary(
        self,
        position: &DiagnosticPrimaryPosition,
    ) -> Option<&'a str> {
        if let Some(source) = position.source {
            if let Some(frozen) = self.frozen_identity
                && let Some(line) = frozen
                    .line_index(source)
                    .and_then(|line_index| line_index.line_text(position.start.line))
            {
                return Some(line);
            }
            if let Some(source_database) = self.source_database
                && let Some(line) = source_database
                    .line_index(source)
                    .and_then(|line_index| line_index.line_text(position.start.line))
            {
                return Some(line);
            }
            return None;
        }

        self.retained_source_line(&position.scope, position.start.line as i32)
    }
}

/// Borrow one retained line for a legacy `InternedPath` scope from a frozen snapshot.
///
/// WHAT: scans the frozen slots for the unique source whose logical path spells the same
/// components, comparing resolved string content (never IDs across tables), then borrows the
/// retained line without copying source text or touching the filesystem.
/// WHY: frozen path tables carry `PathId` identities while legacy callers still pass
/// `InternedPath` scopes; content comparison keeps the frozen row authoritative without a new
/// cross-module API. Ambiguity or absence yields `None`, matching transitional uniqueness.
fn frozen_source_line<'a>(
    frozen: &'a FrozenIdentityContext,
    string_table: &dyn StringTableResolver,
    scope: &InternedPath,
    line_number: i32,
) -> Option<&'a str> {
    let line_number = u32::try_from(line_number).ok()?;
    let mut matched: Option<SourceId> = None;
    for slot in frozen.iter() {
        if !frozen_path_matches_scope(frozen, slot.logical_path, string_table, scope) {
            continue;
        }
        if matched.is_some() {
            return None;
        }
        matched = Some(slot.id);
    }
    frozen.line_index(matched?)?.line_text(line_number)
}

/// Return whether a frozen `PathId` spells the same components as a legacy scope.
///
/// WHAT: compares resolved component text via frozen string access, never the filesystem.
/// WHY: frozen and transitional tables may issue different `StringId`s for the same spelling,
/// so only content comparison is sound. Fallible resolution keeps a foreign ID from panicking.
fn frozen_path_matches_scope(
    frozen: &FrozenIdentityContext,
    path: PathId,
    string_table: &dyn StringTableResolver,
    scope: &InternedPath,
) -> bool {
    let table = frozen.paths();
    let expected = scope.as_components();
    if table.depth(path) as usize != expected.len() {
        return false;
    }
    let mut current = path;
    for expected_component in expected.iter().rev() {
        let Some(frozen_component) = table.component(current) else {
            return false;
        };
        let (Some(frozen_text), Some(expected_text)) = (
            frozen.try_resolve_string(frozen_component),
            string_table.try_resolve(*expected_component),
        ) else {
            return false;
        };
        if frozen_text != expected_text {
            return false;
        }
        let Some(parent) = table.parent(current) else {
            return false;
        };
        current = parent;
    }
    current == PathId::ROOT
}

/// Rebuild the legacy `InternedPath` scope for a frozen source identity.
///
/// WHAT: resolves the frozen logical `PathId` into its component IDs without copying source
/// text or touching the filesystem.
/// WHY: `DiagnosticPrimaryPosition` still carries an `InternedPath` scope for the renderers
/// whose resolver migration lands separately; frozen component IDs share the final merged-root
/// allocation, so the rebuilt scope stays resolvable through either table.
fn frozen_interned_path(frozen: &FrozenIdentityContext, source: SourceId) -> Option<InternedPath> {
    let path = frozen.source_logical_path(source)?;
    let table = frozen.paths();
    let mut components = Vec::with_capacity(table.depth(path) as usize);
    table.resolve_components(path, &mut components);
    Some(InternedPath::from_components(components))
}

/// Display-cell tab stop for caret geometry. A tab advances the caret to the next multiple of
/// this width, matching common terminal emulators and browsers.
pub(crate) const RENDER_TAB_STOP_CELLS: usize = 8;

/// Convert one retained source line plus scalar-column span bounds into caret geometry.
///
/// WHAT: walks the line's scalars from its start, counting display cells: a tab advances to the
/// next [`RENDER_TAB_STOP_CELLS`] multiple and every other scalar contributes its Unicode width
/// (combining marks 0, wide CJK 2, unassigned or control scalars 0). Returns the padding cells
/// before `start_column` and the underline cells covering `start_column..end_column`.
/// WHY: scalar columns misplace carets behind tabs and wide characters, while tooling columns
/// (UTF-16) and scalar source offsets stay separate concerns owned by `LineIndex`. Only this
/// render boundary knows display cells.
pub(crate) fn caret_cells(line: &str, start_column: u32, end_column: u32) -> (usize, usize) {
    let start = start_column as usize;
    let end = end_column.max(start_column) as usize;
    let mut cells = 0usize;
    let mut padding = None;
    let mut underline_end = None;
    for (index, scalar) in line.chars().enumerate() {
        if index == start {
            padding = Some(cells);
        }
        if index == end {
            underline_end = Some(cells);
            break;
        }
        cells = advance_display_cells(cells, scalar);
    }
    let padding = padding.unwrap_or(cells);
    let underline_end = underline_end.unwrap_or(cells);
    (padding, underline_end.saturating_sub(padding).max(1))
}

fn advance_display_cells(cells: usize, scalar: char) -> usize {
    if scalar == '\t' {
        cells + RENDER_TAB_STOP_CELLS - cells % RENDER_TAB_STOP_CELLS
    } else {
        cells + UnicodeWidthChar::width(scalar).unwrap_or(0)
    }
}

/// Count the display cells before a primary span's start column on its retained line.
pub(crate) fn primary_caret_padding(position: &DiagnosticPrimaryPosition, line: &str) -> usize {
    caret_cells(line, position.start.column, position.start.column).0
}

pub(crate) fn primary_underline_length(position: &DiagnosticPrimaryPosition, line: &str) -> usize {
    let end_column = if position.start.line == position.end.line {
        position.end.column
    } else {
        u32::MAX
    };
    caret_cells(line, position.start.column, end_column).1
}

fn legacy_line_position(line: i32, column: i32) -> LinePosition {
    LinePosition {
        line: u32::try_from(line.max(0)).unwrap_or(u32::MAX),
        column: u32::try_from(column.max(0)).unwrap_or(u32::MAX),
    }
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

/// Render a token as source-facing syntax rather than Rust enum debug output.
///
/// WHAT: gives syntax diagnostics one spelling source across terminal, terse, dev-server, and
/// contextless compiler-error fallback while the last bridge call sites are retired.
/// WHY: parser diagnostics carry `TokenKind` facts, but user output should show Moth syntax
/// such as `(` or `name`, not implementation names such as `OpenParenthesis`.
pub(crate) fn token_kind_name(
    token_kind: &TokenKind,
    string_table: &dyn StringTableResolver,
) -> String {
    match token_kind {
        TokenKind::ModuleStart => "module start".to_owned(),
        TokenKind::Eof => "end of file".to_owned(),
        TokenKind::Export => "`export`".to_owned(),
        TokenKind::Hash => "`#`".to_owned(),
        TokenKind::Reactive => "`$`".to_owned(),
        TokenKind::Arrow => "`->`".to_owned(),
        TokenKind::Symbol(name) => format!("name `{}`", string_table.resolve(*name)),
        TokenKind::StyleDirective(name) => {
            format!("style directive `${}`", string_table.resolve(*name))
        }
        TokenKind::StringSliceLiteral(value) => {
            format!("string literal \"{}\"", string_table.resolve(*value))
        }
        // `TokenKind::Path` now carries only a dense handle into a file-owned
        // `PathSyntaxTable`; that table is not part of stable diagnostic facts, so
        // the renderer names the token class without a file-local spelling.
        TokenKind::Path(_) => "path".to_owned(),
        TokenKind::NumericLiteral(token) => {
            let text = string_table.resolve(token.normalized_text);
            match token.kind {
                crate::compiler_frontend::numeric_text::token::NumericLiteralKind::WholeNumber => {
                    format!("integer literal `{text}`")
                }
                _ => format!("float literal `{text}`"),
            }
        }
        TokenKind::CharLiteral(value) => format!("character literal `{value}`"),
        TokenKind::RawStringLiteral(value) => {
            format!("raw string literal `{}`", string_table.resolve(*value))
        }
        TokenKind::BoolLiteral(value) => format!("boolean literal `{value}`"),
        TokenKind::OpenCurly => "`{`".to_owned(),
        TokenKind::CloseCurly => "`}`".to_owned(),
        TokenKind::TypeParameterBracket => "`|`".to_owned(),
        TokenKind::Newline => "newline".to_owned(),
        TokenKind::End => "`;`".to_owned(),
        TokenKind::StartTemplateBody => "`:`".to_owned(),
        TokenKind::Comma => "`,`".to_owned(),
        TokenKind::Dot => "`.`".to_owned(),
        TokenKind::Colon => "`:`".to_owned(),
        TokenKind::DoubleColon => "`::`".to_owned(),
        TokenKind::Assign => "`=`".to_owned(),
        TokenKind::This => "`this`".to_owned(),
        TokenKind::Must => "`must`".to_owned(),
        TokenKind::TraitThis => "`This`".to_owned(),
        TokenKind::OpenParenthesis => "`(`".to_owned(),
        TokenKind::CloseParenthesis => "`)`".to_owned(),
        TokenKind::As => "`as`".to_owned(),
        TokenKind::Type => "`type`".to_owned(),
        TokenKind::Of => "`of`".to_owned(),
        TokenKind::Variadic => "`..`".to_owned(),
        TokenKind::Mutable => "`~`".to_owned(),
        TokenKind::DatatypeNone => "`None` type".to_owned(),
        TokenKind::NoneLiteral => "`none`".to_owned(),
        TokenKind::DatatypeInt => "`Int`".to_owned(),
        TokenKind::DatatypeFloat => "`Float`".to_owned(),
        TokenKind::DatatypeBool => "`Bool`".to_owned(),
        TokenKind::DatatypeTrue => "`True`".to_owned(),
        TokenKind::DatatypeFalse => "`False`".to_owned(),
        TokenKind::DatatypeString => "`String`".to_owned(),
        TokenKind::DatatypeChar => "`Char`".to_owned(),
        TokenKind::Bang => "`!`".to_owned(),
        TokenKind::QuestionMark => "`?`".to_owned(),
        TokenKind::Negative => "unary `-`".to_owned(),
        TokenKind::Exponent => "`^`".to_owned(),
        TokenKind::Multiply => "`*`".to_owned(),
        TokenKind::Divide => "`/`".to_owned(),
        TokenKind::Modulus => "`%`".to_owned(),
        TokenKind::IntDivide => "`//`".to_owned(),
        TokenKind::ExponentAssign => "`^=`".to_owned(),
        TokenKind::MultiplyAssign => "`*=`".to_owned(),
        TokenKind::DivideAssign => "`/=`".to_owned(),
        TokenKind::ModulusAssign => "`%=`".to_owned(),
        TokenKind::IntDivideAssign => "`//=`".to_owned(),
        TokenKind::Add => "`+`".to_owned(),
        TokenKind::Subtract => "`-`".to_owned(),
        TokenKind::AddAssign => "`+=`".to_owned(),
        TokenKind::SubtractAssign => "`-=`".to_owned(),
        TokenKind::Not => "`not`".to_owned(),
        TokenKind::Is => "`is`".to_owned(),
        TokenKind::LessThan => "`<`".to_owned(),
        TokenKind::LessThanOrEqual => "`<=`".to_owned(),
        TokenKind::GreaterThan => "`>`".to_owned(),
        TokenKind::GreaterThanOrEqual => "`>=`".to_owned(),
        TokenKind::And => "`and`".to_owned(),
        TokenKind::Or => "`or`".to_owned(),
        TokenKind::If => "`if`".to_owned(),
        TokenKind::Else => "`else`".to_owned(),
        TokenKind::Return => "`return`".to_owned(),
        TokenKind::ReturnBang => "`return!`".to_owned(),
        TokenKind::Catch => "`catch`".to_owned(),
        TokenKind::Then => "`then`".to_owned(),
        TokenKind::Checked => "`checked`".to_owned(),
        TokenKind::Async => "`async`".to_owned(),
        TokenKind::Loop => "`loop`".to_owned(),
        TokenKind::By => "`by`".to_owned(),
        TokenKind::Break => "`break`".to_owned(),
        TokenKind::Continue => "`continue`".to_owned(),
        TokenKind::ExclusiveRange => "`to`".to_owned(),
        TokenKind::Ampersand => "`&`".to_owned(),
        TokenKind::FatArrow => "`=>`".to_owned(),
        TokenKind::Wildcard => "`_`".to_owned(),
        TokenKind::Copy => "`copy`".to_owned(),
        TokenKind::TemplateClose => "`]`".to_owned(),
        TokenKind::TemplateHead => "`[`".to_owned(),
        TokenKind::ChannelSend => "`>>`".to_owned(),
        TokenKind::ChannelReceive => "`<<`".to_owned(),
        TokenKind::Yield => "`yield`".to_owned(),
        TokenKind::Cast => "`cast`".to_owned(),
        TokenKind::CastBang => "`cast!`".to_owned(),
        TokenKind::Assert => "`assert`".to_owned(),
    }
}

pub(crate) fn expected_token_message(
    expected: &TokenKind,
    found: Option<&TokenKind>,
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
    found: &TokenKind,
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
