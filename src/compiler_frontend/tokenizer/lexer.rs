//! Template-aware lexer for raw Moth source text.
//!
//! WHAT: converts source text into token streams while switching modes for templates, strings, and directives.
//! WHY: lexing owns the first precise source-location mapping and all delimiter-balancing rules;
//! callers can run it against worker-local string tables before deterministic module aggregation.

use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, DiagnosticCompoundAssignmentOperator,
    DiagnosticOperator, MissingWhitespace, SymbolicSpacingConstruct, SymbolicSpacingError,
};
use crate::compiler_frontend::keywords::{
    attached_bang_keyword_token_kind, is_identifier_continue, is_valid_identifier,
    keyword_token_kind,
};
use crate::compiler_frontend::numeric_text::parse::parse_numeric_literal;
use crate::compiler_frontend::numeric_text::token::NumericLiteralSign;
use crate::compiler_frontend::paths::const_paths::parse_file_path;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::newline_handling::normalize_consumed_carriage_return_newline;
use crate::compiler_frontend::tokenizer::numeric::tokenize_numeric_literal;
use crate::compiler_frontend::tokenizer::text_modes::{
    tokenize_code_template_body, tokenize_discard_template_body, tokenize_raw_string,
    tokenize_string, tokenize_template_body,
};
use crate::compiler_frontend::tokenizer::tokens::{
    CharPosition, FileTokens, SourceLocation, TemplateBodyMode, Token, TokenKind, TokenStream,
    TokenizeMode, TokenizerEntryMode,
};
use crate::compiler_frontend::utilities::basic::CharacterParsing;
use crate::projects::settings;
use crate::token_log;
use std::iter::Peekable;
use std::str::Chars;

pub const END_SCOPE_CHAR: char = ';';

/// Boxed diagnostic result shared by every lexer result boundary in this file.
///
/// WHAT: one file-local alias for the boxed `CompilerDiagnostic` error variant returned by
/// `tokenize`, `get_token_kind`, `require_symbolic_spacing`, `tokenize_style_directive`
/// and `tokenize_identifier_or_keyword`.
/// WHY: lexer dispatch propagates one diagnostic through several nested mode helpers and the
/// production callers already own boxed diagnostic boundaries. Numeric and text-mode helpers
/// remain separate owners, so their plain results are adapted only where they enter this family.
type LexerResult<T> = Result<T, Box<CompilerDiagnostic>>;

#[macro_export]
macro_rules! return_token {
    ($kind:expr, $stream:expr $(,)?) => {
        return Ok(Token::new($kind, $stream.new_location()))
    };
}

/// Immediate lexical surroundings for the next token.
///
/// WHAT: carries the previous emitted token and the previous non-newline token into token
/// recognition.
/// WHY: signed literals and spacing diagnostics need a small amount of left context, but the
/// tokenizer must not ask AST parsing whether a token is in expression position.
#[derive(Clone, Copy)]
struct LexerTokenContext<'a> {
    previous_token_kind: Option<&'a TokenKind>,
    last_meaningful_token_kind: Option<&'a TokenKind>,
    meaningful_token_before_last_kind: Option<&'a TokenKind>,
}

impl<'a> LexerTokenContext<'a> {
    fn previous_can_end_expression(self) -> bool {
        self.last_meaningful_token_kind
            .is_some_and(TokenKind::can_end_expression)
    }

    fn has_leading_whitespace(self, whitespace_before_current: bool) -> bool {
        matches!(self.previous_token_kind, Some(TokenKind::Newline)) || whitespace_before_current
    }
}

fn next_char_is_whitespace_or_end(stream: &mut TokenStream<'_>) -> bool {
    stream
        .peek()
        .is_none_or(|character| character.is_whitespace())
}

fn next_char_is_missing_rhs_boundary(stream: &mut TokenStream<'_>) -> bool {
    character_is_missing_rhs_boundary(stream.peek().copied())
}

/// Returns whether the current position begins a line-initial match-arm header.
///
/// WHAT: identifies the one lexical boundary where a leading `-` must not inherit
///       expression-ending context from the preceding line.
/// WHY: the AST match parser owns arm recognition after tokenization, but signed
///      numeric tokenization must decide before that parser can see the `=>`.
fn line_initial_match_arm_header(
    stream: &mut TokenStream<'_>,
    context: LexerTokenContext<'_>,
) -> bool {
    if !matches!(context.previous_token_kind, Some(TokenKind::Newline)) {
        return false;
    }

    let mut remaining_line = stream.chars.clone();
    let Some(numeric_text) = consume_match_arm_numeric_candidate(&mut remaining_line) else {
        return false;
    };

    if parse_numeric_literal(&numeric_text).is_err() {
        return false;
    }

    let had_whitespace_after_literal = consume_line_horizontal_whitespace(&mut remaining_line);
    if consume_fat_arrow(&mut remaining_line) {
        return true;
    }

    if !had_whitespace_after_literal || !consume_match_guard_keyword(&mut remaining_line) {
        return false;
    }

    consume_line_horizontal_whitespace(&mut remaining_line);
    consume_match_guard_newlines(&mut remaining_line);
    line_contains_match_arm_arrow(&mut remaining_line)
}

/// Consume the narrow numeric candidate that can follow a signed match-pattern `-`.
///
/// WHAT: collects only characters that can belong to a numeric literal, then lets the shared
///       numeric grammar validate the complete candidate.
/// WHY: the arm-boundary lookahead must not duplicate numeric separator or exponent rules.
fn consume_match_arm_numeric_candidate(chars: &mut Peekable<Chars<'_>>) -> Option<String> {
    let mut numeric_text = String::new();

    while let Some(&character) = chars.peek() {
        if !matches!(character, '0'..='9' | '_' | '.' | 'e' | 'E' | '+' | '-') {
            break;
        }

        numeric_text.push(chars.next()?);
    }

    numeric_text
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
        .then_some(numeric_text)
}

fn consume_line_horizontal_whitespace(chars: &mut Peekable<Chars<'_>>) -> bool {
    let mut consumed = false;

    while let Some(&character) = chars.peek() {
        if matches!(character, '\n' | '\r') || !character.is_whitespace() {
            break;
        }

        chars.next();
        consumed = true;
    }

    consumed
}

fn consume_fat_arrow(chars: &mut Peekable<Chars<'_>>) -> bool {
    if chars.peek() != Some(&'=') {
        return false;
    }

    chars.next();
    if chars.peek() != Some(&'>') {
        return false;
    }

    chars.next();
    true
}

fn consume_match_guard_keyword(chars: &mut Peekable<Chars<'_>>) -> bool {
    let mut candidate = chars.clone();
    for expected in "if".chars() {
        if candidate.next() != Some(expected) {
            return false;
        }
    }

    if candidate
        .peek()
        .is_some_and(|character| is_identifier_continue(*character))
    {
        return false;
    }

    *chars = candidate;
    true
}

fn consume_match_guard_newlines(chars: &mut Peekable<Chars<'_>>) {
    loop {
        consume_line_horizontal_whitespace(chars);

        if consume_match_guard_comment(chars) {
            if !consume_line_break(chars) {
                break;
            }
            continue;
        }

        if !consume_line_break(chars) {
            break;
        }
    }
}

fn consume_match_guard_comment(chars: &mut Peekable<Chars<'_>>) -> bool {
    let mut comment_start = chars.clone();
    if comment_start.next() != Some('-') || comment_start.next() != Some('-') {
        return false;
    }

    chars.next();
    chars.next();
    while chars
        .peek()
        .is_some_and(|character| !matches!(character, '\n' | '\r'))
    {
        chars.next();
    }
    true
}

fn consume_line_break(chars: &mut Peekable<Chars<'_>>) -> bool {
    match chars.peek().copied() {
        Some('\n') => {
            chars.next();
            true
        }
        Some('\r') => {
            chars.next();
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            true
        }
        _ => false,
    }
}

#[derive(Default)]
struct MatchGuardNesting {
    parentheses: usize,
    curly: usize,
    square: usize,
}

impl MatchGuardNesting {
    fn is_top_level(&self) -> bool {
        self.parentheses == 0 && self.curly == 0 && self.square == 0
    }

    fn step(&mut self, character: char) {
        match character {
            '(' => self.parentheses += 1,
            ')' => self.parentheses = self.parentheses.saturating_sub(1),
            '{' => self.curly += 1,
            '}' => self.curly = self.curly.saturating_sub(1),
            '[' => self.square += 1,
            ']' => self.square = self.square.saturating_sub(1),
            _ => {}
        }
    }
}

fn line_contains_match_arm_arrow(chars: &mut Peekable<Chars<'_>>) -> bool {
    let mut guard_has_content = false;
    let mut nesting = MatchGuardNesting::default();

    while let Some(&character) = chars.peek() {
        if matches!(character, '\n' | '\r') {
            return false;
        }

        if character == ';' {
            return false;
        }

        if character == '-' {
            let mut comment_start = chars.clone();
            comment_start.next();
            if comment_start.peek() == Some(&'-') {
                return false;
            }
        }

        if matches!(character, '"' | '`') {
            guard_has_content = true;
            if !consume_match_guard_string(chars, character) {
                return false;
            }
            continue;
        }

        if character == '=' {
            let mut possible_arrow = chars.clone();
            if nesting.is_top_level() && consume_fat_arrow(&mut possible_arrow) {
                return guard_has_content;
            }

            guard_has_content = true;
            chars.next();
            continue;
        }

        if !character.is_whitespace() {
            guard_has_content = true;
        }
        nesting.step(character);
        chars.next();
    }

    false
}

fn consume_match_guard_string(chars: &mut Peekable<Chars<'_>>, delimiter: char) -> bool {
    chars.next();
    let mut escaped = false;

    for character in chars.by_ref() {
        if matches!(character, '\n' | '\r') {
            return false;
        }

        if delimiter == '"' && escaped {
            escaped = false;
            continue;
        }

        if delimiter == '"' && character == '\\' {
            escaped = true;
            continue;
        }

        if character == delimiter {
            return true;
        }
    }

    false
}

fn character_is_missing_rhs_boundary(character: Option<char>) -> bool {
    character.is_none_or(|character| matches!(character, '\n' | '\r' | ',' | ')' | ']' | '}' | ';'))
}

fn symbolic_spacing_error(
    stream: &mut TokenStream<'_>,
    construct: SymbolicSpacingConstruct,
    missing: MissingWhitespace,
) -> CompilerDiagnostic {
    CompilerDiagnostic::common_syntax_mistake(
        CommonSyntaxMistakeReason::InvalidSymbolicSpacing {
            error: SymbolicSpacingError { construct, missing },
        },
        stream.new_location(),
    )
}

/// Compute the missing whitespace side from independent leading and trailing checks.
fn missing_whitespace_side(missing_left: bool, missing_right: bool) -> Option<MissingWhitespace> {
    match (missing_left, missing_right) {
        (true, true) => Some(MissingWhitespace::Both),
        (true, false) => Some(MissingWhitespace::Before),
        (false, true) => Some(MissingWhitespace::After),
        (false, false) => None,
    }
}

/// Recognise a bare import path that is missing its required `@` prefix.
///
/// WHAT: when the last meaningful token is `import` and the current position starts an
/// identifier-led path (such as `core` or `core/math`) or a `./` relative path, scan the
/// complete authored spelling and return a missing-`@` diagnostic spanning it.
/// WHY: without this, a bare `import core/math` reaches the `/` operator before the
/// compiler realises the prefix is missing, producing a confusing spacing error at the
/// slash instead of pointing at the whole authored path.
///
/// This is a narrow lexical scan for the correction fact only. It does not validate or
/// resolve import paths; that ownership stays with `parse_file_path` and the import
/// clause parser. The scan stops at whitespace, structural delimiters and template
/// boundaries, so it never consumes a grouped `{` clause or an `as` alias (aliases are
/// preceded by whitespace). Parent-relative `../` paths are intentionally not
/// recognised here, because `@../` is not supported syntax.
fn recognise_missing_at_import_prefix(
    stream: &mut TokenStream<'_>,
    context: LexerTokenContext<'_>,
    current_char: char,
    string_table: &mut StringTable,
) -> Option<CompilerDiagnostic> {
    if !matches!(context.last_meaningful_token_kind, Some(TokenKind::Import)) {
        return None;
    }

    let starts_identifier_led = current_char.is_alphabetic()
        || (current_char == '_'
            && stream
                .peek()
                .is_some_and(|character| is_identifier_continue(*character)));
    let starts_relative_current = current_char == '.' && stream.peek() == Some(&'/');

    if !starts_identifier_led && !starts_relative_current {
        return None;
    }

    // `as` directly after `import` is the alias keyword with a missing path, not a path itself.
    // Peek without advancing so the import-clause parser keeps ownership of that mistake.
    let mut alias_lookahead = stream.chars.clone();
    if current_char == 'a'
        && alias_lookahead.next() == Some('s')
        && alias_lookahead
            .peek()
            .is_none_or(|character| is_missing_at_prefix_scan_boundary(*character))
    {
        return None;
    }

    // The first path character was already consumed at the top of the token loop, so the
    // stream cursor rests one column past it. Step back one column so the span covers the
    // complete authored path, matching how `parse_file_path` spans a valid `@` path.
    let path_start = CharPosition {
        char_column: stream.start_position.char_column.saturating_sub(1),
        ..stream.start_position
    };

    let mut authored_path = String::new();
    authored_path.push(current_char);

    if starts_relative_current {
        let slash = stream.advance_after_peek(
            "Tokenizer peeked the relative-current slash but could not advance the stream.",
        );
        authored_path.push(slash);
    }

    while let Some(&next_char) = stream.peek() {
        if is_missing_at_prefix_scan_boundary(next_char) {
            break;
        }

        let consumed = stream.advance_after_peek(
            "Tokenizer peeked a missing-@ import path character but could not advance the stream.",
        );
        authored_path.push(consumed);
    }

    let authored_path_id = string_table.intern(&authored_path);

    Some(CompilerDiagnostic::common_syntax_mistake(
        CommonSyntaxMistakeReason::ImportPathMissingAtPrefix {
            authored_path: authored_path_id,
        },
        SourceLocation::new(stream.file_path.to_owned(), path_start, stream.position),
    ))
}

/// WHAT: characters that end the narrow missing-`@` import-path scan.
/// WHY: the scan captures the authored spelling only. It stops at whitespace and
/// structural, grouped, config and template delimiters so it never consumes a grouped
/// clause, alias or unrelated operator. Path separators (`/` and `\`) plus component
/// characters such as `.` and `-` are consumed so the complete spelling is preserved.
fn is_missing_at_prefix_scan_boundary(character: char) -> bool {
    if character.is_whitespace() {
        return true;
    }

    matches!(
        character,
        '{' | '}' | ',' | ':' | '[' | ']' | '(' | ')' | '<' | '>' | '"' | '|' | '?' | '*' | ';'
    )
}

fn unary_negation_spacing_error(stream: &mut TokenStream<'_>) -> CompilerDiagnostic {
    CompilerDiagnostic::common_syntax_mistake(
        CommonSyntaxMistakeReason::InvalidUnaryNegationSpacing,
        stream.new_location(),
    )
}

/// Enforce outer spacing when the current complete symbolic token follows an expression.
///
/// WHAT: requires whitespace before and after binary operators and compound assignments.
/// WHY: tokenizer-front-loaded spacing catches ambiguous forms such as `a+b` and `a*-1` before
/// later parsing can reinterpret the same characters in a less readable way.
fn require_symbolic_spacing(
    stream: &mut TokenStream<'_>,
    context: LexerTokenContext<'_>,
    whitespace_before_current: bool,
    construct: SymbolicSpacingConstruct,
) -> LexerResult<()> {
    if !context.previous_can_end_expression() {
        return Ok(());
    }

    let missing_left = !context.has_leading_whitespace(whitespace_before_current);
    let missing_right = !next_char_is_whitespace_or_end(stream);

    if let Some(missing) = missing_whitespace_side(missing_left, missing_right) {
        return Err(Box::new(symbolic_spacing_error(stream, construct, missing)));
    }

    Ok(())
}

fn less_than_is_generic_angle_start(
    stream: &mut TokenStream<'_>,
    context: LexerTokenContext<'_>,
    whitespace_before_current: bool,
) -> bool {
    matches!(context.previous_token_kind, Some(TokenKind::Symbol(_)))
        && !whitespace_before_current
        && stream
            .peek()
            .is_some_and(|character| character.is_uppercase())
}

fn greater_than_is_generic_angle_end(
    stream: &mut TokenStream<'_>,
    context: LexerTokenContext<'_>,
    whitespace_before_current: bool,
) -> bool {
    matches!(context.previous_token_kind, Some(TokenKind::Symbol(_)))
        && !whitespace_before_current
        && matches!(stream.peek(), Some('(' | ','))
}

fn less_than_is_template_tag_start(
    stream: &mut TokenStream<'_>,
    context: LexerTokenContext<'_>,
    whitespace_before_current: bool,
) -> bool {
    stream.mode != TokenizeMode::Normal
        && !whitespace_before_current
        && (matches!(stream.peek(), Some('/'))
            || (matches!(context.previous_token_kind, Some(TokenKind::TemplateClose))
                && stream
                    .peek()
                    .is_some_and(|character| character.is_alphabetic())))
}

fn greater_than_is_template_tag_end(
    stream: &TokenStream<'_>,
    context: LexerTokenContext<'_>,
    whitespace_before_current: bool,
) -> bool {
    stream.mode != TokenizeMode::Normal
        && !whitespace_before_current
        && matches!(context.previous_token_kind, Some(TokenKind::Symbol(_)))
        && matches!(
            context.meaningful_token_before_last_kind,
            Some(TokenKind::LessThan | TokenKind::Divide)
        )
}

/// Tokenize one source file and optionally attach stable file identity metadata.
///
/// WHAT: wraps lexing output in `FileTokens` carrying both logical path and optional `FileId`.
/// WHY: later frontend stages should prefer explicit file identity over path string comparisons.
pub fn tokenize(
    source_code: &str,
    src_path: &InternedPath,
    entry_mode: TokenizerEntryMode,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
    file_id: Option<FileId>,
) -> LexerResult<FileTokens> {
    // WHY: Estimating token capacity reduces reallocations for large files.
    // Preliminary tests suggest a ratio of roughly 6 characters per token.
    let initial_capacity = source_code.len() / settings::SRC_TO_TOKEN_RATIO;

    let mut tokens: Vec<Token> = Vec::with_capacity(initial_capacity);
    let mut stream = TokenStream::new(source_code, src_path, entry_mode);

    let mut token: Token = Token::new(TokenKind::ModuleStart, SourceLocation::default());
    let mut last_meaningful_token_kind: Option<TokenKind> = None;
    let mut meaningful_token_before_last_kind: Option<TokenKind> = None;
    let mut token_stats = TokenStats::default();

    loop {
        token_log!(#token);
        token_stats.accumulate(&token.kind);

        if token.kind == TokenKind::Eof {
            break;
        }

        tokens.push(token);

        let previous_token_kind = tokens.last().map(|token| &token.kind);
        if !matches!(previous_token_kind, Some(TokenKind::Newline)) {
            meaningful_token_before_last_kind = last_meaningful_token_kind.clone();
            last_meaningful_token_kind = previous_token_kind.cloned();
        }

        let context = LexerTokenContext {
            previous_token_kind,
            last_meaningful_token_kind: last_meaningful_token_kind.as_ref(),
            meaningful_token_before_last_kind: meaningful_token_before_last_kind.as_ref(),
        };
        token = get_token_kind(&mut stream, style_directives, string_table, context)?;
    }

    tokens.push(token);

    let mut file_tokens = FileTokens::new_with_file_id(src_path.to_owned(), file_id, tokens);
    file_tokens.token_stats = token_stats;
    Ok(file_tokens)
}

fn get_token_kind(
    stream: &mut TokenStream<'_>,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
    context: LexerTokenContext<'_>,
) -> LexerResult<Token> {
    // WHY: Comments do not produce tokens. A labeled loop allows the comment handler
    // to restart tokenization with `continue` instead of a recursive call, preventing
    // stack overflow in files with deep comment blocks.
    'next_token: loop {
        let mut whitespace_before_current = false;

        let mut current_char = match stream.next() {
            Some(ch) => ch,
            None => return_token!(TokenKind::Eof, stream),
        };

        let mut token_value = String::new();

        // -----------------
        //  Template bodies
        // -----------------

        // Template bodies are tokenized as "mostly raw text" so the body parser can
        // treat everything between delimiters as string content unless a nested
        // template begins or the current template closes.
        if stream.mode == TokenizeMode::TemplateBody {
            match stream.current_template_body_mode() {
                TemplateBodyMode::Balanced => {
                    return tokenize_code_template_body(current_char, stream, string_table);
                }
                TemplateBodyMode::DiscardBalanced => {
                    return tokenize_discard_template_body(current_char, stream);
                }
                TemplateBodyMode::Normal => {
                    if current_char != ']' && current_char != '[' {
                        return tokenize_template_body(current_char, stream, string_table);
                    }
                }
            }
        }

        // ------------------------
        //  Raw strings (backticks)
        // ------------------------

        // Preserve raw tokens without enabling them as source expressions. Template-body
        // backticks are consumed as ordinary body text before this code-mode branch.
        if current_char == '`' {
            return tokenize_raw_string(stream, string_table);
        }

        // ------------
        //  Whitespace
        // ------------

        while current_char.is_whitespace() {
            whitespace_before_current = true;

            if current_char == '\n' {
                // Skip trailing whitespace after a newline to reduce redundant tokens.
                // The parser treats consecutive newlines as a single boundary.
                consume_all_whitespace(stream);
                return_token!(TokenKind::Newline, stream);
            } else if current_char == '\r' {
                let _ = normalize_consumed_carriage_return_newline(stream);
                consume_all_whitespace(stream);
                return_token!(TokenKind::Newline, stream);
            } else {
                current_char = match stream.next() {
                    Some(ch) => ch,
                    None => return_token!(TokenKind::Eof, stream),
                };
            }
        }

        // Ignore leading whitespace for the next token's source location.
        stream.update_start_position();

        // A bare import path missing its `@` prefix (such as `import core/math` or
        // `import ./utils`) is recognised here, before normal identifier, dot or
        // operator tokenization turns the path into a confusing operator error.
        if let Some(diagnostic) =
            recognise_missing_at_import_prefix(stream, context, current_char, string_table)
        {
            return Err(Box::new(diagnostic));
        }

        // ---------------------
        //  Template delimiters
        // ---------------------

        if current_char == '[' {
            // Nested templates begin with '[' and switch to TemplateHead mode.
            stream.push_template_mode(TokenizeMode::TemplateHead);
            return_token!(TokenKind::TemplateHead, stream);
        }

        if current_char == ']' {
            if let Some(source_kind) = stream.initial_template_close_rejection() {
                return Err(Box::new(
                    CompilerDiagnostic::unescaped_implicit_template_close(
                        source_kind,
                        stream.new_location(),
                    ),
                ));
            }

            // Closing a template restores the parent template's mode.
            stream.pop_template_mode();
            return_token!(TokenKind::TemplateClose, stream);
        }

        // Colon handling: StartTemplateBody (:) vs DoubleColon (::) vs Colon (:)
        if current_char == ':' {
            if let Some(&next_char) = stream.peek()
                && next_char == ':'
            {
                stream.next();
                return_token!(TokenKind::DoubleColon, stream);
            }

            if stream.mode == TokenizeMode::TemplateHead {
                stream.set_current_template_mode(TokenizeMode::TemplateBody);
                return_token!(TokenKind::StartTemplateBody, stream);
            }

            return_token!(TokenKind::Colon, stream);
        }

        // -------------------
        //  Style directives
        // -------------------

        if current_char == '$' {
            if stream.mode == TokenizeMode::TemplateHead {
                if stream.peek() == Some(&'(') {
                    return_token!(TokenKind::Reactive, stream);
                }

                return tokenize_style_directive(stream, style_directives, string_table);
            }

            return_token!(TokenKind::Reactive, stream);
        }

        if current_char == END_SCOPE_CHAR {
            return_token!(TokenKind::End, stream);
        }

        // ----------------
        //  String literals
        // ----------------

        if current_char == '"' {
            return tokenize_string(stream, string_table);
        }

        if current_char == '\'' {
            if let Some(c) = stream.next()
                && let Some(&char_after_next) = stream.peek()
                && char_after_next == '\''
            {
                stream.next(); // Consume closing quote
                return_token!(TokenKind::CharLiteral(c), stream);
            };

            return Err(Box::new(CompilerDiagnostic::invalid_char_literal(
                stream.new_location(),
            )));
        }

        // -----------------
        //  Basic operators
        // -----------------

        if current_char == '(' {
            return_token!(TokenKind::OpenParenthesis, stream);
        }

        if current_char == ')' {
            return_token!(TokenKind::CloseParenthesis, stream);
        }

        if current_char == '=' {
            if let Some(&next_char) = stream.peek()
                && next_char == '>'
            {
                stream.next();
                return_token!(TokenKind::FatArrow, stream);
            }

            // `==` is a common equality mistake. Keep it on the existing parser diagnostic path
            // instead of reporting the first `=` as a spacing error.
            if stream.peek() != Some(&'=') {
                let previous_is_mutable_marker =
                    matches!(context.previous_token_kind, Some(TokenKind::Mutable));
                let previous_can_start_assignment = context
                    .previous_token_kind
                    .is_some_and(TokenKind::can_end_expression);

                if !previous_is_mutable_marker
                    && previous_can_start_assignment
                    && !matches!(context.previous_token_kind, Some(TokenKind::Bang))
                    && !next_char_is_missing_rhs_boundary(stream)
                {
                    let missing_left = !context.has_leading_whitespace(whitespace_before_current);
                    let missing_right = !next_char_is_whitespace_or_end(stream);

                    if let Some(missing) = missing_whitespace_side(missing_left, missing_right) {
                        return Err(Box::new(symbolic_spacing_error(
                            stream,
                            SymbolicSpacingConstruct::Assignment,
                            missing,
                        )));
                    }
                }
            }

            return_token!(TokenKind::Assign, stream);
        }

        if current_char == ',' {
            return_token!(TokenKind::Comma, stream);
        }

        if current_char == '.' {
            if let Some(&peeked_char) = stream.peek()
                && peeked_char == '.'
            {
                stream.next();
                return_token!(TokenKind::Variadic, stream);
            }

            return_token!(TokenKind::Dot, stream);
        }

        if current_char == '{' {
            return_token!(TokenKind::OpenCurly, stream);
        }

        if current_char == '}' {
            return_token!(TokenKind::CloseCurly, stream);
        }

        if current_char == '|' {
            return_token!(TokenKind::TypeParameterBracket, stream);
        }

        if current_char == '!' {
            return_token!(TokenKind::Bang, stream);
        }

        if current_char == '?' {
            return_token!(TokenKind::QuestionMark, stream);
        }

        // ------------------------------
        //  Subtraction & Line comments
        // ------------------------------

        if current_char == '-'
            && let Some(&next_char) = stream.peek()
        {
            // Line comments (--)
            if next_char == '-' {
                stream.next();

                while let Some(ch) = stream.peek() {
                    if ch == &'\n' || ch == &'\r' {
                        break;
                    }

                    stream.next();
                }

                // WHY: Comments do not produce tokens. Loop back to lex the next item.
                continue 'next_token;
            }

            if next_char == '=' {
                stream.next();
                require_symbolic_spacing(
                    stream,
                    context,
                    whitespace_before_current,
                    SymbolicSpacingConstruct::CompoundAssignment {
                        operator: DiagnosticCompoundAssignmentOperator::Subtract,
                    },
                )?;
                return_token!(TokenKind::SubtractAssign, stream);
            }

            if next_char == '>' {
                stream.next();
                return_token!(TokenKind::Arrow, stream);
            }

            if next_char.is_numeric() {
                if context.previous_can_end_expression()
                    && !line_initial_match_arm_header(stream, context)
                {
                    require_symbolic_spacing(
                        stream,
                        context,
                        whitespace_before_current,
                        SymbolicSpacingConstruct::BinaryOperator {
                            operator: DiagnosticOperator::Subtract,
                        },
                    )?;
                }

                let first_digit = stream.advance_after_peek(
                    "Tokenizer peeked a signed numeric literal digit but could not advance.",
                );
                return tokenize_numeric_literal(
                    first_digit,
                    stream,
                    string_table,
                    NumericLiteralSign::Negative,
                );
            }

            if context.previous_can_end_expression() {
                require_symbolic_spacing(
                    stream,
                    context,
                    whitespace_before_current,
                    SymbolicSpacingConstruct::BinaryOperator {
                        operator: DiagnosticOperator::Subtract,
                    },
                )?;
                return_token!(TokenKind::Subtract, stream);
            }

            if !next_char_is_whitespace_or_end(stream) {
                return_token!(TokenKind::Negative, stream);
            }

            return Err(Box::new(unary_negation_spacing_error(stream)));
        }

        // ------------------------
        //  Mathematical operators
        // ------------------------

        if current_char == '+' {
            if let Some(&next_char) = stream.peek()
                && next_char == '='
            {
                stream.next();
                require_symbolic_spacing(
                    stream,
                    context,
                    whitespace_before_current,
                    SymbolicSpacingConstruct::CompoundAssignment {
                        operator: DiagnosticCompoundAssignmentOperator::Add,
                    },
                )?;
                return_token!(TokenKind::AddAssign, stream);
            }

            if context.previous_can_end_expression() {
                require_symbolic_spacing(
                    stream,
                    context,
                    whitespace_before_current,
                    SymbolicSpacingConstruct::BinaryOperator {
                        operator: DiagnosticOperator::Add,
                    },
                )?;
                return_token!(TokenKind::Add, stream);
            }

            return Err(Box::new(CompilerDiagnostic::common_syntax_mistake(
                CommonSyntaxMistakeReason::UnsupportedUnaryPlus,
                stream.new_location(),
            )));
        }

        if current_char == '*' {
            if let Some(&next_char) = stream.peek()
                && next_char == '='
            {
                stream.next();
                require_symbolic_spacing(
                    stream,
                    context,
                    whitespace_before_current,
                    SymbolicSpacingConstruct::CompoundAssignment {
                        operator: DiagnosticCompoundAssignmentOperator::Multiply,
                    },
                )?;
                return_token!(TokenKind::MultiplyAssign, stream);
            }

            require_symbolic_spacing(
                stream,
                context,
                whitespace_before_current,
                SymbolicSpacingConstruct::BinaryOperator {
                    operator: DiagnosticOperator::Multiply,
                },
            )?;
            return_token!(TokenKind::Multiply, stream);
        }

        if current_char == '/' {
            if let Some(&next_char) = stream.peek() {
                // Integer division (//)
                if next_char == '/' {
                    stream.next();

                    if let Some(&next_next_char) = stream.peek()
                        && next_next_char == '='
                    {
                        stream.next();
                        require_symbolic_spacing(
                            stream,
                            context,
                            whitespace_before_current,
                            SymbolicSpacingConstruct::CompoundAssignment {
                                operator: DiagnosticCompoundAssignmentOperator::IntDivide,
                            },
                        )?;
                        return_token!(TokenKind::IntDivideAssign, stream);
                    }
                    require_symbolic_spacing(
                        stream,
                        context,
                        whitespace_before_current,
                        SymbolicSpacingConstruct::BinaryOperator {
                            operator: DiagnosticOperator::IntDivide,
                        },
                    )?;
                    return_token!(TokenKind::IntDivide, stream);
                }

                // Divide assign (/=)
                if next_char == '=' {
                    stream.next();
                    require_symbolic_spacing(
                        stream,
                        context,
                        whitespace_before_current,
                        SymbolicSpacingConstruct::CompoundAssignment {
                            operator: DiagnosticCompoundAssignmentOperator::Divide,
                        },
                    )?;
                    return_token!(TokenKind::DivideAssign, stream);
                }
            }

            require_symbolic_spacing(
                stream,
                context,
                whitespace_before_current,
                SymbolicSpacingConstruct::BinaryOperator {
                    operator: DiagnosticOperator::Divide,
                },
            )?;
            return_token!(TokenKind::Divide, stream);
        }

        if current_char == '%' {
            if let Some(&next_char) = stream.peek()
                && next_char == '='
            {
                stream.next();
                require_symbolic_spacing(
                    stream,
                    context,
                    whitespace_before_current,
                    SymbolicSpacingConstruct::CompoundAssignment {
                        operator: DiagnosticCompoundAssignmentOperator::Modulus,
                    },
                )?;
                return_token!(TokenKind::ModulusAssign, stream);
            }

            require_symbolic_spacing(
                stream,
                context,
                whitespace_before_current,
                SymbolicSpacingConstruct::BinaryOperator {
                    operator: DiagnosticOperator::Modulus,
                },
            )?;
            return_token!(TokenKind::Modulus, stream);
        }

        if current_char == '^' {
            if let Some(&next_char) = stream.peek()
                && next_char == '='
            {
                stream.next();
                require_symbolic_spacing(
                    stream,
                    context,
                    whitespace_before_current,
                    SymbolicSpacingConstruct::CompoundAssignment {
                        operator: DiagnosticCompoundAssignmentOperator::Exponent,
                    },
                )?;
                return_token!(TokenKind::ExponentAssign, stream);
            }

            require_symbolic_spacing(
                stream,
                context,
                whitespace_before_current,
                SymbolicSpacingConstruct::BinaryOperator {
                    operator: DiagnosticOperator::Exponent,
                },
            )?;
            return_token!(TokenKind::Exponent, stream);
        }

        // -------------------
        //  Logic & Channels
        // -------------------

        if current_char == '>' {
            if let Some(&next_char) = stream.peek() {
                if next_char == '=' {
                    stream.next();
                    require_symbolic_spacing(
                        stream,
                        context,
                        whitespace_before_current,
                        SymbolicSpacingConstruct::BinaryOperator {
                            operator: DiagnosticOperator::GreaterThanOrEqual,
                        },
                    )?;
                    return_token!(TokenKind::GreaterThanOrEqual, stream);
                }

                if next_char == '>' {
                    stream.next();
                    return_token!(TokenKind::ChannelSend, stream);
                }
            }

            if !greater_than_is_generic_angle_end(stream, context, whitespace_before_current)
                && !greater_than_is_template_tag_end(stream, context, whitespace_before_current)
            {
                require_symbolic_spacing(
                    stream,
                    context,
                    whitespace_before_current,
                    SymbolicSpacingConstruct::BinaryOperator {
                        operator: DiagnosticOperator::GreaterThan,
                    },
                )?;
            }
            return_token!(TokenKind::GreaterThan, stream);
        }

        if current_char == '<' {
            if let Some(&next_char) = stream.peek() {
                if next_char == '=' {
                    stream.next();
                    require_symbolic_spacing(
                        stream,
                        context,
                        whitespace_before_current,
                        SymbolicSpacingConstruct::BinaryOperator {
                            operator: DiagnosticOperator::LessThanOrEqual,
                        },
                    )?;
                    return_token!(TokenKind::LessThanOrEqual, stream);
                }

                if next_char == '<' {
                    stream.next();
                    return_token!(TokenKind::ChannelReceive, stream);
                }
            }

            if !less_than_is_generic_angle_start(stream, context, whitespace_before_current)
                && !less_than_is_template_tag_start(stream, context, whitespace_before_current)
            {
                require_symbolic_spacing(
                    stream,
                    context,
                    whitespace_before_current,
                    SymbolicSpacingConstruct::BinaryOperator {
                        operator: DiagnosticOperator::LessThan,
                    },
                )?;
            }
            return_token!(TokenKind::LessThan, stream);
        }

        if current_char == '~' {
            if context.previous_can_end_expression() && stream.peek() == Some(&'=') {
                let mut remaining_chars = stream.chars.clone();
                let marker_assign = remaining_chars.next();
                debug_assert_eq!(marker_assign, Some('='));

                let missing_left = !context.has_leading_whitespace(whitespace_before_current);
                let trailing_char = remaining_chars.next();
                let missing_right = !character_is_missing_rhs_boundary(trailing_char)
                    && trailing_char.is_some_and(|character| !character.is_whitespace());

                if let Some(missing) = missing_whitespace_side(missing_left, missing_right) {
                    stream.advance_after_peek(
                        "Tokenizer peeked the mutable declaration assignment marker but could not advance.",
                    );
                    return Err(Box::new(symbolic_spacing_error(
                        stream,
                        SymbolicSpacingConstruct::MutableDeclaration,
                        missing,
                    )));
                }
            }

            return_token!(TokenKind::Mutable, stream);
        }

        if current_char == '#' {
            return_token!(TokenKind::Hash, stream);
        }

        if current_char == '&' {
            return_token!(TokenKind::Ampersand, stream);
        }

        // -----------------------
        //  Identifiers & Values
        // -----------------------

        // Paths (@/path)
        if current_char == '@' {
            return parse_file_path(stream, string_table);
        }

        // Wildcard or Identifier starting with '_'
        if current_char == '_' {
            if let Some(next_char) = stream.peek()
                && is_identifier_continue(*next_char)
            {
                token_value.push(current_char);
                return tokenize_identifier_or_keyword(&mut token_value, stream, string_table);
            }

            return_token!(TokenKind::Wildcard, stream);
        }

        // Numeric literals
        if current_char.is_numeric() {
            return tokenize_numeric_literal(
                current_char,
                stream,
                string_table,
                NumericLiteralSign::Positive,
            );
        }

        // Keywords or variables starting with a letter
        if current_char.is_alphabetic() {
            token_value.push(current_char);
            return tokenize_identifier_or_keyword(&mut token_value, stream, string_table);
        }

        return Err(Box::new(CompilerDiagnostic::invalid_character(
            current_char,
            stream.new_location(),
        )));
    } // 'next_token loop
}

fn tokenize_style_directive(
    stream: &mut TokenStream<'_>,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
) -> LexerResult<Token> {
    if stream.mode != TokenizeMode::TemplateHead {
        return Err(Box::new(CompilerDiagnostic::invalid_character(
            '$',
            stream.new_location(),
        )));
    }

    let Some(&first_char) = stream.peek() else {
        return Err(Box::new(CompilerDiagnostic::unexpected_end_of_file(
            None,
            stream.new_location(),
        )));
    };

    if !first_char.is_alphabetic() && first_char != '_' {
        return Err(Box::new(CompilerDiagnostic::invalid_character(
            first_char,
            stream.new_location(),
        )));
    }

    let mut directive_text = String::new();
    let first_directive_char = stream.advance_after_peek(
        "Tokenizer validated a style directive name but failed to consume its first character.",
    );
    directive_text.push(first_directive_char);

    while let Some(&next_char) = stream.peek() {
        if !is_identifier_continue(next_char) {
            break;
        }

        let directive_char = stream.advance_after_peek(
            "Tokenizer peeked a style directive character but could not advance the stream.",
        );
        directive_text.push(directive_char);
    }

    let directive = string_table.intern(&directive_text);
    let Some(body_mode) = style_directives.body_mode_for(&directive_text) else {
        // Intern the supported-directives list for the error diagnostic payload.
        // This is diagnostic-only string-table mutation.
        let supported =
            string_table.intern(&style_directives.supported_directives_for_diagnostic());
        return Err(Box::new(CompilerDiagnostic::invalid_style_directive(
            directive,
            supported,
            stream.new_location(),
        )));
    };

    stream.mark_current_template_body_mode(body_mode);
    return_token!(TokenKind::StyleDirective(directive), stream);
}

// ----------------------
//  Variables & Keywords
// ----------------------

pub(crate) fn tokenize_identifier_or_keyword(
    token_value: &mut String,
    stream: &mut TokenStream<'_>,
    string_table: &mut StringTable,
) -> LexerResult<Token> {
    // WHY: Variable names and keywords can contain alphanumeric characters or underscores.
    // The loop keeps consuming identifier characters until a non-identifier boundary is
    // reached, then falls through to keyword and symbol matching.
    loop {
        if let Some(char) = stream.peek()
            && is_identifier_continue(*char)
        {
            let identifier_char = stream.advance_after_peek(
                "Tokenizer peeked an identifier character but could not advance the stream.",
            );
            token_value.push(identifier_char);
            continue;
        }

        if let Some(keyword_kind) = attached_bang_keyword_token_kind(token_value.as_str())
            && stream.peek() == Some(&'!')
        {
            stream.next();
            return_token!(keyword_kind, stream);
        }

        if let Some(keyword_kind) = keyword_token_kind(token_value.as_str()) {
            return_token!(keyword_kind, stream);
        }

        if is_valid_identifier(token_value) {
            let interned_symbol = string_table.intern(token_value);
            return_token!(TokenKind::Symbol(interned_symbol), stream);
        }

        return Err(Box::new(CompilerDiagnostic::invalid_identifier(
            stream.new_location(),
        )));
    }
}

/// Consume horizontal whitespace (spaces, tabs) but stop at newlines.
/// WHY: Newlines are significant tokens; this helper lets callers skip indentation
/// without consuming line boundaries.
pub fn consume_non_newline_whitespace(stream: &mut TokenStream) -> bool {
    let mut consumed = false;

    while stream
        .peek()
        .is_some_and(|character| character.is_non_newline_whitespace())
    {
        stream.next();
        consumed = true;
    }

    consumed
}

/// Consume all whitespace including newlines.
/// WHY: Used after a newline token to skip trailing whitespace on the same line,
/// and before the next meaningful token to normalize inter-token spacing.
pub fn consume_all_whitespace(stream: &mut TokenStream) -> bool {
    let mut consumed = false;

    while stream
        .peek()
        .is_some_and(|character| character.is_whitespace())
    {
        stream.next();
        consumed = true;
    }

    consumed
}

#[cfg(test)]
#[path = "tests/lexer_tests.rs"]
mod lexer_tests;
