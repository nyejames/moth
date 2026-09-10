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
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::tokenizer::tokens::{Token, TokenKind};
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
    pub path: InternedPath,
    pub path_syntax: PathSyntaxId,
    pub path_span: SourceSpan,
}

/// The consolidated path authority for one retained dependency clause.
#[derive(Clone, Debug)]
pub struct RetainedDependencyPath {
    pub dependency_shell_id: DependencyShellId,
    pub path: InternedPath,
    pub path_syntax: PathSyntaxId,
    pub target: DependencyTargetKind,
    pub span: SourceSpan,
}

impl RetainedDependencyPath {
    /// Remap the path components and target extension into a merged string table.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.path.remap_string_ids(remap);
        self.target.remap_string_ids(remap);
    }

    /// Commit the final source identity while preserving module-root-relative paths.
    pub fn commit_source_rebinding(&mut self, file_id: SourceId, _logical_path: &InternedPath) {
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

pub(crate) fn parse_dependency_clause(
    tokens: &[Token],
    start_index: usize,
    path_syntax: &PathSyntaxTable,
    source_id: SourceId,
) -> DependencyClauseResult<(ScannedDependencyClause, usize)> {
    let Some(path_token) = tokens.get(start_index) else {
        return Err(CompilerDiagnostic::invalid_dependency_clause(
            DependencyClauseKind::Namespace,
            InvalidDependencyClauseReason::MissingPath,
            None,
        )
        .into());
    };

    let TokenKind::Path(path_id) = &path_token.kind else {
        return Err(CompilerDiagnostic::invalid_dependency_clause(
            DependencyClauseKind::Namespace,
            InvalidDependencyClauseReason::ExpectedPath,
            Some(SourceSpan::new(source_id, path_token.span)),
        )
        .into());
    };
    let path_row = path_syntax.try_path(*path_id)?;
    let path_span = SourceSpan::new(path_row.span.source(), path_token.span);
    let path_syntax_row = path_syntax.try_path_for_token(*path_id, path_span)?;
    let source_id = path_span.source();
    let mut index = start_index + 1;

    let provider = ScannedDependencyProvider {
        path: path_syntax_row.root.clone(),
        path_syntax: *path_id,
        path_span,
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
        let alias_keyword_span = SourceSpan::new(source_id, tokens[index].span);
        index += 1;
        let Some(alias_token) = tokens.get(index) else {
            return Err(dependency_clause_error(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::MissingAlias,
                Some(alias_keyword_span),
            ));
        };
        let TokenKind::Symbol(alias_name) = alias_token.kind else {
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
        index += 1;
        if !clause_ended(tokens.get(index)) {
            return Err(dependency_clause_error(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::NamespaceAliasWithSelections,
                Some(SourceSpan::new(source_id, tokens[index].span)),
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

    reject_legacy_or_delimited_selection(tokens.get(index), source_id)?;

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
        let TokenKind::Symbol(source_name) = selection_token.kind else {
            return Err(dependency_clause_error(
                DependencyClauseKind::DirectSelection,
                InvalidDependencyClauseReason::ExpectedSelectionName,
                selection_continuation_comma
                    .or(Some(SourceSpan::new(source_id, selection_token.span))),
            ));
        };
        if tokens
            .get(index + 1)
            .is_some_and(|token| token.kind == TokenKind::Dot)
        {
            return Err(CompilerDiagnostic::invalid_path(
                crate::compiler_frontend::compiler_messages::PathKind::WhitespaceMustBeQuoted,
                Some(SourceSpan::new(source_id, selection_token.span)),
            )
            .into());
        }
        let source_span = SourceSpan::new(source_id, selection_token.span);
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
                    Some(source_span),
                ));
            };
            let TokenKind::Symbol(alias_name) = alias_token.kind else {
                return Err(dependency_clause_error(
                    DependencyClauseKind::NamespaceAlias,
                    InvalidDependencyClauseReason::ExpectedAliasName,
                    Some(SourceSpan::new(source_id, alias_token.span)),
                ));
            };
            index += 1;
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

        match tokens.get(index).map(|token| &token.kind) {
            Some(TokenKind::Comma) => {
                let comma_span = SourceSpan::new(source_id, tokens[index].span);
                continuation_comma = Some(comma_span);
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
                        Some(comma_span),
                    ));
                }
                reject_legacy_or_delimited_selection(tokens.get(index), source_id)?;
            }
            Some(TokenKind::Newline | TokenKind::End | TokenKind::Eof) | None => break,
            _ => {
                if let Some(comma_span) = &selection_continuation_comma
                    && classify_symbol_statement_start_at(tokens, index)
                        .starts_statement_after_dependency_selection()
                {
                    return Err(continuation_entered_statement_error(
                        source_span,
                        *comma_span,
                    ));
                }
                return Err(dependency_clause_error(
                    DependencyClauseKind::DirectSelection,
                    InvalidDependencyClauseReason::MissingCommaBetweenSelections,
                    Some(SourceSpan::new(source_id, tokens[index].span)),
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

fn reject_legacy_or_delimited_selection(
    token: Option<&Token>,
    source_id: SourceId,
) -> DependencyClauseResult<()> {
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
