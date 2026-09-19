//! Dependency clause parsing over path tokens.
//!
//! WHAT: validates alias placement and preserves one provider root plus its direct selections.
//! WHY: Stage 0 discovery and header preparation need the same clause-owned semantic
//!      facts; neither stage should expand a clause into provider bindings.

use super::top_level_classifier::classify_symbol_statement_start_at_scanned;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DependencyClauseKind, InvalidDependencyClauseReason,
};
use crate::compiler_frontend::headers::dependency_target::{
    CheckedExternalProviderTarget, DependencyTargetKind,
};
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::utilities::token_scan::{ScannedToken, TokenFactView};
use rustc_hash::FxHashSet;

/// Scanner-local error boundary for dependency-clause parsing.
///
/// WHAT: separates authored syntax diagnostics from malformed retained-data lookup.
/// WHY: a stale or absent path handle is internal compiler corruption, not user syntax. The
///      header parser converts either lane into `HeaderParseFailure` at the owning boundary.
#[derive(Debug)]
pub(crate) enum DependencyClauseParseError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(CompilerError),
}

impl From<CompilerDiagnostic> for DependencyClauseParseError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        Self::Diagnostic(diagnostic)
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

/// One optional local alias with the exact source span that introduced its name.
#[derive(Clone, Debug)]
pub struct DependencyAlias {
    pub name: StringId,
    pub span: SourceSpan,
}

impl DependencyAlias {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
    }
}

/// One provider root produced by the shared clause scanner before header preparation stamps the
/// retained shell identity.
#[derive(Clone, Debug)]
pub struct ScannedDependencyProvider {
    pub path: PathId,
    pub path_syntax: PathSyntaxId,
    pub path_span: SourceSpan,
}

/// The consolidated path authority for one retained dependency clause.
#[derive(Clone, Debug)]
pub struct RetainedDependencyPath {
    pub dependency_shell_id: DependencyShellId,
    pub path: PathId,
    pub path_syntax: PathSyntaxId,
    pub target: DependencyTargetKind,
    /// Checked provider structure retained while the worker-local stores are available.
    pub provider_target: Option<CheckedExternalProviderTarget>,
    /// Final compiler identity for a same-module source edge, when Stage 0 has resolved it.
    pub local_source_id: Option<SourceId>,
    pub span: SourceSpan,
}

impl RetainedDependencyPath {
    /// Remap the target extension and checked suffix identities into a merged string table.
    ///
    /// The complete-path identity and provider prefix remap separately through `remap_path_ids`
    /// after the string delta merges, mirroring the file-owned path-table order (strings first,
    /// then paths).
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.target.remap_string_ids(remap);
        if let Some(provider_target) = &mut self.provider_target {
            provider_target.remap_string_ids(remap);
        }
    }

    /// Remap the complete-path identity and checked provider prefix after its path fork merges.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.path = remap.get(self.path);
        if let Some(provider_target) = &mut self.provider_target {
            provider_target.remap_path_ids(remap);
        }
    }

    /// Commit the final source identity while preserving module-root-relative paths.
    pub fn commit_source_rebinding(&mut self, file_id: SourceId, _logical_path: PathId) {
        self.span = SourceSpan::new(file_id, self.span.local());
        self.dependency_shell_id.source = file_id;
    }
}

/// One direct public-surface name selected from a provider root.
#[derive(Clone, Debug)]
pub struct ScannedDependencySelection {
    pub source_name: StringId,
    pub source_span: SourceSpan,
    pub local_alias: Option<DependencyAlias>,
}

/// Mutually exclusive binding modes produced by the shared dependency-clause scanner.
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
pub struct ScannedDependencyClause {
    pub provider: ScannedDependencyProvider,
    pub binding: ScannedDependencyBinding,
}


/// Canonical-source dependency-clause scan.
///
/// WHAT: parses one clause from source-owned shapes/spans on demand for header preparation.
/// WHY: production dependency facts use the canonical owner without collecting a source slice or
/// retaining durable references.
pub(crate) fn parse_dependency_clause_at_source(
    tokens: &crate::compiler_frontend::tokenizer::tokens::SourceTokens,
    start_offset: usize,
    path_syntax: &PathSyntaxTable,
    source_id: SourceId,
) -> DependencyClauseResult<(ScannedDependencyClause, usize)> {
    if tokens.source() != source_id {
        return Err(DependencyClauseParseError::Infrastructure(
            CompilerError::compiler_error(
                "canonical dependency scan source identity does not match its caller",
            ),
        ));
    }
    parse_dependency_clause_scanned(
        TokenFactView::from_source(tokens),
        start_offset,
        path_syntax,
        source_id,
    )
}

fn checked_next_clause_index(index: usize) -> DependencyClauseResult<usize> {
    index.checked_add(1).ok_or_else(|| {
        DependencyClauseParseError::Infrastructure(CompilerError::compiler_error(
            "dependency clause token index overflowed its source range",
        ))
    })
}

/// Borrowed-view dependency-clause scan over short-lived indexed facts.
///
/// WHAT: parses one clause from `TokenFactView` facts with slice-shaped offsets,
///       preserving path IDs, symbol IDs, export/dependency boundaries, EOF/newline
///       behavior and malformed-handle infrastructure lanes. Only the visited clause
///       tokens are read; no per-query whole-source projection occurs.
/// WHY: the same indexed body serves bounded slices and canonical `SourceTokens` without a
///      second scan body or durable references.
pub(crate) fn parse_dependency_clause_scanned(
    tokens: TokenFactView<'_>,
    start_offset: usize,
    path_syntax: &PathSyntaxTable,
    source_id: SourceId,
) -> DependencyClauseResult<(ScannedDependencyClause, usize)> {
    let Some(path_token) = tokens.get(start_offset) else {
        return Err(CompilerDiagnostic::invalid_dependency_clause(
            DependencyClauseKind::Namespace,
            InvalidDependencyClauseReason::MissingPath,
            None,
        )
        .into());
    };

    let Some(path_id) = path_token
        .path_syntax_id
        .filter(|_| path_token.tag == TokenTag::PATH)
    else {
        return Err(CompilerDiagnostic::invalid_dependency_clause(
            DependencyClauseKind::Namespace,
            InvalidDependencyClauseReason::ExpectedPath,
            Some(SourceSpan::new(source_id, path_token.span)),
        )
        .into());
    };
    let path_span = SourceSpan::new(source_id, path_token.span);
    let path_syntax_row = path_syntax.try_path_for_token(path_id, path_span)?;
    let source_id = path_span.source();
    let mut index = checked_next_clause_index(start_offset)?;

    let provider = ScannedDependencyProvider {
        path: path_syntax_row.root,
        path_syntax: path_id,
        path_span,
    };

    if clause_ended_scanned(tokens.get(index)) {
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
        .is_some_and(|token| token.tag == TokenTag::AS)
    {
        let Some(alias_keyword) = tokens.get(index) else {
            return Err(dependency_clause_error(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::MissingAlias,
                None,
            ));
        };
        let alias_keyword_span = SourceSpan::new(source_id, alias_keyword.span);
        index = checked_next_clause_index(index)?;
        let Some(alias_token) = tokens.get(index) else {
            return Err(dependency_clause_error(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::MissingAlias,
                Some(alias_keyword_span),
            ));
        };
        let Some(alias_name) = alias_token
            .symbol
            .filter(|_| alias_token.tag == TokenTag::SYMBOL)
        else {
            return Err(dependency_clause_error(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::ExpectedAliasName,
                Some(SourceSpan::new(source_id, alias_token.span)),
            ));
        };
        let alias = DependencyAlias {
            name: alias_name,
            span: SourceSpan::new(source_id, alias_token.span),
        };
        index = checked_next_clause_index(index)?;
        if !clause_ended_scanned(tokens.get(index)) {
            let Some(end_token) = tokens.get(index) else {
                return Ok((
                    ScannedDependencyClause {
                        provider,
                        binding: ScannedDependencyBinding::Namespace { alias: Some(alias) },
                    },
                    index,
                ));
            };
            return Err(dependency_clause_error(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::NamespaceAliasWithSelections,
                Some(SourceSpan::new(source_id, end_token.span)),
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

    reject_legacy_or_delimited_selection_scanned(tokens.get(index), source_id)?;

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
                Some(path_span),
            ));
        };
        let Some(source_name) = selection_token
            .symbol
            .filter(|_| selection_token.tag == TokenTag::SYMBOL)
        else {
            return Err(dependency_clause_error(
                DependencyClauseKind::DirectSelection,
                InvalidDependencyClauseReason::ExpectedSelectionName,
                selection_continuation_comma
                    .or(Some(SourceSpan::new(source_id, selection_token.span))),
            ));
        };
        if index
            .checked_add(1)
            .and_then(|next| tokens.get(next))
            .is_some_and(|token| token.tag == TokenTag::DOT)
        {
            return Err(CompilerDiagnostic::invalid_path(
                crate::compiler_frontend::compiler_messages::PathKind::WhitespaceMustBeQuoted,
                Some(SourceSpan::new(source_id, selection_token.span)),
            )
            .into());
        }
        let source_span = SourceSpan::new(source_id, selection_token.span);
        index = checked_next_clause_index(index)?;

        let local_alias = if tokens
            .get(index)
            .is_some_and(|token| token.tag == TokenTag::AS)
        {
            index = checked_next_clause_index(index)?;
            let Some(alias_token) = tokens.get(index) else {
                return Err(dependency_clause_error(
                    DependencyClauseKind::NamespaceAlias,
                    InvalidDependencyClauseReason::MissingAlias,
                    Some(source_span),
                ));
            };
            let Some(alias_name) = alias_token
                .symbol
                .filter(|_| alias_token.tag == TokenTag::SYMBOL)
            else {
                return Err(dependency_clause_error(
                    DependencyClauseKind::NamespaceAlias,
                    InvalidDependencyClauseReason::ExpectedAliasName,
                    Some(SourceSpan::new(source_id, alias_token.span)),
                ));
            };
            index = checked_next_clause_index(index)?;
            Some(DependencyAlias {
                name: alias_name,
                span: SourceSpan::new(source_id, alias_token.span),
            })
        } else {
            None
        };

        if !selected_source_names.insert(source_name) {
            return Err(dependency_clause_error(
                DependencyClauseKind::DirectSelection,
                InvalidDependencyClauseReason::DuplicateSelectionName,
                Some(source_span),
            ));
        }
        let local_name = local_alias.as_ref().map_or(source_name, |alias| alias.name);
        if !selected_local_names.insert(local_name) {
            return Err(dependency_clause_error(
                DependencyClauseKind::DirectSelection,
                InvalidDependencyClauseReason::DuplicateSelectionLocalName,
                Some(local_alias.as_ref().map_or(source_span, |alias| alias.span)),
            ));
        }

        selections.push(ScannedDependencySelection {
            source_name,
            source_span,
            local_alias,
        });

        match tokens.get(index).map(|token| token.tag) {
            Some(TokenTag::COMMA) => {
                let Some(comma_token) = tokens.get(index) else {
                    break;
                };
                let comma_span = SourceSpan::new(source_id, comma_token.span);
                continuation_comma = Some(comma_span);
                index = checked_next_clause_index(index)?;
                while tokens
                    .get(index)
                    .is_some_and(|token| token.tag == TokenTag::NEWLINE)
                {
                    index = checked_next_clause_index(index)?;
                }
                if clause_ended_scanned(tokens.get(index)) {
                    return Err(dependency_clause_error(
                        DependencyClauseKind::DirectSelection,
                        InvalidDependencyClauseReason::MissingSelectionAfterComma,
                        Some(comma_span),
                    ));
                }
                reject_legacy_or_delimited_selection_scanned(tokens.get(index), source_id)?;
            }
            Some(TokenTag::NEWLINE) | Some(TokenTag::END) | Some(TokenTag::EOF) | None => break,
            _ => {
                if let Some(comma_span) = &selection_continuation_comma
                    && classify_symbol_statement_start_at_scanned(tokens, index)
                        .starts_statement_after_dependency_selection()
                {
                    return Err(continuation_entered_statement_error(
                        source_span,
                        *comma_span,
                    ));
                }
                let Some(unexpected) = tokens.get(index) else {
                    break;
                };
                return Err(dependency_clause_error(
                    DependencyClauseKind::DirectSelection,
                    InvalidDependencyClauseReason::MissingCommaBetweenSelections,
                    Some(SourceSpan::new(source_id, unexpected.span)),
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

fn clause_ended_scanned(token: Option<ScannedToken>) -> bool {
    token.is_none_or(|token| matches!(token.tag, TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF))
}

fn reject_legacy_or_delimited_selection_scanned(
    token: Option<ScannedToken>,
    source_id: SourceId,
) -> DependencyClauseResult<()> {
    let Some(token) = token else {
        return Ok(());
    };
    let reason = match token.tag {
        TokenTag::OPEN_CURLY => InvalidDependencyClauseReason::LegacyBraceSelections,
        TokenTag::OPEN_PARENTHESIS | TokenTag::TYPE_PARAMETER_BRACKET | TokenTag::COLON => {
            InvalidDependencyClauseReason::InvalidSelectionDelimiter
        }
        _ => return Ok(()),
    };
    Err(dependency_clause_error(
        DependencyClauseKind::DirectSelection,
        reason,
        Some(SourceSpan::new(source_id, token.span)),
    ))
}
/// Build a continuation-entered-statement diagnostic with a secondary comma label.
fn continuation_entered_statement_error(
    selected_name_span: SourceSpan,
    comma_span: SourceSpan,
) -> DependencyClauseParseError {
    let diagnostic = CompilerDiagnostic::invalid_dependency_clause(
        DependencyClauseKind::DirectSelection,
        InvalidDependencyClauseReason::ContinuationEnteredStatement,
        Some(selected_name_span),
    )
    .with_labels(vec![
        crate::compiler_frontend::compiler_messages::DiagnosticLabel::secondary(
            Some(comma_span),
            None,
        ),
    ]);
    DependencyClauseParseError::Diagnostic(diagnostic)
}

#[cfg(test)]
#[path = "tests/dependency_clause_syntax_tests.rs"]
mod dependency_clause_syntax_tests;

fn dependency_clause_error(
    kind: DependencyClauseKind,
    reason: InvalidDependencyClauseReason,
    span: Option<SourceSpan>,
) -> DependencyClauseParseError {
    CompilerDiagnostic::invalid_dependency_clause(kind, reason, span).into()
}
