//! Built-in `$code` template style support.
//!
//! This module owns both halves of the feature:
//! - parsing the narrow `$code` / `$code("ext")` directive syntax
//! - converting compile-time body string runs into highlighted HTML

use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterInput, FormatterInputPiece, FormatterOutput, FormatterOutputPiece,
};
use crate::compiler_frontend::ast::templates::template::{
    Formatter, FormatterResult, TemplateFormatter,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::style_directives::StyleDirectiveArgumentValue;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::utilities::basic::CharacterParsing;
use std::sync::Arc;
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
                    let highlighted = match self.language {
                        CodeLanguage::Text => text.to_owned(),
                        _ => highlight_code_html(text, self.language),
                    };

                    // Wrap the first text piece with the opening <code> tag.
                    let wrapped = if !first_text_emitted {
                        first_text_emitted = true;
                        format!("<code class='codeblock'>{highlighted}")
                    } else {
                        highlighted
                    };

                    output_pieces.push(FormatterOutputPiece::Text(wrapped));
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
        pre_format_whitespace_passes: Vec::new(),
        formatter: Arc::new(CodeTemplateFormatter { language }),
        post_format_whitespace_passes: Vec::new(),
    }
}

/// Converts raw source code into highlighted HTML markup.
///
/// WHAT:
/// - Strips shared formatting indentation from the code string.
/// - Iterates over characters to identify structural boundaries (strings, numbers, comments, symbols, keywords) without a full lexer pass.
/// - Wraps the matched runs in span classes for CSS styling.
/// - Operates on individual text pieces; opaque anchors between pieces are preserved structurally by the caller.
///
/// WHY:
/// - Provides simple syntax highlighting for documentation without the binary weight of a full parsing dependency like syn/tree-sitter.
/// - Gracefully falls back to plain text for unknown symbols via the `Generic` language profile.
pub(crate) fn highlight_code_html(source: &str, language: CodeLanguage) -> String {
    // Normalise indentation first so the highlighted output reflects the code the user
    // meant to show, not the template indentation needed to keep the source tidy.
    let normalized_source = dedent_code_block(source);
    let chars: Vec<char> = normalized_source.chars().collect();

    let mut highlighted = String::with_capacity(normalized_source.len() + 16);
    let mut word = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        let current = chars[index];

        // Comments are matched before operators, so prefixes like `//` and `--`
        // become a single comment run instead of two separate operator tokens.
        if matches_comment_prefix(&chars, index, language.comment_prefix()) {
            flush_word(&mut highlighted, &mut word, language);
            let prefix = language.comment_prefix().unwrap_or_default();
            highlighted.push_str("<span class='moth-code-comment'>");

            for comment_char in prefix.chars() {
                push_escaped_char(&mut highlighted, comment_char);
            }

            index += prefix.chars().count();

            while index < chars.len() && chars[index] != '\n' {
                push_escaped_char(&mut highlighted, chars[index]);
                index += 1;
            }

            highlighted.push_str("</span>");
            continue;
        }

        if current == '"' || current == '\'' {
            flush_word(&mut highlighted, &mut word, language);
            index = highlight_string(&chars, index, &mut highlighted);
            continue;
        }

        if starts_number_literal(&chars, index) {
            flush_word(&mut highlighted, &mut word, language);
            index = highlight_number_literal(&chars, index, &mut highlighted);
            continue;
        }

        if current.is_bracket() {
            flush_word(&mut highlighted, &mut word, language);
            highlighted.push_str("<span class='moth-code-parenthesis'>");
            push_escaped_char(&mut highlighted, current);
            highlighted.push_str("</span>");
            index += 1;
            continue;
        }

        if is_operator_char(current) {
            flush_word(&mut highlighted, &mut word, language);
            highlighted.push_str("<span class='moth-code-operator'>");
            push_escaped_char(&mut highlighted, current);
            highlighted.push_str("</span>");
            index += 1;
            continue;
        }

        if current.is_whitespace() {
            flush_word(&mut highlighted, &mut word, language);
            highlighted.push(current);
            index += 1;
            continue;
        }

        word.push(current);
        index += 1;
    }

    flush_word(&mut highlighted, &mut word, language);
    highlighted
}

/// Normalizes the minimum indentation of a block of text.
///
/// WHAT:
/// - Scans a block to find the smallest number of leading whitespace characters across all non-empty lines.
/// - Returns a new string with that exact minimum shared indentation stripped from the start of every line.
///
/// WHY:
/// - Template strings often inherit the indentation of their surrounding AST scope to keep the user's code visually tidy.
/// - Preserves the *relative* formatting of code structures within the block while removing the artificial absolute padding.
fn dedent_code_block(source: &str) -> String {
    let mut min_indent: Option<usize> = None;

    for line in source.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let indent = line
            .chars()
            .take_while(|ch| ch.is_non_newline_whitespace())
            .count();

        min_indent = Some(match min_indent {
            Some(existing) => existing.min(indent),
            None => indent,
        });
    }

    let Some(min_indent) = min_indent else {
        return source.to_string();
    };

    let mut dedented = String::with_capacity(source.len());

    for (line_index, line) in source.lines().enumerate() {
        if line_index > 0 {
            dedented.push('\n');
        }

        let mut chars = line.chars();
        let mut removed = 0usize;

        // The template formatter runs on body string slices, which often inherit the
        // surrounding template indentation. Strip the smallest shared indentation so
        // the rendered code keeps its intended relative structure instead of the AST
        // layout indentation.
        while removed < min_indent {
            match chars.next() {
                Some(ch) if ch.is_non_newline_whitespace() => removed += 1,
                Some(ch) => {
                    dedented.push(ch);
                    break;
                }
                None => break,
            }
        }

        dedented.extend(chars);
    }

    if source.ends_with('\n') {
        dedented.push('\n');
    }

    dedented
}

fn matches_comment_prefix(chars: &[char], index: usize, prefix: Option<&str>) -> bool {
    let Some(prefix) = prefix else {
        return false;
    };

    for (offset, expected) in prefix.chars().enumerate() {
        if chars.get(index + offset) != Some(&expected) {
            return false;
        }
    }

    true
}

fn starts_number_literal(chars: &[char], index: usize) -> bool {
    // Starting only on an actual digit avoids swallowing operators from compact
    // expressions like `1+2` into the numeric span.
    chars[index].is_numeric()
}

fn highlight_string(chars: &[char], mut index: usize, output: &mut String) -> usize {
    let quote = chars[index];
    output.push_str("<span class='moth-code-string'>");
    push_escaped_char(output, quote);
    index += 1;

    while index < chars.len() {
        let current = chars[index];
        push_escaped_char(output, current);
        index += 1;

        if current == '\\' && index < chars.len() {
            push_escaped_char(output, chars[index]);
            index += 1;
            continue;
        }

        if current == quote {
            break;
        }
    }

    output.push_str("</span>");
    index
}

fn highlight_number_literal(chars: &[char], mut index: usize, output: &mut String) -> usize {
    output.push_str("<span class='moth-code-number'>");

    // Keep this deliberately narrow for now. The generic highlighter is only
    // meant to recognise obvious numeric runs, not fully parse every literal form.
    while index < chars.len()
        && (chars[index].is_numeric() || chars[index] == '.' || chars[index] == '_')
    {
        push_escaped_char(output, chars[index]);
        index += 1;
    }

    output.push_str("</span>");
    index
}

fn flush_word(output: &mut String, word: &mut String, language: CodeLanguage) {
    if word.is_empty() {
        return;
    }

    let escaped = escape_html(word);

    // Bare identifier runs are classified after the scanner hits a boundary
    // such as whitespace or punctuation. That keeps keyword matching simple and
    // lets generic mode leave unknown identifiers untouched.
    if is_keyword(word, language) {
        output.push_str("<span class='moth-code-keyword'>");
        output.push_str(&escaped);
        output.push_str("</span>");
    } else if is_type_keyword(word, language) {
        output.push_str("<span class='moth-code-type'>");
        output.push_str(&escaped);
        output.push_str("</span>");
    } else if language != CodeLanguage::Generic
        && word.chars().next().is_some_and(|ch| ch.is_uppercase())
    {
        output.push_str("<span class='moth-code-struct'>");
        output.push_str(&escaped);
        output.push_str("</span>");
    } else {
        output.push_str(&escaped);
    }

    word.clear();
}

fn is_keyword(word: &str, language: CodeLanguage) -> bool {
    match language {
        CodeLanguage::Text => false,
        CodeLanguage::Moth => matches!(
            word,
            "if" | "else"
                | "return"
                | "break"
                | "continue"
                | "loop"
                | "in"
                | "to"
                | "by"
                | "as"
                | "copy"
        ),
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
        CodeLanguage::Generic | CodeLanguage::Text => false,
        CodeLanguage::Moth => {
            matches!(
                word,
                "Int" | "Float" | "Bool" | "String" | "None" | "True" | "False"
            )
        }
        CodeLanguage::JavaScript => matches!(word, "true" | "false" | "null" | "undefined"),
        CodeLanguage::TypeScript | CodeLanguage::Rust => matches!(
            word,
            "number"
                | "string"
                | "boolean"
                | "unknown"
                | "never"
                | "void"
                | "any"
                | "true"
                | "false"
                | "null"
                | "undefined"
        ),
        CodeLanguage::Python => matches!(word, "True" | "False" | "None"),
        CodeLanguage::Shell => matches!(word, "true" | "false"),
    }
}

fn is_operator_char(ch: char) -> bool {
    matches!(
        ch,
        '=' | ':'
            | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '^'
            | '!'
            | '?'
            | '|'
            | '&'
            | '<'
            | '>'
            | '~'
            | '@'
            | '#'
            | '$'
            | '`'
    )
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

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());

    for ch in text.chars() {
        push_escaped_char(&mut escaped, ch);
    }

    escaped
}
