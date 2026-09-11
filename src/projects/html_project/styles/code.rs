//! Built-in `$code` template style support.
//!
//! The HTML builder registers `$code` as a style directive and the frontend
//! formatter registry executes it; this module is the HTML-project owner of
//! the directive implementation.
//!
//! This module owns both halves of the feature:
//! - parsing the narrow `$code` / `$code("ext")` directive syntax
//! - converting compile-time body string runs into safe HTML with optional syntax highlighting
//!
//! The shared template formatter pipeline owns whitespace normalization before code reaches this
//! module. This module owns presentation and the `<code>` wrapper; HTML character escaping is
//! shared with `$escape_html` through `styles/escape_html.rs`. Exact Moth source-word classification
//! comes from the compiler-owned keyword module; this module never keeps a second current Moth word
//! list.
//!
//! The production scanner is one byte-indexed pass over borrowed source slices. It batches plain
//! runs, escapes directly into one owned output string per text piece and uses maximal munch for
//! compound Moth operators. Moth contextual roles (contracts, functions, directives, paths and the
//! `io` namespace) are bounded lexical presentation heuristics, never semantic analysis.

use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterInput, FormatterInputPiece, FormatterOutput, FormatterOutputPiece,
};
use crate::compiler_frontend::ast::templates::styles::whitespace::TemplateWhitespacePassProfile;
use crate::compiler_frontend::ast::templates::template::{
    Formatter, FormatterResult, TemplateFormatter,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::style_directives::StyleDirectiveArgumentValue;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::styles::escape_html::push_escaped_html_text;
use std::sync::Arc;

#[path = "language_profiles.rs"]
mod language_profiles;
#[path = "moth_scanner.rs"]
mod moth_scanner;

pub(crate) use language_profiles::CodeLanguage;
#[cfg(test)]
pub(crate) use language_profiles::LANGUAGE_ALIASES;
use language_profiles::classify_non_moth_word;
use moth_scanner::{ContractState, DependencyHighlightState};

/// One language-neutral presentation role shared by every code language profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodeHighlightRole {
    Comment,
    Keyword,
    Literal,
    String,
    Number,
    Operator,
    Type,
    Nominal,
    Delimiter,
    Function,
    Directive,
    Contract,
}

impl CodeHighlightRole {
    fn class_name(self) -> &'static str {
        match self {
            Self::Comment => "moth-code-comment",
            Self::Keyword => "moth-code-keyword",
            Self::Literal => "moth-code-literal",
            Self::String => "moth-code-string",
            Self::Number => "moth-code-number",
            Self::Operator => "moth-code-operator",
            Self::Type => "moth-code-type",
            Self::Nominal => "moth-code-nominal",
            Self::Delimiter => "moth-code-delimiter",
            Self::Function => "moth-code-function",
            Self::Directive => "moth-code-directive",
            Self::Contract => "moth-code-contract",
        }
    }
}

/// Exact byte position expected for a declaration-name role.
///
/// WHAT: records the source start of the one identifier a non-Moth keyword
///       (such as `function` or `trait`) may colour.
/// WHY: a pending role without a position can leak to a later word across
///      delimiters, comments, strings or newlines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExpectedWordRole {
    start: usize,
    role: CodeHighlightRole,
}

pub(crate) fn code_formatter_factory(
    argument: Option<&StyleDirectiveArgumentValue>,
) -> Result<Formatter, String> {
    let language = match argument {
        Some(StyleDirectiveArgumentValue::String(language_name)) => {
            match CodeLanguage::from_alias(language_name) {
                Some(language) => language,
                None => {
                    return Err(format!(
                        "Unsupported '$code(...)' language \"{language_name}\". Supported aliases are {}.",
                        CodeLanguage::supported_aliases()
                    ));
                }
            }
        }
        Some(_) => {
            return Err(
                "The '$code(...)' directive only accepts an optional string argument, for example '$code(\"rust\")'.".to_string(),
            );
        }
        None => CodeLanguage::Generic,
    };

    Ok(code_formatter(language))
}

#[derive(Debug)]
struct CodeTemplateFormatter {
    language: CodeLanguage,
}

impl TemplateFormatter for CodeTemplateFormatter {
    fn format(
        &self,
        input: FormatterInput,
        string_table: &mut StringTable,
    ) -> Result<FormatterResult, CompilerMessages> {
        // Process each text piece through syntax highlighting. Opaque anchors (child
        // templates, dynamic expressions) pass through without highlighting. The
        // <code> wrapper is emitted as explicit boundary pieces so sealed anchors
        // stay inside the block regardless of their position in the body.
        let mut output_pieces: Vec<FormatterOutputPiece> =
            Vec::with_capacity(input.pieces.len() + 2);
        output_pieces.push(FormatterOutputPiece::Text(
            "<code class='codeblock'>".to_owned(),
        ));

        for piece in input.pieces {
            match piece {
                FormatterInputPiece::Text(text_piece) => {
                    let text = string_table.resolve(text_piece.text);

                    // Allocate one output string per text piece so each highlighted
                    // run escapes directly into its own buffer.
                    let mut output = String::with_capacity(text.len() + 16);
                    if self.language == CodeLanguage::Text {
                        push_escaped_html_text(&mut output, text);
                    } else {
                        highlight_code_html_into(text, self.language, &mut output);
                    }

                    output_pieces.push(FormatterOutputPiece::Text(output));
                }
                FormatterInputPiece::Opaque(id) => {
                    output_pieces.push(FormatterOutputPiece::Opaque(id));
                }
            }
        }

        output_pieces.push(FormatterOutputPiece::Text("</code>".to_owned()));

        Ok(FormatterResult {
            output: FormatterOutput {
                pieces: output_pieces,
            },
            warnings: Vec::new(),
        })
    }
}

pub(crate) fn code_formatter(language: CodeLanguage) -> Formatter {
    Formatter {
        pre_format_whitespace_passes: vec![TemplateWhitespacePassProfile::default_template_body()],
        formatter: Arc::new(CodeTemplateFormatter { language }),
        post_format_whitespace_passes: Vec::new(),
    }
}

/// Scans one borrowed source slice directly into `output`.
fn highlight_code_html_into(source: &str, language: CodeLanguage, output: &mut String) {
    let mut scanner = CodeScanner::new(source, language);
    scanner.scan(output);
}

#[cfg(test)]
#[path = "code_test_support.rs"]
pub(crate) mod test_support;

#[cfg(test)]
pub(crate) use test_support::highlight_code_html;

/// One byte-indexed scanner pass over a borrowed source slice.
///
/// WHAT: owns the current position, the plain-run start, the language profile
///       and the bounded Moth and non-Moth contextual state.
/// WHY: a small state owner keeps the scanning control flow explicit and lets
///      every helper read source bytes without copying identifiers or words.
struct CodeScanner<'source> {
    source: &'source str,
    bytes: &'source [u8],
    index: usize,
    plain_start: usize,
    language: CodeLanguage,
    contract_state: ContractState,
    generic_declaration: bool,
    moth_delimiter_depth: usize,
    loop_header_depth: Option<usize>,
    in_pipe_group: bool,
    css_brace_depth: usize,
    expected_word_role: Option<ExpectedWordRole>,
    dependency_state: DependencyHighlightState,
}

impl<'source> CodeScanner<'source> {
    fn new(source: &'source str, language: CodeLanguage) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            index: 0,
            plain_start: 0,
            language,
            contract_state: ContractState::None,
            generic_declaration: false,
            moth_delimiter_depth: 0,
            loop_header_depth: None,
            in_pipe_group: false,
            css_brace_depth: 0,
            expected_word_role: None,
            dependency_state: DependencyHighlightState::None,
        }
    }

    fn scan(&mut self, output: &mut String) {
        while self.index < self.bytes.len() {
            let byte = self.bytes[self.index];
            if byte.is_ascii() {
                self.scan_ascii_byte(byte, output);
            } else {
                self.scan_non_ascii_scalar(output);
            }
        }

        self.flush_plain(output);
    }

    fn scan_ascii_byte(&mut self, byte: u8, output: &mut String) {
        match byte {
            b'"' | b'\'' => self.scan_quoted_run(output),
            b'[' if self.language == CodeLanguage::Toml && self.toml_table_header_starts_here() => {
                self.scan_toml_table_header(output);
            }
            b'(' | b')' | b'[' | b']' | b'{' | b'}' => self.scan_delimiter(output),
            b'0'..=b'9' => self.scan_number(output),
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.scan_word(output),
            _ => {
                // Comments are matched before operators so prefixes like `--` and
                // `//` become one comment run instead of two operator tokens.
                if self.matches_comment_prefix() {
                    self.scan_line_comment(output);
                    return;
                }

                if self.language == CodeLanguage::Html
                    && byte == b'<'
                    && self.html_markup_starts_here()
                {
                    self.scan_html_markup(output);
                    return;
                }

                if self.language == CodeLanguage::Markdown {
                    if byte == b'#' && self.at_line_start() {
                        self.scan_markdown_heading(output);
                        return;
                    }

                    if byte == b'`' && self.scan_markdown_backtick_run(output) {
                        return;
                    }
                }

                // Block comments cover CSS and multi-line C/SQL comments.
                if matches!(
                    self.language,
                    CodeLanguage::Css | CodeLanguage::C | CodeLanguage::Sql
                ) && self.bytes[self.index..].starts_with(b"/*")
                {
                    self.scan_block_comment(output);
                    return;
                }

                if self.language == CodeLanguage::C
                    && byte == b'#'
                    && self.at_line_start()
                    && self
                        .bytes
                        .get(self.index + 1)
                        .is_some_and(|next| next.is_ascii_alphabetic() || *next == b'_')
                {
                    self.scan_c_preprocessor(output);
                    return;
                }

                if self.language == CodeLanguage::Css
                    && byte == b'@'
                    && self.bytes.get(self.index + 1).is_some_and(|next| {
                        next.is_ascii_alphabetic() || matches!(next, b'-' | b'_')
                    })
                {
                    self.scan_css_at_rule(output);
                    return;
                }

                if self.language == CodeLanguage::Yaml
                    && self.at_line_start()
                    && (self.bytes[self.index..].starts_with(b"---")
                        || self.bytes[self.index..].starts_with(b"..."))
                {
                    self.emit_highlighted_range(
                        output,
                        self.index,
                        self.index + 3,
                        CodeHighlightRole::Keyword,
                    );
                    return;
                }

                if self.language == CodeLanguage::Yaml && byte == b'~' {
                    self.emit_highlighted_range(
                        output,
                        self.index,
                        self.index + 1,
                        CodeHighlightRole::Literal,
                    );
                    return;
                }

                if self.language == CodeLanguage::Moth {
                    // Each structural `|` opens or closes a paired pipe group.
                    if byte == b'|' {
                        self.in_pipe_group = !self.in_pipe_group;
                        self.scan_delimiter(output);
                        return;
                    }

                    // Commas continue conformance lists but end a generic bound list.
                    if byte == b',' {
                        self.transition_after_comma();
                        self.continue_dependency_after_comma();
                        self.expected_word_role = None;
                        self.index += 1;
                        return;
                    }

                    if byte == b'$' && self.moth_directive_starts_here() {
                        self.scan_moth_directive(output);
                        return;
                    }

                    if byte == b'@' && self.moth_path_starts_here() {
                        self.scan_moth_path(output);
                        return;
                    }

                    if matches!(byte, b':' | b';') {
                        if self.operator_length().is_some() {
                            self.scan_operator(output);
                        } else {
                            self.scan_delimiter(output);
                        }
                        return;
                    }
                }

                // HTML prose keeps `!` plain outside declarations; it only has
                // markup meaning inside `<!DOCTYPE ...>`.
                if self.operator_length().is_some()
                    && !(self.language == CodeLanguage::Html && byte == b'!')
                {
                    self.scan_operator(output);
                    return;
                }

                // Structural Moth boundaries end declaration context. Newlines
                // reset everything except a conformance continuation that a
                // comma explicitly armed.
                if self.language == CodeLanguage::Moth {
                    if byte == b'\n' {
                        self.reset_after_newline();
                    } else if matches!(
                        byte,
                        b'=' | b'<' | b'>' | b'(' | b')' | b'[' | b']' | b'{' | b'}'
                    ) {
                        self.reset_declaration_context();
                    }
                }

                // Plain punctuation stays in the batched plain run; horizontal
                // whitespace keeps an armed declaration-name expectation alive.
                if !matches!(byte, b' ' | b'\t') {
                    self.expected_word_role = None;
                }
                self.index += 1;
            }
        }
    }

    /// Advances one scalar that is not part of an ASCII dispatch class.
    ///
    /// WHAT: non-ASCII alphanumeric scalars start identifier runs so Unicode
    ///       identifiers classify as whole words; numeric scalars keep the legacy
    ///       number span in non-Moth profiles.
    /// WHY: UTF-8 is only decoded when a non-ASCII scalar must be advanced.
    fn scan_non_ascii_scalar(&mut self, output: &mut String) {
        let ch = self.source[self.index..]
            .chars()
            .next()
            .expect("scan position is always on a char boundary");

        if ch.is_numeric() && self.language != CodeLanguage::Moth {
            self.scan_number(output);
        } else if ch.is_alphanumeric() {
            self.scan_word(output);
        } else {
            self.advance_unicode();
        }
    }

    fn advance_unicode(&mut self) {
        let ch = self.source[self.index..]
            .chars()
            .next()
            .expect("scan position is always on a char boundary");
        self.expected_word_role = None;
        self.index += ch.len_utf8();
    }

    fn scan_word(&mut self, output: &mut String) {
        let word_start = self.index;
        let word_end = self.word_end(word_start);
        let word = &self.source[word_start..word_end];

        if let Some(role) = self.word_role(word, word_start, word_end) {
            self.emit_highlighted_range(output, word_start, word_end, role);
        } else {
            // Unhighlighted words stay part of the current plain run: the cursor
            // advances but `plain_start` is untouched, so the whole run flushes
            // in one write at the next span or end of input.
            self.index = word_end;
        }
    }

    /// Classifies one scanned word, returning its role if it gets a span.
    fn word_role(
        &mut self,
        word: &str,
        word_start: usize,
        word_end: usize,
    ) -> Option<CodeHighlightRole> {
        if self.language == CodeLanguage::Moth {
            self.moth_word_role(word, word_end)
        } else {
            self.non_moth_word_role(word, word_start, word_end)
        }
    }

    /// Classifies one non-Moth word against the exact-position expectation and
    /// the direct per-language word table.
    fn non_moth_word_role(
        &mut self,
        word: &str,
        word_start: usize,
        word_end: usize,
    ) -> Option<CodeHighlightRole> {
        if let Some(expected) = self.expected_word_role.take()
            && expected.start == word_start
        {
            return Some(expected.role);
        }

        // TOML bare and dotted keys are nominal when `=` or `.` follows them.
        if self.language == CodeLanguage::Toml
            && matches!(
                self.next_non_horizontal_whitespace_byte(word_end),
                Some(b'=') | Some(b'.')
            )
        {
            return Some(CodeHighlightRole::Nominal);
        }

        // YAML mapping keys are nominal when a colon follows at a key position.
        if self.language == CodeLanguage::Yaml
            && self.next_non_horizontal_whitespace_byte(word_end) == Some(b':')
            && self.yaml_key_starts_line(word_start)
        {
            return Some(CodeHighlightRole::Nominal);
        }

        // CSS property names are nominal inside declaration blocks.
        if self.language == CodeLanguage::Css
            && self.css_brace_depth > 0
            && self.next_non_horizontal_whitespace_byte(word_end) == Some(b':')
        {
            return Some(CodeHighlightRole::Nominal);
        }

        let class = classify_non_moth_word(self.language, word);
        if let Some(next_role) = class.next_identifier_role
            && let Some(next_start) = self.next_identifier_start(word_end)
        {
            self.expected_word_role = Some(ExpectedWordRole {
                start: next_start,
                role: next_role,
            });
        }

        if class.role.is_none()
            && matches!(self.language, CodeLanguage::C | CodeLanguage::Sql)
            && self.next_non_horizontal_whitespace_byte(word_end) == Some(b'(')
        {
            return Some(CodeHighlightRole::Function);
        }

        class.role.or_else(|| {
            (self.language.has_nominal_fallback()
                && word.chars().next().is_some_and(|ch| ch.is_uppercase()))
            .then_some(CodeHighlightRole::Nominal)
        })
    }

    /// Returns the start of the next identifier after horizontal whitespace, or
    /// `None` when the next source position cannot begin one.
    fn next_identifier_start(&self, from: usize) -> Option<usize> {
        let mut index = from;

        while index < self.bytes.len() {
            match self.bytes[index] {
                b' ' | b'\t' => index += 1,
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => return Some(index),
                _ => {
                    let ch = self.source[index..]
                        .chars()
                        .next()
                        .expect("scan position is always on a char boundary");
                    if ch.is_alphanumeric() {
                        return Some(index);
                    }
                    return None;
                }
            }
        }

        None
    }

    /// Flushes the plain run, emits one token inside a role span, then advances
    /// the cursor. This is the only path that owns the complete flush/span/cursor
    /// sequence for highlighted runs.
    fn emit_highlighted_range(
        &mut self,
        output: &mut String,
        token_start: usize,
        token_end: usize,
        role: CodeHighlightRole,
    ) {
        debug_assert_eq!(self.index, token_start);
        debug_assert!(token_start <= token_end);
        debug_assert!(token_end <= self.bytes.len());

        self.flush_plain(output);
        push_role_span_escaped(output, role, &self.source[token_start..token_end]);

        self.index = token_end;
        self.plain_start = token_end;
    }

    fn scan_line_comment(&mut self, output: &mut String) {
        let run_start = self.index;
        let mut end = self.index;

        // The comment run stops before the newline; the newline stays in the
        // plain run so whitespace is preserved exactly.
        while end < self.bytes.len() && self.bytes[end] != b'\n' {
            end += 1;
        }

        self.expected_word_role = None;
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Comment);
    }

    fn scan_quoted_run(&mut self, output: &mut String) {
        let quote = self.bytes[self.index];
        let run_start = self.index;
        let mut end = self.index + 1;

        while end < self.bytes.len() {
            let byte = self.bytes[end];
            if byte.is_ascii() {
                if byte == b'\\' && end + 1 < self.bytes.len() {
                    end += 1;
                    let escaped_len = if self.bytes[end].is_ascii() {
                        1
                    } else {
                        self.source[end..]
                            .chars()
                            .next()
                            .expect("escape position is on a char boundary")
                            .len_utf8()
                    };
                    end += escaped_len;
                    continue;
                }

                if byte == quote {
                    end += 1;
                    break;
                }

                end += 1;
            } else {
                let ch = self.source[end..]
                    .chars()
                    .next()
                    .expect("string position is on a char boundary");
                end += ch.len_utf8();
            }
        }

        self.expected_word_role = None;

        // JSON and YAML quoted mapping keys keep the key role when a colon
        // follows; every other quoted run stays a string.
        let role = if matches!(self.language, CodeLanguage::Json | CodeLanguage::Yaml)
            && self.next_non_horizontal_whitespace_byte(end) == Some(b':')
            && (self.language == CodeLanguage::Json || self.yaml_key_starts_line(run_start))
        {
            CodeHighlightRole::Nominal
        } else {
            CodeHighlightRole::String
        };
        self.emit_highlighted_range(output, run_start, end, role);
    }

    fn scan_delimiter(&mut self, output: &mut String) {
        let run_start = self.index;
        let end = self.index + 1;

        // CSS braces track declaration blocks so property names can be told
        // apart from selectors.
        if self.language == CodeLanguage::Css {
            match self.bytes[self.index] {
                b'{' => self.css_brace_depth += 1,
                b'}' => self.css_brace_depth = self.css_brace_depth.saturating_sub(1),
                _ => {}
            }
        }

        if self.language == CodeLanguage::Moth {
            self.update_moth_delimiter_depth();
            if matches!(self.bytes[self.index], b'|' | b':') {
                self.end_loop_header_at_top_level();
            }
            self.reset_declaration_context();
        }

        self.expected_word_role = None;
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Delimiter);
    }

    /// True when the cursor sits at the first byte of a line.
    fn at_line_start(&self) -> bool {
        self.index == 0 || self.bytes[self.index - 1] == b'\n'
    }

    /// True when `<` begins an HTML comment, declaration or tag.
    fn html_markup_starts_here(&self) -> bool {
        matches!(
            self.bytes.get(self.index + 1),
            Some(b'/') | Some(b'!') | Some(b'a'..=b'z') | Some(b'A'..=b'Z')
        )
    }

    /// Scans one HTML comment, declaration or tag and emits its role spans.
    ///
    /// WHAT: comments and declarations get one whole-run span; tags get
    ///       delimiter, type, nominal, operator and string spans for their
    ///       parts.
    /// WHY: basic markup highlighting reuses the shared palette without
    ///      building a nested HTML tokenizer.
    fn scan_html_markup(&mut self, output: &mut String) {
        if self.bytes[self.index..].starts_with(b"<!--") {
            let run_start = self.index;
            let end = self.bytes[self.index + 4..]
                .windows(3)
                .position(|window| window == b"-->")
                .map(|offset| self.index + 4 + offset + 3)
                .unwrap_or(self.bytes.len());
            self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Comment);
            return;
        }

        if self.bytes.get(self.index + 1) == Some(&b'!') {
            let run_start = self.index;
            let end = self
                .find_same_line_byte(self.index + 2, b'>')
                .map(|position| position + 1)
                .unwrap_or(self.bytes.len());
            self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Keyword);
            return;
        }

        let closing_tag = self.bytes.get(self.index + 1) == Some(&b'/');
        let open_len = if closing_tag { 2 } else { 1 };
        self.emit_highlighted_range(
            output,
            self.index,
            self.index + open_len,
            CodeHighlightRole::Delimiter,
        );

        let name_end = self.word_end(self.index);
        if name_end > self.index {
            self.emit_highlighted_range(output, self.index, name_end, CodeHighlightRole::Type);
        }

        loop {
            while self.index < self.bytes.len() && matches!(self.bytes[self.index], b' ' | b'\t') {
                self.index += 1;
            }

            if self.index >= self.bytes.len() {
                break;
            }

            match self.bytes[self.index] {
                b'>' => {
                    self.emit_highlighted_range(
                        output,
                        self.index,
                        self.index + 1,
                        CodeHighlightRole::Delimiter,
                    );
                    break;
                }
                b'/' if self.bytes.get(self.index + 1) == Some(&b'>') => {
                    self.emit_highlighted_range(
                        output,
                        self.index,
                        self.index + 2,
                        CodeHighlightRole::Delimiter,
                    );
                    break;
                }
                b'"' | b'\'' => self.scan_quoted_run(output),
                b'=' => {
                    self.emit_highlighted_range(
                        output,
                        self.index,
                        self.index + 1,
                        CodeHighlightRole::Operator,
                    );
                }
                byte if byte.is_ascii_alphanumeric() || byte == b'_' => {
                    let attribute_end = self.word_end(self.index);
                    self.emit_highlighted_range(
                        output,
                        self.index,
                        attribute_end,
                        CodeHighlightRole::Nominal,
                    );
                }
                _ => self.index += 1,
            }
        }
    }

    /// Returns the first `target` byte before the end of the current line.
    fn find_same_line_byte(&self, from: usize, target: u8) -> Option<usize> {
        let mut index = from;
        while index < self.bytes.len() && self.bytes[index] != b'\n' {
            if self.bytes[index] == target {
                return Some(index);
            }
            index += 1;
        }
        None
    }

    /// Scans a Markdown ATX heading marker run (`#` to `######`).
    fn scan_markdown_heading(&mut self, output: &mut String) {
        let run_start = self.index;
        let mut end = self.index;
        while end < self.bytes.len() && self.bytes[end] == b'#' {
            end += 1;
        }
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Keyword);
    }

    /// Scans one Markdown inline code span when its closing run is on the same
    /// line.
    ///
    /// WHY: an unclosed or fenced opening run stays ordinary source instead of
    ///      swallowing the rest of the block.
    fn scan_markdown_backtick_run(&mut self, output: &mut String) -> bool {
        let run_start = self.index;
        let delimiter_len = self.bytes[self.index..]
            .iter()
            .take_while(|&&byte| byte == b'`')
            .count();
        let delimiter = &self.bytes[run_start..run_start + delimiter_len];

        let mut end = self.index + delimiter_len;
        while end < self.bytes.len() && self.bytes[end] != b'\n' {
            if self.bytes[end] == b'`' && self.bytes[end..].starts_with(delimiter) {
                end += delimiter_len;
                self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::String);
                return true;
            }
            end += 1;
        }

        false
    }

    /// True when `[` opens a TOML table header at the start of a line.
    fn toml_table_header_starts_here(&self) -> bool {
        self.at_line_start()
    }

    /// Scans one TOML `[table]` or `[[array-of-tables]]` header.
    ///
    /// WHY: the whole header is one keyword span so dotted and quoted names do
    ///      not need a second parser; a header without a closing bracket falls
    ///      back to ordinary delimiter scanning.
    fn scan_toml_table_header(&mut self, output: &mut String) {
        let run_start = self.index;
        let double_bracket = self.bytes.get(self.index + 1) == Some(&b'[');
        let mut end = self.index + if double_bracket { 2 } else { 1 };

        while end < self.bytes.len() && self.bytes[end] != b'\n' {
            if !double_bracket && self.bytes[end] == b']' {
                self.emit_highlighted_range(output, run_start, end + 1, CodeHighlightRole::Keyword);
                return;
            }

            if double_bracket && self.bytes[end] == b']' && self.bytes.get(end + 1) == Some(&b']') {
                self.emit_highlighted_range(output, run_start, end + 2, CodeHighlightRole::Keyword);
                return;
            }

            end += 1;
        }

        self.scan_delimiter(output);
    }

    /// Scans one `/* ... */` block comment through the first `*/`.
    fn scan_block_comment(&mut self, output: &mut String) {
        let run_start = self.index;
        let end = self.bytes[self.index + 2..]
            .windows(2)
            .position(|window| window == b"*/")
            .map(|offset| self.index + 2 + offset + 2)
            .unwrap_or(self.bytes.len());

        self.expected_word_role = None;
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Comment);
    }

    /// Scans one C preprocessor directive name after `#`.
    fn scan_c_preprocessor(&mut self, output: &mut String) {
        let run_start = self.index;
        let mut end = self.index + 1;

        while end < self.bytes.len()
            && (self.bytes[end].is_ascii_alphanumeric() || self.bytes[end] == b'_')
        {
            end += 1;
        }

        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Keyword);
    }

    /// Scans one CSS at-rule name after `@`.
    fn scan_css_at_rule(&mut self, output: &mut String) {
        let run_start = self.index;
        let mut end = self.index + 1;

        while end < self.bytes.len()
            && (self.bytes[end].is_ascii_alphanumeric() || matches!(self.bytes[end], b'-' | b'_'))
        {
            end += 1;
        }

        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Keyword);
    }

    /// True when `word_start` begins a YAML mapping key position: the start of
    /// a line, optionally after a `- ` list marker.
    fn yaml_key_starts_line(&self, word_start: usize) -> bool {
        let mut index = word_start;

        while index > 0 && matches!(self.bytes[index - 1], b' ' | b'\t') {
            index -= 1;
        }

        if index > 0 && self.bytes[index - 1] == b'-' {
            index -= 1;
            while index > 0 && matches!(self.bytes[index - 1], b' ' | b'\t') {
                index -= 1;
            }
        }

        index == 0 || self.bytes[index - 1] == b'\n'
    }

    fn matches_comment_prefix(&self) -> bool {
        let Some(prefix) = self.language.comment_prefix() else {
            return false;
        };

        self.bytes[self.index..].starts_with(prefix.as_bytes())
    }

    fn next_non_horizontal_whitespace_byte(&self, from: usize) -> Option<u8> {
        let mut index = from;
        while index < self.bytes.len() {
            match self.bytes[index] {
                b' ' | b'\t' => index += 1,
                byte => return Some(byte),
            }
        }
        None
    }

    fn flush_plain(&mut self, output: &mut String) {
        if self.plain_start < self.index {
            push_escaped_html_text(output, &self.source[self.plain_start..self.index]);
        }
        self.plain_start = self.index;
    }
}

/// Escapes one source slice and wraps it in a role span.
fn push_role_span_escaped(output: &mut String, role: CodeHighlightRole, text: &str) {
    output.push_str("<span class='");
    output.push_str(role.class_name());
    output.push_str("'>");
    push_escaped_html_text(output, text);
    output.push_str("</span>");
}
