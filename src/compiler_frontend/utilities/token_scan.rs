//! Shared frontend token-scanning utilities.
//!
//! WHAT: centralizes reusable delimiter-depth and balanced-template scan helpers.
//! WHY: declaration parsing, multi-bind parsing, header parsing, expression
//! boundary scanning, and template parsing previously maintained duplicate depth
//! bookkeeping logic.
//!
//! This module owns generic scan mechanics only.
//! It does NOT own statement/feature semantics or diagnostics policy.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, SourceTokens, Token, TokenIndex, TokenKind, TokenRange, TokenRangeError, TokenRef,
    TokenTag,
};

/// Failure lanes for declaration-initializer scanning.
///
/// User-authored unterminated constructs remain structured diagnostics. An impossible scanner
/// state stays a compiler error so the declaration-shell boundary can preserve it as
/// infrastructure instead of fabricating a user-facing diagnostic.
#[derive(Debug)]
pub(crate) enum TokenScanFailure {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(CompilerError),
}

/// A lightweight value-reference hint extracted from a token slice.
///
/// WHAT: records symbol-shaped references in an expression token slice without resolving or
/// parsing the full expression.
/// WHY: dependency sorting and capacity-expression reference discovery both need shallow
/// reference facts without duplicating the scan logic.
#[derive(Clone, Debug)]
pub struct InitializerReference {
    pub name: StringId,
    pub dot_member: Option<StringId>,
    /// Exact authored source span of the referenced name, when the token belongs to a source
    /// with a retained identity.
    pub span: Option<SourceSpan>,
    pub followed_by_call: bool,
    pub followed_by_choice_namespace: bool,
}

impl InitializerReference {
    /// Remap the reference names into a merged string table.
    ///
    // Called by per-file frontend output remapping before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
        if let Some(dot_member) = &mut self.dot_member {
            *dot_member = remap.get(*dot_member);
        }
    }
}

/// Scan short-lived scanned facts for symbol-shaped references.
///
/// WHAT: produces `InitializerReference` hints for every bare symbol that is not a
/// dot/namespace accessor, an assignment target, or preceded by a dot/double-colon.
/// WHY: indexed callers read on demand from slices or canonical stores without projecting
/// a per-query window first; the scan touches only its window.
pub(crate) fn collect_scanned_symbol_references(
    tokens: TokenFactView<'_>,
    source_id: SourceId,
) -> Vec<InitializerReference> {
    let mut references = Vec::new();
    let mut in_config_qualifier = false;

    for index in 0..tokens.len() {
        let Some(token) = tokens.get(index) else {
            continue;
        };
        if in_config_qualifier {
            if token.tag == TokenTag::ASSIGN || token.tag == TokenTag::COMMA {
                in_config_qualifier = false;
            } else {
                continue;
            }
        }

        let Some(name) = token.symbol.filter(|_| token.tag == TokenTag::SYMBOL) else {
            continue;
        };

        let previous = index.checked_sub(1).and_then(|i| tokens.get(i)).map(|t| t.tag);
        if previous == Some(TokenTag::DOT) || previous == Some(TokenTag::DOUBLE_COLON) {
            continue;
        }

        let next = index
            .checked_add(1)
            .and_then(|next_index| tokens.get(next_index))
            .map(|t| t.tag);
        if next == Some(TokenTag::ASSIGN) {
            continue;
        }

        if next == Some(TokenTag::HASH)
            && index
                .checked_add(2)
                .and_then(|next_index| tokens.get(next_index))
                .is_some_and(|t| t.tag == TokenTag::SYMBOL)
            && index
                .checked_add(3)
                .and_then(|next_index| tokens.get(next_index))
                .is_some_and(|t| t.tag == TokenTag::OF)
        {
            in_config_qualifier = true;
            continue;
        }

        let dot_member = if next == Some(TokenTag::DOT) {
            index
                .checked_add(2)
                .and_then(|next_index| tokens.get(next_index))
                .and_then(|member| {
                    if member.tag == TokenTag::SYMBOL {
                        member.symbol
                    } else {
                        None
                    }
                })
        } else {
            None
        };

        references.push(InitializerReference {
            name,
            dot_member,
            span: Some(SourceSpan::new(source_id, token.span)),
            followed_by_call: next == Some(TokenTag::OPEN_PARENTHESIS),
            followed_by_choice_namespace: next == Some(TokenTag::DOUBLE_COLON),
        });
    }

    references
}

/// One short-lived scanned token fact for tag-based shared scanners.
///
/// WHAT: carries the stable `TokenTag`, optional interned symbol payload and exact
///       source-local span for one token in a scan window.
/// WHY: header cursor migration needs one scan implementation shared by borrowed
///      `SourceTokens` views and legacy `Token` slices without duplicating scan bodies
///      or retaining durable references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScannedToken {
    pub(crate) tag: TokenTag,
    pub(crate) symbol: Option<StringId>,
    pub(crate) path_syntax_id: Option<crate::compiler_frontend::paths::path_syntax::PathSyntaxId>,
    pub(crate) span: crate::compiler_frontend::source::LocalSpan,
}

impl ScannedToken {
    pub(crate) fn from_token(token: &Token) -> Self {
        let tag = token.kind.token_tag();
        let symbol = match token.kind {
            TokenKind::Symbol(name) => Some(name),
            _ => None,
        };
        let path_syntax_id = match token.kind {
            TokenKind::Path(id) => Some(id),
            _ => None,
        };
        Self {
            tag,
            symbol,
            path_syntax_id,
            span: token.span,
        }
    }
}

/// Indexed on-demand view over short-lived token facts.
///
/// WHAT: reads one `ScannedToken` per index from a legacy slice, a bounded live
///       parser stream, or canonical `SourceTokens` shapes/spans without collecting the
///       source or retaining durable references.
/// WHY: dependency and classifier scans only touch their clause or line; per-query
///      whole-source `Vec<ScannedToken>` projection pays O(N) allocation for O(1) facts.
///      This view keeps slice-shaped `usize` offsets while callers migrate to the owner.
#[derive(Clone, Copy, Debug)]
pub(crate) enum TokenFactView<'a> {
    Slice(&'a [Token]),
    Stream { tokens: &'a [Token], end: usize },
    Canonical { tokens: &'a SourceTokens, start: usize, end: usize },
}

impl<'a> TokenFactView<'a> {
    pub(crate) fn from_slice(tokens: &'a [Token]) -> Self {
        Self::Slice(tokens)
    }

    /// Borrow one live parser stream without copying it.
    ///
    /// WHAT: exposes the stream's already-remapped compatibility vector through the same
    /// indexed facts, so string/path remaps stay visible and no snapshot can go stale.
    /// WHY: header streams remap payloads before publication; copying one clause window
    ///      would freeze pre-remap identities the path table no longer recognises.
    pub(crate) fn from_stream(stream: &'a FileTokens) -> Self {
        Self::Stream {
            tokens: &stream.tokens,
            end: stream.length,
        }
    }

    pub(crate) fn from_source(tokens: &'a SourceTokens) -> Self {
        Self::Canonical {
            tokens,
            start: 0,
            end: tokens.len(),
        }
    }

    /// Borrow one checked contiguous canonical source range.
    ///
    /// The range is revalidated against the source owner before its indexes enter the view. The
    /// resulting adapter remains zero-copy and exposes range-local offsets to shared scanners.
    pub(crate) fn from_source_range(
        tokens: &'a SourceTokens,
        range: TokenRange,
    ) -> Result<Self, TokenRangeError> {
        if range.source() != tokens.source() {
            return Err(TokenRangeError::ForeignSource {
                expected: tokens.source(),
                actual: range.source(),
            });
        }
        let range = tokens.range(range.start(), range.end())?;
        Ok(Self::Canonical {
            tokens,
            start: range.start().index(),
            end: range.end().index(),
        })
    }

    /// Restrict this view to a checked local subrange without projecting token facts.
    ///
    /// Canonical callers use this for condition expressions inside a retained template range;
    /// legacy callers retain their slice/stream identity while narrowing the same bounded window.
    pub(crate) fn subrange(self, start: usize, end: usize) -> Option<Self> {
        if start > end || end > self.len() {
            return None;
        }
        match self {
            Self::Slice(tokens) => tokens.get(start..end).map(Self::Slice),
            Self::Stream {
                tokens,
                end: stream_end,
            } => tokens.get(start..end).map(|tokens| Self::Stream {
                tokens,
                end: tokens.len().min(stream_end),
            }),
            Self::Canonical { tokens, start: base, end: base_end } => {
                let bounded_start = base.checked_add(start)?;
                let bounded_end = base.checked_add(end)?;
                (bounded_end <= base_end).then_some(Self::Canonical {
                    tokens,
                    start: bounded_start,
                    end: bounded_end,
                })
            }
        }
    }

    pub(crate) fn len(self) -> usize {
        match self {
            Self::Slice(tokens) => tokens.len(),
            Self::Stream { tokens, end } => tokens.len().min(end),
            Self::Canonical { start, end, .. } => end - start,
        }
    }


    pub(crate) fn get(self, index: usize) -> Option<ScannedToken> {
        match self {
            Self::Slice(tokens) => tokens.get(index).map(ScannedToken::from_token),
            Self::Stream { tokens, end } => {
                (index < end).then(|| tokens.get(index)).flatten().map(ScannedToken::from_token)
            }
            Self::Canonical { tokens, start, end } => {
                let absolute = start.checked_add(index)?;
                if absolute >= end {
                    return None;
                }
                let shape = tokens.shapes().get(absolute).copied()?;
                let span = tokens.spans().get(absolute).copied()?;
                let tag = shape.tag();
                Some(ScannedToken {
                    tag,
                    symbol: if tag == TokenTag::SYMBOL {
                        shape.string_id()
                    } else {
                        None
                    },
                    path_syntax_id: shape.path_syntax_id(),
                    span,
                })
            }
        }
    }
}

pub(crate) fn collect_symbol_references(
    tokens: &[Token],
    source_id: SourceId,
) -> Vec<InitializerReference> {
    collect_scanned_symbol_references(TokenFactView::from_slice(tokens), source_id)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NestingDepth {
    parenthesis: usize,
    curly: usize,
    template: usize,
}

impl NestingDepth {
    pub(crate) fn is_top_level(self) -> bool {
        self.parenthesis == 0 && self.curly == 0 && self.template == 0
    }

    pub(crate) fn step_tag(&mut self, tag: TokenTag) {
        if tag == TokenTag::OPEN_PARENTHESIS {
            self.parenthesis = self.parenthesis.saturating_add(1);
        } else if tag == TokenTag::CLOSE_PARENTHESIS {
            self.parenthesis = self.parenthesis.saturating_sub(1);
        } else if tag == TokenTag::OPEN_CURLY {
            self.curly = self.curly.saturating_add(1);
        } else if tag == TokenTag::CLOSE_CURLY {
            self.curly = self.curly.saturating_sub(1);
        } else if tag == TokenTag::TEMPLATE_HEAD {
            self.template = self.template.saturating_add(1);
        } else if tag == TokenTag::TEMPLATE_CLOSE {
            self.template = self.template.saturating_sub(1);
        }
    }

    pub(crate) fn step(&mut self, token_kind: &TokenKind) {
        self.step_tag(token_kind.token_tag());
    }
}

/// The innermost open construct tracked by the declaration-initializer scanner.
///
/// WHAT: models which construct owns the next expected closing delimiter at EOF.
/// WHY: the fixed `]` fallback could misreport the delimiter inside a value-producing
///      block, a catch body, a parenthesis or a collection. Each construct owns its
///      own terminator and must report it precisely at end-of-file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenConstruct {
    Template,
    Parenthesis,
    CollectionOrMap,
    CatchBlock,
    ValueIfBlock,
    PipeList,
}

impl OpenConstruct {
    fn expected_delimiter(self) -> &'static str {
        match self {
            OpenConstruct::Template => "]",
            OpenConstruct::Parenthesis => ")",
            OpenConstruct::CollectionOrMap => "}",
            OpenConstruct::CatchBlock => ";",
            OpenConstruct::ValueIfBlock => ";",
            OpenConstruct::PipeList => "|",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordPipeAction {
    Open,
    Close,
    Ignore,
}
fn classify_pipe_list(
    tokens: &[Token],
    pipe_index: usize,
    pipe_list_depth: usize,
    nesting_is_top_level: bool,
    allow_value_first: bool,
) -> RecordPipeAction {
    record_pipe_action(
        pipe_list_depth,
        nesting_is_top_level,
        pipe_opens_member_list(tokens, pipe_index, allow_value_first),
    )
}

/// True when `|` at `pipe_index` opens a balanced `|...|` member list.
///
/// Empty lists (`| |`) and lists whose first member is a name, `this`, or a value
/// literal stay pipe regions, including malformed first members. Catch bindings
/// (`|err|`) and option captures (`|name|`) stay outside so `|` keeps continuing
/// the surrounding expression.
fn pipe_opens_member_list(tokens: &[Token], pipe_index: usize, allow_value_first: bool) -> bool {
    pipe_opens_member_list_with(
        |cursor| {
            tokens
                .get(cursor)
                .map(|token| PipeMemberFact::of_kind(&token.kind))
        },
        pipe_index,
        allow_value_first,
    )
}

/// Shared `|...|` member-list lookahead over an index-addressed token view.
///
/// WHAT: single decision body for both the legacy slice scan and the live
///       [`AstCursor`] scan; each caller supplies one lightweight fact probe.
/// WHY: both lanes must preserve identical lookahead/newline boundaries without
///      maintaining two copies of the match, and without cloning non-`Copy`
///      token payloads only to classify their shape.
fn pipe_opens_member_list_with(
    fact_at: impl Fn(usize) -> Option<PipeMemberFact>,
    pipe_index: usize,
    allow_value_first: bool,
) -> bool {
    let skip_newlines = |mut cursor: usize| {
        while fact_at(cursor).is_some_and(|fact| fact.tag == TokenTag::NEWLINE) {
            cursor = cursor.saturating_add(1);
        }
        cursor
    };

    let mut cursor = skip_newlines(pipe_index.saturating_add(1));
    if fact_at(cursor).is_some_and(|fact| fact.tag == TokenTag::TYPE_PARAMETER_BRACKET) {
        return true;
    }
    match fact_at(cursor).map(|fact| fact.shape) {
        Some(PipeMemberShape::Name) => {
            cursor = skip_newlines(cursor.saturating_add(1));
            !fact_at(cursor).is_some_and(|fact| fact.tag == TokenTag::TYPE_PARAMETER_BRACKET)
        }
        Some(PipeMemberShape::Value) => allow_value_first,
        Some(PipeMemberShape::Other) | None => false,
    }
}

/// Lightweight pipe-list member fact shared by both scan lanes.
///
/// Only the distinctions consumed by the shared lookahead are modeled, so both
/// lanes avoid cloning full token payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PipeMemberFact {
    tag: TokenTag,
    shape: PipeMemberShape,
}

impl PipeMemberFact {
    fn of_kind(kind: &TokenKind) -> Self {
        Self {
            tag: kind.token_tag(),
            shape: PipeMemberShape::of_kind(kind),
        }
    }
}

/// Narrow shape probe for pipe-list member classification.
///
/// Only the distinctions consumed by the shared lookahead are modeled, so both
/// lanes avoid cloning full token payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PipeMemberShape {
    Name,
    Value,
    Other,
}

impl PipeMemberShape {
    fn of_kind(kind: &TokenKind) -> Self {
        match kind {
            TokenKind::Symbol(_) | TokenKind::This => Self::Name,
            TokenKind::NumericLiteral(_)
            | TokenKind::StringSliceLiteral(_)
            | TokenKind::RawStringLiteral(_)
            | TokenKind::CharLiteral(_)
            | TokenKind::BoolLiteral(_)
            | TokenKind::NoneLiteral
            | TokenKind::Path(_) => Self::Value,
            _ => Self::Other,
        }
    }
}

/// True when `|` at `pipe_offset` opens a balanced `|...|` member list on a live cursor.
///
/// This reads through [`AstCursor::token_kind_at_offset`] so the multi-bind lookahead follows the
/// logical parser view without touching the legacy token vector.
fn pipe_opens_member_list_at_cursor(
    token_stream: &AstCursor,
    pipe_offset: usize,
    allow_value_first: bool,
) -> bool {
    pipe_opens_member_list_with(
        |offset| {
            token_stream
                .token_kind_at_offset(offset)
                .map(|kind| PipeMemberFact::of_kind(&kind))
        },
        pipe_offset,
        allow_value_first,
    )
}

fn classify_pipe_list_at_cursor(
    token_stream: &AstCursor,
    pipe_index: usize,
    pipe_list_depth: usize,
    nesting_is_top_level: bool,
    allow_value_first: bool,
) -> RecordPipeAction {
    record_pipe_action(
        pipe_list_depth,
        nesting_is_top_level,
        pipe_opens_member_list_at_cursor(token_stream, pipe_index, allow_value_first),
    )
}

/// Map one pipe observation to its record-list depth action.
///
/// WHAT: single branching body shared by the slice and cursor classifiers.
/// WHY: depth/top-level handling must stay identical across both scan lanes.
fn record_pipe_action(
    pipe_list_depth: usize,
    nesting_is_top_level: bool,
    opens_member_list: bool,
) -> RecordPipeAction {
    if !nesting_is_top_level {
        return RecordPipeAction::Ignore;
    }

    if pipe_list_depth == 0 && opens_member_list {
        RecordPipeAction::Open
    } else if pipe_list_depth > 0 {
        RecordPipeAction::Close
    } else {
        RecordPipeAction::Ignore
    }
}

/// Returns the construct that most recently opened and still owns a closing delimiter.
///
/// WHY: depth counters can say which construct kinds are open but not their nesting order.
/// The scanner keeps this stack so mixed forms report the actual innermost delimiter.
pub(crate) fn innermost_open_construct(open_constructs: &[OpenConstruct]) -> Option<OpenConstruct> {
    open_constructs.last().copied()
}

fn close_open_construct(open_constructs: &mut Vec<OpenConstruct>, expected: OpenConstruct) {
    if open_constructs.last() == Some(&expected) {
        open_constructs.pop();
    }
}

fn close_statement_construct(open_constructs: &mut Vec<OpenConstruct>) {
    if matches!(
        open_constructs.last(),
        Some(OpenConstruct::CatchBlock | OpenConstruct::ValueIfBlock)
    ) {
        open_constructs.pop();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExpressionBoundaryDepth {
    parenthesis: usize,
    curly: usize,
}

impl ExpressionBoundaryDepth {
    pub(crate) fn is_top_level(self) -> bool {
        self.parenthesis == 0 && self.curly == 0
    }

    pub(crate) fn step_tag(&mut self, tag: TokenTag) {
        if tag == TokenTag::OPEN_PARENTHESIS {
            self.parenthesis = self.parenthesis.saturating_add(1);
        } else if tag == TokenTag::CLOSE_PARENTHESIS {
            self.parenthesis = self.parenthesis.saturating_sub(1);
        } else if tag == TokenTag::OPEN_CURLY {
            self.curly = self.curly.saturating_add(1);
        } else if tag == TokenTag::CLOSE_CURLY {
            self.curly = self.curly.saturating_sub(1);
        }
    }

    pub(crate) fn step(&mut self, token_kind: &TokenKind) {
        self.step_tag(token_kind.token_tag());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TemplateBalance {
    opened: usize,
    closed: usize,
}

impl TemplateBalance {
    pub(crate) fn with_opening_template() -> Self {
        Self {
            opened: 1,
            closed: 0,
        }
    }

    pub(crate) fn has_unclosed_templates(self) -> bool {
        self.opened > self.closed
    }

    pub(crate) fn step_tag(&mut self, tag: TokenTag) {
        if tag == TokenTag::TEMPLATE_HEAD {
            self.opened = self.opened.saturating_add(1);
        } else if tag == TokenTag::TEMPLATE_CLOSE {
            self.closed = self.closed.saturating_add(1);
        }
    }

    pub(crate) fn step(&mut self, token_kind: &TokenKind) {
        self.step_tag(token_kind.token_tag());
    }
}

/// Two-lane result for declaration-initializer token scanning.
///
/// Structured unexpected-EOF diagnostics stay in the diagnostic lane while impossible scanner
/// states remain typed infrastructure failures for the declaration-shell boundary.
pub(crate) type TokenScanResult<T> = Result<T, TokenScanFailure>;

pub(crate) fn collect_declaration_initializer_tokens(
    token_stream: &mut FileTokens,
    string_table: &mut StringTable,
) -> TokenScanResult<Vec<Token>> {
    let mut collected = Vec::new();
    let mut depth = NestingDepth::default();
    let mut catch_block_depth = 0usize;
    let mut catch_header_pending = false;
    let mut value_if_block_depth = 0usize;
    let mut value_if_header_pending = false;
    let mut inline_value_if_missing_else_depth = 0usize;
    let mut initializer_closed_by_statement_block = false;
    let mut open_constructs = Vec::new();
    // A top-level `|...|` list owns newlines and commas until its closing pipe.
    // Nested parentheses, collections and templates still own inner pipes.
    let mut record_pipe_depth = 0usize;
    let mut last_closed_record_pipe = false;
    while token_stream.index < token_stream.length {
        if initializer_closed_by_statement_block {
            break;
        }

        let token_kind = token_stream.current_token_kind().clone();
        let inside_record_region = record_pipe_depth > 0;
        let at_top_level = depth.is_top_level()
            && catch_block_depth == 0
            && value_if_block_depth == 0
            && !inside_record_region;

        let continues_multiline_expression = if matches!(token_kind, TokenKind::Newline) {
            let prev_continues = if last_closed_record_pipe {
                // A closing record pipe ends the record value. Catch/receiver pipes still
                // continue because `|` is a continues_expression token.
                false
            } else {
                collected
                    .last()
                    .is_some_and(|token: &Token| token.kind.continues_expression())
            };
            let next_non_newline = token_stream
                .tokens
                .iter()
                .skip(token_stream.index + 1)
                .find(|token| token.kind != TokenKind::Newline)
                .map(|token| &token.kind);
            let next_continues = next_non_newline.is_some_and(TokenKind::continues_expression);
            let continues_to_authored_else = inline_value_if_missing_else_depth > 0
                && matches!(next_non_newline, Some(TokenKind::Else));

            prev_continues || next_continues || continues_to_authored_else
        } else {
            false
        };

        if at_top_level
            && matches!(
                token_kind,
                TokenKind::Comma | TokenKind::End | TokenKind::Eof
            )
        {
            if inline_value_if_missing_else_depth > 0
                || collected.last().is_some_and(|token| token.kind == TokenKind::Else)
            {
                collected.push(token_stream.current_token());
            }
            break;
        }

        if at_top_level
            && matches!(token_kind, TokenKind::Newline)
            && !continues_multiline_expression
        {
            if inline_value_if_missing_else_depth > 0
                || collected.last().is_some_and(|token| token.kind == TokenKind::Else)
            {
                collected.push(token_stream.current_token());
            }
            break;
        }

        if matches!(token_kind, TokenKind::Eof) && (!at_top_level || inside_record_region) {
            let expected_delimiter = match innermost_open_construct(&open_constructs) {
                Some(open_construct) => {
                    Some(string_table.get_or_intern(open_construct.expected_delimiter().to_owned()))
                }
                None => {
                    // No construct is open but the scanner believes it is nested. This is an
                    // internal scanner invariant, not a user-facing syntax error. Report it
                    // through the infrastructure error lane instead of fabricating a delimiter.
                    return Err(TokenScanFailure::Infrastructure(
                        CompilerError::compiler_error(
                            "declaration-initializer scanner reported a nested state with no open construct",
                        ),
                    ));
                }
            };
            return Err(TokenScanFailure::Diagnostic(
                CompilerDiagnostic::unexpected_end_of_file(
                    expected_delimiter,
                    Some(token_stream.current_span()),
                ),
            ));
        }
        // Declaration initializers can end with receiver-owned statement blocks such as
        // `catch:` and value-producing `if ...:`. Their bodies belong to the initializer even
        // though they are statement-shaped, so newline termination is suspended until the
        // matching outer `;` is collected.
        if depth.is_top_level() {
            match token_kind {
                TokenKind::Catch => catch_header_pending = true,
                TokenKind::If if catch_block_depth == 0 => value_if_header_pending = true,
                TokenKind::Colon if catch_header_pending => {
                    catch_header_pending = false;
                    catch_block_depth = catch_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstruct::CatchBlock);
                }
                TokenKind::Colon if catch_block_depth > 0 => {
                    catch_block_depth = catch_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstruct::CatchBlock);
                }
                TokenKind::Colon if value_if_header_pending => {
                    value_if_header_pending = false;
                    value_if_block_depth = value_if_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstruct::ValueIfBlock);
                }
                TokenKind::Colon if value_if_block_depth > 0 => {
                    value_if_block_depth = value_if_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstruct::ValueIfBlock);
                }
                TokenKind::Then if value_if_header_pending => {
                    value_if_header_pending = false;
                    inline_value_if_missing_else_depth =
                        inline_value_if_missing_else_depth.saturating_add(1);
                }
                TokenKind::Else if inline_value_if_missing_else_depth > 0 => {
                    inline_value_if_missing_else_depth =
                        inline_value_if_missing_else_depth.saturating_sub(1);
                }
                TokenKind::End if catch_block_depth > 0 => {
                    let closing_outer_catch_block = catch_block_depth == 1;
                    catch_block_depth = catch_block_depth.saturating_sub(1);
                    catch_header_pending = false;
                    initializer_closed_by_statement_block = closing_outer_catch_block;
                    close_statement_construct(&mut open_constructs);
                }
                TokenKind::End if value_if_block_depth > 0 => {
                    let closing_outer_value_if_block = value_if_block_depth == 1;
                    value_if_block_depth = value_if_block_depth.saturating_sub(1);
                    value_if_header_pending = false;
                    initializer_closed_by_statement_block = closing_outer_value_if_block;
                    close_statement_construct(&mut open_constructs);
                }
                TokenKind::Then | TokenKind::Arrow | TokenKind::Newline | TokenKind::Eof => {
                    catch_header_pending = false;
                    value_if_header_pending = false;
                }
                _ => {}
            }
        }

        match token_kind {
            TokenKind::OpenParenthesis => open_constructs.push(OpenConstruct::Parenthesis),
            TokenKind::CloseParenthesis => {
                close_open_construct(&mut open_constructs, OpenConstruct::Parenthesis);
            }
            TokenKind::OpenCurly => open_constructs.push(OpenConstruct::CollectionOrMap),
            TokenKind::CloseCurly => {
                close_open_construct(&mut open_constructs, OpenConstruct::CollectionOrMap);
            }
            TokenKind::TemplateHead => open_constructs.push(OpenConstruct::Template),
            TokenKind::TemplateClose => {
                close_open_construct(&mut open_constructs, OpenConstruct::Template);
            }
            TokenKind::TypeParameterBracket => {
                match classify_pipe_list(
                    &token_stream.tokens,
                    token_stream.index,
                    record_pipe_depth,
                    depth.is_top_level(),
                    collected
                        .iter()
                        .all(|token| matches!(token.kind, TokenKind::Newline)),
                ) {
                    RecordPipeAction::Open => {
                        open_constructs.push(OpenConstruct::PipeList);
                        record_pipe_depth = record_pipe_depth.saturating_add(1);
                        last_closed_record_pipe = false;
                    }
                    RecordPipeAction::Close => {
                        close_open_construct(&mut open_constructs, OpenConstruct::PipeList);
                        record_pipe_depth = record_pipe_depth.saturating_sub(1);
                        last_closed_record_pipe = true;
                    }
                    RecordPipeAction::Ignore => last_closed_record_pipe = false,
                }
            }
            _ => {}
        }
        if !matches!(
            token_kind,
            TokenKind::TypeParameterBracket | TokenKind::Newline
        ) {
            last_closed_record_pipe = false;
        }
        depth.step(&token_kind);

        collected.push(token_stream.current_token());
        token_stream.advance();
    }

    Ok(collected)
}

/// Scan one declaration initializer directly through a canonical cursor.
///
/// The scan keeps only short-lived tag facts for delimiter decisions. The returned shell retains
/// the checked source range and derives reference hints from that range, never a cloned token
/// vector. The cursor remains on the declaration boundary so its caller can continue the outer
/// parser walk.
pub(crate) fn collect_declaration_initializer_range(
    cursor: &mut crate::compiler_frontend::tokenizer::tokens::TokenCursor<'_>,
    string_table: &mut StringTable,
) -> TokenScanResult<(TokenRange, Vec<InitializerReference>)> {
    let source_id = cursor.range().source();
    let source_tokens = cursor.source_tokens();
    let start = cursor.position();
    let mut collected = Vec::new();
    let mut depth = NestingDepth::default();
    let mut catch_block_depth = 0usize;
    let mut catch_header_pending = false;
    let mut inline_catch_value_pending = false;
    let mut value_if_block_depth = 0usize;
    let mut value_if_header_pending = false;
    let mut inline_value_if_missing_else_depth = 0usize;
    let mut initializer_closed_by_statement_block = false;
    let mut open_constructs = Vec::new();
    let mut record_pipe_depth = 0usize;
    let mut last_closed_record_pipe = false;

    loop {
        if initializer_closed_by_statement_block {
            break;
        }

        let Some(current_ref) = cursor.peek() else {
            break;
        };
        let current_tag = current_ref.tag();
        let inside_record_region = record_pipe_depth > 0;
        let at_top_level = depth.is_top_level()
            && catch_block_depth == 0
            && value_if_block_depth == 0
            && !inside_record_region;

        let next_non_newline_tag = {
            let mut lookahead = *cursor;
            let _ = lookahead.advance();
            loop {
                let Some(next) = lookahead.peek() else {
                    break None;
                };
                if next.tag() != TokenTag::NEWLINE {
                    break Some(next.tag());
                }
                let _ = lookahead.advance();
            }
        };

        let continues_multiline_expression = if current_tag == TokenTag::NEWLINE {
            let previous_continues = if last_closed_record_pipe {
                false
            } else {
                collected
                    .last()
                    .is_some_and(|token: &ScannedToken| token.tag.continues_expression())
            };
            let next_continues = next_non_newline_tag
                .is_some_and(TokenTag::continues_expression);
            let continues_to_authored_else = inline_value_if_missing_else_depth > 0
                && next_non_newline_tag == Some(TokenTag::ELSE);
            let continues_to_catch_header =
                catch_header_pending || inline_catch_value_pending;
            previous_continues
                || next_continues
                || continues_to_authored_else
                || continues_to_catch_header
        } else {
            false
        };

        let boundary_end = if at_top_level
            && matches!(
                current_tag,
                TokenTag::COMMA | TokenTag::END | TokenTag::EOF
            ) {
            let include_boundary = inline_value_if_missing_else_depth > 0
                || collected.last().is_some_and(|token| token.tag == TokenTag::ELSE);
            Some(if include_boundary {
                TokenIndex::try_from_index(current_ref.index().index().saturating_add(1))
                    .ok_or_else(|| {
                        TokenScanFailure::Infrastructure(CompilerError::compiler_error(
                            "declaration initializer boundary exceeded its checked token index",
                        ))
                    })?
            } else {
                current_ref.index()
            })
        } else if at_top_level
            && current_tag == TokenTag::NEWLINE
            && !continues_multiline_expression
        {
            let include_boundary = inline_value_if_missing_else_depth > 0
                || collected.last().is_some_and(|token| token.tag == TokenTag::ELSE);
            Some(if include_boundary {
                TokenIndex::try_from_index(current_ref.index().index().saturating_add(1))
                    .ok_or_else(|| {
                        TokenScanFailure::Infrastructure(CompilerError::compiler_error(
                            "declaration initializer newline boundary exceeded its checked token index",
                        ))
                    })?
            } else {
                current_ref.index()
            })
        } else {
            None
        };

        if let Some(end) = boundary_end {
            let range = TokenRange::try_new_for(source_tokens, start, end).map_err(|error| {
                TokenScanFailure::Infrastructure(CompilerError::compiler_error(format!(
                    "declaration initializer range was invalid: {error:?}",
                )))
            })?;
            let facts = TokenFactView::from_source_range(source_tokens, range).map_err(|error| {
                TokenScanFailure::Infrastructure(CompilerError::compiler_error(format!(
                    "declaration initializer reference view was invalid: {error:?}",
                )))
            })?;
            return Ok((
                range,
                collect_scanned_symbol_references(facts, source_id),
            ));
        }

        if current_tag == TokenTag::EOF && (!at_top_level || inside_record_region) {
            let expected_delimiter = match innermost_open_construct(&open_constructs) {
                Some(open_construct) => {
                    Some(string_table.get_or_intern(open_construct.expected_delimiter().to_owned()))
                }
                None => {
                    return Err(TokenScanFailure::Infrastructure(
                        CompilerError::compiler_error(
                            "declaration-initializer scanner reported a nested state with no open construct",
                        ),
                    ));
                }
            };
            return Err(TokenScanFailure::Diagnostic(
                CompilerDiagnostic::unexpected_end_of_file(
                    expected_delimiter,
                    Some(current_ref.source_span()),
                ),
            ));
        }

        if depth.is_top_level() {
            match current_tag {
                TokenTag::CATCH => {
                    catch_header_pending = true;
                    inline_catch_value_pending = false;
                }
                TokenTag::IF if catch_block_depth == 0 => value_if_header_pending = true,
                TokenTag::COLON if catch_header_pending => {
                    catch_header_pending = false;
                    catch_block_depth = catch_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstruct::CatchBlock);
                }
                TokenTag::COLON if catch_block_depth > 0 => {
                    catch_block_depth = catch_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstruct::CatchBlock);
                }
                TokenTag::COLON if value_if_header_pending => {
                    value_if_header_pending = false;
                    value_if_block_depth = value_if_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstruct::ValueIfBlock);
                }
                TokenTag::COLON if value_if_block_depth > 0 => {
                    value_if_block_depth = value_if_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstruct::ValueIfBlock);
                }
                TokenTag::THEN if catch_header_pending => {
                    catch_header_pending = false;
                    inline_catch_value_pending = true;
                }
                TokenTag::THEN if value_if_header_pending => {
                    value_if_header_pending = false;
                    inline_value_if_missing_else_depth =
                        inline_value_if_missing_else_depth.saturating_add(1);
                }
                TokenTag::ELSE if inline_value_if_missing_else_depth > 0 => {
                    inline_value_if_missing_else_depth =
                        inline_value_if_missing_else_depth.saturating_sub(1);
                }
                TokenTag::END if catch_block_depth > 0 => {
                    let closing_outer_catch_block = catch_block_depth == 1;
                    catch_block_depth = catch_block_depth.saturating_sub(1);
                    catch_header_pending = false;
                    initializer_closed_by_statement_block = closing_outer_catch_block;
                    close_statement_construct(&mut open_constructs);
                }
                TokenTag::END if value_if_block_depth > 0 => {
                    let closing_outer_value_if_block = value_if_block_depth == 1;
                    value_if_block_depth = value_if_block_depth.saturating_sub(1);
                    value_if_header_pending = false;
                    initializer_closed_by_statement_block = closing_outer_value_if_block;
                    close_statement_construct(&mut open_constructs);
                }
                TokenTag::THEN | TokenTag::ARROW | TokenTag::NEWLINE | TokenTag::EOF => {
                    catch_header_pending = false;
                    value_if_header_pending = false;
                }
                _ => {}
            }
        }
        if current_tag != TokenTag::NEWLINE && current_tag != TokenTag::THEN {
            inline_catch_value_pending = false;
        }

        match current_tag {
            TokenTag::OPEN_PARENTHESIS => open_constructs.push(OpenConstruct::Parenthesis),
            TokenTag::CLOSE_PARENTHESIS => {
                close_open_construct(&mut open_constructs, OpenConstruct::Parenthesis);
            }
            TokenTag::OPEN_CURLY => open_constructs.push(OpenConstruct::CollectionOrMap),
            TokenTag::CLOSE_CURLY => {
                close_open_construct(&mut open_constructs, OpenConstruct::CollectionOrMap);
            }
            TokenTag::TEMPLATE_HEAD => open_constructs.push(OpenConstruct::Template),
            TokenTag::TEMPLATE_CLOSE => {
                close_open_construct(&mut open_constructs, OpenConstruct::Template);
            }
            TokenTag::TYPE_PARAMETER_BRACKET => {
                match classify_canonical_pipe(
                    cursor,
                    record_pipe_depth,
                    depth.is_top_level(),
                    collected.iter().all(|token| token.tag == TokenTag::NEWLINE),
                ) {
                    RecordPipeAction::Open => {
                        open_constructs.push(OpenConstruct::PipeList);
                        record_pipe_depth = record_pipe_depth.saturating_add(1);
                        last_closed_record_pipe = false;
                    }
                    RecordPipeAction::Close => {
                        close_open_construct(&mut open_constructs, OpenConstruct::PipeList);
                        record_pipe_depth = record_pipe_depth.saturating_sub(1);
                        last_closed_record_pipe = true;
                    }
                    RecordPipeAction::Ignore => last_closed_record_pipe = false,
                }
            }
            _ => {}
        }

        if !matches!(current_tag, TokenTag::TYPE_PARAMETER_BRACKET | TokenTag::NEWLINE) {
            last_closed_record_pipe = false;
        }
        depth.step_tag(current_tag);
        collected.push(ScannedToken {
            tag: current_tag,
            symbol: current_ref.string_id(),
            path_syntax_id: current_ref.path_syntax_id(),
            span: current_ref.span(),
        });
        let _ = cursor.advance();
    }

    let end = cursor.position();
    let range = TokenRange::try_new_for(source_tokens, start, end).map_err(|error| {
        TokenScanFailure::Infrastructure(CompilerError::compiler_error(format!(
            "declaration initializer range was invalid at stream end: {error:?}",
        )))
    })?;
    let facts = TokenFactView::from_source_range(source_tokens, range).map_err(|error| {
        TokenScanFailure::Infrastructure(CompilerError::compiler_error(format!(
            "declaration initializer reference view was invalid at stream end: {error:?}",
        )))
    })?;
    Ok((range, collect_scanned_symbol_references(facts, source_id)))
}

fn classify_canonical_pipe(
    cursor: &crate::compiler_frontend::tokenizer::tokens::TokenCursor<'_>,
    pipe_list_depth: usize,
    nesting_is_top_level: bool,
    allow_value_first: bool,
) -> RecordPipeAction {
    if !nesting_is_top_level {
        return RecordPipeAction::Ignore;
    }
    if pipe_list_depth > 0 {
        return RecordPipeAction::Close;
    }

    let opens = pipe_opens_member_list_with(
        |offset| {
            let mut lookahead = *cursor;
            for _ in 0..offset {
                lookahead.advance()?;
            }
            lookahead
                .peek()
                .and_then(|token| token.to_token_kind().ok())
                .map(|kind| PipeMemberFact::of_kind(&kind))
        },
        0,
        allow_value_first,
    );
    if opens {
        RecordPipeAction::Open
    } else {
        RecordPipeAction::Ignore
    }
}

pub(crate) fn has_top_level_comma_before_statement_end(token_stream: &AstCursor) -> bool {
    let mut depth = NestingDepth::default();
    let mut record_pipe_depth = 0usize;
    let mut offset = 0usize;
    let mut seen_non_newline = false;

    while let Some(token_kind) = token_stream.token_kind_at_offset(offset) {
        if token_kind == TokenKind::TypeParameterBracket {
            match classify_pipe_list_at_cursor(
                token_stream,
                offset,
                record_pipe_depth,
                depth.is_top_level(),
                !seen_non_newline,
            ) {
                RecordPipeAction::Open => {
                    record_pipe_depth = record_pipe_depth.saturating_add(1);
                }
                RecordPipeAction::Close => {
                    record_pipe_depth = record_pipe_depth.saturating_sub(1);
                }
                RecordPipeAction::Ignore => {}
            }
        }

        if record_pipe_depth == 0 && depth.is_top_level() && token_kind == TokenKind::Comma {
            return true;
        }

        if record_pipe_depth == 0
            && depth.is_top_level()
            && matches!(
                token_kind,
                TokenKind::Newline | TokenKind::End | TokenKind::Eof
            )
        {
            break;
        }

        if token_kind != TokenKind::Newline {
            seen_non_newline = true;
        }
        depth.step(&token_kind);
        offset += 1;
    }

    false
}

pub(crate) fn find_expression_end_index(
    tokens: &[Token],
    start_index: usize,
    stop_tokens: &[TokenKind],
) -> usize {
    let mut index = start_index;
    let mut depth = ExpressionBoundaryDepth::default();

    while index < tokens.len() {
        let token_kind = &tokens[index].kind;

        if depth.is_top_level() && stop_tokens.iter().any(|stop| token_kind == stop) {
            break;
        }

        depth.step(token_kind);

        if matches!(token_kind, TokenKind::Eof) {
            break;
        }

        index += 1;
    }

    index
}

pub(crate) fn consume_balanced_template_region<E>(
    token_stream: &mut FileTokens,
    mut on_token: impl FnMut(Token, &TokenKind),
    on_eof_error: impl Fn(SourceSpan) -> E,
) -> Result<(), E> {
    let mut balance = TemplateBalance::with_opening_template();

    while balance.has_unclosed_templates() {
        let token_kind = token_stream.current_token_kind().clone();
        if matches!(token_kind, TokenKind::Eof) {
            return Err(on_eof_error(token_stream.current_span()));
        }

        balance.step(&token_kind);
        on_token(token_stream.current_token(), &token_kind);
        token_stream.advance();
    }

    Ok(())
}

/// Consume a header-owned balanced template through the canonical source owner.
///
/// The compatibility stream is advanced only to keep deferred parser cursors synchronized; all
/// tags and spans observed by the callback come from checked `TokenRef`s. The opening token is
/// already consumed by the caller, so the returned index is the first token after the close.
pub(crate) fn consume_balanced_template_region_from_source<E>(
    token_stream: &mut FileTokens,
    opening_index: usize,
    mut on_token: impl FnMut(TokenRef<'_>),
    on_eof_error: impl Fn(SourceSpan) -> E,
    on_infrastructure_error: impl Fn(CompilerError) -> E,
) -> Result<usize, E> {
    let opening = TokenIndex::try_from_index(opening_index)
        .ok_or_else(|| on_infrastructure_error(CompilerError::compiler_error(
            "template opening exceeded the source token index space",
        )))?;
    let expected_current = opening_index
        .checked_add(1)
        .ok_or_else(|| on_infrastructure_error(CompilerError::compiler_error(
            "template body index exceeded the source token index space",
        )))?;
    if token_stream.index != expected_current {
        return Err(on_infrastructure_error(CompilerError::compiler_error(
            "template compatibility cursor was not positioned after its opening token",
        )));
    }

    {
        let canonical = token_stream
            .source_tokens()
            .map_err(&on_infrastructure_error)?;
        if canonical.source() != token_stream.file_id {
            return Err(on_infrastructure_error(CompilerError::compiler_error(
                "template source token owner does not match its file identity",
            )));
        }
        if token_stream.length != canonical.len() || token_stream.tokens.len() != canonical.len() {
            return Err(on_infrastructure_error(CompilerError::compiler_error(
                "template token adapter length does not match its source owner",
            )));
        }
        let opening_token = canonical.token(opening).map_err(|error| {
            on_infrastructure_error(CompilerError::compiler_error(format!(
                "template opening exceeded its source token owner: {error:?}",
            )))
        })?;
        if opening_token.tag() != TokenTag::TEMPLATE_HEAD {
            return Err(on_infrastructure_error(CompilerError::compiler_error(
                "template range did not begin with a template opening token",
            )));
        }
    }

    let mut balance = TemplateBalance::with_opening_template();
    while balance.has_unclosed_templates() {
        let index = token_stream.index;
        let position = TokenIndex::try_from_index(index).ok_or_else(|| {
            on_infrastructure_error(CompilerError::compiler_error(
                "template token index exceeded its checked domain",
            ))
        })?;

        {
            let canonical = token_stream
                .source_tokens()
                .map_err(&on_infrastructure_error)?;
            if canonical.source() != token_stream.file_id {
                return Err(on_infrastructure_error(CompilerError::compiler_error(
                    "template source token owner changed its file identity",
                )));
            }
            let token = canonical.token(position).map_err(|error| {
                on_infrastructure_error(CompilerError::compiler_error(format!(
                    "template token index exceeded its source token owner: {error:?}",
                )))
            })?;
            if token.is_eof() {
                return Err(on_eof_error(token.source_span()));
            }
            balance.step_tag(token.tag());
            on_token(token);
        }

        token_stream.advance();
    }

    Ok(token_stream.index)
}
