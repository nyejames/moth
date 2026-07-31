//! Path component parsing and validation.
//!
//! WHAT: parses individual path components (bare and quoted) and validates them.
//! WHY: ordinary paths and grouped entries share the same component grammar; keeping
//!      component logic in one module avoids duplication and makes validation rules
//!      easy to audit.

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, PathKind};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::{
    consume_all_whitespace, consume_non_newline_whitespace,
};
use crate::compiler_frontend::tokenizer::tokens::TokenStream;

use super::{ParseComponentContext, PathComponents};

/// Boxed diagnostic result for the connected path-component family.
///
/// Component parsing and validation feed boxed grouped and ordinary path helpers directly. The
/// public path parser unboxes once when it returns the diagnostic to its tokenizer-facing caller.
type ComponentResult<T> = Result<T, Box<CompilerDiagnostic>>;

/// WHAT: Parsed result of one path component, with its raw text and whether it was quoted.
/// WHY: downstream validation needs to know whether quotes were used to allow spaces.
#[derive(Debug)]
pub(super) struct ParsedComponent {
    pub(super) value: String,
    pub(super) was_quoted: bool,
}

/// WHAT: Parses exactly one path component (bare or quoted) from the current stream position.
/// WHY: Ordinary paths and grouped entries must share the same component grammar and escapes.
pub(super) fn parse_component(
    stream: &mut TokenStream,
    context: ParseComponentContext,
    string_table: &StringTable,
) -> ComponentResult<ParsedComponent> {
    if stream.peek() == Some(&'"') {
        return parse_quoted_component(stream, string_table);
    }

    parse_bare_component(stream, context, string_table)
}

/// WHAT: Parses a quoted path component using path-literal escapes.
/// WHY: Quoted components are the only syntax that allows whitespace inside a component.
fn parse_quoted_component(
    stream: &mut TokenStream,
    _string_table: &StringTable,
) -> ComponentResult<ParsedComponent> {
    assert_eq!(
        stream.peek().copied(),
        Some('"'),
        "Quoted path component parsing expected to start on '\"'."
    );

    stream.next();
    let mut value = String::new();

    loop {
        let Some(next) = stream.peek().copied() else {
            return Err(Box::new(CompilerDiagnostic::invalid_path(
                PathKind::MissingClosingQuote,
                stream.new_location(),
            )));
        };

        if next == '"' {
            stream.next();
            return Ok(ParsedComponent {
                value,
                was_quoted: true,
            });
        }

        if next == '\\' {
            stream.next();

            let Some(escaped) = stream.peek().copied() else {
                return Err(Box::new(CompilerDiagnostic::invalid_path(
                    PathKind::MissingClosingQuote,
                    stream.new_location(),
                )));
            };

            match escaped {
                '"' | '\\' => {
                    value.push(escaped);
                    stream.next();
                }
                _ => {
                    return Err(Box::new(CompilerDiagnostic::invalid_path(
                        PathKind::InvalidEscape,
                        stream.new_location(),
                    )));
                }
            }

            continue;
        }

        value.push(next);
        stream.next();
    }
}

/// WHAT: Parses an unquoted path component and enforces quote-required whitespace rules.
/// WHY: Bare components must remain unambiguous path tokens without internal whitespace.
pub(super) fn parse_bare_component(
    stream: &mut TokenStream,
    context: ParseComponentContext,
    _string_table: &StringTable,
) -> ComponentResult<ParsedComponent> {
    let mut value = String::new();

    while let Some(next) = stream.peek().copied() {
        if next.is_whitespace()
            || super::is_component_terminator(stream, context, next, value.is_empty())
        {
            break;
        }

        value.push(next);
        stream.next();
    }

    if value.is_empty() {
        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::EmptyComponent,
            stream.new_location(),
        )));
    }

    if stream
        .peek()
        .is_some_and(|character| character.is_whitespace())
    {
        match context {
            ParseComponentContext::OrdinaryPath => {
                consume_non_newline_whitespace(stream);
            }
            ParseComponentContext::GroupedEntry => {
                consume_all_whitespace(stream);
            }
        }

        if let Some(next) = stream.peek().copied()
            && !super::is_component_terminator(stream, context, next, true)
        {
            // Allow the `as` keyword to follow a bare path component without quoting.
            // WHAT: `import @path/symbol as alias` is valid syntax; `as` is a keyword.
            // WHY: without this, the path tokenizer treats `as` as an unquoted multi-word
            //      path component and emits a confusing error.
            if matches!(context, ParseComponentContext::OrdinaryPath)
                && next == 'a'
                && super::peek_keyword_as(stream)
            {
                // Return the component normally; `parse_path_prefix` will stop at `as`.
            } else {
                return Err(Box::new(CompilerDiagnostic::invalid_path(
                    PathKind::WhitespaceMustBeQuoted,
                    stream.new_location(),
                )));
            }
        }
    }

    Ok(ParsedComponent {
        value,
        was_quoted: false,
    })
}

/// WHAT: Validates and interns one parsed component.
/// WHY: Keeps grouped and ordinary paths aligned on one validation boundary.
pub(super) fn push_validated_component(
    components: &mut PathComponents,
    parsed_component: ParsedComponent,
    allow_leading_relative_markers: bool,
    seen_non_relative_component: &mut bool,
    stream: &mut TokenStream,
    string_table: &mut StringTable,
) -> ComponentResult<()> {
    let allow_relative_marker = allow_leading_relative_markers && !*seen_non_relative_component;

    validate_path_component(
        &parsed_component.value,
        allow_relative_marker,
        parsed_component.was_quoted,
        stream,
        string_table,
    )?;

    if parsed_component.value != "." && parsed_component.value != ".." {
        *seen_non_relative_component = true;
    }

    components.push(string_table.intern(&parsed_component.value));
    Ok(())
}

fn validate_path_component(
    component: &str,
    allow_relative_marker: bool,
    was_quoted: bool,
    stream: &mut TokenStream,
    _string_table: &StringTable,
) -> ComponentResult<()> {
    if component.is_empty() {
        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::EmptyComponent,
            stream.new_location(),
        )));
    }

    // Reject a path component that starts with `@` after the import introducer was consumed.
    // WHAT: the first `@` in `import @path` is the import-path introducer consumed by the lexer.
    //      A second `@` starting any component (such as `@@pages` or `@helper/@home`) is not a
    //      valid module name. Normal module-root filenames are cosmetic filesystem markers.
    if !was_quoted && component.starts_with('@') {
        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::LeadingAtInPathComponent,
            stream.new_location(),
        )));
    }

    if component == "." || component == ".." {
        if allow_relative_marker {
            return Ok(());
        }

        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::InvalidComponent,
            stream.new_location(),
        )));
    }

    if component.ends_with('.') {
        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::InvalidComponent,
            stream.new_location(),
        )));
    }

    if component
        .chars()
        .any(|character| !is_valid_component_char(character, was_quoted))
    {
        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::InvalidComponent,
            stream.new_location(),
        )));
    }

    if is_reserved_windows_name(component) {
        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::InvalidComponent,
            stream.new_location(),
        )));
    }

    Ok(())
}

fn is_valid_component_char(character: char, allow_spaces: bool) -> bool {
    if character.is_control() {
        return false;
    }

    if character.is_whitespace() {
        if allow_spaces && character == ' ' {
            return true;
        }

        return false;
    }

    !matches!(
        character,
        '[' | ']'
            | '{'
            | '}'
            | ','
            | '('
            | ')'
            | '/'
            | '\\'
            | '<'
            | '>'
            | ':'
            | '"'
            | '|'
            | '?'
            | '*'
    )
}

fn is_reserved_windows_name(component: &str) -> bool {
    let prefix = component.split('.').next().unwrap_or(component);

    matches!(
        prefix.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}
