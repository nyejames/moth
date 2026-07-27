//! Import clause parsing over path tokens.
//!
//! WHAT: validates alias placement and expands path-token items into import records.
//! WHY: Stage 0 import scanning and header parsing need the same alias-aware clause
//! parser, but this logic is separate from raw path-token lexing.

use super::*;

use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

/// Boxed diagnostic result for the connected import-clause family.
///
/// Stage 0 import scanning and header import preparation share these parsers. One error shape lets
/// header parsing propagate diagnostics directly while Stage 0 adapts once to its discovery error.
type ImportClauseResult<T> = Result<T, Box<CompilerDiagnostic>>;

/// WHAT: one parsed provider path paired with its exact source location.
/// WHY: Stage 0 reachable discovery and retained header import shells both need the provider
///      path and the source position that introduced it. Keeping them in one type-distinct value
///      separates structural provider references from imported-symbol alias/export metadata at
///      the shared import-clause syntax boundary, so Stage 0 can carry the graph boundary
///      location alongside the path it resolves today.
#[derive(Clone, Debug, PartialEq)]
pub struct StructuralProviderReference {
    pub path: InternedPath,
    pub path_location: SourceLocation,
    /// Whether the last path component is an imported item rather than part of the provider.
    ///
    /// WHAT: grouped syntax such as `@helper { greet }` is tokenized as the complete path
    ///       `@helper/greet`; this flag preserves that `greet` is the requested item while
    ///       `@helper` is the structural provider.
    /// WHY: Stage 0 namespace resolution must distinguish a grouped public-surface import from
    ///      the directly authored private path `@helper/greet` without guessing from path shape.
    pub from_grouped: bool,
}

impl StructuralProviderReference {
    /// Remap the interned path and source location into a merged string table.
    ///
    /// WHAT: shifts the `InternedPath` and `SourceLocation` string IDs after a string-table merge.
    /// WHY: per-file frontend preparation uses local string tables; the nested reference remaps
    ///      exactly once when its owning `FileImport` or `ScannedImportSource` is merged into the
    ///      module or global table.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.path.remap_string_ids(remap);
        self.path_location.remap_string_ids(remap);
    }
}

#[derive(Clone, Debug)]
pub struct ParsedImportItem {
    /// Structural provider reference carrying the parsed path and its source location.
    pub provider: StructuralProviderReference,
    pub alias: Option<StringId>,
    pub alias_location: Option<SourceLocation>,
    pub from_grouped: bool,
}

pub fn parse_import_clause_items(
    tokens: &[Token],
    start_index: usize,
    _string_table: &StringTable,
) -> ImportClauseResult<(Vec<ParsedImportItem>, usize)> {
    parse_path_clause_items(tokens, start_index)
}

fn parse_path_clause_items(
    tokens: &[Token],
    start_index: usize,
) -> ImportClauseResult<(Vec<ParsedImportItem>, usize)> {
    let Some(clause_token) = tokens.get(start_index) else {
        return Err(Box::new(CompilerDiagnostic::invalid_import_clause(
            ImportClauseKind::Import,
            InvalidImportClauseReason::MissingPath,
            SourceLocation::default(),
        )));
    };

    if clause_token.kind != TokenKind::Import {
        return Err(Box::new(CompilerDiagnostic::invalid_import_clause(
            ImportClauseKind::Import,
            InvalidImportClauseReason::ExpectedPath,
            clause_token.location.clone(),
        )));
    }

    let mut index = start_index + 1;
    while tokens
        .get(index)
        .is_some_and(|token| matches!(token.kind, TokenKind::Newline))
    {
        index += 1;
    }

    let Some(path_token) = tokens.get(index) else {
        return Err(Box::new(CompilerDiagnostic::invalid_import_clause(
            ImportClauseKind::Import,
            InvalidImportClauseReason::MissingPath,
            clause_token.location.clone(),
        )));
    };

    let TokenKind::Path(items) = &path_token.kind else {
        return Err(Box::new(CompilerDiagnostic::invalid_import_clause(
            ImportClauseKind::Import,
            InvalidImportClauseReason::ExpectedPath,
            path_token.location.clone(),
        )));
    };

    let mut index = index + 1;
    let mut trailing_alias: Option<StringId> = None;
    let mut trailing_alias_location: Option<SourceLocation> = None;

    // Check for `as alias_name` after the path token.
    if tokens
        .get(index)
        .is_some_and(|token| matches!(token.kind, TokenKind::As))
    {
        index += 1;
        let Some(alias_token) = tokens.get(index) else {
            return Err(Box::new(CompilerDiagnostic::invalid_import_clause(
                ImportClauseKind::Alias,
                InvalidImportClauseReason::MissingAlias,
                path_token.location.clone(),
            )));
        };
        let TokenKind::Symbol(alias_name) = alias_token.kind else {
            return Err(Box::new(CompilerDiagnostic::invalid_import_clause(
                ImportClauseKind::Alias,
                InvalidImportClauseReason::ExpectedAliasName,
                alias_token.location.clone(),
            )));
        };
        let path_uses_grouped_syntax = items.iter().any(|item| item.from_grouped);

        if path_uses_grouped_syntax {
            let reason = if items.iter().any(|item| item.alias.is_some()) {
                InvalidImportClauseReason::PerEntryAndTrailingAlias
            } else {
                InvalidImportClauseReason::GroupedWithTrailingAlias
            };

            return Err(Box::new(CompilerDiagnostic::invalid_import_clause(
                ImportClauseKind::Grouped,
                reason,
                alias_token.location.clone(),
            )));
        }
        trailing_alias = Some(alias_name);
        trailing_alias_location = Some(alias_token.location.clone());
        index += 1;

        // Reject a second trailing `as` in single-import clauses.
        if tokens
            .get(index)
            .is_some_and(|token| matches!(token.kind, TokenKind::As))
        {
            return Err(Box::new(CompilerDiagnostic::invalid_import_clause(
                ImportClauseKind::Alias,
                InvalidImportClauseReason::MultipleTrailingAliases,
                tokens[index].location.clone(),
            )));
        }
    }

    let parsed_items = items
        .iter()
        .map(|item| ParsedImportItem {
            provider: StructuralProviderReference {
                path: item.path.clone(),
                path_location: item.path_location.clone(),
                from_grouped: item.from_grouped,
            },
            alias: item.alias.or(trailing_alias),
            alias_location: item
                .alias_location
                .clone()
                .or(trailing_alias_location.clone()),
            from_grouped: item.from_grouped,
        })
        .collect();

    Ok((parsed_items, index))
}

/// Collect structural provider references from every top-level import clause in a token stream.
///
/// WHAT: walks authored tokens, skips imports inside an `export:` block, and returns one
/// `StructuralProviderReference` per parsed import path with its exact source location.
/// WHY: Stage 0 reachable discovery consumes these values directly, using `path` for current
///      resolution while retaining `path_location` for the graph boundary. Header import
///      preparation uses `parse_import_clause_items` when it also needs alias metadata.
pub fn collect_provider_references_from_tokens(
    tokens: &[Token],
) -> ImportClauseResult<Vec<StructuralProviderReference>> {
    let mut references = Vec::new();
    let mut index = 0usize;

    while index < tokens.len() {
        if matches!(tokens[index].kind, TokenKind::Import) {
            if previous_significant_token_kind(tokens, index)
                .is_some_and(|kind| matches!(kind, TokenKind::Export))
            {
                // Stage 0 only gathers reachable files. Imports inside an `export:` block are
                // not separate top-level imports; they remain ordinary Import tokens after the
                // block colon and are recorded by header export-block handling instead.
                index += 1;
                continue;
            }

            let (items, next_index) = parse_path_clause_items(tokens, index)?;
            references.extend(items.into_iter().map(|item| item.provider));
            index = next_index;
            continue;
        }

        index += 1;
    }

    Ok(references)
}

fn previous_significant_token_kind(tokens: &[Token], index: usize) -> Option<&TokenKind> {
    tokens[..index]
        .iter()
        .rev()
        .find(|token| !matches!(token.kind, TokenKind::Newline))
        .map(|token| &token.kind)
}
