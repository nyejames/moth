//! HTML-project-owned `$html` formatter/validator.
//!
//! WHAT:
//! - Preserves HTML text exactly as authored while emitting sanitation warnings.
//! - Uses shared flattened-source span tracking so warnings stay source-mapped.
//!
//! WHY:
//! - HTML-specific safety heuristics are output-policy concerns owned by the HTML project builder.

use crate::compiler_frontend::ast::templates::formatter_contract::FormatterInput;
use crate::compiler_frontend::ast::templates::styles::whitespace::TemplateWhitespacePassProfile;
use crate::compiler_frontend::ast::templates::template::{
    Formatter, FormatterResult, TemplateFormatter,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;

use crate::compiler_frontend::compiler_messages::{
    HtmlTemplateWarning, MalformedTemplateReason, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::style_directives::StyleDirectiveArgumentValue;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::styles::validation::{PassThroughFormatterInput, SourceWarning};
use std::sync::Arc;

#[derive(Debug)]
struct HtmlValidationTemplateFormatter;

pub(crate) fn html_validation_formatter() -> Formatter {
    Formatter {
        pre_format_whitespace_passes: vec![TemplateWhitespacePassProfile::default_template_body()],
        formatter: Arc::new(HtmlValidationTemplateFormatter),
        post_format_whitespace_passes: Vec::new(),
    }
}

pub(crate) fn html_formatter_factory(
    argument: Option<&StyleDirectiveArgumentValue>,
) -> Result<Formatter, String> {
    if argument.is_some() {
        return Err("'$html' does not accept arguments.".to_string());
    }
    Ok(html_validation_formatter())
}

impl TemplateFormatter for HtmlValidationTemplateFormatter {
    fn format(
        &self,
        input: FormatterInput,
        string_table: &mut StringTable,
    ) -> Result<FormatterResult, CompilerMessages> {
        let flattened_input = PassThroughFormatterInput::from_input(input, string_table);
        let warnings = flattened_input.map_warnings(
            validate_html_source(&flattened_input.flattened_source),
            SyntaxDiagnosticKind::MalformedHtmlTemplate,
        );

        Ok(flattened_input.into_formatter_result(warnings))
    }
}

fn validate_html_source(source: &str) -> Vec<SourceWarning> {
    let lowered = source.to_ascii_lowercase();
    let chars: Vec<char> = lowered.chars().collect();
    let mut warnings = Vec::new();

    push_literal_match_warnings(
        &chars,
        "<script",
        HtmlTemplateWarning::ScriptTag,
        &mut warnings,
    );

    push_literal_match_warnings(
        &chars,
        "javascript:",
        HtmlTemplateWarning::JavascriptUrl,
        &mut warnings,
    );

    warnings.extend(scan_inline_event_handler_warnings(&chars));
    warnings
}

fn push_literal_match_warnings(
    chars: &[char],
    pattern: &str,
    reason: HtmlTemplateWarning,
    warnings: &mut Vec<SourceWarning>,
) {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    if pattern_chars.is_empty() || pattern_chars.len() > chars.len() {
        return;
    }

    for index in 0..=chars.len() - pattern_chars.len() {
        if chars[index..index + pattern_chars.len()] == *pattern_chars {
            warnings.push(SourceWarning {
                reason: MalformedTemplateReason::Html(reason),
                start_offset: index,
                end_offset: index + pattern_chars.len(),
            });
        }
    }
}

fn scan_inline_event_handler_warnings(chars: &[char]) -> Vec<SourceWarning> {
    let mut warnings = Vec::new();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] != 'o' || chars.get(index + 1) != Some(&'n') {
            index += 1;
            continue;
        }

        if index > 0 && is_attribute_name_char(chars[index - 1]) {
            index += 1;
            continue;
        }

        let mut cursor = index + 2;
        let mut has_attribute_name = false;
        while cursor < chars.len() && is_attribute_name_char(chars[cursor]) {
            has_attribute_name = true;
            cursor += 1;
        }

        if !has_attribute_name {
            index += 1;
            continue;
        }

        while cursor < chars.len() && chars[cursor].is_whitespace() {
            cursor += 1;
        }

        if cursor < chars.len() && chars[cursor] == '=' {
            warnings.push(SourceWarning {
                reason: MalformedTemplateReason::Html(HtmlTemplateWarning::InlineEventHandler),
                start_offset: index,
                end_offset: cursor.saturating_add(1),
            });
            index = cursor.saturating_add(1);
            continue;
        }

        index += 1;
    }

    warnings
}

fn is_attribute_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}
