//! Moth path syntax parsing for path literals and imports.
//!
//! This parser sits directly on tokenizer tokens and returns typed `CompilerDiagnostic` values for
//! user-authored path mistakes. Connected helpers and the public `parse_file_path` entry point
//! share one boxed diagnostic shape that flows directly into the tokenizer's result family.

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, ImportClauseKind, InvalidImportClauseReason, PathKind,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::consume_non_newline_whitespace;
use crate::compiler_frontend::tokenizer::tokens::{
    PathTokenItem, SourceLocation, Token, TokenKind, TokenStream, TokenizeMode,
};
use crate::return_token;

mod components;
mod grouped;
mod import_clauses;

pub use import_clauses::*;

type PathComponents = Vec<StringId>;

/// WHAT: One expanded entry from a grouped block, with optional alias and source locations.
/// WHY: Grouped import aliases must preserve per-entry metadata through tokenization.
#[derive(Debug)]
struct GroupedPathExpansion {
    components: PathComponents,
    alias: Option<StringId>,
    path_location: SourceLocation,
    alias_location: Option<SourceLocation>,
}

type RelativePathExpansions = Vec<GroupedPathExpansion>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathStopReason {
    EndOfInput,
    Newline,
    TemplateHeadDelimiter,
    ConfigDelimiter,
    GroupStart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParseComponentContext {
    OrdinaryPath,
    GroupedEntry,
}

#[derive(Debug)]
struct ParsedPathPrefix {
    components: PathComponents,
    stop_reason: PathStopReason,
    ended_with_separator: bool,
}

/// WHAT: Result of parsing one grouped entry, including optional alias.
/// WHY: Grouped entries may end with `as alias`; this captures both the path
///      components and the alias metadata.
#[derive(Debug)]
struct ParsedGroupedEntry {
    components: PathComponents,
    ended_with_separator: bool,
    alias: Option<StringId>,
    path_location: SourceLocation,
    alias_location: Option<SourceLocation>,
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
    //
    // @docs {
    //     intro.md,
    //     "my folder"/"my file.md",
    //     guides {
    //         ownership.md,
    //         memory.md,
    //     },
    // }

    consume_non_newline_whitespace(stream);

    // WHAT: Accept exact `@/` as the singleton public-root path literal.
    // WHY: Site templates commonly need the public root itself, and the existing
    // empty `InternedPath` representation models that case cleanly without
    // expanding path grammar into a generic slash-prefixed family.
    if stream.peek() == Some(&'/') {
        stream.next();

        match stream.peek().copied() {
            None => return_token!(
                TokenKind::Path(vec![PathTokenItem {
                    path: InternedPath::new(),
                    alias: None,
                    path_location: SourceLocation::new(
                        stream.file_path.to_owned(),
                        stream.start_position,
                        stream.position
                    ),
                    alias_location: None,
                    from_grouped: false,
                }]),
                stream
            ),
            Some(next) => {
                if let Some(stop_reason) = ordinary_stop_reason(stream.mode, next)
                    && stop_reason != PathStopReason::GroupStart
                {
                    return_token!(
                        TokenKind::Path(vec![PathTokenItem {
                            path: InternedPath::new(),
                            alias: None,
                            path_location: SourceLocation::new(
                                stream.file_path.to_owned(),
                                stream.start_position,
                                stream.position
                            ),
                            alias_location: None,
                            from_grouped: false,
                        }]),
                        stream
                    );
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

    if parsed_prefix.ended_with_separator && parsed_prefix.stop_reason == PathStopReason::GroupStart
    {
        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::SlashBeforeGroup,
            stream.new_location(),
        )));
    }

    if parsed_prefix.ended_with_separator {
        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::TrailingSeparator,
            stream.new_location(),
        )));
    }

    let mut parsed_paths = Vec::with_capacity(1);

    if parsed_prefix.stop_reason != PathStopReason::GroupStart {
        let path = InternedPath::from_components(parsed_prefix.components);
        let path_location = SourceLocation::new(
            stream.file_path.to_owned(),
            stream.start_position,
            stream.position,
        );
        parsed_paths.push(PathTokenItem {
            path,
            alias: None,
            path_location,
            alias_location: None,
            from_grouped: false,
        });
        return_token!(TokenKind::Path(parsed_paths), stream);
    }

    // Consume the opening grouped brace so the grouped parser starts at the first entry.
    stream.next();
    let grouped_suffixes = grouped::parse_grouped_block(stream, string_table)?;

    for suffix in grouped_suffixes {
        let mut full_components = parsed_prefix.components.clone();
        full_components.extend(suffix.components);
        let path = InternedPath::from_components(full_components);
        parsed_paths.push(PathTokenItem {
            path,
            alias: suffix.alias,
            path_location: suffix.path_location,
            alias_location: suffix.alias_location,
            from_grouped: true,
        });
    }

    return_token!(TokenKind::Path(parsed_paths), stream)
}

/// WHAT: Parses the base path prefix before an optional grouped block.
/// WHY: Keeps context-sensitive ordinary-path stop conditions isolated from grouped expansion.
fn parse_path_prefix(
    stream: &mut TokenStream,
    string_table: &mut StringTable,
) -> Result<ParsedPathPrefix, Box<CompilerDiagnostic>> {
    let mut components = Vec::with_capacity(2);
    let mut seen_non_relative_component = false;
    let mut ended_with_separator = false;
    let mut expect_component = true;

    loop {
        if expect_component {
            consume_non_newline_whitespace(stream);

            let Some(next) = stream.peek().copied() else {
                return Ok(ParsedPathPrefix {
                    components,
                    stop_reason: PathStopReason::EndOfInput,
                    ended_with_separator,
                });
            };

            if let Some(stop_reason) = ordinary_stop_reason(stream.mode, next) {
                return Ok(ParsedPathPrefix {
                    components,
                    stop_reason,
                    ended_with_separator,
                });
            }

            if matches!(next, '/' | '\\') {
                return Err(Box::new(CompilerDiagnostic::invalid_path(
                    PathKind::EmptyComponent,
                    stream.new_location(),
                )));
            }

            let parsed_component = components::parse_component(
                stream,
                ParseComponentContext::OrdinaryPath,
                string_table,
            )?;
            components::push_validated_component(
                &mut components,
                parsed_component,
                true,
                &mut seen_non_relative_component,
                stream,
                string_table,
            )?;

            expect_component = false;
            ended_with_separator = false;
            continue;
        }

        let skipped_whitespace = consume_non_newline_whitespace(stream);

        let Some(next) = stream.peek().copied() else {
            return Ok(ParsedPathPrefix {
                components,
                stop_reason: PathStopReason::EndOfInput,
                ended_with_separator,
            });
        };

        if let Some(stop_reason) = ordinary_stop_reason(stream.mode, next) {
            return Ok(ParsedPathPrefix {
                components,
                stop_reason,
                ended_with_separator,
            });
        }

        // Stop path parsing at the `as` keyword (used in import aliases).
        // WHAT: `as` is a language keyword, not a valid path component.
        // WHY: without this, `import @path/symbol as alias` tokenizes the `as alias`
        //      as part of the path, producing a confusing "whitespace must be quoted" error.
        if next == 'a' && peek_keyword_as(stream) {
            return Ok(ParsedPathPrefix {
                components,
                stop_reason: PathStopReason::EndOfInput,
                ended_with_separator,
            });
        }

        if matches!(next, '/' | '\\') {
            stream.next();
            expect_component = true;
            ended_with_separator = true;
            continue;
        }

        if skipped_whitespace {
            return Err(Box::new(CompilerDiagnostic::invalid_path(
                PathKind::WhitespaceMustBeQuoted,
                stream.new_location(),
            )));
        }

        return Err(Box::new(CompilerDiagnostic::invalid_path(
            PathKind::MissingSeparator,
            stream.new_location(),
        )));
    }
}

/// WHAT: Peeks ahead to check if the stream currently points at the keyword `as`.
/// WHY: `as` is a language keyword used for import aliases and type aliases. It must not
///      be consumed as part of a path component.
fn peek_keyword_as(stream: &TokenStream) -> bool {
    // stream.peek() is already 'a'; check the next character.
    let mut chars = stream.chars.clone();
    chars.next(); // skip 'a'
    let Some(second) = chars.next() else {
        return false;
    };
    if second != 's' {
        return false;
    }
    // `as` must be followed by whitespace, a path terminator, or EOF.
    match chars.next() {
        None => true,
        Some(c) => c.is_whitespace() || ordinary_stop_reason(stream.mode, c).is_some(),
    }
}

fn ordinary_stop_reason(mode: TokenizeMode, character: char) -> Option<PathStopReason> {
    if matches!(character, '\n' | '\r') {
        return Some(PathStopReason::Newline);
    }

    if mode == TokenizeMode::TemplateHead && matches!(character, ']' | ':') {
        return Some(PathStopReason::TemplateHeadDelimiter);
    }

    if matches!(character, ',' | '}') {
        return Some(PathStopReason::ConfigDelimiter);
    }

    if character == '{' {
        return Some(PathStopReason::GroupStart);
    }

    None
}

/// WHAT: Checks whether the current character ends a grouped entry component.
/// WHY: Entries stop at commas, braces, or the `as` keyword. The `as` check needs
///      the stream to peek ahead and verify it appears before a new component, not inside a
///      component like `ExportedAlias`.
fn is_grouped_entry_stop_char(
    stream: &TokenStream,
    character: char,
    component_is_empty: bool,
) -> bool {
    if matches!(character, ',' | '}' | '{') {
        return true;
    }

    if component_is_empty && character == 'a' && peek_keyword_as(stream) {
        return true;
    }

    false
}

/// WHAT: Consumes the `as` keyword from the stream.
/// WHY: After `peek_keyword_as` confirms the keyword is present, this advances past it.
fn consume_keyword_as(stream: &mut TokenStream) {
    stream.next(); // 'a'
    stream.next(); // 's'
}

fn is_component_terminator(
    stream: &TokenStream,
    context: ParseComponentContext,
    character: char,
    component_is_empty: bool,
) -> bool {
    if matches!(character, '/' | '\\') {
        return true;
    }

    match context {
        ParseComponentContext::OrdinaryPath => {
            ordinary_stop_reason(stream.mode, character).is_some()
        }
        ParseComponentContext::GroupedEntry => {
            is_grouped_entry_stop_char(stream, character, component_is_empty)
        }
    }
}

#[cfg(test)]
#[path = "../tests/paths_tests.rs"]
mod paths_tests;
