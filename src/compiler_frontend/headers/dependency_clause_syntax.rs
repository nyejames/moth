//! Dependency clause parsing over path tokens.
//!
//! WHAT: validates alias placement and preserves one provider root plus its direct selections.
//! WHY: Stage 0 discovery and header preparation need the same clause-owned semantic
//!      facts; neither stage should expand a clause into provider bindings.

use super::top_level_classifier::classify_symbol_statement_start_at;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DependencyClauseKind, InvalidDependencyClauseReason,
};
use crate::compiler_frontend::headers::dependency_target::DependencyTargetKind;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::symbols::identity::{DependencyShellId, FileId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::tokenizer::tokens::{SourceLocation, Token, TokenKind};
use rustc_hash::FxHashSet;

/// Scanner-local error boundary for dependency-clause parsing.
///
/// WHAT: separates authored syntax diagnostics from malformed retained-data lookup.
/// WHY: a stale or absent path handle is internal compiler corruption, not user syntax. The
///      header parser converts either lane into `HeaderParseFailure` at the owning boundary.
#[derive(Debug)]
pub(crate) enum DependencyClauseParseError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(CompilerError),
}

impl From<Box<CompilerDiagnostic>> for DependencyClauseParseError {
    fn from(diagnostic: Box<CompilerDiagnostic>) -> Self {
        Self::Diagnostic(diagnostic)
    }
}

impl From<CompilerDiagnostic> for DependencyClauseParseError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        Self::Diagnostic(Box::new(diagnostic))
    }
}

impl From<CompilerError> for DependencyClauseParseError {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error)
    }
}

/// Result boundary for the connected dependency-clause family.
///
/// WHAT: carries authored-syntax diagnostics and infrastructure failures through one scanner
///       result type.
/// WHY: clause parsing can fail on user syntax or on a stale path handle; later header
///      conversion needs both lanes without treating infrastructure as a diagnostic.
type DependencyClauseResult<T> = Result<T, DependencyClauseParseError>;

/// One optional local alias with the source location that introduced its name.
///
/// WHAT: keeps an alias name and its diagnostic span inseparable while dependency syntax is
///       transferred from path scanning into retained header facts.
/// WHY: collision diagnostics must point at the alias itself, not at the whole dependency clause.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DependencyAlias {
    pub name: StringId,
    pub location: SourceLocation,
}

impl DependencyAlias {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
        self.location.remap_string_ids(remap);
    }

    pub fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.location.rebind_source_identity(logical_path);
    }
}

/// One provider root produced by the shared clause scanner before header preparation stamps the
/// retained shell identity.
#[derive(Clone, Debug, PartialEq)]
pub struct ScannedDependencyProvider {
    pub path: InternedPath,
    pub path_location: SourceLocation,
}

/// The consolidated path authority for one retained dependency clause.
///
/// WHAT: stores one shell, one structural path, one target classification and one path location.
/// WHY: later stages must not rediscover the provider boundary from path spelling.
#[derive(Clone, Debug, PartialEq)]
pub struct RetainedDependencyPath {
    pub dependency_shell_id: DependencyShellId,
    pub path: InternedPath,
    pub target: DependencyTargetKind,
    pub location: SourceLocation,
}

impl RetainedDependencyPath {
    /// Remap the path components, target extension and location into a merged string table.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.path.remap_string_ids(remap);
        self.target.remap_string_ids(remap);
        self.location.remap_string_ids(remap);
    }

    /// Commit the final source identity while preserving module-root-relative paths.
    pub fn commit_source_rebinding(&mut self, file_id: FileId, logical_path: &InternedPath) {
        self.location.rebind_source_identity(logical_path);
        self.dependency_shell_id.source = file_id;
    }
}

/// One direct public-surface name selected from a provider root.
#[derive(Clone, Debug, PartialEq)]
pub struct ScannedDependencySelection {
    pub source_name: StringId,
    pub source_location: SourceLocation,
    pub local_alias: Option<DependencyAlias>,
}

/// Mutually exclusive binding modes produced by the shared dependency-clause scanner.
#[derive(Clone, Debug, PartialEq)]
pub enum ScannedDependencyBinding {
    Namespace {
        alias: Option<DependencyAlias>,
    },
    DirectSelections {
        selections: Vec<ScannedDependencySelection>,
    },
}

/// The complete syntax payload of one authored dependency clause before string-table remapping
/// and shell assignment.
#[derive(Clone, Debug, PartialEq)]
pub struct ScannedDependencyClause {
    pub provider: ScannedDependencyProvider,
    pub binding: ScannedDependencyBinding,
}

/// Parse one dependency clause into one provider root and a flat direct-selection list.
pub(crate) fn parse_dependency_clause(
    tokens: &[Token],
    start_index: usize,
    path_syntax: &PathSyntaxTable,
) -> DependencyClauseResult<(ScannedDependencyClause, usize)> {
    let Some(path_token) = tokens.get(start_index) else {
        return Err(CompilerDiagnostic::invalid_dependency_clause(
            DependencyClauseKind::Namespace,
            InvalidDependencyClauseReason::MissingPath,
            SourceLocation::default(),
        )
        .into());
    };

    let TokenKind::Path(path_id) = &path_token.kind else {
        return Err(CompilerDiagnostic::invalid_dependency_clause(
            DependencyClauseKind::Namespace,
            InvalidDependencyClauseReason::ExpectedPath,
            path_token.location.clone(),
        )
        .into());
    };
    let path_syntax_row = path_syntax.try_path_for_token(*path_id, &path_token.location)?;
    let mut index = start_index + 1;

    let provider = ScannedDependencyProvider {
        path: path_syntax_row.root.clone(),
        path_location: path_syntax_row.location.clone(),
    };

    if clause_ended(tokens.get(index)) {
        return Ok((
            ScannedDependencyClause {
                provider,
                binding: ScannedDependencyBinding::Namespace { alias: None },
            },
            index,
        ));
    }

    if tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::As)
    {
        let alias_keyword_location = tokens[index].location.clone();
        index += 1;
        let Some(alias_token) = tokens.get(index) else {
            return Err(dependency_clause_error(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::MissingAlias,
                alias_keyword_location,
            ));
        };
        let TokenKind::Symbol(alias_name) = alias_token.kind else {
            return Err(dependency_clause_error(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::ExpectedAliasName,
                alias_token.location.clone(),
            ));
        };
        let alias = DependencyAlias {
            name: alias_name,
            location: alias_token.location.clone(),
        };
        index += 1;
        if !clause_ended(tokens.get(index)) {
            return Err(dependency_clause_error(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::NamespaceAliasWithSelections,
                tokens[index].location.clone(),
            ));
        }
        return Ok((
            ScannedDependencyClause {
                provider,
                binding: ScannedDependencyBinding::Namespace { alias: Some(alias) },
            },
            index,
        ));
    }

    reject_legacy_or_delimited_selection(tokens.get(index))?;

    let mut selections = Vec::new();
    let mut selected_source_names = FxHashSet::default();
    let mut selected_local_names = FxHashSet::default();
    let mut continuation_comma = None;
    loop {
        let selection_continuation_comma = continuation_comma.take();
        let Some(selection_token) = tokens.get(index) else {
            return Err(dependency_clause_error(
                DependencyClauseKind::DirectSelection,
                InvalidDependencyClauseReason::MissingSelectionAfterComma,
                path_token.location.clone(),
            ));
        };
        let TokenKind::Symbol(source_name) = selection_token.kind else {
            return Err(dependency_clause_error(
                DependencyClauseKind::DirectSelection,
                InvalidDependencyClauseReason::ExpectedSelectionName,
                selection_continuation_comma
                    .clone()
                    .unwrap_or_else(|| selection_token.location.clone()),
            ));
        };
        if tokens
            .get(index + 1)
            .is_some_and(|token| token.kind == TokenKind::Dot)
        {
            return Err(CompilerDiagnostic::invalid_path(
                crate::compiler_frontend::compiler_messages::PathKind::WhitespaceMustBeQuoted,
                selection_token.location.clone(),
            )
            .into());
        }
        let source_location = selection_token.location.clone();
        index += 1;

        let local_alias = if tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::As)
        {
            index += 1;
            let Some(alias_token) = tokens.get(index) else {
                return Err(dependency_clause_error(
                    DependencyClauseKind::NamespaceAlias,
                    InvalidDependencyClauseReason::MissingAlias,
                    source_location,
                ));
            };
            let TokenKind::Symbol(alias_name) = alias_token.kind else {
                return Err(dependency_clause_error(
                    DependencyClauseKind::NamespaceAlias,
                    InvalidDependencyClauseReason::ExpectedAliasName,
                    alias_token.location.clone(),
                ));
            };
            index += 1;
            Some(DependencyAlias {
                name: alias_name,
                location: alias_token.location.clone(),
            })
        } else {
            None
        };

        if !selected_source_names.insert(source_name) {
            return Err(dependency_clause_error(
                DependencyClauseKind::DirectSelection,
                InvalidDependencyClauseReason::DuplicateSelectionName,
                source_location,
            ));
        }
        let local_name = local_alias.as_ref().map_or(source_name, |alias| alias.name);
        if !selected_local_names.insert(local_name) {
            return Err(dependency_clause_error(
                DependencyClauseKind::DirectSelection,
                InvalidDependencyClauseReason::DuplicateSelectionLocalName,
                local_alias
                    .as_ref()
                    .map_or_else(|| source_location.clone(), |alias| alias.location.clone()),
            ));
        }

        selections.push(ScannedDependencySelection {
            source_name,
            source_location: source_location.clone(),
            local_alias,
        });

        match tokens.get(index).map(|token| &token.kind) {
            Some(TokenKind::Comma) => {
                let comma_location = tokens[index].location.clone();
                continuation_comma = Some(comma_location.clone());
                index += 1;
                while tokens
                    .get(index)
                    .is_some_and(|token| token.kind == TokenKind::Newline)
                {
                    index += 1;
                }
                if clause_ended(tokens.get(index)) {
                    return Err(dependency_clause_error(
                        DependencyClauseKind::DirectSelection,
                        InvalidDependencyClauseReason::MissingSelectionAfterComma,
                        comma_location,
                    ));
                }
                reject_legacy_or_delimited_selection(tokens.get(index))?;
            }
            Some(TokenKind::Newline | TokenKind::End | TokenKind::Eof) | None => break,
            _ => {
                // If a continuation comma kept the clause open and the next token starts a
                // declaration, the comma consumed this name as a dependency selection. Report
                // the actual parse decision instead of a generic missing-comma diagnostic.
                if let Some(comma_location) = &selection_continuation_comma
                    && classify_symbol_statement_start_at(tokens, index)
                        .starts_statement_after_dependency_selection()
                {
                    return Err(continuation_entered_statement_error(
                        source_location,
                        comma_location.clone(),
                    ));
                }
                return Err(dependency_clause_error(
                    DependencyClauseKind::DirectSelection,
                    InvalidDependencyClauseReason::MissingCommaBetweenSelections,
                    tokens[index].location.clone(),
                ));
            }
        }
    }

    Ok((
        ScannedDependencyClause {
            provider,
            binding: ScannedDependencyBinding::DirectSelections { selections },
        },
        index,
    ))
}

fn clause_ended(token: Option<&Token>) -> bool {
    token.is_none_or(|token| {
        matches!(
            token.kind,
            TokenKind::Newline | TokenKind::End | TokenKind::Eof
        )
    })
}

fn reject_legacy_or_delimited_selection(token: Option<&Token>) -> DependencyClauseResult<()> {
    let Some(token) = token else {
        return Ok(());
    };
    let reason = match token.kind {
        TokenKind::OpenCurly => InvalidDependencyClauseReason::LegacyBraceSelections,
        TokenKind::OpenParenthesis | TokenKind::TypeParameterBracket | TokenKind::Colon => {
            InvalidDependencyClauseReason::InvalidSelectionDelimiter
        }
        _ => return Ok(()),
    };
    Err(dependency_clause_error(
        DependencyClauseKind::DirectSelection,
        reason,
        token.location.clone(),
    ))
}

/// Build a continuation-entered-statement diagnostic with primary and secondary labels.
///
/// The primary span is the consumed selected/declaration name. The secondary span is the
/// comma that kept the clause open. The reason is payload-free because the primary span
/// already identifies the name.
fn continuation_entered_statement_error(
    selected_name_location: SourceLocation,
    comma_location: SourceLocation,
) -> DependencyClauseParseError {
    let diagnostic = CompilerDiagnostic::invalid_dependency_clause(
        DependencyClauseKind::DirectSelection,
        InvalidDependencyClauseReason::ContinuationEnteredStatement,
        selected_name_location.clone(),
    )
    .with_labels(vec![
        crate::compiler_frontend::compiler_messages::DiagnosticLabel::primary(
            selected_name_location,
        ),
        crate::compiler_frontend::compiler_messages::DiagnosticLabel::secondary(
            comma_location,
            None,
        ),
    ]);
    DependencyClauseParseError::Diagnostic(Box::new(diagnostic))
}

#[cfg(test)]
#[path = "tests/dependency_clause_syntax_tests.rs"]
mod dependency_clause_syntax_tests;

fn dependency_clause_error(
    kind: DependencyClauseKind,
    reason: InvalidDependencyClauseReason,
    location: SourceLocation,
) -> DependencyClauseParseError {
    CompilerDiagnostic::invalid_dependency_clause(kind, reason, location).into()
}
