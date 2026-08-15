//! Path component parsing and validation.
//!
//! WHAT: parses individual path components (bare and quoted) and validates them.
//! WHY: one path token owns one component grammar; keeping component logic in one module avoids
//!      duplication and makes validation rules easy to audit.

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, PathKind};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, TokenStream};

use super::PathComponents;

/// Boxed diagnostic result for the connected path-component family.
///
/// Component parsing and validation feed the public path parser, which unboxes once when it
/// returns the diagnostic to its tokenizer-facing caller.
type ComponentResult<T> = Result<T, Box<CompilerDiagnostic>>;

/// WHAT: Parsed result of one path component, with its raw text and whether it was quoted.
/// WHY: downstream validation needs to know whether quotes were used to allow spaces.
#[derive(Debug)]
pub(super) struct ParsedComponent {
    pub(super) value: String,
    pub(super) was_quoted: bool,
    pub(super) end_position: CharPosition,
}

/// WHAT: Parses exactly one path component (bare or quoted) from the current stream position.
/// WHY: ordinary paths use the one component grammar and escapes.
pub(super) fn parse_component(
    stream: &mut TokenStream,
    string_table: &StringTable,
) -> ComponentResult<ParsedComponent> {
    if stream.peek() == Some(&'"') {
        return parse_quoted_component(stream, string_table);
    }

    parse_bare_component(stream, string_table)
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
                end_position: stream.position,
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

/// WHAT: Parses an unquoted path component and stops at whitespace or a structural boundary.
/// WHY: Bare components remain unambiguous path tokens without internal whitespace, and the
///      path token itself is terminated by unquoted whitespace.
pub(super) fn parse_bare_component(
    stream: &mut TokenStream,
    _string_table: &StringTable,
) -> ComponentResult<ParsedComponent> {
    let mut value = String::new();

    while let Some(next) = stream.peek().copied() {
        if is_component_terminator(next) {
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

    let end_position = stream.position;

    Ok(ParsedComponent {
        value,
        was_quoted: false,
        end_position,
    })
}

/// WHAT: Characters that end one bare component and/or the whole path token.
/// WHY: separators continue the path; whitespace and structural delimiters end it so the
///      consuming parser owns selections, operators and old brace syntax.
fn is_component_terminator(character: char) -> bool {
    if character.is_whitespace() || matches!(character, '/' | '\\') {
        return true;
    }

    matches!(
        character,
        '[' | ']' | '{' | '}' | ',' | '(' | ')' | '<' | '>' | ':' | '"' | '|' | '?' | '*' | ';'
    )
}

/// Returns whether a canonical component can be emitted without quotes and parsed back unchanged.
///
/// Migration diagnostics use this predicate because canonical path rows retain component values,
/// not whether the author quoted them. Components that need quotes must not receive a lossy fix-it.
pub(super) fn can_serialize_bare_component(component: &str) -> bool {
    !component.is_empty()
        && !component.starts_with('@')
        && component
            .chars()
            .all(|character| !is_component_terminator(character))
        && component
            .chars()
            .all(|character| is_valid_component_char(character, false))
}

/// WHAT: Validates and interns one parsed component.
/// WHY: keeps all paths aligned on one validation boundary.
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

    // Reject a path component that starts with `@` after the path introducer was consumed.
    // WHAT: the leading `@` in `@path` is the path introducer consumed by the lexer. A second
    //      `@` starting any component (such as `@@pages` or `@helper/@home`) is not a valid
    //      module name. Normal module-root filenames are cosmetic filesystem markers.
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
