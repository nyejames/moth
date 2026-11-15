//! Shared frontend token-scanning utilities.
//!
//! WHAT: centralizes reusable delimiter-depth and balanced-template scan helpers.
//! WHY: shared depth bookkeeping keeps declaration, multi-bind, header, expression-boundary and
//! template scanners consistent.
//!
//! This module owns generic scan mechanics only.
//! It does NOT own statement/feature semantics or diagnostics policy.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::SourceScopeConstruct;
use crate::compiler_frontend::compiler_messages::diagnostics::{
    draft::DiagnosticDraft,
    families::{templates, token_lexical},
    labels::LabelDraftMessage,
};
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::declaration_syntax::declaration_shell::parse_binding_target_syntax;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::style_directives::{INSERT_DIRECTIVE_NAME, SLOT_DIRECTIVE_NAME};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TokenCursor, TokenIndex, TokenRange, TokenRangeError, TokenRef, TokenTag,
};
use rustc_hash::FxHashSet;

/// Failure lanes for declaration-initializer scanning.
///
/// User-authored unterminated constructs remain structured diagnostics. An impossible scanner
/// state stays a compiler error so the declaration-shell boundary can preserve it as
/// infrastructure instead of fabricating a user-facing diagnostic.
#[derive(Debug)]
pub(crate) enum TokenScanFailure {
    Diagnostic(DiagnosticDraft),
    Infrastructure(InfrastructureFailure),
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

/// One expression-qualified selection recognized in a token window.
///
/// WHAT: pairs one authored `PATH` handle with the single `SYMBOL` that immediately follows it on
///       the same physical line, keeping both source-local spans.
/// WHY: the syntax-shape decision "is this one qualified reference?" must have exactly one owner.
///      Occurrence collection and shallow initializer-reference scanning read this recognition
///      instead of each re-deriving an ad-hoc predecessor check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScannedQualifiedSelection {
    pub(crate) path_syntax_id: PathSyntaxId,
    pub(crate) path_span: LocalSpan,
    pub(crate) selected_name: StringId,
    pub(crate) selected_span: LocalSpan,
}

/// Recognize one expression-qualified selection whose `PATH` token sits at `path_index`.
///
/// WHAT: recognizes a dependency path immediately followed by one symbol token.
/// WHY: whitespace produces no token, so an authored physical newline between the path and its
///      selected symbol can only appear as a `NEWLINE` token. Requiring the next token to be the
///      `SYMBOL` therefore enforces the same-line contract without a second line index or a
///      second path parser. Recognition decides syntax shape only; provider and value validity
///      belong to later stages.
pub(crate) fn recognize_qualified_selection(
    tokens: TokenFactView<'_>,
    path_index: usize,
) -> Option<ScannedQualifiedSelection> {
    let path = tokens.get(path_index)?;
    if path.tag != TokenTag::PATH {
        return None;
    }
    let path_syntax_id = path.path_syntax_id?;
    let selected = tokens.get(path_index.checked_add(1)?)?;
    if selected.tag != TokenTag::SYMBOL {
        return None;
    }
    Some(ScannedQualifiedSelection {
        path_syntax_id,
        path_span: path.span,
        selected_name: selected.symbol?,
        selected_span: selected.span,
    })
}

/// Whether the symbol at `symbol_index` is the selected name owned by the preceding path token.
///
/// WHY: a selected name is not an unrelated bare value reference. Shallow scanners must skip it
///      so no false local-name dependency or collision is derived from the occurrence.
pub(crate) fn scanned_symbol_is_qualified_selection(
    tokens: TokenFactView<'_>,
    symbol_index: usize,
) -> bool {
    let Some(path_index) = symbol_index.checked_sub(1) else {
        return false;
    };
    recognize_qualified_selection(tokens, path_index).is_some()
}

/// Scan short-lived scanned facts for symbol-shaped references.
///
/// WHAT: produces `InitializerReference` hints for every bare symbol that is not a
/// dot/namespace accessor, an assignment target, preceded by a dot/double-colon, a direct
/// `$slot`/`$in` target, or the selected symbol of a preceding qualified path.
/// WHY: indexed callers read on demand from slices or canonical stores without projecting
/// a per-query window first; the scan touches only its window.
pub(crate) fn collect_scanned_symbol_references(
    tokens: TokenFactView<'_>,
    source_id: SourceId,
    string_table: &StringTable,
) -> Vec<InitializerReference> {
    let mut references = Vec::new();

    for index in 0..tokens.len() {
        let Some(token) = tokens.get(index) else {
            continue;
        };

        let Some(name) = token.symbol.filter(|_| token.tag == TokenTag::SYMBOL) else {
            continue;
        };

        let previous = index
            .checked_sub(1)
            .and_then(|i| tokens.get(i))
            .map(|t| t.tag);
        if previous == Some(TokenTag::DOT) || previous == Some(TokenTag::DOUBLE_COLON) {
            continue;
        }

        // A symbol immediately after a dependency path is that occurrence's selected name, not a
        // bare value reference. Skipping it here keeps the qualified occurrence the single owner
        // of the selected token, so no false local-name edge or collision is derived.
        if scanned_symbol_is_qualified_selection(tokens, index) {
            continue;
        }

        if is_static_slot_target(tokens, index, string_table) {
            continue;
        }

        let next = index
            .checked_add(1)
            .and_then(|next_index| tokens.get(next_index))
            .map(|t| t.tag);
        if next == Some(TokenTag::ASSIGN) || next == Some(TokenTag::AS) {
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

/// Collect every local binding name declared in one const-function body token view.
///
/// WHAT: recognizes explicit declaration targets with the shared declaration-shell grammar, plus
///       loop/catch bindings, multi-bind targets and match payload bindings.
/// WHY: const-body dependency scanning sees parameter and local names as bare symbols. Moth
///      forbids shadowing, so a body-local spelling is never a reference to a top-level
///      declaration; filtering these names prevents false graph edges and false cycles without a
///      second expression parser.
pub(crate) fn collect_body_local_binding_names(
    tokens: TokenFactView<'_>,
    string_table: &mut StringTable,
) -> Result<FxHashSet<StringId>, InfrastructureFailure> {
    let mut names = FxHashSet::default();
    let mut index = 0usize;
    let mut parenthesis_depth = 0usize;
    let mut curly_depth = 0usize;
    while index < tokens.len() {
        let Some(token) = tokens.get(index) else {
            break;
        };
        match token.tag {
            TokenTag::OPEN_PARENTHESIS => parenthesis_depth += 1,
            TokenTag::CLOSE_PARENTHESIS => {
                parenthesis_depth = parenthesis_depth.saturating_sub(1);
            }
            TokenTag::OPEN_CURLY => curly_depth += 1,
            TokenTag::CLOSE_CURLY => {
                curly_depth = curly_depth.saturating_sub(1);
            }
            _ => {}
        }
        let statement_level = parenthesis_depth == 0 && curly_depth == 0;
        if token.tag == TokenTag::TYPE_PARAMETER_BRACKET && statement_level {
            index = collect_bar_bindings(tokens, index, &mut names);
            continue;
        }
        if token.tag == TokenTag::SYMBOL
            && let Some(name) = token.symbol
        {
            // Multi-bind and declaration targets are statement forms. Restrict them to the
            // statement-level token context so map entries, named arguments and field writes
            // cannot make their spellings look like local declarations.
            if statement_level
                && let Some(next) =
                    collect_binding_target_names(tokens, index, name, string_table, &mut names)?
            {
                index = next;
                continue;
            }
            if statement_level {
                // The payload scan walks its own parentheses; ordinary scanning can resume
                // at the next token.
                collect_match_payload_names(tokens, index, &mut names);
            }
        }
        index += 1;
    }
    Ok(names)
}

/// Collect the name or names declared by one statement-level binding target.
///
/// WHAT: parses the target grammar once, then extends the same parse across a comma-separated
///       multi-bind list until its `=` separator.
/// WHY: one parse of the shared binding-target grammar yields both the single target and the comma
///      list, so the collected name set and the resume index cannot disagree. A malformed first
///      target contributes no local names and ends the scan.
fn collect_binding_target_names(
    tokens: TokenFactView<'_>,
    index: usize,
    name: StringId,
    string_table: &mut StringTable,
    names: &mut FxHashSet<StringId>,
) -> Result<Option<usize>, InfrastructureFailure> {
    let mut declaration_cursor = DeclarationCursor::new(tokens.cursor_at(index + 1)?);
    let initial_position = declaration_cursor.position();
    let mut span_builder = ExtendedSpanBuilder::new();
    let Ok(first_target) = parse_binding_target_syntax(
        name,
        &mut declaration_cursor,
        string_table,
        &mut span_builder,
    ) else {
        return Ok(None);
    };
    let consumed = declaration_cursor
        .position()
        .saturating_sub(initial_position);
    let mut cursor = index + 1 + consumed;
    let mut pending = vec![name];

    while let Some(separator) = tokens.get(cursor) {
        match separator.tag {
            TokenTag::COMMA => {
                let Some(next) = tokens.get(cursor + 1) else {
                    break;
                };
                let Some(next_name) = next.symbol.filter(|_| next.tag == TokenTag::SYMBOL) else {
                    break;
                };
                // Each multi-bind target may carry its own annotation or marker after its name.
                let Some(after_name) = tokens.cursor_at(cursor + 2).ok() else {
                    break;
                };
                pending.push(next_name);
                let mut declaration_cursor = DeclarationCursor::new(after_name);
                let initial_position = declaration_cursor.position();
                let mut span_builder = ExtendedSpanBuilder::new();
                let _ = parse_binding_target_syntax(
                    next_name,
                    &mut declaration_cursor,
                    string_table,
                    &mut span_builder,
                );
                let consumed = declaration_cursor
                    .position()
                    .saturating_sub(initial_position);
                let Some(next_cursor) = (cursor + 2).checked_add(consumed) else {
                    break;
                };
                cursor = next_cursor;
            }
            TokenTag::ASSIGN if pending.len() > 1 => {
                names.extend(pending);
                return Ok(Some(cursor + 1));
            }
            _ => break,
        }
    }

    if !first_target.declares_explicitly() {
        return Ok(None);
    }
    names.insert(name);
    Ok(Some(index + 1 + consumed))
}

fn collect_bar_bindings(
    tokens: TokenFactView<'_>,
    start: usize,
    names: &mut FxHashSet<StringId>,
) -> usize {
    let mut inside = false;
    let mut index = start;
    while index < tokens.len() {
        let Some(token) = tokens.get(index) else {
            break;
        };
        if token.tag == TokenTag::TYPE_PARAMETER_BRACKET {
            if inside {
                return index + 1;
            }
            inside = true;
        } else if inside
            && token.tag == TokenTag::SYMBOL
            && let Some(name) = token.symbol
        {
            names.insert(name);
        }
        index += 1;
    }
    index
}

fn collect_match_payload_names(
    tokens: TokenFactView<'_>,
    index: usize,
    names: &mut FxHashSet<StringId>,
) {
    // Match arms are line-oriented. Reusing that boundary keeps a guard call such as
    // `Failed(x) if positive(threshold) =>` from looking like a payload pattern of its own.
    if index > 0
        && !tokens
            .get(index - 1)
            .is_some_and(|token| token.tag == TokenTag::NEWLINE)
    {
        return;
    }
    let Some(open) = tokens.get(index + 1) else {
        return;
    };
    if open.tag != TokenTag::OPEN_PARENTHESIS {
        return;
    }
    let mut depth = 1usize;
    let mut cursor = index + 2;
    let mut payload_names = Vec::new();
    while cursor < tokens.len() {
        let Some(token) = tokens.get(cursor) else {
            break;
        };
        match token.tag {
            TokenTag::OPEN_PARENTHESIS => depth += 1,
            TokenTag::CLOSE_PARENTHESIS => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let Some(arrow) = tokens.get(cursor + 1) else {
                        return;
                    };
                    if arrow.tag != TokenTag::FAT_ARROW {
                        return;
                    }
                    names.extend(payload_names);
                    return;
                }
            }
            TokenTag::SYMBOL => {
                let previous = cursor
                    .checked_sub(1)
                    .and_then(|previous| tokens.get(previous))
                    .map(|token| token.tag);
                let next = tokens.get(cursor + 1).map(|token| token.tag);
                if previous != Some(TokenTag::DOT)
                    && previous != Some(TokenTag::DOUBLE_COLON)
                    && next != Some(TokenTag::OPEN_PARENTHESIS)
                    && next != Some(TokenTag::AS)
                    && let Some(name) = token.symbol
                {
                    // `field as local` names the field before `as`; only the renamed local is a
                    // body binding.
                    payload_names.push(name);
                }
            }
            _ => {}
        }
        cursor += 1;
    }
}

/// Whether the symbol at `index` is the static target name immediately after `$slot` or `$in`.
///
/// WHY: slot and insert target names live in the receiving template's slot schema, not in
/// lexical value scope, so a constant with the same spelling is not a reference here. Only the
/// target is skipped: default expressions and insert values continue through ordinary scanning.
fn is_static_slot_target(
    tokens: TokenFactView<'_>,
    index: usize,
    string_table: &StringTable,
) -> bool {
    let mut previous_index = index;

    while let Some(candidate) = previous_index.checked_sub(1) {
        previous_index = candidate;
        let Some(token) = tokens.get(candidate) else {
            return false;
        };

        match token.tag {
            // The AST slot parser skips authored newlines before the target.
            TokenTag::NEWLINE => continue,
            TokenTag::STYLE_DIRECTIVE => {
                return tokens.style_directive_name(candidate).is_some_and(|name| {
                    let name = string_table.resolve(name);
                    name == SLOT_DIRECTIVE_NAME || name == INSERT_DIRECTIVE_NAME
                });
            }
            _ => return false,
        }
    }

    false
}

/// One short-lived scanned token fact for tag-based shared scanners.
///
/// WHAT: carries the stable `TokenTag`, optional interned symbol payload and exact
///       source-local span for one token in a scan window.
/// WHY: canonical source/token views share one scan implementation without retaining durable
///      references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScannedToken {
    pub(crate) tag: TokenTag,
    pub(crate) symbol: Option<StringId>,
    pub(crate) path_syntax_id: Option<PathSyntaxId>,
    pub(crate) span: LocalSpan,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TokenFactView<'a> {
    Canonical {
        tokens: &'a SourceTokens,
        start: usize,
        end: usize,
    },
}

impl<'a> TokenFactView<'a> {
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
    pub(crate) fn subrange(self, start: usize, end: usize) -> Option<Self> {
        if start > end || end > self.len() {
            return None;
        }
        let Self::Canonical {
            tokens,
            start: base,
            end: base_end,
        } = self;
        let bounded_start = base.checked_add(start)?;
        let bounded_end = base.checked_add(end)?;
        (bounded_end <= base_end).then_some(Self::Canonical {
            tokens,
            start: bounded_start,
            end: bounded_end,
        })
    }

    /// Construct a bounded canonical cursor beginning at `offset` in this view.
    ///
    /// WHAT: exposes the same checked token owner the view scans so declaration-shell parsers can
    ///       read one local binding target without materialising a separate token window.
    /// WHY: const-body dependency filtering must reuse the binding grammar instead of inventing a
    ///      second declaration recognizer.
    fn cursor_at(self, offset: usize) -> Result<TokenCursor<'a>, InfrastructureFailure> {
        let Self::Canonical { tokens, start, end } = self;
        let absolute = start.checked_add(offset).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "const body local-binding cursor offset overflowed",
            )
        })?;
        if absolute > end {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "const body local-binding cursor escaped its token view",
            ));
        }
        let start = u32::try_from(absolute).map_err(|_| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "const body local-binding token index exceeds its representation",
            )
        })?;
        let end = u32::try_from(end).map_err(|_| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "const body local-binding token end exceeds its representation",
            )
        })?;
        let range = TokenRange::try_new_for(
            tokens,
            TokenIndex::from_raw(start),
            TokenIndex::from_raw(end),
        )
        .map_err(|error| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!("const body local-binding token range failed: {error:?}"),
            )
        })?;
        TokenCursor::new(tokens, range).map_err(|error| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!("const body local-binding cursor failed: {error:?}"),
            )
        })
    }

    pub(crate) fn len(self) -> usize {
        match self {
            Self::Canonical { start, end, .. } => end - start,
        }
    }

    pub(crate) fn get(self, index: usize) -> Option<ScannedToken> {
        let Self::Canonical { tokens, start, end } = self;
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

    /// The interned `$directive` name when the token at `index` is a style directive.
    ///
    /// WHY: directive identity is a token payload rather than a symbol, so reference scanning
    /// needs one checked payload read to recognise directives whose next symbol is not a value.
    fn style_directive_name(self, index: usize) -> Option<StringId> {
        let Self::Canonical { tokens, start, end } = self;
        let absolute = start.checked_add(index)?;
        if absolute >= end {
            return None;
        }
        let shape = tokens.shapes().get(absolute).copied()?;
        if shape.tag() != TokenTag::STYLE_DIRECTIVE {
            return None;
        }
        shape.string_id()
    }
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
}

/// Count the remaining top-level positional items of a parenthesised list, from the
/// cursor through the list's matching `)`.
///
/// WHAT: call arguments and choice payload captures report the total authored count when
/// they reject the first extra item.
/// WHY: the list's opening parenthesis is already consumed, so `NestingDepth` starts at zero.
/// A named entry (`name = ...`) is reported by its own diagnostic, so the scan stops there
/// and the count names positional items only.
pub(crate) fn count_remaining_top_level_positional_items(token_stream: &AstCursor) -> usize {
    let mut depth = NestingDepth::default();
    let mut offset = 0usize;
    let mut count = 0usize;
    let mut in_item = false;
    let remaining = token_stream
        .length()
        .saturating_sub(token_stream.position());

    while offset < remaining {
        let Some(token) = token_stream.token_ref_at_offset(offset) else {
            break;
        };
        let tag = token.tag();
        if tag == TokenTag::EOF {
            break;
        }

        if depth.is_top_level() {
            if tag == TokenTag::CLOSE_PARENTHESIS {
                break;
            }
            if tag == TokenTag::COMMA {
                in_item = false;
                offset += 1;
                continue;
            }
            if tag == TokenTag::NEWLINE {
                offset += 1;
                continue;
            }
            if !in_item {
                if item_at_offset_is_named(token_stream, offset) {
                    break;
                }
                count += 1;
                in_item = true;
            }
        }

        depth.step_tag(tag);
        offset += 1;
    }
    count
}

fn item_at_offset_is_named(token_stream: &AstCursor, offset: usize) -> bool {
    token_stream
        .token_ref_at_offset(offset)
        .map(|token| token.tag())
        == Some(TokenTag::SYMBOL)
        && token_stream
            .token_ref_at_offset(offset.saturating_add(1))
            .map(|token| token.tag())
            == Some(TokenTag::ASSIGN)
}

/// The innermost open construct tracked by the declaration-initializer scanner.
///
/// WHAT: models which construct owns the next expected closing delimiter at EOF.
/// WHY: the fixed `]` fallback could misreport the delimiter inside a value-producing
///      block, a catch body, a parenthesis or a collection. Each construct owns its
///      own terminator and must report it precisely at end-of-file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenConstruct {
    Template,
    Parenthesis,
    CollectionOrMap,
    PipeList,
    /// A `;`-terminated source scope opened inside the declaration initializer.
    SourceScope(SourceScopeConstruct),
}

impl OpenConstruct {
    fn expected_delimiter(self) -> &'static str {
        match self {
            OpenConstruct::Template => "]",
            OpenConstruct::Parenthesis => ")",
            OpenConstruct::CollectionOrMap => "}",
            OpenConstruct::PipeList => "|",
            OpenConstruct::SourceScope(_) => ";",
        }
    }

    /// The source construct this opener began, when it closes with `;`.
    ///
    /// WHAT: distinguishes a missing source closer from an unrelated expected delimiter.
    /// WHY: a `;`-terminated block is a source scope, so its truncation must name the construct
    ///      rather than the delimiter family that an open `(` or `{` still owns.
    const fn source_scope(self) -> Option<SourceScopeConstruct> {
        match self {
            OpenConstruct::SourceScope(construct) => Some(construct),
            OpenConstruct::Template
            | OpenConstruct::Parenthesis
            | OpenConstruct::CollectionOrMap
            | OpenConstruct::PipeList => None,
        }
    }
}

/// The construct an `if` header opened, refined to a match when `is` ends its header.
///
/// WHAT: the token before the body colon is the one fact that distinguishes `if cond:` from
///       `if subject is:` in a declaration initializer.
/// WHY: both forms open a source scope that closes with `;`, but the report must name the
///      construct the author wrote.
fn if_header_construct(previous_meaningful_tag: Option<TokenTag>) -> SourceScopeConstruct {
    if previous_meaningful_tag == Some(TokenTag::IS) {
        SourceScopeConstruct::MatchBlock
    } else {
        SourceScopeConstruct::IfBlock
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordPipeAction {
    Open,
    Close,
    Ignore,
}

/// Shared `|...|` member-list lookahead over an index-addressed token view.
///
/// Both canonical cursor callers supply one lightweight fact probe, keeping lookahead boundaries
/// identical without cloning non-`Copy` token payloads.
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

/// Only the distinctions consumed by the shared lookahead are modeled, so canonical callers avoid
/// cloning full token payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PipeMemberFact {
    tag: TokenTag,
    shape: PipeMemberShape,
}

impl PipeMemberFact {
    fn of_ref(token: TokenRef<'_>) -> Self {
        Self::of_tag(token.tag())
    }

    fn of_tag(tag: TokenTag) -> Self {
        Self {
            tag,
            shape: PipeMemberShape::of_tag(tag),
        }
    }
}

/// Narrow shape probe for pipe-list member classification.
///
/// Only the distinctions consumed by the shared lookahead are modeled, so canonical callers avoid
/// cloning full token payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PipeMemberShape {
    Name,
    Value,
    Other,
}
impl PipeMemberShape {
    fn of_tag(tag: TokenTag) -> Self {
        match tag {
            TokenTag::SYMBOL | TokenTag::THIS => Self::Name,
            TokenTag::NUMERIC_LITERAL
            | TokenTag::STRING_SLICE_LITERAL
            | TokenTag::RAW_STRING_LITERAL
            | TokenTag::CHAR_LITERAL
            | TokenTag::BOOL_LITERAL
            | TokenTag::NONE_LITERAL
            | TokenTag::PATH => Self::Value,
            _ => Self::Other,
        }
    }
}

/// True when `|` at `pipe_offset` opens a balanced `|...|` member list on a live cursor.
///
/// This reads through [`AstCursor::token_ref_at_offset`] so the multi-bind lookahead follows the
fn pipe_opens_member_list_at_cursor(
    token_stream: &AstCursor,
    pipe_offset: usize,
    allow_value_first: bool,
) -> bool {
    pipe_opens_member_list_with(
        |offset| {
            token_stream
                .token_ref_at_offset(offset)
                .map(PipeMemberFact::of_ref)
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

/// One construct the declaration-initializer scanner has opened and not yet closed.
///
/// WHAT: pairs the construct kind with the authored opener that owns its closing delimiter.
/// WHY: the end-of-file diagnostic labels that opener, so the kind and its span travel as one
///      stack entry. Separate stacks could drift and label the wrong token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OpenConstructSite {
    construct: OpenConstruct,
    opening_span: SourceSpan,
}

impl OpenConstructSite {
    const fn new(construct: OpenConstruct, opening_span: SourceSpan) -> Self {
        Self {
            construct,
            opening_span,
        }
    }
}

fn close_open_construct(open_constructs: &mut Vec<OpenConstructSite>, expected: OpenConstruct) {
    if open_constructs
        .last()
        .is_some_and(|site| site.construct == expected)
    {
        open_constructs.pop();
    }
}

/// Pop the innermost open source scope when a `;` closes it.
///
/// WHAT: returns whether a source scope was closed, so the caller can record the closer span.
/// WHY: the end-of-file report names the last `;` only when it closed a direct child of the
///      still-open innermost scope.
fn close_statement_construct(open_constructs: &mut Vec<OpenConstructSite>) -> bool {
    if open_constructs
        .last()
        .is_some_and(|site| site.construct.source_scope().is_some())
    {
        open_constructs.pop();
        true
    } else {
        false
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
}

/// Tracks template nesting for header-stage scans.
///
/// WHAT: balances `TEMPLATE_HEAD`/`TEMPLATE_CLOSE` tokens.
/// WHY: header-stage scanners run before AST body parsing and only need the token-level balance
/// that tokenization guarantees. Whether a balanced bracket group later becomes literal text
/// under a suppressing style is a body-parser decision, not a header-scan exception.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TemplateNestingTracker {
    balance: TemplateBalance,
}

impl TemplateNestingTracker {
    /// Starts a scan whose authored opening token is already consumed.
    pub(crate) fn with_opening_template() -> Self {
        Self {
            balance: TemplateBalance::with_opening_template(),
        }
    }

    pub(crate) fn has_unclosed_templates(self) -> bool {
        self.balance.has_unclosed_templates()
    }

    /// Steps over one token tag.
    pub(crate) fn step_tag(&mut self, tag: TokenTag) {
        self.balance.step_tag(tag);
    }
}

/// One still-open delimiter found by the EOF scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnclosedDelimiter {
    Template(SourceSpan),
    Parenthesis(SourceSpan),
    Collection(SourceSpan),
}

impl UnclosedDelimiter {
    /// The delimiter EOF is missing.
    pub(crate) const fn expected_delimiter(self) -> &'static str {
        match self {
            Self::Template(_) => "]",
            Self::Parenthesis(_) => ")",
            Self::Collection(_) => "}",
        }
    }

    /// The authored opener that still owns a closing delimiter.
    const fn opening_span(self) -> SourceSpan {
        match self {
            Self::Template(span) | Self::Parenthesis(span) | Self::Collection(span) => span,
        }
    }
}

/// Which kind of opener the forward EOF scan found.
#[derive(Clone, Copy, Debug)]
enum UnclosedDelimiterKind {
    Template,
    Parenthesis,
    Collection,
}

/// The innermost opener still unclosed when `eof` truncates a token region.
///
/// WHAT: forward-scans from `start` to `eof`, balancing templates, parentheses and braces
/// independently. The unmatched opener closest to `eof` has no matching close to its right, so it
/// is the innermost unmatched opener.
/// WHY: template tokens carry their own open/close structure, so a reverse pass over authored
/// template heads identifies the innermost unmatched opener without reconstructing literal-body
/// rules. The scan only runs on the EOF diagnostic path.
///
/// `|...|` member lists are not tracked here. `TYPE_PARAMETER_BRACKET` is shared by every pipe
/// use, so recognising an opener needs the declaration-initializer scanner's lookahead
/// classification. A truncation there reports the enclosing source scope when one is open, or the
/// generic delimiter path otherwise.
pub(crate) fn innermost_unclosed_delimiter(
    source: &SourceTokens,
    start: TokenIndex,
    eof: TokenIndex,
) -> Option<UnclosedDelimiter> {
    walk_to_unclosed_delimiter(source, start, eof, true)
}

/// The innermost unclosed template opener, for scanners that only diagnose missing `]`.
///
/// Parentheses and braces are ignored, so an open `(` inside a nested template head still
/// labels that nested template rather than an outer one.
pub(crate) fn innermost_unclosed_template_opening(
    source: &SourceTokens,
    start: TokenIndex,
    eof: TokenIndex,
) -> Option<SourceSpan> {
    match walk_to_unclosed_delimiter(source, start, eof, false) {
        Some(UnclosedDelimiter::Template(span)) => Some(span),
        Some(UnclosedDelimiter::Parenthesis(_) | UnclosedDelimiter::Collection(_)) | None => None,
    }
}

/// Builds the EOF diagnostic for a token region left open by a truncated scan.
///
/// WHAT: names a missing template bracket or the innermost unmatched `)`/`}`, falling back to
/// the caller's expected delimiter (or a bare EOF) when the region balances.
/// WHY: header function-body capture and signature-member default scanning share one EOF rule;
/// keeping it here makes their template-vs-delimiter precedence and wording identical. The
/// opener label is the only link from the truncation back to the authored `(` or `{`, so callers
/// pass the opener the scan already found rather than rescanning the region here.
pub(crate) fn unclosed_delimiter_eof_diagnostic(
    unclosed: Option<UnclosedDelimiter>,
    eof_span: Option<SourceSpan>,
    fallback_expected_delimiter: Option<&'static str>,
    string_table: &mut StringTable,
) -> DiagnosticDraft {
    match unclosed {
        Some(UnclosedDelimiter::Template(opening)) => {
            templates::missing_closing_bracket(eof_span, Some(opening))
        }
        Some(delimiter) => token_lexical::unexpected_end_of_file(
            Some(string_table.intern(delimiter.expected_delimiter())),
            eof_span,
        )
        .with_label(
            Some(delimiter.opening_span()),
            Some(LabelDraftMessage::UnclosedDelimiterStartsHere),
        ),
        None => token_lexical::unexpected_end_of_file(
            fallback_expected_delimiter.map(|delimiter| string_table.intern(delimiter)),
            eof_span,
        ),
    }
}

fn walk_to_unclosed_delimiter(
    source: &SourceTokens,
    start: TokenIndex,
    eof: TokenIndex,
    track_groups: bool,
) -> Option<UnclosedDelimiter> {
    let mut templates = Vec::new();
    let mut parentheses = Vec::new();
    let mut collections = Vec::new();

    let end = eof.index().min(source.len());
    for index in start.index()..end {
        let token_index = TokenIndex::try_from_index(index)?;
        let token = source.token(token_index).ok()?;
        match token.tag() {
            TokenTag::TEMPLATE_HEAD => templates.push(token_index),
            TokenTag::TEMPLATE_CLOSE => {
                templates.pop();
            }
            _ if !track_groups => {}
            TokenTag::OPEN_PARENTHESIS => parentheses.push(token_index),
            TokenTag::CLOSE_PARENTHESIS => {
                parentheses.pop();
            }
            TokenTag::OPEN_CURLY => collections.push(token_index),
            TokenTag::CLOSE_CURLY => {
                collections.pop();
            }
            _ => {}
        }
    }

    let mut candidates = Vec::new();
    candidates.extend(
        templates
            .last()
            .copied()
            .map(|index| (index, UnclosedDelimiterKind::Template)),
    );
    candidates.extend(
        parentheses
            .last()
            .copied()
            .map(|index| (index, UnclosedDelimiterKind::Parenthesis)),
    );
    candidates.extend(
        collections
            .last()
            .copied()
            .map(|index| (index, UnclosedDelimiterKind::Collection)),
    );
    let (index, kind) = candidates
        .into_iter()
        .max_by_key(|(index, _)| index.index())?;
    let span = source.token(index).ok()?.source_span();
    Some(match kind {
        UnclosedDelimiterKind::Template => UnclosedDelimiter::Template(span),
        UnclosedDelimiterKind::Parenthesis => UnclosedDelimiter::Parenthesis(span),
        UnclosedDelimiterKind::Collection => UnclosedDelimiter::Collection(span),
    })
}

/// Result type for declaration-initializer token scanning.
///
/// Structured unexpected-EOF diagnostics stay in the diagnostic lane while impossible scanner
/// states remain typed infrastructure failures for the declaration-shell boundary.
///
/// Scan one declaration initializer directly through a canonical cursor.
///
/// The scan keeps only short-lived tag facts for delimiter decisions. The returned shell retains
/// the checked source range and derives reference hints from that range, never a cloned token
/// vector. The cursor remains on the declaration boundary so its caller can continue the outer
/// parser walk.
pub(crate) fn collect_declaration_initializer_range(
    cursor: &mut crate::compiler_frontend::tokenizer::tokens::TokenCursor<'_>,
    string_table: &mut StringTable,
) -> Result<(TokenRange, Vec<InitializerReference>), TokenScanFailure> {
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
    // Loops open a statement block as well. Without their own pending flag a `loop:` inside a
    // value-producing `if` leaves the nested `;` to close the outer block.
    let mut loop_header_pending = false;
    let mut inline_value_if_missing_else_depth = 0usize;
    let mut initializer_closed_by_statement_block = false;
    let mut open_constructs: Vec<OpenConstructSite> = Vec::new();
    // The most recent `;` that closed a nested source scope, with the stack depth left behind.
    // At end of file that depth identifies a direct child of the reported innermost scope.
    let mut last_inner_closer: Option<(usize, SourceSpan)> = None;
    let mut record_pipe_depth = 0usize;
    let mut last_closed_record_pipe = false;
    // The token before a body colon distinguishes `if cond:` from `if subject is:`.
    let mut previous_meaningful_tag: Option<TokenTag> = None;

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
            let next_continues = next_non_newline_tag.is_some_and(TokenTag::continues_expression);
            let continues_to_authored_else = inline_value_if_missing_else_depth > 0
                && next_non_newline_tag == Some(TokenTag::ELSE);
            let continues_to_catch_header = catch_header_pending || inline_catch_value_pending;
            previous_continues
                || next_continues
                || continues_to_authored_else
                || continues_to_catch_header
        } else {
            false
        };

        let boundary_end = if at_top_level
            && matches!(current_tag, TokenTag::COMMA | TokenTag::END | TokenTag::EOF)
        {
            let include_boundary = inline_value_if_missing_else_depth > 0
                || collected
                    .last()
                    .is_some_and(|token| token.tag == TokenTag::ELSE);
            Some(if include_boundary {
                TokenIndex::try_from_index(current_ref.index().index().saturating_add(1))
                    .ok_or_else(|| {
                        TokenScanFailure::Infrastructure(InfrastructureFailure::internal(
                            CompilerStage::AstConstruction,
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
                || collected
                    .last()
                    .is_some_and(|token| token.tag == TokenTag::ELSE);
            Some(if include_boundary {
                TokenIndex::try_from_index(current_ref.index().index().saturating_add(1))
                    .ok_or_else(|| {
                        TokenScanFailure::Infrastructure(InfrastructureFailure::internal(CompilerStage::AstConstruction,
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
                TokenScanFailure::Infrastructure(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!("declaration initializer range was invalid: {error:?}",),
                ))
            })?;
            let facts =
                TokenFactView::from_source_range(source_tokens, range).map_err(|error| {
                    TokenScanFailure::Infrastructure(InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        format!("declaration initializer reference view was invalid: {error:?}",),
                    ))
                })?;
            return Ok((
                range,
                collect_scanned_symbol_references(facts, source_id, string_table),
            ));
        }

        if current_tag == TokenTag::EOF && (!at_top_level || inside_record_region) {
            let Some(innermost) = open_constructs.last().copied() else {
                return Err(TokenScanFailure::Infrastructure(
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "declaration-initializer scanner reported a nested state with no open construct",
                    ),
                ));
            };
            if innermost.construct == OpenConstruct::Template {
                return Err(TokenScanFailure::Diagnostic(
                    templates::missing_closing_bracket(
                        Some(current_ref.source_span()),
                        Some(innermost.opening_span),
                    ),
                ));
            }
            // A `;`-terminated block names the construct that never closed. Its opener is the
            // only authored site that explains why the initializer has not ended yet.
            if let Some(construct) = innermost.construct.source_scope() {
                // Name the last closer only when it closed a direct child of the reported
                // innermost scope and nothing has opened above it since.
                let inner_closer = last_inner_closer
                    .filter(|(closer_depth, _)| *closer_depth == open_constructs.len())
                    .map(|(_, closer)| closer);
                let diagnostic = token_lexical::unclosed_source_scope(
                    construct,
                    Some(current_ref.source_span()),
                    innermost.opening_span,
                    inner_closer,
                );
                return Err(TokenScanFailure::Diagnostic(diagnostic));
            }
            let expected_delimiter = Some(
                string_table.get_or_intern(innermost.construct.expected_delimiter().to_owned()),
            );
            return Err(TokenScanFailure::Diagnostic(
                token_lexical::unexpected_end_of_file(
                    expected_delimiter,
                    Some(current_ref.source_span()),
                )
                .with_label(
                    Some(innermost.opening_span),
                    Some(LabelDraftMessage::UnclosedDelimiterStartsHere),
                ),
            ));
        }

        if depth.is_top_level() {
            match current_tag {
                TokenTag::CATCH => {
                    catch_header_pending = true;
                    inline_catch_value_pending = false;
                }
                TokenTag::IF => value_if_header_pending = true,
                TokenTag::LOOP => loop_header_pending = true,
                TokenTag::COLON if catch_header_pending => {
                    catch_header_pending = false;
                    value_if_header_pending = false;
                    loop_header_pending = false;
                    catch_block_depth = catch_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstructSite::new(
                        OpenConstruct::SourceScope(SourceScopeConstruct::CatchHandler),
                        current_ref.source_span(),
                    ));
                }
                // The catch counter balances every nested colon so the closing `;` pairs with
                // the same counter that opened it. The pushed site still names the header that
                // opened this block, so an end-of-file report can name the innermost construct.
                TokenTag::COLON if catch_block_depth > 0 => {
                    let construct = if value_if_header_pending {
                        if_header_construct(previous_meaningful_tag)
                    } else if loop_header_pending {
                        SourceScopeConstruct::LoopBlock
                    } else {
                        // A colon with no recognised header still belongs to the catch body, so
                        // the catch remains the innermost scope that can be named.
                        SourceScopeConstruct::CatchHandler
                    };
                    value_if_header_pending = false;
                    loop_header_pending = false;
                    catch_block_depth = catch_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstructSite::new(
                        OpenConstruct::SourceScope(construct),
                        current_ref.source_span(),
                    ));
                }
                TokenTag::COLON if value_if_header_pending || loop_header_pending => {
                    let construct = if value_if_header_pending {
                        if_header_construct(previous_meaningful_tag)
                    } else {
                        SourceScopeConstruct::LoopBlock
                    };
                    value_if_header_pending = false;
                    loop_header_pending = false;
                    value_if_block_depth = value_if_block_depth.saturating_add(1);
                    open_constructs.push(OpenConstructSite::new(
                        OpenConstruct::SourceScope(construct),
                        current_ref.source_span(),
                    ));
                }
                // A colon without an `if`, `loop` or `catch` header above it cannot open a
                // block. Match-arm headers such as `1:` or `else:` stay arm syntax, and the arm
                // parser reports the missing `=>` instead of the scanner swallowing the block's
                // `;`.
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
                    loop_header_pending = false;
                    initializer_closed_by_statement_block = closing_outer_catch_block;
                    if close_statement_construct(&mut open_constructs) {
                        last_inner_closer =
                            Some((open_constructs.len(), current_ref.source_span()));
                    }
                }
                TokenTag::END if value_if_block_depth > 0 => {
                    let closing_outer_value_if_block = value_if_block_depth == 1;
                    value_if_block_depth = value_if_block_depth.saturating_sub(1);
                    value_if_header_pending = false;
                    loop_header_pending = false;
                    initializer_closed_by_statement_block = closing_outer_value_if_block;
                    if close_statement_construct(&mut open_constructs) {
                        last_inner_closer =
                            Some((open_constructs.len(), current_ref.source_span()));
                    }
                }
                // A newline may continue an `if` or `loop` condition, so only the catch header
                // clears here. `then` and `->` end an inline header, and EOF ends the scan.
                TokenTag::NEWLINE => {
                    catch_header_pending = false;
                }
                TokenTag::THEN | TokenTag::ARROW | TokenTag::EOF => {
                    catch_header_pending = false;
                    value_if_header_pending = false;
                    loop_header_pending = false;
                }
                _ => {}
            }
        }
        if current_tag != TokenTag::NEWLINE && current_tag != TokenTag::THEN {
            inline_catch_value_pending = false;
        }

        match current_tag {
            TokenTag::OPEN_PARENTHESIS => {
                open_constructs.push(OpenConstructSite::new(
                    OpenConstruct::Parenthesis,
                    current_ref.source_span(),
                ));
            }
            TokenTag::CLOSE_PARENTHESIS => {
                close_open_construct(&mut open_constructs, OpenConstruct::Parenthesis);
            }
            TokenTag::OPEN_CURLY => {
                open_constructs.push(OpenConstructSite::new(
                    OpenConstruct::CollectionOrMap,
                    current_ref.source_span(),
                ));
            }
            TokenTag::CLOSE_CURLY => {
                close_open_construct(&mut open_constructs, OpenConstruct::CollectionOrMap);
            }
            TokenTag::TEMPLATE_HEAD => {
                open_constructs.push(OpenConstructSite::new(
                    OpenConstruct::Template,
                    current_ref.source_span(),
                ));
            }
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
                        open_constructs.push(OpenConstructSite::new(
                            OpenConstruct::PipeList,
                            current_ref.source_span(),
                        ));
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
            current_tag,
            TokenTag::TYPE_PARAMETER_BRACKET | TokenTag::NEWLINE
        ) {
            last_closed_record_pipe = false;
        }
        if current_tag != TokenTag::NEWLINE {
            previous_meaningful_tag = Some(current_tag);
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
        TokenScanFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("declaration initializer range was invalid at stream end: {error:?}",),
        ))
    })?;
    let facts = TokenFactView::from_source_range(source_tokens, range).map_err(|error| {
        TokenScanFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("declaration initializer reference view was invalid at stream end: {error:?}",),
        ))
    })?;
    Ok((
        range,
        collect_scanned_symbol_references(facts, source_id, string_table),
    ))
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
            lookahead.peek().map(PipeMemberFact::of_ref)
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
    let remaining_tokens = token_stream
        .length()
        .saturating_sub(token_stream.position());
    let mut seen_non_newline = false;

    while offset < remaining_tokens {
        let Some(token) = token_stream.token_ref_at_offset(offset) else {
            break;
        };
        let tag = token.tag();
        if tag == TokenTag::TYPE_PARAMETER_BRACKET {
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

        if record_pipe_depth == 0 && depth.is_top_level() && tag == TokenTag::COMMA {
            return true;
        }

        // A comma in the producer does not turn a single receiver into a target list.
        if record_pipe_depth == 0 && depth.is_top_level() && tag.is_assignment_operator() {
            break;
        }

        if record_pipe_depth == 0
            && depth.is_top_level()
            && matches!(
                tag,
                TokenTag::NEWLINE
                    | TokenTag::END
                    | TokenTag::EOF
                    | TokenTag::CATCH
                    | TokenTag::COLON
                    | TokenTag::ELSE
                    | TokenTag::THEN
            )
        {
            break;
        }

        if tag != TokenTag::NEWLINE {
            seen_non_newline = true;
        }
        depth.step_tag(tag);
        offset += 1;
    }

    false
}

/// Consume a header-owned balanced template through the canonical source owner.
///
/// The opening token is already consumed by the caller; the returned position is the first token
/// after the close. Tags and spans are read only through checked `TokenRef`s owned by the cursor's
/// canonical source, so this helper never creates a separate token window.
pub(crate) fn consume_balanced_template_region_from_source<E>(
    cursor: &mut TokenCursor<'_>,
    opening: TokenRef<'_>,
    mut on_token: impl FnMut(TokenRef<'_>),
    on_eof_error: impl Fn(SourceSpan, SourceSpan) -> E,
    on_infrastructure_error: impl Fn(InfrastructureFailure) -> E,
) -> Result<TokenIndex, E> {
    let source_tokens = cursor.source_tokens();
    let source_id = source_tokens.source();
    if opening.source() != source_id {
        return Err(on_infrastructure_error(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "template opening source token owner does not match its cursor owner",
        )));
    }

    let expected_current = opening
        .index()
        .index()
        .checked_add(1)
        .and_then(TokenIndex::try_from_index)
        .ok_or_else(|| {
            on_infrastructure_error(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "template body index exceeded the source token index space",
            ))
        })?;
    if cursor.position() != expected_current {
        return Err(on_infrastructure_error(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "template cursor was not positioned after its opening token",
        )));
    }
    if opening.tag() != TokenTag::TEMPLATE_HEAD {
        return Err(on_infrastructure_error(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "template range did not begin with a template opening token",
        )));
    }

    let mut balance = TemplateNestingTracker::with_opening_template();
    while balance.has_unclosed_templates() {
        let token = cursor.current().ok_or_else(|| {
            on_infrastructure_error(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "template cursor exceeded its source token owner before its closing delimiter",
            ))
        })?;
        if token.source() != source_id {
            return Err(on_infrastructure_error(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "template cursor token owner changed its source identity",
            )));
        }
        if token.is_eof() {
            return Err(on_eof_error(
                token.source_span(),
                innermost_unclosed_template_opening(source_tokens, opening.index(), token.index())
                    .unwrap_or_else(|| opening.source_span()),
            ));
        }
        balance.step_tag(token.tag());
        on_token(token);
        let _ = cursor.advance();
    }

    Ok(cursor.position())
}
