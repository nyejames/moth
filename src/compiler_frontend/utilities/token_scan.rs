//! Shared frontend token-scanning utilities.
//!
//! WHAT: centralizes reusable delimiter-depth and balanced-template scan helpers.
//! WHY: declaration parsing, multi-bind parsing, header parsing, expression
//! boundary scanning, and template parsing previously maintained duplicate depth
//! bookkeeping logic.
//!
//! This module owns generic scan mechanics only.
//! It does NOT own statement/feature semantics or diagnostics policy.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};

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
    pub location: SourceLocation,
    pub followed_by_call: bool,
    pub followed_by_choice_namespace: bool,
}

impl InitializerReference {
    /// Remap the reference name and source location into a merged string table.
    ///
    // Called by per-file frontend output remapping before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
        if let Some(dot_member) = &mut self.dot_member {
            *dot_member = remap.get(*dot_member);
        }
        self.location.remap_string_ids(remap);
    }

    pub fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.location.rebind_source_identity(logical_path);
    }
}

/// Scan a token slice for symbol-shaped references.
///
/// WHAT: produces `InitializerReference` hints for every bare symbol that is not a
/// dot/namespace accessor, an assignment target, or preceded by a dot/double-colon.
/// WHY: dependency sorting and capacity-expression reference discovery both need
/// shallow reference facts without duplicating the scan logic.
pub(crate) fn collect_symbol_references(tokens: &[Token]) -> Vec<InitializerReference> {
    let mut references = Vec::new();

    for (index, token) in tokens.iter().enumerate() {
        let TokenKind::Symbol(name) = &token.kind else {
            continue;
        };

        let previous = index
            .checked_sub(1)
            .and_then(|previous_index| tokens.get(previous_index))
            .map(|previous_token| &previous_token.kind);
        if matches!(previous, Some(TokenKind::Dot | TokenKind::DoubleColon)) {
            continue;
        }

        let next = tokens.get(index + 1).map(|next_token| &next_token.kind);
        if matches!(next, Some(TokenKind::Assign)) {
            continue;
        }

        // Header dependency sorting only needs a shallow member hint. AST still owns the full
        // expression parse, but `namespace.member` constants need this member name so dependencies
        // like `intro.content` can create an ordering edge to the imported constant.
        let dot_member = if matches!(next, Some(TokenKind::Dot)) {
            tokens
                .get(index + 2)
                .and_then(|member_token| match &member_token.kind {
                    TokenKind::Symbol(member_name) => Some(*member_name),
                    _ => None,
                })
        } else {
            None
        };

        references.push(InitializerReference {
            name: *name,
            dot_member,
            location: token.location.clone(),
            followed_by_call: matches!(next, Some(TokenKind::OpenParenthesis)),
            followed_by_choice_namespace: matches!(next, Some(TokenKind::DoubleColon)),
        });
    }

    references
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

    pub(crate) fn step(&mut self, token_kind: &TokenKind) {
        match token_kind {
            TokenKind::OpenParenthesis => self.parenthesis = self.parenthesis.saturating_add(1),
            TokenKind::CloseParenthesis => {
                self.parenthesis = self.parenthesis.saturating_sub(1);
            }
            TokenKind::OpenCurly => self.curly = self.curly.saturating_add(1),
            TokenKind::CloseCurly => {
                self.curly = self.curly.saturating_sub(1);
            }
            TokenKind::TemplateHead => self.template = self.template.saturating_add(1),
            TokenKind::TemplateClose => {
                self.template = self.template.saturating_sub(1);
            }
            _ => {}
        }
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
    AnonymousRecord,
}

impl OpenConstruct {
    fn expected_delimiter(self) -> &'static str {
        match self {
            OpenConstruct::Template => "]",
            OpenConstruct::Parenthesis => ")",
            OpenConstruct::CollectionOrMap => "}",
            OpenConstruct::CatchBlock => ";",
            OpenConstruct::ValueIfBlock => ";",
            OpenConstruct::AnonymousRecord => "|",
        }
    }
}

/// True when `|` at `pipe_index` opens `| |` or `| name =` / `| name ,` record syntax.
///
/// Catch bindings (`|err|`) and struct shells (`| name Type |`) stay false so those
/// owners keep their existing pipe grammars.
pub(crate) fn pipe_opens_anonymous_record(tokens: &[Token], pipe_index: usize) -> bool {
    let kind_at = |cursor: usize| tokens.get(cursor).map(|token| &token.kind);
    let skip_newlines = |mut cursor: usize| {
        while matches!(kind_at(cursor), Some(TokenKind::Newline)) {
            cursor += 1;
        }
        cursor
    };

    let mut cursor = skip_newlines(pipe_index + 1);
    if matches!(kind_at(cursor), Some(TokenKind::TypeParameterBracket)) {
        return true;
    }

    if !matches!(kind_at(cursor), Some(TokenKind::Symbol(_))) {
        return false;
    }

    cursor = skip_newlines(cursor + 1);
    matches!(
        kind_at(cursor),
        Some(TokenKind::Assign) | Some(TokenKind::Comma)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordPipeAction {
    Open,
    Close,
    Ignore,
}

/// Classify a `|` as a record opener, closer, or a pipe owned by another grammar.
///
/// Parentheses, collections and templates own inner pipes. A top-level `|` after the
/// opening pipe closes this parameter list so the parser can report a missing value
/// or a nested `|...|` list from the tokens it actually sees.
fn classify_record_pipe(
    tokens: &[Token],
    pipe_index: usize,
    record_pipe_depth: usize,
    nesting_is_top_level: bool,
) -> RecordPipeAction {
    if !nesting_is_top_level {
        return RecordPipeAction::Ignore;
    }

    if record_pipe_depth == 0 && pipe_opens_anonymous_record(tokens, pipe_index) {
        return RecordPipeAction::Open;
    }

    if record_pipe_depth > 0 {
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

    pub(crate) fn step(&mut self, token_kind: &TokenKind) {
        match token_kind {
            TokenKind::OpenParenthesis => self.parenthesis = self.parenthesis.saturating_add(1),
            TokenKind::CloseParenthesis => self.parenthesis = self.parenthesis.saturating_sub(1),
            TokenKind::OpenCurly => self.curly = self.curly.saturating_add(1),
            TokenKind::CloseCurly => self.curly = self.curly.saturating_sub(1),
            _ => {}
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

    pub(crate) fn step(&mut self, token_kind: &TokenKind) {
        match token_kind {
            TokenKind::TemplateHead => {
                self.opened = self.opened.saturating_add(1);
            }
            TokenKind::TemplateClose => {
                self.closed = self.closed.saturating_add(1);
            }
            _ => {}
        }
    }
}

/// Boxed diagnostic result for declaration-initializer token scanning.
///
/// WHAT: carries the scanner's single structured EOF diagnostic in the same shape as its owner.
/// WHY: declaration-shell parsing already uses a boxed diagnostic family, so the scan result flows
///      into that boundary directly without an unbox/rebox adapter.
pub(crate) type TokenScanResult<T> = Result<T, Box<CompilerDiagnostic>>;

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
    // Anonymous const records that start the initializer own `|...|` newlines and commas.
    // Catch/receiver pipes such as `|err|` must not enter that region.
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
            if inline_value_if_missing_else_depth > 0 {
                collected.push(token_stream.current_token());
            }
            break;
        }

        if at_top_level
            && matches!(token_kind, TokenKind::Newline)
            && !continues_multiline_expression
        {
            if inline_value_if_missing_else_depth > 0 {
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
                    return Err(Box::new(CompilerDiagnostic::from(
                        CompilerError::compiler_error(
                            "declaration-initializer scanner reported a nested state with no open construct",
                        ),
                    )));
                }
            };
            return Err(Box::new(CompilerDiagnostic::unexpected_end_of_file(
                expected_delimiter,
                token_stream.current_location(),
            )));
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
                match classify_record_pipe(
                    &token_stream.tokens,
                    token_stream.index,
                    record_pipe_depth,
                    depth.is_top_level(),
                ) {
                    RecordPipeAction::Open => {
                        open_constructs.push(OpenConstruct::AnonymousRecord);
                        record_pipe_depth = record_pipe_depth.saturating_add(1);
                        last_closed_record_pipe = false;
                    }
                    RecordPipeAction::Close => {
                        close_open_construct(&mut open_constructs, OpenConstruct::AnonymousRecord);
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

pub(crate) fn has_top_level_comma_before_statement_end(token_stream: &FileTokens) -> bool {
    let mut depth = NestingDepth::default();
    let mut record_pipe_depth = 0usize;
    let mut index = token_stream.index;
    let tokens = &token_stream.tokens;

    while index < token_stream.length {
        let token_kind = &tokens[index].kind;

        if matches!(token_kind, TokenKind::TypeParameterBracket) {
            match classify_record_pipe(tokens, index, record_pipe_depth, depth.is_top_level()) {
                RecordPipeAction::Open => {
                    record_pipe_depth = record_pipe_depth.saturating_add(1);
                }
                RecordPipeAction::Close => {
                    record_pipe_depth = record_pipe_depth.saturating_sub(1);
                }
                RecordPipeAction::Ignore => {}
            }
        }

        if record_pipe_depth == 0 && depth.is_top_level() && matches!(token_kind, TokenKind::Comma)
        {
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

        depth.step(token_kind);
        index += 1;
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
    on_eof_error: impl Fn(SourceLocation) -> E,
) -> Result<(), E> {
    let mut balance = TemplateBalance::with_opening_template();

    while balance.has_unclosed_templates() {
        let token_kind = token_stream.current_token_kind().clone();
        if matches!(token_kind, TokenKind::Eof) {
            return Err(on_eof_error(token_stream.current_location()));
        }

        balance.step(&token_kind);
        on_token(token_stream.current_token(), &token_kind);
        token_stream.advance();
    }

    Ok(())
}
