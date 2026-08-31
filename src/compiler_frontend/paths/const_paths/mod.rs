//! Moth path syntax parsing for path literals.
//!
//! This parser sits directly on tokenizer tokens and returns typed `CompilerDiagnostic` values for
//! user-authored path mistakes. Connected helpers and the public `parse_file_path` entry point
//! share one boxed diagnostic shape that flows directly into the tokenizer's result family.
//!
//! Path tokens are terminated by unquoted whitespace. Dependency selections are ordinary
//! identifier and punctuation tokens parsed by the header-owned dependency-clause parser,
//! never part of a path row.

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, PathKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    CharPosition, SourceLocation, Token, TokenKind, TokenStream,
};
use crate::return_token;

mod components;
type PathComponents = Vec<StringId>;

/// Returns whether a canonical path component has a lossless bare source spelling.
pub(crate) fn can_serialize_path_component_bare(component: &str) -> bool {
    components::can_serialize_bare_component(component)
}

#[derive(Debug)]
struct ParsedPathPrefix {
    components: PathComponents,
    last_component_end: Option<CharPosition>,
    ended_with_separator: bool,
}

pub fn parse_file_path(
    stream: &mut TokenStream,
    string_table: &mut StringTable,
) -> Result<Token, Box<CompilerDiagnostic>> {
    // Path syntax accepted by the tokenizer.
    //
    // Canonical examples:
    // @path/to/file
    // @docs/"my file.md"

    // WHAT: Tokenize exact `@/` as the empty canonical path.
    // WHY: in dependency position `@/` stays a rejected spelling whose owner reports its own
    //      diagnostic, while expression position now accepts bare `@/` as the structural
    //      site-root value. Tokenizing the spelling here lets each owner act on it instead of
    //      failing as an unrecognised path introducer.
    if stream.peek() == Some(&'/') {
        stream.next();

        match stream.peek().copied() {
            None => {
                let path_id = stream.path_syntax.push(
                    InternedPath::new(),
                    SourceLocation::new(
                        stream.file_path.to_owned(),
                        stream.start_position,
                        stream.position,
                    ),
                );
                return_token!(TokenKind::Path(path_id), stream);
            }
            Some(next) => {
                if next.is_whitespace() || matches!(next, ':' | ']' | ')' | '}' | ',' | ';') {
                    let path_id = stream.path_syntax.push(
                        InternedPath::new(),
                        SourceLocation::new(
                            stream.file_path.to_owned(),
                            stream.start_position,
                            stream.position,
                        ),
                    );
                    return_token!(TokenKind::Path(path_id), stream);
                }

                return Err(Box::new(CompilerDiagnostic::invalid_path(
                    PathKind::OnlyRootSlashSupported,
                    stream.new_location(),
                )));
            }
        }
    }

    let parsed_prefix = parse_path_prefix(stream, string_table)?;

    if parsed_prefix.components.is_empty() {
        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::Empty,
            stream.new_location(),
        )));
    }

    if parsed_prefix.ended_with_separator {
        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::TrailingSeparator,
            stream.new_location(),
        )));
    }

    let root = InternedPath::from_components(parsed_prefix.components);
    let path_location = SourceLocation::new(
        stream.file_path.to_owned(),
        stream.start_position,
        parsed_prefix
            .last_component_end
            .expect("a non-empty path has a final component position"),
    );

    let path_id = stream.path_syntax.push(root, path_location);
    return_token!(TokenKind::Path(path_id), stream)
}

/// WHAT: Parses the path components of one path token.
/// WHY: an unquoted whitespace or a non-component character after a component terminates the
///      path token; `/` and `\` continue it into another component.
fn parse_path_prefix(
    stream: &mut TokenStream,
    string_table: &mut StringTable,
) -> Result<ParsedPathPrefix, Box<CompilerDiagnostic>> {
    let mut components = Vec::with_capacity(2);
    let mut seen_non_relative_component = false;
    let mut ended_with_separator = false;
    let mut last_component_end = None;
    let mut expect_component = true;

    loop {
        if expect_component {
            let Some(next) = stream.peek().copied() else {
                return Ok(ParsedPathPrefix {
                    components,
                    last_component_end,
                    ended_with_separator,
                });
            };

            if next.is_whitespace() {
                let path_kind = if components.is_empty() {
                    PathKind::Empty
                } else {
                    PathKind::TrailingSeparator
                };
                return Err(Box::new(CompilerDiagnostic::invalid_path(
                    path_kind,
                    stream.new_location(),
                )));
            }

            if matches!(next, '/' | '\\') {
                return Err(Box::new(CompilerDiagnostic::invalid_path(
                    PathKind::EmptyComponent,
                    stream.new_location(),
                )));
            }

            let parsed_component = components::parse_component(stream, string_table)?;
            let component_end = parsed_component.end_position;
            components::push_validated_component(
                &mut components,
                parsed_component,
                true,
                &mut seen_non_relative_component,
                stream,
                string_table,
            )?;
            last_component_end = Some(component_end);

            expect_component = false;
            ended_with_separator = false;
            continue;
        }

        let Some(next) = stream.peek().copied() else {
            return Ok(ParsedPathPrefix {
                components,
                last_component_end,
                ended_with_separator,
            });
        };

        // Unquoted whitespace terminates the path token. The consuming parser diagnoses
        // a likely unquoted path component with a quote suggestion.
        if next.is_whitespace() {
            return Ok(ParsedPathPrefix {
                components,
                last_component_end,
                ended_with_separator,
            });
        }

        if matches!(next, '/' | '\\') {
            stream.next();
            expect_component = true;
            ended_with_separator = true;
            continue;
        }

        // Any other character ends the path token: structural delimiters, old selection
        // braces, template-head delimiters or an unrelated operator.
        return Ok(ParsedPathPrefix {
            components,
            last_component_end,
            ended_with_separator,
        });
    }
}

#[cfg(test)]
#[path = "../tests/paths_tests.rs"]
mod paths_tests;
