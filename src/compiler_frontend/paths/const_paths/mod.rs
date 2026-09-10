//! Moth path syntax parsing for path literals.
//!
//! This parser sits directly on tokenizer tokens and returns typed `CompilerDiagnostic` values for
//! user-authored path mistakes. The lexical entry point also propagates token span infrastructure
//! failures through the shared tokenizer result; component grammar helpers own only diagnostics.
//!
//! Path tokens are terminated by unquoted whitespace. Dependency selections are ordinary
//! identifier and punctuation tokens parsed by the header-owned dependency-clause parser,
//! never part of a path row.

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, PathKind};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::{TokenizeResult, current_source_span, mint_token};
use crate::compiler_frontend::tokenizer::tokens::{Token, TokenKind, TokenStream};

mod components;
type PathComponents = Vec<StringId>;

/// Returns whether a canonical path component has a lossless bare source spelling.
pub(crate) fn can_serialize_path_component_bare(component: &str) -> bool {
    components::can_serialize_bare_component(component)
}

#[derive(Debug)]
struct ParsedPathPrefix {
    components: PathComponents,
    ended_with_separator: bool,
}

pub fn parse_file_path(
    stream: &mut TokenStream,
    string_table: &mut StringTable,
) -> TokenizeResult<Token> {
    // Path syntax accepted by the tokenizer.
    //
    // Canonical examples:
    // @path/to/file
    // @docs/"my file.md"

    // WHAT: Tokenize exact `@/` as the empty canonical path.
    // WHY: in dependency position `@/` stays a rejected spelling whose owner reports its own
    //      diagnostic, while expression position now accepts bare `@/` as the structural
    //      site-root value.
    if stream.peek() == Some(&'/') {
        stream.next();

        match stream.peek().copied() {
            None => return mint_path_token(stream, InternedPath::new()),
            Some(next) => {
                if next.is_whitespace() || matches!(next, ':' | ']' | ')' | '}' | ',' | ';') {
                    return mint_path_token(stream, InternedPath::new());
                }

                return Err(CompilerDiagnostic::invalid_path(
                    PathKind::OnlyRootSlashSupported,
                    Some(current_source_span(stream)?),
                )
                .into());
            }
        }
    }

    let parsed_prefix = parse_path_prefix(stream, string_table)?;

    if parsed_prefix.components.is_empty() {
        return Err(CompilerDiagnostic::invalid_path(
            PathKind::Empty,
            Some(current_source_span(stream)?),
        )
        .into());
    }

    if parsed_prefix.ended_with_separator {
        return Err(CompilerDiagnostic::invalid_path(
            PathKind::TrailingSeparator,
            Some(current_source_span(stream)?),
        )
        .into());
    }

    let root = InternedPath::from_components(parsed_prefix.components);
    mint_path_token(stream, root)
}

/// Mint one span and share it between the path token and its source-owned syntax row.
fn mint_path_token(stream: &mut TokenStream<'_>, root: InternedPath) -> TokenizeResult<Token> {
    let mut token = mint_token(stream, TokenKind::Path(PathSyntaxId::NONE))?;
    let source_span = crate::compiler_frontend::source::SourceSpan::new(stream.file_id, token.span);
    let id = stream.path_syntax.push(root, source_span);
    token.kind = TokenKind::Path(id);
    Ok(token)
}

/// WHAT: Parses the path components of one path token.
/// WHY: an unquoted whitespace or a non-component character after a component terminates the
///      path token; `/` and `\` continue it into another component.
fn parse_path_prefix(
    stream: &mut TokenStream,
    string_table: &mut StringTable,
) -> TokenizeResult<ParsedPathPrefix> {
    let mut components = Vec::with_capacity(2);
    let mut seen_non_relative_component = false;
    let mut ended_with_separator = false;
    let mut expect_component = true;

    loop {
        if expect_component {
            let Some(next) = stream.peek().copied() else {
                return Ok(ParsedPathPrefix {
                    components,
                    ended_with_separator,
                });
            };

            if next.is_whitespace() {
                let path_kind = if components.is_empty() {
                    PathKind::Empty
                } else {
                    PathKind::TrailingSeparator
                };
                return Err(CompilerDiagnostic::invalid_path(
                    path_kind,
                    Some(current_source_span(stream)?),
                )
                .into());
            }

            if matches!(next, '/' | '\\') {
                return Err(CompilerDiagnostic::invalid_path(
                    PathKind::EmptyComponent,
                    Some(current_source_span(stream)?),
                )
                .into());
            }

            let parsed_component = components::parse_component(stream, string_table)?;
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

        let Some(next) = stream.peek().copied() else {
            return Ok(ParsedPathPrefix {
                components,
                ended_with_separator,
            });
        };

        // Unquoted whitespace terminates the path token.
        if next.is_whitespace() {
            return Ok(ParsedPathPrefix {
                components,
                ended_with_separator,
            });
        }

        if matches!(next, '/' | '\\') {
            stream.next();
            expect_component = true;
            ended_with_separator = true;
            continue;
        }

        // Any other character ends the path token: structural delimiters, old selection braces,
        // template-head delimiters or an unrelated operator.
        return Ok(ParsedPathPrefix {
            components,
            ended_with_separator,
        });
    }
}

#[cfg(test)]
#[path = "../tests/paths_tests.rs"]
mod paths_tests;
