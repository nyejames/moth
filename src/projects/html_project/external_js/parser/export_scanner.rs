//! Scans JavaScript source text for supported export declarations.
//!
//! WHAT: identifies callable and literal-backed `export const` declarations, counts plain
//!       JS parameters and provides shared JS lexical skipping used by import scanning.
//! WHY: the binder matches each annotation to the declaration's kind, while
//!      import scanning reuses this module's cursor and lexical boundaries.
//!      Keeping export scanning separate from comment extraction lets each stage stay focused
//!      and testable.
//!
//! Supported export forms:
//! - `export function jsName(param1, param2) { ... }`
//! - `export const jsName = (param1, param2) => { ... }`
//! - `export const jsName = <unsigned decimal integer literal>`
//!
//! Rejected forms:
//! - `export default ...`
//! - `export { name }` (local export-list syntax)
//! - `export { name } from "..."` / `export * from "..."` / `export * as ns from "..."` (re-exports)
//! - `export class ...`
//! - `module.exports = ...` (CommonJS)
//! - `exports.name = ...` (CommonJS)

use super::parsed_js_module::{
    JsDiagnosticKind, JsParserDiagnostic, JsSourceSpan, ParsedRuntimeImport,
};
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;

/// A single JS export discovered by the scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsExport {
    pub js_name: String,
    pub kind: JsExportKind,
    pub span: JsSourceSpan,
}

/// Retained declaration facts, not evaluated JavaScript values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsExportKind {
    Callable { parameter_count: usize },
    Constant { literal_span: Option<JsSourceSpan> },
}

/// Result of scanning a JS file for exports.
pub struct ExportScanResult {
    pub exports: Vec<JsExport>,
    pub runtime_imports: Vec<ParsedRuntimeImport>,
    pub diagnostics: Vec<JsParserDiagnostic>,
}

/// Scans source text for supported JS exports and reports unsupported forms.
///
/// WHAT: finds `export function` / `export const` declarations and validates
///       `import` statements against the provided runtime module registry.
/// WHY: one scanner cursor coordinates export recognition with the import policy
///      implemented in `import_scan`.
pub fn scan_exports(source: &str, registry: &RuntimeModuleRegistry) -> ExportScanResult {
    let mut scanner = ExportScanner::new(source, registry);
    scanner.scan()
}

/// Binding annotations attach across trivia only, not across a private statement or an
/// unsupported export. Reuse the scanner's lexical boundary instead of a second comment lexer.
pub(super) fn annotation_precedes_export(
    source: &str,
    registry: &RuntimeModuleRegistry,
    annotation_end: usize,
    export_start: usize,
) -> bool {
    let mut scanner = ExportScanner::new(source, registry);
    scanner.pos = annotation_end;
    scanner.skip_whitespace_and_comments() && scanner.pos == export_start
}

pub(super) struct ExportScanner<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pub(super) pos: usize,
    exports: Vec<JsExport>,
    pub(super) runtime_imports: Vec<ParsedRuntimeImport>,
    pub(super) diagnostics: Vec<JsParserDiagnostic>,
    pub(super) registry: &'a RuntimeModuleRegistry,
}

impl<'a> ExportScanner<'a> {
    pub(super) fn new(source: &'a str, registry: &'a RuntimeModuleRegistry) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            exports: Vec::new(),
            runtime_imports: Vec::new(),
            diagnostics: Vec::new(),
            registry,
        }
    }

    fn scan(&mut self) -> ExportScanResult {
        while !self.is_at_end() {
            if self.peek_str("//") {
                self.skip_line_comment();
            } else if self.peek_str("/*") {
                self.skip_block_comment();
            } else if matches!(self.current_char_opt(), Some('"') | Some('\'')) {
                self.skip_string_literal();
            } else if self.current_char_opt() == Some('`') {
                self.skip_template_literal();
            } else if self.peek_str("export") && self.is_word_boundary_around("export".len()) {
                self.read_export_statement();
            } else if self.peek_str("module.exports")
                && self.is_word_boundary_around("module.exports".len())
            {
                self.emit_diagnostic_at_current(
                    "CommonJS `module.exports` is not supported in Moth JS modules.",
                    JsDiagnosticKind::CommonJsExport,
                );
                self.skip_to_statement_end();
            } else if self.peek_str("exports.") && self.is_word_boundary_around("exports".len()) {
                self.emit_diagnostic_at_current(
                    "CommonJS `exports.name` is not supported in Moth JS modules.",
                    JsDiagnosticKind::CommonJsExport,
                );
                self.skip_to_statement_end();
            } else if self.read_module_loading_at_current() {
                continue;
            } else if self.current_char_opt() == Some('/')
                && !self.peek_str("//")
                && !self.peek_str("/*")
            {
                if self.slash_starts_regular_expression() {
                    self.skip_regular_expression();
                } else {
                    self.advance_char();
                }
            } else {
                self.advance_char();
            }
        }

        ExportScanResult {
            exports: std::mem::take(&mut self.exports),
            runtime_imports: std::mem::take(&mut self.runtime_imports),
            diagnostics: std::mem::take(&mut self.diagnostics),
        }
    }

    /// Every executable scanner context applies the same module-loading policy. Lexical
    /// lookalikes are skipped by callers before this dispatch, including inside initializers.
    fn read_module_loading_at_current(&mut self) -> bool {
        if self.peek_str("import") && self.is_word_boundary_around("import".len()) {
            if self
                .source
                .get(self.pos + "import".len()..)
                .is_some_and(|rest| rest.starts_with(".meta"))
            {
                self.advance_chars("import".len());
            } else {
                self.read_import_statement();
            }
            return true;
        }
        if self.peek_str("require") && self.is_word_boundary_around("require".len()) {
            self.read_require_statement();
            return true;
        }
        false
    }

    // ------------------------
    //  Export statement
    // ------------------------

    fn read_export_statement(&mut self) {
        let export_start_byte = self.pos;

        // Consume `export`
        self.advance_chars("export".len());
        self.skip_whitespace_and_comments();

        if self.consume_str("default") {
            let span = self.make_span(export_start_byte);
            self.emit_diagnostic(
                "Default exports are not supported in Moth JS modules.",
                JsDiagnosticKind::DefaultExport,
                span,
            );
            self.skip_to_statement_end();
            return;
        }

        if self.consume_char('{') {
            self.skip_export_list_to_close();
            let has_from_clause = self.has_from_module_specifier_ahead();
            let span = self.make_span(export_start_byte);
            let (message, kind) = if has_from_clause {
                (
                    "Re-export forms such as `export { name } from \"...\"` are not supported \
                     in Moth JS modules.",
                    JsDiagnosticKind::ReExportFrom,
                )
            } else {
                (
                    "Export-list forms such as `export { name }` are not supported in Moth JS \
                     modules.",
                    JsDiagnosticKind::ReExport,
                )
            };
            self.emit_diagnostic(message, kind, span);
            self.skip_to_statement_end();
            return;
        }

        if self.consume_char('*') {
            let has_from_clause = self.has_from_module_specifier_ahead();
            let span = self.make_span(export_start_byte);
            let (message, kind) = if has_from_clause {
                (
                    "Re-export forms such as `export * from \"...\"` are not supported in Moth JS \
                     modules.",
                    JsDiagnosticKind::ReExportFrom,
                )
            } else {
                (
                    "Star exports without a `from` module specifier are not supported in Moth JS \
                     modules.",
                    JsDiagnosticKind::ReExport,
                )
            };
            self.emit_diagnostic(message, kind, span);
            self.skip_to_statement_end();
            return;
        }

        if self.consume_str("function") && self.is_word_boundary_at(0) {
            self.skip_whitespace_and_comments();
            if let Some(js_name) = self.parse_identifier() {
                self.skip_whitespace_and_comments();
                if self.consume_char('(') {
                    let parameter_count = self.count_plain_parameters();
                    let span = self.make_span(export_start_byte);
                    self.exports.push(JsExport {
                        js_name,
                        kind: JsExportKind::Callable { parameter_count },
                        span,
                    });
                    self.skip_to_statement_end();
                    return;
                }
            }
            self.skip_to_statement_end();
            return;
        }

        if self.consume_str("const") && self.is_word_boundary_at(0) {
            self.skip_whitespace_and_comments();
            if let Some(js_name) = self.parse_identifier() {
                self.skip_whitespace_and_comments();
                if !self.consume_char('=') {
                    self.record_constant_export(js_name, export_start_byte, None);
                    self.emit_diagnostic_at_current(
                        "Literal-backed exports require an initializer.",
                        JsDiagnosticKind::InvalidConstant,
                    );
                    self.skip_to_statement_end();
                    return;
                }

                self.skip_whitespace_and_comments();
                let initializer_start = self.pos;
                if self.consume_char('(') {
                    let diagnostic_count = self.diagnostics.len();
                    let parameter_count = self.count_plain_parameters();
                    self.skip_whitespace_and_comments();
                    if self.consume_str("=>") {
                        self.skip_whitespace_and_comments();
                        if !self.consume_char('{') {
                            let span = self.make_span(export_start_byte);
                            self.emit_diagnostic(
                                "Expression-bodied arrow exports are not supported in Moth JS modules. \
                                 Use a block body `=> { ... }`.",
                                JsDiagnosticKind::ExpressionBodiedArrowExport,
                                span,
                            );
                            self.skip_to_statement_end();
                            return;
                        }
                        let span = self.make_span(export_start_byte);
                        self.exports.push(JsExport {
                            js_name,
                            kind: JsExportKind::Callable { parameter_count },
                            span,
                        });
                        self.skip_to_statement_end();
                        return;
                    }
                    // A grouped expression is not an arrow. Recover through the same constant
                    // path without retaining speculative parameter-pattern diagnostics.
                    self.pos = initializer_start;
                    self.diagnostics.truncate(diagnostic_count);
                }

                self.read_constant_export(js_name, export_start_byte);
                return;
            }
            self.emit_diagnostic_at_current(
                "Literal-backed exports require one plain JS identifier.",
                JsDiagnosticKind::InvalidConstant,
            );
            self.skip_to_statement_end();
            return;
        }

        if self.consume_str("class") && self.is_word_boundary_at(0) {
            let span = self.make_span(export_start_byte);
            self.emit_diagnostic(
                "Class exports are not supported in Moth JS modules.",
                JsDiagnosticKind::ClassExport,
                span,
            );
            self.skip_to_statement_end();
            return;
        }

        // Unknown export form; skip it
        self.skip_to_statement_end();
    }

    /// Retain the authored token without interpreting its number. The binder owns numeric
    /// grammar and U32 materialisation; this scanner only establishes the declaration boundary.
    fn read_constant_export(&mut self, js_name: String, export_start: usize) {
        let literal_start = self.pos;
        if self.is_at_end()
            || self.current_char_opt() == Some(';')
            || (self.peek_str("export") && self.is_word_boundary_around("export".len()))
            || (self.peek_str("import") && self.is_word_boundary_around("import".len()))
        {
            self.record_constant_export(js_name, export_start, None);
            self.emit_diagnostic_at_current(
                "Literal-backed exports require an unsigned decimal integer initializer.",
                JsDiagnosticKind::InvalidConstant,
            );
            self.consume_char(';');
            return;
        }

        while !self.is_at_end() {
            let character = self.current_char();
            if character.is_whitespace()
                || matches!(character, ';' | ',')
                || self.peek_str("//")
                || self.peek_str("/*")
            {
                break;
            }
            if self.skip_lexical_content_at_current() {
                continue;
            }
            if character == '/' && self.slash_starts_regular_expression() {
                self.skip_regular_expression();
                continue;
            }
            if self.read_module_loading_at_current() {
                // The import/require owner may recover through the terminator. Stop this atom
                // here so it cannot absorb the next declaration after that recovery.
                break;
            }
            self.advance_char();
        }
        let literal_span = JsSourceSpan::range(literal_start, self.pos);
        let literal_end = self.pos;
        let has_closed_trivia = self.skip_whitespace_and_comments();
        let has_line_break = self.source[literal_end..self.pos]
            .chars()
            .any(is_js_line_terminator);
        let at_end = self.is_at_end();
        let has_semicolon = self.current_char_opt() == Some(';');
        let next_continues_expression = self.current_char_opt().is_some_and(|character| {
            matches!(
                character,
                '+' | '-'
                    | '*'
                    | '/'
                    | '%'
                    | '&'
                    | '|'
                    | '^'
                    | '<'
                    | '>'
                    | '='
                    | '?'
                    | '.'
                    | '('
                    | '['
                    | '`'
                    | ','
            )
        }) || ["in", "instanceof"]
            .iter()
            .any(|keyword| self.peek_str(keyword) && self.is_word_boundary_around(keyword.len()));
        let starts_prefix_update = self.peek_str("++") || self.peek_str("--");

        let has_boundary = has_closed_trivia
            && (at_end
                || has_semicolon
                || (has_line_break && (!next_continues_expression || starts_prefix_update)));
        if has_boundary {
            // Leave trivia before the next statement for the outer scanner. A semicolon, unlike
            // a newline, explicitly closes the declaration even before a continuation token.
            if has_semicolon {
                self.advance_char();
            } else {
                self.pos = literal_end;
            }
            self.record_constant_export(js_name, export_start, Some(literal_span));
        } else {
            self.emit_diagnostic(
                "Literal-backed exports require one unsigned decimal integer literal and closed \
                 comments, not multiple declarators or expressions.",
                JsDiagnosticKind::InvalidConstant,
                JsSourceSpan::range(literal_start, self.pos),
            );
            self.record_constant_export(js_name, export_start, None);
            self.skip_to_statement_end();
        }
    }

    fn record_constant_export(
        &mut self,
        js_name: String,
        export_start: usize,
        literal_span: Option<JsSourceSpan>,
    ) {
        self.exports.push(JsExport {
            js_name,
            kind: JsExportKind::Constant { literal_span },
            span: self.make_span(export_start),
        });
    }

    /// Looks for a `from "..."` clause after an export list or star, including across newlines.
    ///
    /// Automatic semicolon insertion would otherwise end the export statement before a following
    /// `from` specifier. A star re-export may also bind a namespace with `as <ident>` before
    /// `from`. The cursor and any interpolation diagnostics are restored so the later statement
    /// skip remains the sole consumer of the remaining source.
    fn has_from_module_specifier_ahead(&mut self) -> bool {
        let original_pos = self.pos;
        let original_diagnostic_count = self.diagnostics.len();
        let original_runtime_import_count = self.runtime_imports.len();

        self.skip_whitespace_and_comments();
        if self.peek_str("as") && self.is_word_boundary_around("as".len()) {
            self.advance_chars("as".len());
            self.skip_whitespace_and_comments();
            if self.parse_identifier().is_none() {
                self.pos = original_pos;
                self.diagnostics.truncate(original_diagnostic_count);
                self.runtime_imports.truncate(original_runtime_import_count);
                return false;
            }
            self.skip_whitespace_and_comments();
        }

        let found = if self.peek_str("from") && self.is_word_boundary_around("from".len()) {
            self.advance_chars("from".len());
            self.skip_whitespace_and_comments();
            matches!(self.current_char_opt(), Some('"') | Some('\''))
        } else {
            false
        };

        self.pos = original_pos;
        self.diagnostics.truncate(original_diagnostic_count);
        self.runtime_imports.truncate(original_runtime_import_count);
        found
    }

    fn skip_export_list_to_close(&mut self) {
        let mut nested_brace_depth = 0usize;

        while !self.is_at_end() {
            if self.skip_lexical_content_at_current() {
                continue;
            }

            match self.current_char() {
                '{' => {
                    nested_brace_depth += 1;
                    self.advance_char();
                }
                '}' => {
                    self.advance_char();
                    if nested_brace_depth == 0 {
                        return;
                    }
                    nested_brace_depth -= 1;
                }
                _ => self.advance_char(),
            }
        }
    }

    pub(super) fn slash_starts_regular_expression(&self) -> bool {
        let prefix = self.source[..self.pos].trim_end();
        let Some(previous) = prefix.chars().next_back() else {
            return true;
        };

        if matches!(
            previous,
            '(' | '['
                | '{'
                | ','
                | ';'
                | '='
                | '!'
                | '?'
                | ':'
                | '&'
                | '|'
                | '~'
                | '^'
                | '%'
                | '*'
                | '<'
                | '>'
        ) {
            return true;
        }

        if !is_identifier_continuation(previous) {
            return false;
        }

        let word_start = prefix
            .char_indices()
            .rev()
            .find(|(_, character)| !is_identifier_continuation(*character))
            .map(|(index, character)| index + character.len_utf8())
            .unwrap_or(0);

        if word_start > 0 && prefix[..word_start].ends_with('.') {
            return false;
        }
        matches!(
            &prefix[word_start..],
            "return"
                | "throw"
                | "case"
                | "else"
                | "do"
                | "in"
                | "of"
                | "typeof"
                | "void"
                | "delete"
                | "new"
                | "await"
                | "yield"
        )
    }

    pub(super) fn skip_regular_expression(&mut self) {
        self.advance_char();
        let mut in_character_class = false;

        while !self.is_at_end() {
            let character = self.current_char();
            if character == '\\' {
                self.advance_char();
                self.advance_char();
                continue;
            }
            if character == '[' {
                in_character_class = true;
            } else if character == ']' {
                in_character_class = false;
            } else if character == '/' && !in_character_class {
                self.advance_char();
                while !self.is_at_end() && self.current_char().is_ascii_alphabetic() {
                    self.advance_char();
                }
                return;
            }
            self.advance_char();
        }
    }

    // ------------------------
    //  Parameter counting
    // ------------------------

    /// Counts plain parameters inside `(...)` for arity checking.
    ///
    /// WHAT: walks the parameter list and counts top-level comma-separated items,
    ///       while detecting rest `...`, destructuring `{`/`[`, and default values `=`.
    ///
    /// The caller must have already consumed the opening `(`.
    fn count_plain_parameters(&mut self) -> usize {
        let mut count = 0;
        let mut depth = 1; // we are inside `(...)`
        let mut in_default_value = false;

        while !self.is_at_end() && depth > 0 {
            if self.peek_str("export") && self.is_word_boundary_around("export".len()) {
                self.emit_diagnostic_at_current(
                    "A JS parameter list cannot contain another export declaration.",
                    JsDiagnosticKind::UnsupportedParameterPattern,
                );
                break;
            }
            if self.skip_lexical_content_at_current() {
                continue;
            }

            let ch = self.current_char();

            if ch == '(' {
                depth += 1;
                self.advance_char();
                continue;
            }

            if ch == ')' {
                depth -= 1;
                if depth == 0 {
                    self.advance_char();
                    break;
                }
                self.advance_char();
                continue;
            }

            if ch == '{' || ch == '[' {
                self.emit_diagnostic_at_current(
                    "Destructuring parameters are not supported in Moth JS module signatures.",
                    JsDiagnosticKind::UnsupportedParameterPattern,
                );
                self.skip_balanced_braces_and_parens();
                continue;
            }

            if self.peek_str("...") {
                self.emit_diagnostic_at_current(
                    "Rest parameters are not supported in Moth JS module signatures.",
                    JsDiagnosticKind::UnsupportedParameterPattern,
                );
                self.skip_to_char(')');
                continue;
            }

            // Look for an identifier at depth 1, but not while recovering inside a default value.
            let parameter_name = if depth == 1 && !in_default_value {
                self.parse_identifier()
            } else {
                None
            };

            if let Some(name) = parameter_name {
                if !name.is_empty() {
                    count += 1;
                }
                self.skip_whitespace();
                if self.consume_char('=') {
                    self.emit_diagnostic_at_current(
                        "Default parameters are not supported in Moth JS module signatures.",
                        JsDiagnosticKind::UnsupportedParameterPattern,
                    );
                    in_default_value = true;
                }
                if self.consume_char('?') {
                    self.emit_diagnostic_at_current(
                        "Optional parameters are not supported in Moth JS module signatures.",
                        JsDiagnosticKind::UnsupportedParameterPattern,
                    );
                }
                continue;
            }

            if ch == ',' && depth == 1 {
                in_default_value = false;
            }

            self.advance_char();
        }

        count
    }

    // ------------------------
    //  Helpers
    // ------------------------

    pub(super) fn parse_identifier(&mut self) -> Option<String> {
        self.skip_whitespace();

        let mut name = String::new();
        let ch = self.current_char_opt()?;
        if ch.is_alphabetic() || ch == '_' || ch == '$' {
            name.push(ch);
            self.advance_char();
        } else {
            return None;
        }

        while let Some(ch) = self.current_char_opt() {
            if is_identifier_continuation(ch) {
                name.push(ch);
                self.advance_char();
            } else {
                break;
            }
        }

        Some(name)
    }

    pub(super) fn skip_to_statement_end(&mut self) {
        let mut brace_depth = 0;
        while !self.is_at_end() {
            if self.skip_lexical_content_at_current() {
                continue;
            }
            if brace_depth == 0
                && self.peek_str("export")
                && self.is_word_boundary_around("export".len())
            {
                return;
            }

            if self.read_module_loading_at_current() {
                continue;
            }

            let ch = self.current_char();
            if ch == '{' {
                brace_depth += 1;
            } else if ch == '}' {
                if brace_depth == 0 {
                    self.advance_char();
                    return;
                }
                brace_depth -= 1;
            } else if ch == ';' && brace_depth == 0 {
                self.advance_char();
                return;
            } else if is_js_line_terminator(ch) && brace_depth == 0 {
                return;
            }
            self.advance_char();
        }
    }

    fn skip_to_char(&mut self, target: char) {
        while !self.is_at_end() && self.current_char() != target {
            if self.skip_lexical_content_at_current() {
                continue;
            }
            self.advance_char();
        }
    }

    /// Skips source text that must not be interpreted as scanner-level syntax.
    ///
    /// WHAT: consumes comments and string/template literals at the current cursor.
    /// WHY: statement and body scanners share this lexical boundary so `import`,
    ///      `export`, braces, or separators inside values do not affect top-level scanning.
    pub(super) fn skip_lexical_content_at_current(&mut self) -> bool {
        if self.peek_str("//") {
            self.skip_line_comment();
            return true;
        }
        if self.peek_str("/*") {
            self.skip_block_comment();
            return true;
        }
        if matches!(self.current_char_opt(), Some('"') | Some('\'')) {
            self.skip_string_literal();
            return true;
        }
        if self.current_char_opt() == Some('`') {
            self.skip_template_literal();
            return true;
        }

        false
    }

    fn skip_line_comment(&mut self) {
        self.advance_chars(2);
        while !self.is_at_end() && !is_js_line_terminator(self.current_char()) {
            self.advance_char();
        }
    }

    fn skip_block_comment(&mut self) -> bool {
        self.advance_chars(2);
        while !self.is_at_end() {
            if self.peek_str("*/") {
                self.advance_chars(2);
                return true;
            }
            self.advance_char();
        }
        false
    }

    fn skip_string_literal(&mut self) {
        let Some(quote) = self.current_char_opt() else {
            return;
        };
        self.advance_char();
        while !self.is_at_end() {
            let ch = self.current_char();
            self.advance_char();
            if ch == '\\' {
                self.advance_char();
                continue;
            }
            if ch == quote {
                return;
            }
        }
    }

    fn skip_template_literal(&mut self) {
        self.advance_char(); // skip opening `
        while !self.is_at_end() {
            let ch = self.current_char();

            if ch == '\\' {
                self.advance_char();
                self.advance_char();
                continue;
            }

            if ch == '$' && self.peek_str("${") {
                self.advance_chars(2); // skip ${
                let mut depth = 1;
                while !self.is_at_end() && depth > 0 {
                    let inner = self.current_char();
                    if inner == '{' {
                        depth += 1;
                    } else if inner == '}' {
                        depth -= 1;
                    } else if matches!(inner, '"' | '\'') {
                        self.skip_string_literal();
                        continue;
                    } else if inner == '`' {
                        self.skip_template_literal();
                        continue;
                    } else if self.peek_str("//") {
                        self.skip_line_comment();
                        continue;
                    } else if self.peek_str("/*") {
                        self.skip_block_comment();
                        continue;
                    } else if self.read_module_loading_at_current() {
                        continue;
                    }
                    self.advance_char();
                }
                continue;
            }

            self.advance_char();
            if ch == '`' {
                return;
            }
        }
    }

    fn skip_balanced_braces_and_parens(&mut self) {
        let mut depth = 0;
        while !self.is_at_end() {
            if self.skip_lexical_content_at_current() {
                continue;
            }

            let ch = self.current_char();
            if ch == '(' || ch == '{' || ch == '[' {
                depth += 1;
            } else if ch == ')' || ch == '}' || ch == ']' {
                if depth == 0 {
                    return;
                }
                depth -= 1;
                if depth == 0 {
                    self.advance_char();
                    return;
                }
            }
            self.advance_char();
        }
    }

    pub(super) fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char_opt() {
            if ch.is_whitespace() {
                self.advance_char();
            } else {
                break;
            }
        }
    }

    pub(super) fn skip_whitespace_and_comments(&mut self) -> bool {
        loop {
            self.skip_whitespace();
            if self.peek_str("//") {
                self.skip_line_comment();
                continue;
            }
            if self.peek_str("/*") {
                if !self.skip_block_comment() {
                    return false;
                }
                continue;
            }
            return true;
        }
    }

    pub(super) fn consume_char(&mut self, expected: char) -> bool {
        if self.current_char_opt() == Some(expected) {
            self.advance_char();
            true
        } else {
            false
        }
    }

    fn consume_str(&mut self, s: &str) -> bool {
        if self.peek_str(s) {
            self.advance_chars(s.len());
            true
        } else {
            false
        }
    }

    pub(super) fn peek_str(&self, s: &str) -> bool {
        self.source[self.pos..].starts_with(s)
    }

    fn is_word_boundary_at(&self, offset: usize) -> bool {
        let next_pos = self.pos + offset;
        if next_pos >= self.bytes.len() {
            return true;
        }
        let next_ch = self.source[next_pos..]
            .chars()
            .next()
            .expect("word-boundary offset is inside source");
        !is_identifier_continuation(next_ch)
    }

    pub(super) fn is_word_boundary_around(&self, candidate_len: usize) -> bool {
        let before_boundary = self
            .source
            .get(..self.pos)
            .and_then(|prefix| prefix.chars().next_back())
            .is_none_or(|ch| !is_identifier_continuation(ch) && ch != '.');
        before_boundary && self.is_word_boundary_at(candidate_len)
    }

    /// Current UTF-8 character; callers must have checked `is_at_end`.
    pub(super) fn current_char(&self) -> char {
        self.current_char_opt().expect("scanner is not at end")
    }

    pub(super) fn current_char_opt(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    pub(super) fn is_at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    pub(super) fn advance_char(&mut self) {
        if self.is_at_end() {
            return;
        }
        let ch = self.current_char();
        self.pos += ch.len_utf8();
    }

    pub(super) fn advance_chars(&mut self, count: usize) {
        for _ in 0..count {
            self.advance_char();
        }
    }

    fn make_span(&self, start_byte: usize) -> JsSourceSpan {
        JsSourceSpan::range(start_byte, self.pos)
    }

    fn emit_diagnostic_at_current(&mut self, message: impl Into<String>, kind: JsDiagnosticKind) {
        let span = JsSourceSpan::at(self.pos);
        self.diagnostics.push(JsParserDiagnostic {
            message: message.into(),
            span,
            kind,
        });
    }

    pub(super) fn emit_diagnostic(
        &mut self,
        message: impl Into<String>,
        kind: JsDiagnosticKind,
        span: JsSourceSpan,
    ) {
        self.diagnostics.push(JsParserDiagnostic {
            message: message.into(),
            span,
            kind,
        });
    }
}

fn is_identifier_continuation(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$'
}

fn is_js_line_terminator(character: char) -> bool {
    matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}
