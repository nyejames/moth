//! Lightweight parser for `@moth.sig` signature bodies.
//!
//! WHAT: parses Moth parameter-list syntax such as `|this ~Canvas2d, x Float| -> Error!`
//!       into structured `ParsedParameter` and `ParsedReturnType` values.
//! WHY: the scanner should not embed full Moth parser dependency, but it must
//!      understand enough of the signature shape to validate arity and detect
//!      receiver-shaped signatures that external package registration rejects.
//!
//! Limitations (intentional):
//! - Does not resolve type names against a registry; type names are returned as strings.
//! - Rejects `Void`, multi-success returns, collections, options, callbacks, and generics.
//! - Does not validate that receiver types were declared with `@moth.opaque`.

use super::parsed_js_module::{
    JsDiagnosticKind, JsParserDiagnostic, JsSourceSpan, ParsedParameter, ParsedReturnType,
    ParsedSignature,
};

/// Input to the signature parser.
///
/// WHAT: carries the raw signature text plus the JS-file position where it starts
///       so diagnostics can point back to source.
pub struct SignatureParseInput {
    pub text: String,
    pub base_byte: usize,
    pub base_line: usize,
    pub base_column: usize,
}

/// Result of parsing one signature body.
pub struct SignatureParseResult {
    pub signature: ParsedSignature,
    pub diagnostics: Vec<JsParserDiagnostic>,
}

/// Parses a `@moth.sig` body after the Moth-facing name has been extracted.
///
/// WHAT: expects text starting with a parameter list `|...|` and optionally
///       followed by `->` and return types.
///
/// Examples of valid input:
/// - `|id String| -> CanvasElement, Error!`
/// - `|ctx ~Canvas2d, x Float|`
/// - `|| -> Error!`
pub fn parse_signature(input: SignatureParseInput) -> SignatureParseResult {
    let mut scanner = SignatureScanner::new(input);
    scanner.parse()
}

struct SignatureScanner {
    text: Vec<char>,
    pos: usize,
    base_byte: usize,
    base_line: usize,
    base_column: usize,
    diagnostics: Vec<JsParserDiagnostic>,
}

impl SignatureScanner {
    fn new(input: SignatureParseInput) -> Self {
        Self {
            text: input.text.chars().collect(),
            pos: 0,
            base_byte: input.base_byte,
            base_line: input.base_line,
            base_column: input.base_column,
            diagnostics: Vec::new(),
        }
    }

    fn parse(&mut self) -> SignatureParseResult {
        self.skip_whitespace();

        let has_unsupported_generic_parameters = self.consume_generic_parameter_preamble();
        self.skip_whitespace();

        if !self.consume_char('|') {
            self.emit_diagnostic(
                "Signature must start with parameter list `|...|`.",
                JsDiagnosticKind::UnsupportedTypeSyntax,
            );
            return self.empty_result();
        }

        let parameters = self.parse_parameters();

        self.skip_whitespace();

        let mut returns = Vec::new();
        let mut has_error_return = false;

        if self.consume_arrow() {
            let return_result = self.parse_returns();
            returns = return_result.returns;
            has_error_return = return_result.has_error;
        }

        SignatureParseResult {
            signature: ParsedSignature {
                parameters,
                returns,
                has_error_return,
                has_unsupported_generic_parameters,
            },
            diagnostics: std::mem::take(&mut self.diagnostics),
        }
    }

    fn empty_result(&self) -> SignatureParseResult {
        SignatureParseResult {
            signature: ParsedSignature {
                parameters: Vec::new(),
                returns: Vec::new(),
                has_error_return: false,
                has_unsupported_generic_parameters: false,
            },
            diagnostics: self.diagnostics.clone(),
        }
    }

    fn consume_generic_parameter_preamble(&mut self) -> bool {
        if !self.consume_keyword("type") {
            return false;
        }

        self.emit_diagnostic(
            "External package functions cannot be generic. Expose concrete external functions or wrap them with source Moth generic functions.",
            JsDiagnosticKind::GenericExternalFunction,
        );

        // Recover at the real ABI parameter list so arity and receiver parsing stay
        // deterministic without treating generic parameter names as external types.
        while !self.is_at_end() && !self.peek_char('|') {
            self.advance();
        }

        true
    }

    // ------------------------
    //  Parameter list
    // ------------------------

    fn parse_parameters(&mut self) -> Vec<ParsedParameter> {
        let mut parameters = Vec::new();
        let mut index = 0;
        let mut has_seen_receiver = false;

        loop {
            self.skip_whitespace();

            if self.consume_char('|') {
                break;
            }

            if self.is_at_end() {
                self.emit_diagnostic(
                    "Unclosed parameter list in signature.",
                    JsDiagnosticKind::UnsupportedTypeSyntax,
                );
                break;
            }

            let parameter_start = self.pos;
            if let Some(parameter) = self.parse_parameter(index, &mut has_seen_receiver) {
                parameters.push(parameter);
            }

            if self.pos != parameter_start {
                index += 1;
            }

            self.skip_whitespace();

            if self.consume_char('|') {
                break;
            }

            if !self.consume_char(',') {
                self.emit_diagnostic(
                    "Parameters in a signature must be separated by commas.",
                    JsDiagnosticKind::UnsupportedTypeSyntax,
                );
                // Attempt recovery: skip to next comma or pipe
                self.skip_to_parameter_boundary();
            }
        }

        parameters
    }

    fn parse_parameter(
        &mut self,
        parameter_index: usize,
        has_seen_receiver: &mut bool,
    ) -> Option<ParsedParameter> {
        self.skip_whitespace();

        // Rest parameters are rejected before type parsing so recovery can stop
        // at the next boundary instead of consuming the rest of the signature.
        if self.peek_str("...") {
            self.emit_diagnostic(
                "Rest parameters are not supported in Moth JS module signatures.",
                JsDiagnosticKind::UnsupportedParameterPattern,
            );
            self.skip_to_parameter_boundary();
            return None;
        }

        // Destructuring patterns are rejected for the same reason: the subset
        // only accepts flat ABI slots.
        if self.peek_char('{') || self.peek_char('[') {
            self.emit_diagnostic(
                "Destructuring parameters are not supported in Moth JS module signatures.",
                JsDiagnosticKind::UnsupportedParameterPattern,
            );
            self.skip_to_parameter_boundary();
            return None;
        }

        let name = self.parse_identifier()?;

        // Optional parameters are rejected in the external signature subset.
        if self.consume_char('?') {
            self.emit_diagnostic(
                "Optional parameters are not supported in Moth JS module signatures.",
                JsDiagnosticKind::UnsupportedParameterPattern,
            );
            self.skip_to_parameter_boundary();
            return None;
        }

        let is_receiver = name == "this";
        self.skip_whitespace();

        // The `~T` marker preserves the mutable/exclusive ABI contract for both
        // receivers (`this ~T`) and regular parameters (`name ~T`).
        let is_mutable = self.consume_char('~');
        self.skip_whitespace();

        let type_name = self.parse_type_annotation();

        if type_name.is_empty() {
            self.emit_diagnostic(
                "Missing type annotation in signature parameter.",
                JsDiagnosticKind::UnsupportedTypeSyntax,
            );
        }

        if is_receiver {
            if parameter_index != 0 {
                self.emit_diagnostic(
                    "Receiver parameter `this` must be the first parameter in a Moth JS module signature.",
                    JsDiagnosticKind::InvalidReceiverParameter,
                );
            }
            if *has_seen_receiver {
                self.emit_diagnostic(
                    "Receiver parameter `this` may appear at most once in a Moth JS module signature.",
                    JsDiagnosticKind::InvalidReceiverParameter,
                );
            }
            *has_seen_receiver = true;
        }

        Some(ParsedParameter {
            name,
            type_name,
            is_receiver,
            is_mutable,
        })
    }

    fn parse_type_annotation(&mut self) -> String {
        self.skip_whitespace();

        let mut type_name = String::new();

        // Preserve collection spellings verbatim so the caller can surface a
        // targeted unsupported-type diagnostic while still recovering.
        if self.consume_char('{') {
            type_name.push('{');
            while !self.is_at_end() && !self.peek_char('}') {
                type_name.push(self.current_char());
                self.advance();
            }
            if self.consume_char('}') {
                type_name.push('}');
            }
            self.emit_diagnostic(
                "Collection types are not supported in Moth JS module signatures yet.",
                JsDiagnosticKind::UnsupportedTypeSyntax,
            );
            return type_name;
        }

        while let Some(ch) = self.current_char_opt() {
            if ch.is_alphanumeric() || ch == '_' {
                type_name.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if self.consume_char('?') {
            type_name.push('?');
            self.emit_diagnostic(
                "Option types are not supported in Moth JS module signatures yet.",
                JsDiagnosticKind::UnsupportedTypeSyntax,
            );
        }

        type_name
    }

    // ------------------------
    //  Return types
    // ------------------------

    fn parse_returns(&mut self) -> ReturnParseResult {
        let mut returns = Vec::new();
        let mut has_error = false;
        loop {
            self.skip_whitespace();

            if self.is_at_end() {
                break;
            }

            // Preserve the `Error!` sentinel so success-return validation still
            // runs on the remaining return slots.
            if self.peek_str("Error") {
                let error_word = self.parse_identifier().unwrap_or_default();
                if error_word == "Error" && self.consume_char('!') {
                    has_error = true;
                    self.skip_whitespace();
                    if self.consume_char(',') {
                        self.emit_diagnostic(
                            "`Error!` must be the final return slot.",
                            JsDiagnosticKind::UnsupportedTypeSyntax,
                        );
                    }
                    break;
                } else {
                    // Fall back to treating `Error` as an ordinary success type name.
                    returns.push(ParsedReturnType {
                        type_name: error_word,
                    });
                }
            } else {
                let type_name = self.parse_type_annotation();
                if type_name.is_empty() {
                    break;
                }

                // Normalize common unit spellings to the same unsupported-return
                // diagnostic so callers see one consistent failure mode.
                if type_name.eq_ignore_ascii_case("void")
                    || type_name.eq_ignore_ascii_case("none")
                    || type_name.eq_ignore_ascii_case("unit")
                    || type_name == "()"
                {
                    self.emit_diagnostic(
                        "Void/None/Unit return spellings are not supported. Omit `->` for no success return.",
                        JsDiagnosticKind::VoidReturn,
                    );
                }

                returns.push(ParsedReturnType { type_name });
            }

            self.skip_whitespace();

            if !self.consume_char(',') {
                break;
            }
        }

        // More than one success return is not supported in this parser subset.
        if returns.len() > 1 {
            self.emit_diagnostic(
                "Multi-success returns are not supported in Moth JS module signatures yet.",
                JsDiagnosticKind::MultiSuccessReturn,
            );
        }

        ReturnParseResult { returns, has_error }
    }

    // ------------------------
    //  Helpers
    // ------------------------

    fn parse_identifier(&mut self) -> Option<String> {
        self.skip_whitespace();

        let mut name = String::new();
        let ch = self.current_char_opt()?;
        if ch.is_alphabetic() || ch == '_' {
            name.push(ch);
            self.advance();
        } else {
            return None;
        }

        while let Some(ch) = self.current_char_opt() {
            if ch.is_alphanumeric() || ch == '_' {
                name.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        Some(name)
    }

    fn consume_keyword(&mut self, expected: &str) -> bool {
        self.skip_whitespace();

        if !self.peek_str(expected) {
            return false;
        }

        let after_keyword = self.pos + expected.chars().count();
        if self
            .text
            .get(after_keyword)
            .is_some_and(|ch| ch.is_alphanumeric() || *ch == '_')
        {
            return false;
        }

        self.pos = after_keyword;
        true
    }

    fn consume_arrow(&mut self) -> bool {
        self.skip_whitespace();
        if self.peek_str("->") {
            self.pos += 2;
            true
        } else {
            false
        }
    }

    fn consume_char(&mut self, expected: char) -> bool {
        if self.current_char_opt() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn peek_char(&self, expected: char) -> bool {
        self.current_char_opt() == Some(expected)
    }

    fn peek_str(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        if self.pos + chars.len() > self.text.len() {
            return false;
        }
        self.text[self.pos..self.pos + chars.len()] == chars[..]
    }

    fn current_char(&self) -> char {
        self.text[self.pos]
    }

    fn current_char_opt(&self) -> Option<char> {
        self.text.get(self.pos).copied()
    }

    fn advance(&mut self) {
        if self.pos < self.text.len() {
            self.pos += 1;
        }
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.text.len()
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current_char_opt() {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_to_parameter_boundary(&mut self) {
        while let Some(ch) = self.current_char_opt() {
            if ch == ',' || ch == '|' {
                break;
            }
            self.advance();
        }
    }

    fn emit_diagnostic(&mut self, message: impl Into<String>, kind: JsDiagnosticKind) {
        self.diagnostics.push(JsParserDiagnostic {
            message: message.into(),
            span: JsSourceSpan::at(self.base_byte, self.base_line, self.base_column),
            kind,
        });
    }
}

struct ReturnParseResult {
    returns: Vec<ParsedReturnType>,
    has_error: bool,
}
