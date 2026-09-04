//! Scans JavaScript source text for supported export declarations.
//!
//! WHAT: identifies `export function name(...)` and `export const name = (...) => { ... }`
//!       patterns, counts plain JS parameters, and rejects unsupported export forms.
//! WHY: the binder needs a list of JS exports to match against `@moth.sig` annotations.
//!      Keeping export scanning separate from comment extraction lets each stage stay
//!      focused and testable.
//!
//! Supported export forms:
//! - `export function jsName(param1, param2) { ... }`
//! - `export const jsName = (param1, param2) => { ... }`
//!
//! Rejected forms:
//! - `export default ...`
//! - `export { name }` (re-exports)
//! - `export class ...`
//! - `module.exports = ...` (CommonJS)
//! - `exports.name = ...` (CommonJS)
//! - `export const name = value` where value is not an arrow function

use super::parsed_js_module::{
    JsDiagnosticKind, JsParserDiagnostic, JsSourceSpan, ParsedRuntimeImport,
};
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;

/// Classification of a scanned JS export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsExportKind {
    Function,
    ConstArrow,
}

/// A single JS export discovered by the scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsExport {
    pub js_name: String,
    pub kind: JsExportKind,
    pub parameter_count: usize,
    pub span: JsSourceSpan,
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
/// WHY: the caller decides which JS runtime modules are registered; the scanner
///      stays agnostic to v1 vs later registry shapes.
pub fn scan_exports(source: &str, registry: &RuntimeModuleRegistry) -> ExportScanResult {
    let mut scanner = ExportScanner::new(source, registry);
    scanner.scan()
}

struct ExportScanner<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    column: usize,
    exports: Vec<JsExport>,
    runtime_imports: Vec<ParsedRuntimeImport>,
    diagnostics: Vec<JsParserDiagnostic>,
    registry: &'a RuntimeModuleRegistry,
}

impl<'a> ExportScanner<'a> {
    fn new(source: &'a str, registry: &'a RuntimeModuleRegistry) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            line: 1,
            column: 1,
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
            } else if self.peek_str("export") && self.is_word_boundary_at("export".len()) {
                self.read_export_statement();
            } else if self.peek_str("module.exports")
                && self.is_word_boundary_at("module.exports".len())
            {
                self.emit_diagnostic_at_current(
                    "CommonJS `module.exports` is not supported in Moth JS modules.",
                    JsDiagnosticKind::CommonJsExport,
                );
                self.skip_to_statement_end();
            } else if self.peek_str("exports.") {
                self.emit_diagnostic_at_current(
                    "CommonJS `exports.name` is not supported in Moth JS modules.",
                    JsDiagnosticKind::CommonJsExport,
                );
                self.skip_to_statement_end();
            } else if self.peek_str("import") && self.is_word_boundary_at("import".len()) {
                if self
                    .source
                    .get(self.pos + "import".len()..)
                    .is_some_and(|rest| rest.starts_with(".meta"))
                {
                    self.advance_chars("import".len());
                } else {
                    self.read_import_statement();
                }
            } else if self.peek_str("require") && self.is_word_boundary_at("require".len()) {
                self.read_require_statement();
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

    // ------------------------
    //  Export statement
    // ------------------------

    fn read_export_statement(&mut self) {
        let export_start_byte = self.pos;
        let export_start_line = self.line;
        let export_start_column = self.column;

        // Consume `export`
        self.advance_chars("export".len());
        self.skip_whitespace();

        if self.consume_str("default") {
            let span = self.make_span(export_start_byte, export_start_line, export_start_column);
            self.emit_diagnostic(
                "Default exports are not supported in Moth JS modules.",
                JsDiagnosticKind::DefaultExport,
                span,
            );
            self.skip_to_statement_end();
            return;
        }

        if self.consume_char('{') {
            let span = self.make_span(export_start_byte, export_start_line, export_start_column);
            self.emit_diagnostic(
                "Re-export forms such as `export { name }` are not supported in Moth JS modules.",
                JsDiagnosticKind::ReExport,
                span,
            );
            self.skip_to_statement_end();
            return;
        }

        if self.consume_char('*') {
            let span = self.make_span(export_start_byte, export_start_line, export_start_column);
            self.emit_diagnostic(
                "Re-export forms such as `export * from \"...\"` are not supported in Moth JS modules.",
                JsDiagnosticKind::ReExport,
                span,
            );
            self.skip_to_statement_end();
            return;
        }

        if self.consume_str("function") && self.is_word_boundary_at(0) {
            self.skip_whitespace();
            if let Some(js_name) = self.parse_identifier() {
                self.skip_whitespace();
                if self.consume_char('(') {
                    let parameter_count = self.count_plain_parameters();
                    let span =
                        self.make_span(export_start_byte, export_start_line, export_start_column);
                    self.exports.push(JsExport {
                        js_name,
                        kind: JsExportKind::Function,
                        parameter_count,
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
            self.skip_whitespace();
            if let Some(js_name) = self.parse_identifier() {
                self.skip_whitespace();
                if self.consume_char('=') {
                    self.skip_whitespace();
                    if self.consume_char('(') {
                        let parameter_count = self.count_plain_parameters();
                        self.skip_whitespace();
                        if !self.consume_str("=>") {
                            let span = self.make_span(
                                export_start_byte,
                                export_start_line,
                                export_start_column,
                            );
                            self.emit_diagnostic(
                                "`export const` must be bound to an arrow function in Moth JS modules.",
                                JsDiagnosticKind::UnsupportedParameterPattern,
                                span,
                            );
                            self.skip_to_statement_end();
                            return;
                        }
                        self.skip_whitespace();
                        if !self.consume_char('{') {
                            let span = self.make_span(
                                export_start_byte,
                                export_start_line,
                                export_start_column,
                            );
                            self.emit_diagnostic(
                                "Expression-bodied arrow exports are not supported in Moth JS modules. \
                                 Use a block body `=> { ... }`.",
                                JsDiagnosticKind::ExpressionBodiedArrowExport,
                                span,
                            );
                            self.skip_to_statement_end();
                            return;
                        }
                        let span = self.make_span(
                            export_start_byte,
                            export_start_line,
                            export_start_column,
                        );
                        self.exports.push(JsExport {
                            js_name,
                            kind: JsExportKind::ConstArrow,
                            parameter_count,
                            span,
                        });
                        self.skip_to_statement_end();
                        return;
                    } else {
                        // `export const name = value` where value is not an arrow function
                        let span = self.make_span(
                            export_start_byte,
                            export_start_line,
                            export_start_column,
                        );
                        self.emit_diagnostic(
                            "`export const` must be bound to an arrow function in Moth JS modules.",
                            JsDiagnosticKind::UnsupportedParameterPattern,
                            span,
                        );
                        self.skip_to_statement_end();
                        return;
                    }
                }
            }
            self.skip_to_statement_end();
            return;
        }

        if self.consume_str("class") && self.is_word_boundary_at(0) {
            let span = self.make_span(export_start_byte, export_start_line, export_start_column);
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

    // ------------------------
    //  Import statement scanning
    // ------------------------

    fn read_import_statement(&mut self) {
        let import_start_byte = self.pos;
        let import_start_line = self.line;
        let import_start_column = self.column;

        self.advance_chars("import".len());
        self.skip_whitespace();

        // Dynamic import: `import(...)`
        if self.consume_char('(') {
            self.emit_diagnostic(
                "Dynamic `import()` is not supported in Moth JS modules.",
                JsDiagnosticKind::DynamicImport,
                JsSourceSpan::range(
                    import_start_byte,
                    self.pos,
                    import_start_line,
                    import_start_column,
                ),
            );
            self.skip_to_statement_end();
            return;
        }

        let statement_end = self.find_statement_end_byte();
        let statement = &self.source[self.pos..statement_end];
        let specifier = extract_static_import_specifier(statement);

        let Some(specifier) = specifier else {
            self.emit_diagnostic(
                "JavaScript static import is not supported in Moth JS module files yet. \
                 Only registered Moth core runtime modules are supported.",
                JsDiagnosticKind::ArbitraryImport,
                JsSourceSpan::range(
                    import_start_byte,
                    self.pos,
                    import_start_line,
                    import_start_column,
                ),
            );
            self.advance_to_byte(statement_end);
            self.skip_to_statement_end();
            return;
        };

        if self.registry.is_registered(&specifier) {
            let span = JsSourceSpan::range(
                import_start_byte,
                statement_end,
                import_start_line,
                import_start_column,
            );

            match parse_named_import_names(statement) {
                Ok(names) if !names.is_empty() => {
                    let mut valid_names = Vec::new();
                    let mut has_unknown = false;

                    for name in names {
                        if self.registry.is_exported_name(&specifier, &name) {
                            valid_names.push(name);
                        } else {
                            has_unknown = true;
                            self.emit_diagnostic(
                                format!(
                                    "Unknown runtime import name `{name}` from module `{specifier}`."
                                ),
                                JsDiagnosticKind::UnknownRuntimeImportName,
                                span.clone(),
                            );
                        }
                    }

                    if !has_unknown {
                        self.runtime_imports.push(ParsedRuntimeImport {
                            module_name: specifier,
                            imported_names: valid_names,
                            span,
                        });
                    }
                }
                _ => {
                    self.emit_diagnostic(
                        "Unsupported runtime import form. Only named static imports such as \
                         `import { mothOk, mothErr } from \"@moth/runtime\";` are supported.",
                        JsDiagnosticKind::UnsupportedRuntimeImportForm,
                        span,
                    );
                }
            }
        } else {
            self.emit_diagnostic(
                format!(
                    "JavaScript import `{specifier}` is not supported in Moth JS module files yet. \
                     Only registered Moth core runtime modules are supported."
                ),
                JsDiagnosticKind::ArbitraryImport,
                JsSourceSpan::range(
                    import_start_byte,
                    self.pos,
                    import_start_line,
                    import_start_column,
                ),
            );
        }

        self.advance_to_byte(statement_end);
        self.skip_to_statement_end();
    }

    fn read_require_statement(&mut self) {
        let require_start_byte = self.pos;
        let require_start_line = self.line;
        let require_start_column = self.column;

        self.advance_chars("require".len());
        self.skip_whitespace_and_comments();

        if !self.consume_char('(') {
            return;
        }

        self.emit_diagnostic(
            "CommonJS `require()` is not supported in Moth JS modules.",
            JsDiagnosticKind::CommonJsExport,
            JsSourceSpan::range(
                require_start_byte,
                self.pos,
                require_start_line,
                require_start_column,
            ),
        );
        self.skip_to_statement_end();
    }

    fn slash_starts_regular_expression(&self) -> bool {
        match self.source[..self.pos]
            .chars()
            .rev()
            .find(|character| !character.is_whitespace())
        {
            None => true,
            Some(character) => matches!(
                character,
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
            ),
        }
    }

    fn skip_regular_expression(&mut self) {
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

    fn parse_identifier(&mut self) -> Option<String> {
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
            if ch.is_alphanumeric() || ch == '_' || ch == '$' {
                name.push(ch);
                self.advance_char();
            } else {
                break;
            }
        }

        Some(name)
    }

    fn find_statement_end_byte(&self) -> usize {
        let mut index = self.pos;
        let mut paren_depth: usize = 0;
        let mut brace_depth: usize = 0;
        let mut bracket_depth: usize = 0;

        while index < self.bytes.len() {
            let ch = self.source[index..].chars().next().unwrap_or('\0');

            // Skip quoted strings so their contents cannot break statement boundaries.
            if ch == '"' || ch == '\'' {
                index += ch.len_utf8();
                while index < self.bytes.len() {
                    let inner = self.source[index..].chars().next().unwrap_or('\0');
                    index += inner.len_utf8();
                    if inner == '\\' && index < self.bytes.len() {
                        index += self.source[index..]
                            .chars()
                            .next()
                            .unwrap_or('\0')
                            .len_utf8();
                        continue;
                    }
                    if inner == ch {
                        break;
                    }
                }
                continue;
            }

            // Skip template literals, including ${...} interpolations.
            if ch == '`' {
                index += ch.len_utf8();
                while index < self.bytes.len() {
                    let inner = self.source[index..].chars().next().unwrap_or('\0');
                    index += inner.len_utf8();
                    if inner == '\\' && index < self.bytes.len() {
                        index += self.source[index..]
                            .chars()
                            .next()
                            .unwrap_or('\0')
                            .len_utf8();
                        continue;
                    }
                    if inner == '`' {
                        break;
                    }
                    if inner == '$'
                        && index < self.bytes.len()
                        && self.source[index..].starts_with('{')
                    {
                        index += 1; // skip '{'
                        let mut depth = 1;
                        while index < self.bytes.len() && depth > 0 {
                            let tch = self.source[index..].chars().next().unwrap_or('\0');
                            index += tch.len_utf8();
                            if tch == '{' {
                                depth += 1;
                            } else if tch == '}' {
                                depth -= 1;
                            }
                        }
                    }
                }
                continue;
            }

            // Skip line comments.
            if self.source[index..].starts_with("//") {
                index += 2;
                while index < self.bytes.len() && !self.source[index..].starts_with('\n') {
                    index += self.source[index..]
                        .chars()
                        .next()
                        .unwrap_or('\0')
                        .len_utf8();
                }
                continue;
            }

            // Skip block comments.
            if self.source[index..].starts_with("/*") {
                index += 2;
                while index + 1 < self.bytes.len() && !self.source[index..].starts_with("*/") {
                    index += self.source[index..]
                        .chars()
                        .next()
                        .unwrap_or('\0')
                        .len_utf8();
                }
                if index + 1 < self.bytes.len() {
                    index += 2;
                }
                continue;
            }

            match ch {
                '(' => paren_depth += 1,
                ')' => paren_depth = paren_depth.saturating_sub(1),
                '{' => brace_depth += 1,
                '}' => brace_depth = brace_depth.saturating_sub(1),
                '[' => bracket_depth += 1,
                ']' => bracket_depth = bracket_depth.saturating_sub(1),
                ';' if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => {
                    index += ch.len_utf8();
                    break;
                }
                '\n' if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => {
                    break;
                }
                _ => {}
            }

            index += ch.len_utf8();
        }

        index
    }

    fn skip_to_statement_end(&mut self) {
        let mut brace_depth = 0;
        while !self.is_at_end() {
            if self.skip_lexical_content_at_current() {
                continue;
            }

            if self.peek_str("import") && self.is_word_boundary_at("import".len()) {
                if self
                    .source
                    .get(self.pos + "import".len()..)
                    .is_some_and(|rest| rest.starts_with(".meta"))
                {
                    self.advance_chars("import".len());
                } else {
                    self.read_import_statement();
                }
                continue;
            }

            if self.peek_str("require") && self.is_word_boundary_at("require".len()) {
                self.read_require_statement();
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
            } else if ch == '\n' && brace_depth == 0 {
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
    fn skip_lexical_content_at_current(&mut self) -> bool {
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
        while !self.is_at_end() && self.current_char() != '\n' {
            self.advance_char();
        }
    }

    fn skip_block_comment(&mut self) {
        self.advance_chars(2);
        while !self.is_at_end() {
            if self.peek_str("*/") {
                self.advance_chars(2);
                return;
            }
            self.advance_char();
        }
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
                    } else if self.peek_str("import") && self.is_word_boundary_at("import".len()) {
                        if self
                            .source
                            .get(self.pos + "import".len()..)
                            .is_some_and(|rest| rest.starts_with(".meta"))
                        {
                            self.advance_chars("import".len());
                        } else {
                            self.read_import_statement();
                        }
                        continue;
                    } else if self.peek_str("require") && self.is_word_boundary_at("require".len())
                    {
                        self.read_require_statement();
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

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char_opt() {
            if ch.is_whitespace() {
                self.advance_char();
            } else {
                break;
            }
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            self.skip_whitespace();
            if self.peek_str("//") {
                self.skip_line_comment();
                continue;
            }
            if self.peek_str("/*") {
                self.skip_block_comment();
                continue;
            }
            break;
        }
    }

    fn consume_char(&mut self, expected: char) -> bool {
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

    fn peek_str(&self, s: &str) -> bool {
        self.source[self.pos..].starts_with(s)
    }

    fn is_word_boundary_at(&self, offset: usize) -> bool {
        let next_pos = self.pos + offset;
        if next_pos >= self.bytes.len() {
            return true;
        }
        let next_ch = self.source[next_pos..].chars().next().unwrap_or('\0');
        !next_ch.is_alphanumeric() && next_ch != '_' && next_ch != '$'
    }

    fn current_char(&self) -> char {
        self.source[self.pos..].chars().next().unwrap_or('\0')
    }

    fn current_char_opt(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn advance_char(&mut self) {
        if self.is_at_end() {
            return;
        }
        let ch = self.current_char();
        self.pos += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }

    fn advance_chars(&mut self, count: usize) {
        for _ in 0..count {
            self.advance_char();
        }
    }

    fn advance_to_byte(&mut self, target: usize) {
        while self.pos < target && !self.is_at_end() {
            self.advance_char();
        }
    }

    fn make_span(&self, start_byte: usize, start_line: usize, start_column: usize) -> JsSourceSpan {
        JsSourceSpan::range(start_byte, self.pos, start_line, start_column)
    }

    fn emit_diagnostic_at_current(&mut self, message: impl Into<String>, kind: JsDiagnosticKind) {
        let span = JsSourceSpan::at(self.pos, self.line, self.column);
        self.diagnostics.push(JsParserDiagnostic {
            message: message.into(),
            span,
            kind,
        });
    }

    fn emit_diagnostic(
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

fn strip_js_comments_preserving_strings(source: &str) -> String {
    let mut stripped = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();

    while let Some(character) = chars.next() {
        if character == '"' || character == '\'' {
            stripped.push(character);
            let quote = character;
            while let Some(inner) = chars.next() {
                stripped.push(inner);
                if inner == '\\' {
                    if let Some(escaped) = chars.next() {
                        stripped.push(escaped);
                    }
                    continue;
                }
                if inner == quote {
                    break;
                }
            }
            continue;
        }

        if character == '/' {
            match chars.peek().copied() {
                Some('/') => {
                    chars.next();
                    for comment_character in chars.by_ref() {
                        if comment_character == '\n' {
                            stripped.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    let mut previous = '\0';
                    for comment_character in chars.by_ref() {
                        if previous == '*' && comment_character == '/' {
                            break;
                        }
                        previous = comment_character;
                    }
                    stripped.push(' ');
                }
                _ => stripped.push(character),
            }
            continue;
        }

        stripped.push(character);
    }

    stripped
}

fn extract_static_import_specifier(statement: &str) -> Option<String> {
    let stripped = strip_js_comments_preserving_strings(statement);
    if let Some(from_index) = find_word(&stripped, "from") {
        return parse_string_literal_from(&stripped[from_index + "from".len()..]);
    }

    parse_string_literal_from(&stripped)
}

fn find_word(text: &str, word: &str) -> Option<usize> {
    let mut search_start = 0;

    while let Some(relative_index) = text[search_start..].find(word) {
        let index = search_start + relative_index;
        let before = text[..index].chars().next_back();
        let after = text[index + word.len()..].chars().next();
        let before_boundary =
            before.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_' && ch != '$');
        let after_boundary = after.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_' && ch != '$');

        if before_boundary && after_boundary {
            return Some(index);
        }

        search_start = index + word.len();
    }

    None
}

fn parse_string_literal_from(text: &str) -> Option<String> {
    let mut chars = text.char_indices().peekable();

    while let Some((_, ch)) = chars.next() {
        if ch != '"' && ch != '\'' {
            continue;
        }

        let quote = ch;
        let mut value = String::new();
        while let Some((_, inner)) = chars.next() {
            if inner == '\\' {
                if let Some((_, escaped)) = chars.next() {
                    value.push(match escaped {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '0' => '\0',
                        other => other,
                    });
                }
                continue;
            }

            if inner == quote {
                return Some(value);
            }

            value.push(inner);
        }

        return None;
    }

    None
}

/// Parses simple named import identifiers from the text after `import`.
///
/// WHAT: extracts comma-separated identifiers from `{ a, b }` syntax.
/// WHY: the scanner needs to know which symbols a registered runtime import
///      references so the registry can validate them.
///
/// Returns `Ok(names)` for a plain named import followed by a `from` clause.
/// Returns `Err(())` for any other form (default, namespace, alias, empty).
fn parse_named_import_names(statement: &str) -> Result<Vec<String>, ()> {
    let trimmed = statement.trim_start();
    let after_brace = trimmed.strip_prefix('{').ok_or(())?;
    let close_brace = after_brace.find('}').ok_or(())?;
    let content = strip_js_comments_from_named_import_list(&after_brace[..close_brace]);
    let from_clause = after_brace[close_brace + 1..].trim_start();
    let after_from = from_clause.strip_prefix("from").ok_or(())?;

    if after_from
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')
    {
        return Err(());
    }

    let mut names = Vec::new();
    for part in content.split(',') {
        let name = part.trim();
        if name.is_empty() {
            continue;
        }
        if name.split_whitespace().count() > 1 || !is_runtime_import_identifier(name) {
            // Aliases or other unsupported forms inside braces.
            return Err(());
        }
        names.push(name.to_owned());
    }

    if names.is_empty() {
        return Err(());
    }

    Ok(names)
}

/// Removes comments from a named runtime import list before identifier validation.
///
/// WHAT: accepts comments inside the `{ ... }` portion of a runtime import without treating
///       comment text as part of an imported name.
/// WHY: statement scanning already treats comments as lexical trivia, and import-list parsing
///      should preserve that same boundary without growing into a full JavaScript lexer.
fn strip_js_comments_from_named_import_list(content: &str) -> String {
    let mut stripped = String::new();
    let mut chars = content.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '/' {
            stripped.push(ch);
            continue;
        }

        match chars.peek().copied() {
            Some('/') => {
                chars.next();
                for comment_ch in chars.by_ref() {
                    if comment_ch == '\n' {
                        stripped.push('\n');
                        break;
                    }
                }
            }
            Some('*') => {
                chars.next();
                let mut previous = '\0';
                for comment_ch in chars.by_ref() {
                    if previous == '*' && comment_ch == '/' {
                        break;
                    }
                    previous = comment_ch;
                }
                stripped.push(' ');
            }
            _ => stripped.push(ch),
        }
    }

    stripped
}

fn is_runtime_import_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !first.is_ascii_alphabetic() && first != '_' && first != '$' {
        return false;
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
}
