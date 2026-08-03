//! Built-in `$code` template style support.
//!
//! This module owns both halves of the feature:
//! - parsing the narrow `$code` / `$code("ext")` directive syntax
//! - converting compile-time body string runs into safe HTML with optional syntax highlighting
//!
//! The shared template formatter pipeline owns whitespace normalization before code reaches this
//! module. This module owns escaping, presentation and the `<code>` wrapper. Exact Moth source-word
//! classification comes from the compiler-owned keyword module; this module never keeps a second
//! current Moth word list.
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
use crate::compiler_frontend::builtins::error_type::ERROR_TYPE_NAME;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::external_packages::IO_NAMESPACE_NAME;
use crate::compiler_frontend::keywords::{
    SourceWordClass, attached_bang_keyword_token_kind, classify_source_word,
};
use crate::compiler_frontend::style_directives::StyleDirectiveArgumentValue;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::sync::Arc;

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

/// Single-character operators kept for the non-Moth profiles.
///
/// WHAT: preserves the pre-scanner operator surface for languages whose profiles
///       do not define compound forms yet.
/// WHY: Moth owns the maximal-munch table; other profiles keep their current
///      lexical behaviour until they adopt the shared palette in the same pass.
const NON_MOTH_OPERATOR_BYTES: &[u8] = b"=:-+*/%^!?|&<>~@#$`";

/// Bounded contract-list state for the Moth heuristic.
///
/// WHAT: remembers whether the next ALL_CAPS identifier sits inside trait or
///       conformance syntax (`must`, `must not`, generic `is` or a comma/`and`
///       continuation).
/// WHY: ALL_CAPS casing alone must never decide the Contract role, so the
///      scanner needs a tiny expectation that structural boundaries reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MothContractExpectation {
    None,
    AfterMust,
    AfterMustNot,
    AfterIs,
    InContractList,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodeLanguage {
    Generic,
    Text,
    Moth,
    JavaScript,
    TypeScript,
    Python,
    Rust,
    Shell,
}

impl CodeLanguage {
    pub(crate) fn from_alias(alias: &str) -> Option<Self> {
        match alias {
            "txt" | "text" => Some(Self::Text),
            "moth" => Some(Self::Moth),
            "js" | "javascript" => Some(Self::JavaScript),
            "ts" | "typescript" => Some(Self::TypeScript),
            "py" | "python" => Some(Self::Python),
            "rs" | "rust" => Some(Self::Rust),
            "bash" | "sh" | "shell" => Some(Self::Shell),
            _ => None,
        }
    }

    pub(crate) fn supported_aliases() -> &'static str {
        "\"txt\"/\"text\", \"moth\", \"js\"/\"javascript\", \"ts\"/\"typescript\", \"py\"/\"python\", \"rs\"/\"rust\", \"bash\"/\"sh\"/\"shell\""
    }

    fn comment_prefix(self) -> Option<&'static str> {
        match self {
            Self::Text => None,
            Self::Generic => Some("//"),
            Self::Moth => Some("--"),
            Self::JavaScript | Self::TypeScript | Self::Rust => Some("//"),
            Self::Python => Some("#"),
            Self::Shell => Some("#"),
        }
    }
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
        // templates, dynamic expressions) pass through without highlighting.
        let mut output_pieces: Vec<FormatterOutputPiece> = Vec::with_capacity(input.pieces.len());
        let mut first_text_emitted = false;

        for piece in input.pieces {
            match piece {
                FormatterInputPiece::Text(text_piece) => {
                    let text = string_table.resolve(text_piece.text);

                    // Allocate one output string per text piece and write the opening
                    // wrapper directly into it, avoiding a second format! allocation.
                    let mut output = String::with_capacity(text.len() + 32);
                    if !first_text_emitted {
                        first_text_emitted = true;
                        output.push_str("<code class='codeblock'>");
                    }

                    if self.language == CodeLanguage::Text {
                        push_escaped_text(&mut output, text);
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

        // Close the <code> block on the last text piece.
        if first_text_emitted {
            for piece in output_pieces.iter_mut().rev() {
                if let FormatterOutputPiece::Text(text) = piece {
                    text.push_str("</code>");
                    break;
                }
            }
        }

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

/// Converts raw source code into highlighted HTML markup.
///
/// WHAT: entry point used by the formatter tests; the formatter itself scans
///       directly into its own output buffer through `highlight_code_html_into`.
/// WHY: keeping one owned output string per formatter text piece avoids a full
///      source copy and a second wrapper allocation.
#[cfg(test)]
pub(crate) fn highlight_code_html(source: &str, language: CodeLanguage) -> String {
    let mut output = String::with_capacity(source.len() + 16);
    highlight_code_html_into(source, language, &mut output);
    output
}

/// Scans one borrowed source slice directly into `output`.
fn highlight_code_html_into(source: &str, language: CodeLanguage, output: &mut String) {
    let mut scanner = CodeScanner::new(source, language);
    scanner.scan(output);
}

/// One byte-indexed scanner pass over a borrowed source slice.
///
/// WHAT: owns the current position, the plain-run start, the language profile
///       and the bounded Moth contract expectation.
/// WHY: a small state owner keeps the scanning control flow explicit and lets
///      every helper read source bytes without copying identifiers or words.
struct CodeScanner<'source> {
    source: &'source str,
    bytes: &'source [u8],
    index: usize,
    plain_start: usize,
    language: CodeLanguage,
    moth_contract_expectation: MothContractExpectation,
    pending_word_role: Option<CodeHighlightRole>,
}

impl<'source> CodeScanner<'source> {
    fn new(source: &'source str, language: CodeLanguage) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            index: 0,
            plain_start: 0,
            language,
            moth_contract_expectation: MothContractExpectation::None,
            pending_word_role: None,
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

                if self.language == CodeLanguage::Moth {
                    if byte == b'$' && self.moth_directive_starts_here() {
                        self.scan_moth_directive(output);
                        return;
                    }

                    if byte == b'@' && self.moth_path_starts_here() {
                        self.scan_moth_path(output);
                        return;
                    }

                    if matches!(byte, b':' | b';' | b'|') {
                        if self.operator_length().is_some() {
                            self.scan_operator(output);
                        } else {
                            self.scan_delimiter(output);
                        }
                        return;
                    }
                }

                if self.operator_length().is_some() {
                    self.scan_operator(output);
                    return;
                }

                // Structural Moth boundaries reset the contract expectation.
                if self.language == CodeLanguage::Moth
                    && matches!(
                        byte,
                        b'\n' | b'=' | b'<' | b'>' | b'(' | b')' | b'[' | b']' | b'{' | b'}'
                    )
                {
                    self.moth_contract_expectation = MothContractExpectation::None;
                }

                // Plain punctuation and whitespace stay in the batched plain run.
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
        self.index += ch.len_utf8();
    }

    fn scan_word(&mut self, output: &mut String) {
        let word_start = self.index;
        self.flush_plain(output);

        while self.index < self.bytes.len() {
            let byte = self.bytes[self.index];
            if byte.is_ascii() {
                if byte.is_ascii_alphanumeric() || byte == b'_' {
                    self.index += 1;
                } else {
                    break;
                }
            } else {
                self.advance_unicode();
            }
        }

        // `return!` and `cast!` keep the attached bang inside the keyword span.
        if self.language == CodeLanguage::Moth
            && self.index < self.bytes.len()
            && self.bytes[self.index] == b'!'
            && attached_bang_keyword_token_kind(&self.source[word_start..self.index]).is_some()
        {
            self.index += 1;
        }

        let word = &self.source[word_start..self.index];
        self.emit_word(output, word);
        self.plain_start = self.index;
    }

    fn emit_word(&mut self, output: &mut String, word: &str) {
        if self.language == CodeLanguage::Moth {
            self.emit_moth_word(output, word);
            return;
        }

        if let Some(role) = self.pending_word_role.take() {
            push_role_span_escaped(output, role, word);
            return;
        }

        if is_keyword(word, self.language) {
            self.set_non_moth_lookahead(word);
            push_role_span_escaped(output, CodeHighlightRole::Keyword, word);
        } else if is_type_keyword(word, self.language) {
            push_role_span_escaped(output, CodeHighlightRole::Type, word);
        } else if is_literal_word(word, self.language) {
            push_role_span_escaped(output, CodeHighlightRole::Literal, word);
        } else if self.language != CodeLanguage::Generic
            && word.chars().next().is_some_and(|ch| ch.is_uppercase())
        {
            push_role_span_escaped(output, CodeHighlightRole::Nominal, word);
        } else {
            output.push_str(word);
        }
    }

    /// Classifies one Moth word through the compiler-owned classes and the
    /// bounded lexical heuristics.
    fn emit_moth_word(&mut self, output: &mut String, word: &str) {
        let Some(role) = self.moth_word_role(word) else {
            output.push_str(word);
            return;
        };

        push_role_span_escaped(output, role, word);
    }

    fn moth_word_role(&mut self, word: &str) -> Option<CodeHighlightRole> {
        // Attached bang forms are keyword spans.
        if let Some(prefix) = word.strip_suffix('!')
            && attached_bang_keyword_token_kind(prefix).is_some()
        {
            self.moth_contract_expectation = MothContractExpectation::None;
            return Some(CodeHighlightRole::Keyword);
        }

        if let Some(classified) = classify_source_word(word) {
            let role = match classified.class {
                SourceWordClass::Keyword => CodeHighlightRole::Keyword,
                SourceWordClass::WordOperator => CodeHighlightRole::Operator,
                SourceWordClass::Literal => CodeHighlightRole::Literal,
                SourceWordClass::BuiltinType => CodeHighlightRole::Type,
            };

            // Word-level contract-list transitions.
            match word {
                "must" => self.moth_contract_expectation = MothContractExpectation::AfterMust,
                "not" if self.moth_contract_expectation == MothContractExpectation::AfterMust => {
                    self.moth_contract_expectation = MothContractExpectation::AfterMustNot;
                }
                "is" => self.moth_contract_expectation = MothContractExpectation::AfterIs,
                "and"
                    if self.moth_contract_expectation
                        == MothContractExpectation::InContractList => {}
                _ => self.moth_contract_expectation = MothContractExpectation::None,
            }

            return Some(role);
        }

        // Canonical builtin spellings that are not tokenizer keywords.
        if word == ERROR_TYPE_NAME {
            self.moth_contract_expectation = MothContractExpectation::None;
            return Some(CodeHighlightRole::Type);
        }

        if word == IO_NAMESPACE_NAME && self.bytes.get(self.index) == Some(&b'.') {
            self.moth_contract_expectation = MothContractExpectation::None;
            return Some(CodeHighlightRole::Type);
        }

        if is_all_caps_word(word) {
            let in_contract_context = self.moth_contract_expectation
                != MothContractExpectation::None
                || self.all_caps_followed_by_must(self.index);
            self.moth_contract_expectation = MothContractExpectation::None;

            if in_contract_context {
                self.moth_contract_expectation = MothContractExpectation::InContractList;
                return Some(CodeHighlightRole::Contract);
            }

            return None;
        }

        if is_pascal_case_word(word) {
            self.moth_contract_expectation = MothContractExpectation::None;
            return Some(CodeHighlightRole::Nominal);
        }

        // Ordinary identifiers become functions only before `(` or `|`.
        self.moth_contract_expectation = MothContractExpectation::None;
        match self.next_non_horizontal_whitespace_byte(self.index) {
            Some(b'(') | Some(b'|') => Some(CodeHighlightRole::Function),
            _ => None,
        }
    }

    fn set_non_moth_lookahead(&mut self, word: &str) {
        match self.language {
            CodeLanguage::JavaScript | CodeLanguage::TypeScript | CodeLanguage::Shell
                if word == "function" =>
            {
                self.pending_word_role = Some(CodeHighlightRole::Function);
            }
            CodeLanguage::Python if word == "def" => {
                self.pending_word_role = Some(CodeHighlightRole::Function);
            }
            CodeLanguage::Rust if word == "fn" => {
                self.pending_word_role = Some(CodeHighlightRole::Function);
            }
            CodeLanguage::TypeScript if word == "interface" => {
                self.pending_word_role = Some(CodeHighlightRole::Contract);
            }
            CodeLanguage::Rust if word == "trait" => {
                self.pending_word_role = Some(CodeHighlightRole::Contract);
            }
            _ => {}
        }
    }

    fn scan_line_comment(&mut self, output: &mut String) {
        let run_start = self.index;
        let mut end = self.index;

        // The comment run stops before the newline; the newline stays in the
        // plain run so whitespace is preserved exactly.
        while end < self.bytes.len() && self.bytes[end] != b'\n' {
            end += 1;
        }

        self.flush_plain(output);
        push_role_span_escaped(
            output,
            CodeHighlightRole::Comment,
            &self.source[run_start..end],
        );
        self.index = end;
        self.plain_start = end;
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

        self.flush_plain(output);
        push_role_span_escaped(
            output,
            CodeHighlightRole::String,
            &self.source[run_start..end],
        );
        self.index = end;
        self.plain_start = end;
    }

    fn scan_delimiter(&mut self, output: &mut String) {
        let run_start = self.index;
        self.flush_plain(output);
        self.index += 1;

        if self.language == CodeLanguage::Moth {
            self.moth_contract_expectation = MothContractExpectation::None;
        }

        push_role_span_escaped(
            output,
            CodeHighlightRole::Delimiter,
            &self.source[run_start..self.index],
        );
        self.plain_start = self.index;
    }

    fn scan_moth_directive(&mut self, output: &mut String) {
        let run_start = self.index;
        self.index += 1;

        while self.index < self.bytes.len() {
            let byte = self.bytes[self.index];
            if byte.is_ascii_alphanumeric() || byte == b'_' {
                self.index += 1;
            } else {
                break;
            }
        }

        self.flush_plain(output);
        push_role_span_escaped(
            output,
            CodeHighlightRole::Directive,
            &self.source[run_start..self.index],
        );
        self.plain_start = self.index;
    }

    fn scan_moth_path(&mut self, output: &mut String) {
        let run_start = self.index;
        self.index += 1;

        while self.index < self.bytes.len() && is_moth_path_byte(self.bytes[self.index]) {
            self.index += 1;
        }

        self.flush_plain(output);
        push_role_span_escaped(
            output,
            CodeHighlightRole::String,
            &self.source[run_start..self.index],
        );
        self.plain_start = self.index;
    }

    fn scan_number(&mut self, output: &mut String) {
        let run_start = self.index;
        let end = match self.language {
            CodeLanguage::Moth => self.moth_number_end(),
            _ => self.legacy_number_end(),
        };

        self.flush_plain(output);
        push_role_span_escaped(
            output,
            CodeHighlightRole::Number,
            &self.source[run_start..end],
        );
        self.index = end;
        self.plain_start = end;
    }

    /// Recognises Moth numeric runs: digits with separators, a decimal fraction
    /// and a lowercase exponent with an optional sign.
    ///
    /// WHY: tolerant by design. Range, separator placement and finiteness are
    ///      validated by the real tokenizer, never by the presentation scanner.
    fn moth_number_end(&self) -> usize {
        let mut end = consume_while(self.bytes, self.index, |byte| {
            byte.is_ascii_digit() || byte == b'_'
        });

        if end + 1 < self.bytes.len()
            && self.bytes[end] == b'.'
            && self.bytes[end + 1].is_ascii_digit()
        {
            end = consume_while(self.bytes, end + 1, |byte| {
                byte.is_ascii_digit() || byte == b'_'
            });
        }

        if end < self.bytes.len() && self.bytes[end] == b'e' {
            let mut exponent_end = end + 1;
            if exponent_end < self.bytes.len()
                && (self.bytes[exponent_end] == b'+' || self.bytes[exponent_end] == b'-')
            {
                exponent_end += 1;
            }

            if exponent_end < self.bytes.len() && self.bytes[exponent_end].is_ascii_digit() {
                end = consume_while(self.bytes, exponent_end, |byte| {
                    byte.is_ascii_digit() || byte == b'_'
                });
            }
        }

        end
    }

    /// Preserves the pre-scanner numeric run for non-Moth profiles: digits,
    /// dots, underscores and Unicode numeric scalars.
    fn legacy_number_end(&self) -> usize {
        let mut end = self.index;

        while end < self.bytes.len() {
            let byte = self.bytes[end];
            if byte.is_ascii() {
                if byte.is_ascii_digit() || byte == b'.' || byte == b'_' {
                    end += 1;
                } else {
                    break;
                }
            } else {
                let ch = self.source[end..]
                    .chars()
                    .next()
                    .expect("number position is on a char boundary");
                if ch.is_numeric() {
                    end += ch.len_utf8();
                } else {
                    break;
                }
            }
        }

        end
    }

    fn scan_operator(&mut self, output: &mut String) {
        let Some(length) = self.operator_length() else {
            self.index += 1;
            return;
        };

        let run_start = self.index;
        self.flush_plain(output);
        self.index += length;

        // Moth operators are structural boundaries for the contract heuristic,
        // including `=`, `->`, `<=` and `>=`, which never continue a contract list.
        if self.language == CodeLanguage::Moth {
            self.moth_contract_expectation = MothContractExpectation::None;
        }

        push_role_span_escaped(
            output,
            CodeHighlightRole::Operator,
            &self.source[run_start..self.index],
        );
        self.plain_start = self.index;
    }

    fn operator_length(&self) -> Option<usize> {
        match self.language {
            CodeLanguage::Moth => self.moth_operator_length(),
            _ => {
                if NON_MOTH_OPERATOR_BYTES.contains(&self.bytes[self.index]) {
                    Some(1)
                } else {
                    None
                }
            }
        }
    }

    /// Returns the maximal-munch length of one Moth operator.
    ///
    /// WHAT: checks longer compound forms before their prefixes so `//=` is one
    ///       span and `::`, `..`, `->` and `=>` stay whole tokens.
    /// WHY: `==`, `!=` and `&&` are not Moth operators, so the fallback table
    ///      never combines those byte pairs into one invented token.
    fn moth_operator_length(&self) -> Option<usize> {
        let bytes = &self.bytes[self.index..];
        let next = |offset: usize| bytes.get(offset).copied();

        match bytes[0] {
            b'/' => {
                if next(1) == Some(b'/') {
                    if next(2) == Some(b'=') {
                        return Some(3);
                    }
                    return Some(2);
                }
                if next(1) == Some(b'=') {
                    return Some(2);
                }
                Some(1)
            }
            b'+' | b'*' | b'%' | b'^' | b'#' | b'~' | b'$' => {
                if next(1) == Some(b'=') {
                    Some(2)
                } else {
                    Some(1)
                }
            }
            b'-' => {
                if next(1) == Some(b'=') || next(1) == Some(b'>') {
                    Some(2)
                } else {
                    Some(1)
                }
            }
            b'=' => {
                if next(1) == Some(b'>') {
                    Some(2)
                } else {
                    Some(1)
                }
            }
            b'<' => {
                if next(1) == Some(b'<') || next(1) == Some(b'=') {
                    Some(2)
                } else {
                    Some(1)
                }
            }
            b'>' => {
                if next(1) == Some(b'>') || next(1) == Some(b'=') {
                    Some(2)
                } else {
                    Some(1)
                }
            }
            b':' => {
                if next(1) == Some(b':') {
                    Some(2)
                } else {
                    None
                }
            }
            b'.' => {
                if next(1) == Some(b'.') {
                    Some(2)
                } else {
                    None
                }
            }
            b'!' | b'?' | b'&' | b'@' => Some(1),
            _ => None,
        }
    }

    fn matches_comment_prefix(&self) -> bool {
        let Some(prefix) = self.language.comment_prefix() else {
            return false;
        };

        self.bytes[self.index..].starts_with(prefix.as_bytes())
    }

    fn moth_directive_starts_here(&self) -> bool {
        matches!(
            self.bytes.get(self.index + 1),
            Some(b'a'..=b'z') | Some(b'_')
        )
    }

    fn moth_path_starts_here(&self) -> bool {
        matches!(self.bytes.get(self.index + 1), Some(byte) if is_moth_path_byte(*byte))
    }

    fn all_caps_followed_by_must(&self, word_end: usize) -> bool {
        let mut index = word_end;
        while index < self.bytes.len() && (self.bytes[index] == b' ' || self.bytes[index] == b'\t')
        {
            index += 1;
        }

        if !self.bytes[index..].starts_with(b"must") {
            return false;
        }

        !matches!(
            self.bytes.get(index + 4),
            None | Some(b'a'..=b'z') | Some(b'A'..=b'Z') | Some(b'0'..=b'9') | Some(b'_')
        )
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
            push_escaped_text(output, &self.source[self.plain_start..self.index]);
        }
        self.plain_start = self.index;
    }
}

/// Advances `index` while the byte predicate holds.
fn consume_while(bytes: &[u8], mut index: usize, predicate: impl Fn(u8) -> bool) -> usize {
    while index < bytes.len() && predicate(bytes[index]) {
        index += 1;
    }
    index
}

/// True when `byte` may continue a tolerant Moth import/resource path run.
fn is_moth_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'/' | b'.' | b'-')
}

/// True for ALL_CAPS identifiers with at least two letters.
///
/// WHY: single uppercase letters act as generic parameter names and stay
///      nominal, while `PI`, `TAU` and `DISPLAY_TEXT` use the all-caps shape.
fn is_all_caps_word(word: &str) -> bool {
    let mut letter_count = 0usize;

    for ch in word.chars() {
        if ch == '_' {
            continue;
        }
        if !ch.is_uppercase() {
            return false;
        }
        letter_count += 1;
    }

    letter_count >= 2
}

/// True for PascalCase identifiers that are not all-caps.
fn is_pascal_case_word(word: &str) -> bool {
    word.chars().next().is_some_and(|ch| ch.is_uppercase()) && !is_all_caps_word(word)
}

/// Escapes one source slice and wraps it in a role span.
fn push_role_span_escaped(output: &mut String, role: CodeHighlightRole, text: &str) {
    output.push_str("<span class='");
    output.push_str(role.class_name());
    output.push_str("'>");
    push_escaped_text(output, text);
    output.push_str("</span>");
}

fn is_keyword(word: &str, language: CodeLanguage) -> bool {
    match language {
        CodeLanguage::Text | CodeLanguage::Moth => false,
        CodeLanguage::JavaScript => matches!(
            word,
            "if" | "else"
                | "return"
                | "break"
                | "continue"
                | "for"
                | "while"
                | "in"
                | "function"
                | "const"
                | "let"
                | "var"
        ),
        CodeLanguage::TypeScript | CodeLanguage::Generic => matches!(
            word,
            "if" | "else"
                | "return"
                | "break"
                | "continue"
                | "for"
                | "while"
                | "in"
                | "function"
                | "const"
                | "let"
                | "var"
                | "type"
                | "interface"
                | "enum"
        ),
        CodeLanguage::Python => matches!(
            word,
            "if" | "elif"
                | "else"
                | "return"
                | "break"
                | "continue"
                | "for"
                | "while"
                | "in"
                | "def"
                | "class"
                | "import"
                | "from"
                | "as"
        ),
        CodeLanguage::Rust => matches!(
            word,
            "if" | "else"
                | "return"
                | "break"
                | "continue"
                | "for"
                | "while"
                | "in"
                | "fn"
                | "let"
                | "mut"
                | "const"
                | "static"
                | "struct"
                | "enum"
                | "impl"
                | "trait"
                | "mod"
                | "use"
                | "pub"
                | "crate"
                | "super"
                | "self"
                | "match"
                | "async"
                | "await"
                | "move"
                | "ref"
                | "type"
                | "where"
                | "unsafe"
                | "extern"
                | "dyn"
        ),
        CodeLanguage::Shell => matches!(
            word,
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "for"
                | "while"
                | "do"
                | "done"
                | "in"
                | "function"
        ),
    }
}

fn is_type_keyword(word: &str, language: CodeLanguage) -> bool {
    match language {
        CodeLanguage::Generic | CodeLanguage::Text | CodeLanguage::Moth => false,
        CodeLanguage::TypeScript => matches!(
            word,
            "number" | "string" | "boolean" | "unknown" | "never" | "void" | "any"
        ),
        CodeLanguage::Rust => matches!(
            word,
            "i8" | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "isize"
                | "usize"
                | "f32"
                | "f64"
                | "bool"
                | "char"
                | "str"
        ),
        CodeLanguage::JavaScript | CodeLanguage::Python | CodeLanguage::Shell => false,
    }
}

fn is_literal_word(word: &str, language: CodeLanguage) -> bool {
    match language {
        CodeLanguage::JavaScript | CodeLanguage::TypeScript => {
            matches!(word, "true" | "false" | "null" | "undefined")
        }
        CodeLanguage::Python => matches!(word, "True" | "False" | "None"),
        CodeLanguage::Rust | CodeLanguage::Shell => matches!(word, "true" | "false"),
        CodeLanguage::Generic | CodeLanguage::Text | CodeLanguage::Moth => false,
    }
}

fn push_escaped_text(output: &mut String, text: &str) {
    for ch in text.chars() {
        push_escaped_char(output, ch);
    }
}

fn push_escaped_char(output: &mut String, ch: char) {
    match ch {
        '&' => output.push_str("&amp;"),
        '<' => output.push_str("&lt;"),
        '>' => output.push_str("&gt;"),
        '"' => output.push_str("&quot;"),
        '\'' => output.push_str("&#39;"),
        _ => output.push(ch),
    }
}
