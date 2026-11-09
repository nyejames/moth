use super::{CodeLanguage, highlight_code_html_into};

/// Converts raw source code into highlighted HTML markup for scanner tests.
pub(crate) fn highlight_code_html(source: &str, language: CodeLanguage) -> String {
    let mut output = String::with_capacity(source.len() + 16);
    highlight_code_html_into(source, language, &mut output);
    output
}
