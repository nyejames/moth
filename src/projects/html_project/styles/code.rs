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
use crate::compiler_frontend::builtins::error_type::ERROR_TYPE_NAME;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::external_packages::IO_NAMESPACE_NAME;
use crate::compiler_frontend::keywords::{
    SourceWordClass, attached_bang_keyword_token_kind, classify_source_word,
};
use crate::compiler_frontend::style_directives::StyleDirectiveArgumentValue;
use crate::compiler_frontend::symbols::identifier_policy::is_uppercase_constant_name;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::styles::escape_html::push_escaped_html_text;
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

/// Contract-list kind for the Moth heuristic.
///
/// WHAT: distinguishes trait conformance lists (`must`, `must not`) from
///       generic-bound lists after `is` inside a generic declaration.
/// WHY: commas continue conformance lists but end a generic bound list so the
///      next identifier can be a new generic parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContractListKind {
    Conformance,
    GenericBound,
}

/// Bounded contract-list state for the Moth heuristic.
///
/// WHAT: remembers whether the next uppercase-constant identifier is a
///       contract name and which kind of list expects it. An expectation
///       armed by a conformance comma survives a newline; every other
///       expectation dies at declaration boundaries.
/// WHY: casing alone must never decide the Contract role, so the scanner
///      needs a tiny expectation that structural boundaries reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContractState {
    None,
    ExpectName {
        kind: ContractListKind,
        continued_after_comma: bool,
    },
    AfterName(ContractListKind),
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
    Html,
    Markdown,
    Toml,
    Json,
    Yaml,
    Css,
    C,
    Sql,
}

/// Canonical short and long aliases for every supported `$code` language.
///
/// WHAT: one table owns alias resolution and the supported-values diagnostic,
///       so the two can never drift apart.
/// WHY: adding a language means extending this table, its formatter rules and
///      the documentation lists, not a second alias match somewhere else.
pub(crate) const LANGUAGE_ALIASES: &[(&str, CodeLanguage)] = &[
    ("txt", CodeLanguage::Text),
    ("text", CodeLanguage::Text),
    ("html", CodeLanguage::Html),
    ("md", CodeLanguage::Markdown),
    ("markdown", CodeLanguage::Markdown),
    ("toml", CodeLanguage::Toml),
    ("json", CodeLanguage::Json),
    ("yaml", CodeLanguage::Yaml),
    ("yml", CodeLanguage::Yaml),
    ("css", CodeLanguage::Css),
    ("c", CodeLanguage::C),
    ("sql", CodeLanguage::Sql),
    ("moth", CodeLanguage::Moth),
    ("js", CodeLanguage::JavaScript),
    ("javascript", CodeLanguage::JavaScript),
    ("ts", CodeLanguage::TypeScript),
    ("typescript", CodeLanguage::TypeScript),
    ("py", CodeLanguage::Python),
    ("python", CodeLanguage::Python),
    ("rs", CodeLanguage::Rust),
    ("rust", CodeLanguage::Rust),
    ("bash", CodeLanguage::Shell),
    ("sh", CodeLanguage::Shell),
    ("shell", CodeLanguage::Shell),
];

impl CodeLanguage {
    pub(crate) fn from_alias(alias: &str) -> Option<Self> {
        LANGUAGE_ALIASES
            .iter()
            .find(|(candidate, _)| *candidate == alias)
            .map(|(_, language)| *language)
    }

    /// Renders the supported alias groups in table order, for example
    /// `"txt"/"text", "html", ...`.
    ///
    /// WHY: the unsupported-language diagnostic should show the exact aliases
    ///      `from_alias` accepts without keeping a second hand-written list.
    pub(crate) fn supported_aliases() -> String {
        let mut groups: Vec<(CodeLanguage, Vec<&str>)> = Vec::new();

        for (alias, language) in LANGUAGE_ALIASES {
            match groups.last_mut() {
                Some((group_language, aliases)) if *group_language == *language => {
                    aliases.push(alias);
                }
                _ => groups.push((*language, vec![alias])),
            }
        }

        groups
            .into_iter()
            .map(|(_, aliases)| {
                aliases
                    .iter()
                    .map(|alias| format!("\"{alias}\""))
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn comment_prefix(self) -> Option<&'static str> {
        match self {
            Self::Text | Self::Html | Self::Markdown | Self::Css => None,
            Self::Generic | Self::Json | Self::C => Some("//"),
            Self::Moth => Some("--"),
            Self::JavaScript | Self::TypeScript | Self::Rust => Some("//"),
            Self::Python | Self::Shell | Self::Toml | Self::Yaml => Some("#"),
            Self::Sql => Some("--"),
        }
    }

    /// True when capitalized words receive the nominal fallback role.
    ///
    /// WHY: code languages use the fallback to surface type-like names, while
    ///      prose-bearing profiles (HTML, Markdown, TOML) keep ordinary words
    ///      plain so content stays readable.
    fn has_nominal_fallback(self) -> bool {
        matches!(
            self,
            Self::JavaScript | Self::TypeScript | Self::Python | Self::Rust | Self::Shell | Self::C
        )
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

    /// Returns the byte index just past one identifier word.
    ///
    /// WHAT: scans ASCII alphanumerics and underscores, consumes one scalar for
    ///       any non-ASCII byte, then includes an attached `return!` / `cast!`
    ///       bang for Moth.
    /// WHY: the word range is computed locally so the shared emitter can hold the
    ///      single invariant that the cursor is still at the token start.
    fn word_end(&self, start: usize) -> usize {
        let mut end = start;

        while end < self.bytes.len() {
            let byte = self.bytes[end];
            if byte.is_ascii() {
                if byte.is_ascii_alphanumeric() || byte == b'_' {
                    end += 1;
                } else {
                    break;
                }
            } else {
                let ch = self.source[end..]
                    .chars()
                    .next()
                    .expect("scan position is always on a char boundary");
                end += ch.len_utf8();
            }
        }

        // `return!` and `cast!` keep the attached bang inside the keyword span.
        if self.language == CodeLanguage::Moth
            && end < self.bytes.len()
            && self.bytes[end] == b'!'
            && attached_bang_keyword_token_kind(&self.source[start..end]).is_some()
        {
            end += 1;
        }

        end
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

    /// Classifies one Moth word through the compiler-owned classes and the
    /// bounded lexical heuristics.
    fn moth_word_role(&mut self, word: &str, word_end: usize) -> Option<CodeHighlightRole> {
        // Attached bang forms are keyword spans.
        if let Some(prefix) = word.strip_suffix('!')
            && attached_bang_keyword_token_kind(prefix).is_some()
        {
            self.reset_declaration_context();
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
                "type" => {
                    self.generic_declaration = true;
                    self.contract_state = ContractState::None;
                }
                "loop" => {
                    // A collection or range loop keeps its source/projection
                    // unclassified until the header ends at `:` or a newline.
                    self.loop_header_depth = Some(self.moth_delimiter_depth);
                    self.contract_state = ContractState::None;
                }
                "must" => {
                    self.generic_declaration = false;
                    self.contract_state = ContractState::ExpectName {
                        kind: ContractListKind::Conformance,
                        continued_after_comma: false,
                    };
                }
                "not"
                    if matches!(
                        self.contract_state,
                        ContractState::ExpectName {
                            kind: ContractListKind::Conformance,
                            ..
                        }
                    ) => {}
                "is" => {
                    self.contract_state = if self.generic_declaration {
                        ContractState::ExpectName {
                            kind: ContractListKind::GenericBound,
                            continued_after_comma: false,
                        }
                    } else {
                        ContractState::None
                    };
                }
                "and" if matches!(self.contract_state, ContractState::AfterName(_)) => {
                    let ContractState::AfterName(kind) = self.contract_state else {
                        unreachable!("guarded by the match arm above");
                    };
                    self.contract_state = ContractState::ExpectName {
                        kind,
                        continued_after_comma: false,
                    };
                }
                _ => self.contract_state = ContractState::None,
            }

            return Some(role);
        }

        // Canonical builtin spellings that are not tokenizer keywords.
        if word == ERROR_TYPE_NAME {
            self.contract_state = ContractState::None;
            return Some(CodeHighlightRole::Type);
        }

        if word == IO_NAMESPACE_NAME && self.bytes.get(word_end) == Some(&b'.') {
            self.contract_state = ContractState::None;
            return Some(CodeHighlightRole::Type);
        }

        // Contract names follow the compiler's uppercase-constant policy so
        // single letters, digits and underscores classify exactly as traits do
        // in Moth. The policy applies only in contract context; ordinary `A`,
        // `E` and digit-bearing constants keep their nominal/plain fallback.
        if is_uppercase_constant_name(word) {
            let in_contract_context = self.contract_state != ContractState::None
                || self.all_caps_followed_by_must(word_end);
            if in_contract_context {
                let kind = match self.contract_state {
                    ContractState::ExpectName { kind, .. } | ContractState::AfterName(kind) => kind,
                    // An uppercase-constant name followed by `must` declares a trait.
                    ContractState::None => ContractListKind::Conformance,
                };
                self.contract_state = ContractState::AfterName(kind);

                return Some(CodeHighlightRole::Contract);
            }

            // Outside contract context the compiler policy does not apply:
            // `A` and `E` keep their nominal fallback below, while all-caps
            // constants such as `PI` and `MAX_SIZE` stay plain.
            self.contract_state = ContractState::None;
        }

        if is_pascal_case_word(word) {
            self.contract_state = ContractState::None;
            return Some(CodeHighlightRole::Nominal);
        }

        // Ordinary identifiers become functions before `(`, before a pipe that
        // opens a new group, or when they own a generic declaration
        // (`name type T ...`). Loop sources and projections are not
        // declarations even though a binding pipe follows, and identifiers
        // inside `|...|` stay plain.
        self.contract_state = ContractState::None;
        match self.next_non_horizontal_whitespace_byte(word_end) {
            Some(b'(') => Some(CodeHighlightRole::Function),
            Some(b'|') if !self.in_pipe_group && self.loop_header_depth.is_none() => {
                Some(CodeHighlightRole::Function)
            }
            _ if !self.in_pipe_group && self.next_word_is(word_end, "type") => {
                Some(CodeHighlightRole::Function)
            }
            _ => None,
        }
    }

    /// Resets contract-list and generic-declaration context at structural
    /// boundaries so later source cannot inherit stale expectations.
    ///
    /// Loop-header state is deliberately separate: it must survive nested
    /// delimiters and source-expression operators and ends only at its own
    /// top-level `|`, header `:` or terminating newline.
    fn reset_declaration_context(&mut self) {
        self.contract_state = ContractState::None;
        self.generic_declaration = false;
    }

    /// Tracks Moth delimiter nesting for the loop-header heuristic.
    fn update_moth_delimiter_depth(&mut self) {
        match self.bytes[self.index] {
            b'(' | b'[' | b'{' => self.moth_delimiter_depth += 1,
            b')' | b']' | b'}' => {
                self.moth_delimiter_depth = self.moth_delimiter_depth.saturating_sub(1)
            }
            _ => {}
        }
    }

    /// Ends loop-header context at a pipe or colon at the same nesting depth
    /// as the `loop` keyword.
    ///
    /// WHY: the binding pipe and the header colon are top-level boundaries;
    ///      the same byte inside a nested source expression must not end the
    ///      header early.
    fn end_loop_header_at_top_level(&mut self) {
        if self.loop_header_depth == Some(self.moth_delimiter_depth) {
            self.loop_header_depth = None;
        }
    }

    /// Resets declaration context at a newline, preserving only a conformance
    /// continuation that a comma explicitly armed.
    ///
    /// WHY: `Label must FIRST,\n SECOND` stays one conformance list, while an
    ///      ordinary newline ends every declaration expectation.
    fn reset_after_newline(&mut self) {
        self.loop_header_depth = None;
        self.generic_declaration = false;

        if !matches!(
            self.contract_state,
            ContractState::ExpectName {
                kind: ContractListKind::Conformance,
                continued_after_comma: true,
            }
        ) {
            self.contract_state = ContractState::None;
        }
    }

    /// Continues a conformance list after a comma or ends a generic bound list
    /// so the next word can be a new generic parameter.
    fn transition_after_comma(&mut self) {
        self.contract_state = match self.contract_state {
            ContractState::AfterName(ContractListKind::Conformance) => ContractState::ExpectName {
                kind: ContractListKind::Conformance,
                continued_after_comma: true,
            },
            ContractState::AfterName(ContractListKind::GenericBound) => ContractState::None,
            _ => self.contract_state,
        };
    }

    /// True when the next word after horizontal whitespace is exactly
    /// `expected`.
    ///
    /// WHY: a generic function owner is recognised by its `name type T ...`
    ///      shape, but only when `type` is the immediate next word.
    fn next_word_is(&self, from: usize, expected: &str) -> bool {
        let mut index = from;
        while index < self.bytes.len() && matches!(self.bytes[index], b' ' | b'\t') {
            index += 1;
        }

        let end = self.word_end(index);
        end > index && self.source[index..end] == *expected
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

    fn scan_moth_directive(&mut self, output: &mut String) {
        let run_start = self.index;
        let mut end = self.index + 1;

        while end < self.bytes.len() {
            let byte = self.bytes[end];
            if byte.is_ascii_alphanumeric() || byte == b'_' {
                end += 1;
            } else {
                break;
            }
        }

        self.expected_word_role = None;
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Directive);
    }

    fn scan_moth_path(&mut self, output: &mut String) {
        let run_start = self.index;
        let mut end = self.index + 1;

        while end < self.bytes.len() && is_moth_path_byte(self.bytes[end]) {
            end += 1;
        }

        self.expected_word_role = None;
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::String);
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

    fn scan_number(&mut self, output: &mut String) {
        let run_start = self.index;
        let end = match self.language {
            CodeLanguage::Moth => self.moth_number_end(),
            _ => self.legacy_number_end(),
        };

        self.expected_word_role = None;
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Number);
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
    /// underscores, Unicode numeric scalars and a decimal point that is
    /// followed by a digit.
    ///
    /// WHY: allowing every dot would swallow Rust range operators (`0..10`)
    ///      and float method access (`1.0.to_string()`), so a dot only joins
    ///      the run when it starts a fractional part.
    fn legacy_number_end(&self) -> usize {
        let mut end = self.index;

        while end < self.bytes.len() {
            let byte = self.bytes[end];
            if byte.is_ascii() {
                if byte.is_ascii_digit() || byte == b'_' {
                    end += 1;
                    continue;
                }
                if byte == b'.'
                    && self
                        .bytes
                        .get(end + 1)
                        .is_some_and(|next| next.is_ascii_digit())
                {
                    end += 1;
                    continue;
                }
                break;
            }

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

        end
    }

    fn scan_operator(&mut self, output: &mut String) {
        let Some(length) = self.operator_length() else {
            self.index += 1;
            return;
        };

        let run_start = self.index;
        let end = run_start + length;

        // Moth operators are structural boundaries for the contract heuristic,
        // including `=`, `->`, `<=` and `>=`, which never continue a contract list.
        if self.language == CodeLanguage::Moth {
            self.reset_declaration_context();
        }

        self.expected_word_role = None;
        self.emit_highlighted_range(output, run_start, end, CodeHighlightRole::Operator);
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
        if !matches!(self.bytes.get(self.index + 1), Some(byte) if is_moth_path_byte(*byte)) {
            return false;
        }

        // A path may start only at a lexical boundary: not directly after
        // another `@` and not after a path or identifier continuation byte,
        // including a Unicode identifier continuation. This keeps invalid
        // doubled prefixes such as `@@name` and attached forms such as
        // `π@core/io` visible as plain source.
        self.index == 0 || !self.previous_scalar_continues_path_or_word()
    }

    /// True when the scalar immediately before the cursor is a path or
    /// identifier continuation.
    ///
    /// WHY: ASCII continuation bytes are checked directly, while a non-ASCII
    ///      previous byte must be decoded so a Unicode identifier such as `π`
    ///      blocks an attached `@path`.
    fn previous_scalar_continues_path_or_word(&self) -> bool {
        let before = self.index - 1;
        let byte = self.bytes[before];

        if byte.is_ascii() {
            return byte == b'@' || is_moth_path_byte(byte);
        }

        // Walk back from the previous byte over UTF-8 continuation bytes to
        // the scalar's leading byte, then reject when that scalar continues
        // an identifier.
        let mut scalar_start = before;
        while scalar_start > 0 && self.bytes[scalar_start] & 0xC0 == 0x80 {
            scalar_start -= 1;
        }

        self.source[scalar_start..self.index]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
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
            push_escaped_html_text(output, &self.source[self.plain_start..self.index]);
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
///      This is the presentation split for the nominal fallback; contract
///      eligibility reuses the compiler's `is_uppercase_constant_name`
///      policy, so `A`, `TRAIT2` and `HTTP_2` classify as traits in contract
///      context.
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
    push_escaped_html_text(output, text);
    output.push_str("</span>");
}

/// Direct non-Moth word classification result.
///
/// WHAT: carries the current word role and the optional role for the exact next
///       identifier, so one classifier owns all per-language vocabulary.
struct NonMothWordClass {
    role: Option<CodeHighlightRole>,
    next_identifier_role: Option<CodeHighlightRole>,
}

/// Classifies one non-Moth word from direct per-language matches.
///
/// WHAT: returns the word role and optionally arms a declaration-name role for
///       the exact next identifier. `Generic` has no language vocabulary.
/// WHY: one local classifier replaces the previous keyword/type/literal helper
///      trio plus the loose pending-role state.
fn classify_non_moth_word(language: CodeLanguage, word: &str) -> NonMothWordClass {
    let mut role = None;
    let mut next_identifier_role = None;

    match language {
        CodeLanguage::JavaScript => match word {
            "if" | "else" | "return" | "break" | "continue" | "for" | "while" | "in" | "const"
            | "let" | "var" => role = Some(CodeHighlightRole::Keyword),
            "true" | "false" | "null" | "undefined" => {
                role = Some(CodeHighlightRole::Literal);
            }
            "function" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Function);
            }
            _ => {}
        },
        CodeLanguage::TypeScript => match word {
            "if" | "else" | "return" | "break" | "continue" | "for" | "while" | "in" | "const"
            | "let" | "var" | "type" | "enum" => {
                role = Some(CodeHighlightRole::Keyword);
            }
            "number" | "string" | "boolean" | "unknown" | "never" | "void" | "any" => {
                role = Some(CodeHighlightRole::Type);
            }
            "true" | "false" | "null" | "undefined" => {
                role = Some(CodeHighlightRole::Literal);
            }
            "function" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Function);
            }
            "interface" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Contract);
            }
            _ => {}
        },
        CodeLanguage::Python => match word {
            "if" | "elif" | "else" | "return" | "break" | "continue" | "for" | "while" | "in"
            | "class" | "import" | "from" | "as" => {
                role = Some(CodeHighlightRole::Keyword);
            }
            "True" | "False" | "None" => role = Some(CodeHighlightRole::Literal),
            "def" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Function);
            }
            _ => {}
        },
        CodeLanguage::Rust => match word {
            "if" | "else" | "return" | "break" | "continue" | "for" | "while" | "in" | "let"
            | "mut" | "const" | "static" | "struct" | "enum" | "impl" | "mod" | "use" | "pub"
            | "crate" | "super" | "self" | "match" | "async" | "await" | "move" | "ref"
            | "type" | "where" | "unsafe" | "extern" | "dyn" => {
                role = Some(CodeHighlightRole::Keyword);
            }
            "i8" | "i16" | "i32" | "i64" | "i128" | "u8" | "u16" | "u32" | "u64" | "u128"
            | "isize" | "usize" | "f32" | "f64" | "bool" | "char" | "str" => {
                role = Some(CodeHighlightRole::Type);
            }
            "true" | "false" => role = Some(CodeHighlightRole::Literal),
            "fn" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Function);
            }
            "trait" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Contract);
            }
            _ => {}
        },
        CodeLanguage::Shell => match word {
            "if" | "then" | "else" | "elif" | "fi" | "for" | "while" | "do" | "done" | "in" => {
                role = Some(CodeHighlightRole::Keyword)
            }
            "true" | "false" => role = Some(CodeHighlightRole::Literal),
            "function" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Function);
            }
            _ => {}
        },
        CodeLanguage::Toml => match word {
            "true" | "false" => role = Some(CodeHighlightRole::Literal),
            _ => {}
        },
        CodeLanguage::Json => match word {
            "true" | "false" | "null" => role = Some(CodeHighlightRole::Literal),
            _ => {}
        },
        CodeLanguage::Yaml => {
            if is_yaml_literal(word) {
                role = Some(CodeHighlightRole::Literal);
            }
        }
        CodeLanguage::Css => {}
        CodeLanguage::C => match word {
            "if" | "else" | "for" | "while" | "do" | "switch" | "case" | "default" | "break"
            | "continue" | "return" | "goto" | "sizeof" | "struct" | "union" | "enum"
            | "typedef" | "static" | "const" | "extern" | "volatile" | "register" | "signed"
            | "unsigned" | "long" | "short" | "inline" => {
                role = Some(CodeHighlightRole::Keyword);
            }
            "int" | "char" | "float" | "double" | "void" | "bool" | "size_t" | "ssize_t"
            | "int8_t" | "int16_t" | "int32_t" | "int64_t" | "uint8_t" | "uint16_t"
            | "uint32_t" | "uint64_t" => {
                role = Some(CodeHighlightRole::Type);
            }
            "true" | "false" | "NULL" => role = Some(CodeHighlightRole::Literal),
            _ => {}
        },
        CodeLanguage::Sql => role = sql_word_role(word),
        CodeLanguage::Generic
        | CodeLanguage::Text
        | CodeLanguage::Moth
        | CodeLanguage::Html
        | CodeLanguage::Markdown => {}
    }

    NonMothWordClass {
        role,
        next_identifier_role,
    }
}

/// True for YAML boolean and null scalar spellings, case-insensitively.
fn is_yaml_literal(word: &str) -> bool {
    ["true", "false", "yes", "no", "on", "off", "null"]
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate))
}

/// Classifies one SQL word case-insensitively into the shared roles.
fn sql_word_role(word: &str) -> Option<CodeHighlightRole> {
    const KEYWORDS: &[&str] = &[
        "select",
        "from",
        "where",
        "insert",
        "into",
        "values",
        "update",
        "set",
        "delete",
        "create",
        "table",
        "database",
        "index",
        "drop",
        "alter",
        "add",
        "column",
        "join",
        "inner",
        "left",
        "right",
        "full",
        "outer",
        "on",
        "group",
        "by",
        "order",
        "having",
        "limit",
        "offset",
        "and",
        "or",
        "not",
        "primary",
        "key",
        "foreign",
        "references",
        "unique",
        "default",
        "check",
        "constraint",
        "as",
        "distinct",
        "union",
        "all",
        "exists",
        "between",
        "like",
        "in",
        "is",
        "case",
        "when",
        "then",
        "else",
        "end",
        "begin",
        "commit",
        "rollback",
        "transaction",
    ];
    const TYPES: &[&str] = &[
        "int",
        "integer",
        "bigint",
        "smallint",
        "tinyint",
        "real",
        "float",
        "double",
        "numeric",
        "decimal",
        "text",
        "varchar",
        "char",
        "boolean",
        "date",
        "time",
        "timestamp",
        "blob",
    ];

    if ["true", "false", "null"]
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate))
    {
        return Some(CodeHighlightRole::Literal);
    }

    if TYPES
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate))
    {
        return Some(CodeHighlightRole::Type);
    }

    KEYWORDS
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate))
        .then_some(CodeHighlightRole::Keyword)
}
