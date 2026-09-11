//! Scans static JavaScript imports and CommonJS `require()` calls.
//!
//! WHAT: recognizes supported static import clauses and validates their module
//!       specifiers and names against the `RuntimeModuleRegistry`.
//! WHY: keeps module-loading policy separate from export-form recognition while
//!      sharing the same `ExportScanner` cursor and lexical helpers.

use super::export_scanner::ExportScanner;
use super::parsed_js_module::{JsDiagnosticKind, JsSourceSpan, ParsedRuntimeImport};

enum ImportClause {
    Missing,
    PlainNamed(Vec<String>),
    Other,
}

impl<'a> ExportScanner<'a> {
    // ------------------------
    //  Import statement scanning
    // ------------------------

    pub(super) fn read_import_statement(&mut self) {
        let import_start_byte = self.pos;

        self.advance_chars("import".len());
        self.skip_whitespace();

        // Dynamic import: `import(...)`
        if self.consume_char('(') {
            self.emit_diagnostic(
                "Dynamic `import()` is not supported in Moth JS modules.",
                JsDiagnosticKind::DynamicImport,
                JsSourceSpan::range(import_start_byte, self.pos),
            );
            self.skip_to_statement_end();
            return;
        }

        // Parse the clause and module specifier with the scanner's current cursor.
        // A named `{ ... }` clause requires `from` before the specifier. A missing
        // clause may be a side-effect `import "module"` whose specifier is the
        // next string literal.
        let import_clause_start = self.pos;
        let import_clause = self.parse_import_clause();
        let allow_side_effect_specifier = matches!(import_clause, ImportClause::Missing);
        let specifier = self.parse_import_specifier(allow_side_effect_specifier);
        let Some(specifier) = specifier else {
            self.emit_diagnostic(
                "JavaScript static import is not supported in Moth JS module files yet. \
                 Only registered Moth core runtime modules are supported.",
                JsDiagnosticKind::ArbitraryImport,
                JsSourceSpan::range(import_start_byte, import_clause_start),
            );
            self.skip_to_statement_end();
            return;
        };

        if self.registry.is_registered(&specifier) {
            let diagnostic_start = self.diagnostics.len();
            let runtime_import_start = self.runtime_imports.len();
            let span = JsSourceSpan::range(import_start_byte, self.pos);

            match import_clause {
                ImportClause::PlainNamed(names) if !names.is_empty() => {
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
                            span: span.clone(),
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

            // Extend registered-import spans through the same statement boundary used by
            // the scanner. Nested findings discovered during the skip keep their own spans.
            let diagnostic_end = self.diagnostics.len();
            let runtime_import_end = self.runtime_imports.len();
            self.skip_to_statement_end();
            let statement_span = JsSourceSpan::range(import_start_byte, self.pos);

            for diagnostic in &mut self.diagnostics[diagnostic_start..diagnostic_end] {
                diagnostic.span = statement_span.clone();
            }
            for runtime_import in
                &mut self.runtime_imports[runtime_import_start..runtime_import_end]
            {
                runtime_import.span = statement_span.clone();
            }
            return;
        }

        self.emit_diagnostic(
            format!(
                "JavaScript import `{specifier}` is not supported in Moth JS module files yet. \
                 Only registered Moth core runtime modules are supported."
            ),
            JsDiagnosticKind::ArbitraryImport,
            JsSourceSpan::range(import_start_byte, import_clause_start),
        );
        self.skip_to_statement_end();
    }

    fn parse_import_clause(&mut self) -> ImportClause {
        self.skip_whitespace_and_comments();

        if !self.consume_char('{') {
            return ImportClause::Missing;
        }

        match self.parse_named_import_list() {
            Some(names) => ImportClause::PlainNamed(names),
            None => ImportClause::Other,
        }
    }

    fn parse_named_import_list(&mut self) -> Option<Vec<String>> {
        let mut names = Vec::new();
        let mut is_plain_named_list = true;
        let mut expecting_name = true;

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();
            if self.is_at_end() {
                break;
            }

            if matches!(self.current_char_opt(), Some('"') | Some('\'') | Some('`')) {
                is_plain_named_list = false;
                self.skip_lexical_content_at_current();
                continue;
            }

            if self.consume_char('}') {
                if names.is_empty() || !is_plain_named_list {
                    return None;
                }
                return Some(names);
            }

            if expecting_name {
                if let Some(name) = self.parse_identifier() {
                    // Runtime named imports only accept ASCII identifier names. Unicode
                    // letters are valid JS identifiers, but they are not a supported
                    // `import { name }` clause for registered modules.
                    if !is_plain_named_import_identifier(&name) {
                        is_plain_named_list = false;
                    }
                    names.push(name);
                    expecting_name = false;
                    continue;
                }

                is_plain_named_list = false;
            } else if self.consume_char(',') {
                expecting_name = true;
                continue;
            } else {
                is_plain_named_list = false;
            }

            if self.current_char_opt() == Some('/')
                && !self.peek_str("//")
                && !self.peek_str("/*")
                && self.slash_starts_regular_expression()
            {
                self.skip_regular_expression();
            } else {
                self.advance_char();
            }
        }

        None
    }

    fn parse_import_specifier(&mut self, allow_side_effect_specifier: bool) -> Option<String> {
        self.skip_whitespace_and_comments();

        if allow_side_effect_specifier && matches!(self.current_char_opt(), Some('"') | Some('\''))
        {
            return self.parse_import_string_literal();
        }

        let mut parenthesis_depth = 0usize;
        let mut brace_depth = 0usize;
        let mut bracket_depth = 0usize;

        while !self.is_at_end() {
            if self.skip_lexical_content_at_current() {
                continue;
            }

            if self.current_char_opt() == Some('/')
                && !self.peek_str("//")
                && !self.peek_str("/*")
                && self.slash_starts_regular_expression()
            {
                self.skip_regular_expression();
                continue;
            }

            let at_top_level = parenthesis_depth == 0 && brace_depth == 0 && bracket_depth == 0;
            if at_top_level && self.peek_str("from") && self.is_word_boundary_around("from".len()) {
                self.advance_chars("from".len());
                self.skip_whitespace_and_comments();
                return self.parse_import_string_literal();
            }

            let current_character = self.current_char();
            match current_character {
                '(' => parenthesis_depth += 1,
                ')' => parenthesis_depth = parenthesis_depth.saturating_sub(1),
                '{' => brace_depth += 1,
                '}' => brace_depth = brace_depth.saturating_sub(1),
                '[' => bracket_depth += 1,
                ']' => bracket_depth = bracket_depth.saturating_sub(1),
                ';' if at_top_level => return None,
                '\n' if at_top_level => {
                    let newline_position = self.pos;
                    self.advance_char();
                    self.skip_whitespace_and_comments();
                    if self.peek_str("from") && self.is_word_boundary_around("from".len()) {
                        continue;
                    }
                    self.pos = newline_position;
                    return None;
                }
                _ => {}
            }
            self.advance_char();
        }

        None
    }

    fn parse_import_string_literal(&mut self) -> Option<String> {
        let quote = self.current_char_opt()?;
        if quote != '"' && quote != '\'' {
            return None;
        }

        self.advance_char();
        let mut value = String::new();

        while !self.is_at_end() {
            let current_character = self.current_char();
            self.advance_char();

            if current_character == '\\' {
                if let Some(escaped_character) = self.current_char_opt() {
                    value.push(match escaped_character {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '0' => '\0',
                        other => other,
                    });
                    self.advance_char();
                }
                continue;
            }

            if current_character == quote {
                return Some(value);
            }

            value.push(current_character);
        }

        None
    }

    pub(super) fn read_require_statement(&mut self) {
        let require_start_byte = self.pos;

        self.advance_chars("require".len());
        self.skip_whitespace_and_comments();

        if !self.consume_char('(') {
            return;
        }

        self.emit_diagnostic(
            "CommonJS `require()` is not supported in Moth JS modules.",
            JsDiagnosticKind::CommonJsRequire,
            JsSourceSpan::range(require_start_byte, self.pos),
        );
        self.skip_to_statement_end();
    }
}

fn is_plain_named_import_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };

    if !first.is_ascii_alphabetic() && first != '_' && first != '$' {
        return false;
    }

    characters
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '$')
}
