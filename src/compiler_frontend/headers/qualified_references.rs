//! Expression-qualified reference occurrence recognition.
//!
//! WHAT: recognizes authored `PATH` + same-line `SYMBOL` pairs inside expression-bearing token
//!       ranges and retains one compact fact per occurrence on its prepared source file.
//! WHY: Stage 0 provider discovery, header binding, declaration ordering and AST lowering must
//!      share one syntax fact instead of each re-reading the same tokens. This module decides
//!      syntax shape only: provider validity, visibility, declaration kind and constness remain
//!      later owners' decisions.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax;
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::headers::dependency_paths::validate_dependency_path;
use crate::compiler_frontend::headers::dependency_target::{
    checked_provider_target, classify_dependency_target,
};
use crate::compiler_frontend::headers::types::{Header, HeaderKind, HeaderParseFailure};
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::path_interner::{PathIdRemap, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{SourceTokens, TokenIndex, TokenRange, TokenTag};
use crate::compiler_frontend::utilities::token_scan::{
    TokenFactView, recognize_qualified_selection,
};

/// One authored expression-qualified reference retained by its preparing source file.
///
/// WHAT: records the retained provider path, the one selected symbol, its exact span and the
///       provider-handoff shell owned by this occurrence.
/// WHY: later stages need one semantic row per authored occurrence keyed by
///      `(owning source, path_syntax)`. The provider path reuses the clause-owned retained fact so
///      Stage 0 resolves the identical target/extension structure and stores the same
///      same-module source identity. The syntax mints no local declaration name, so this fact is
///      deliberately not a dependency clause and never enters file visibility.
#[derive(Clone, Debug)]
pub struct QualifiedReferenceOccurrence {
    /// Authored provider path and shell identity, shared with ordinary direct-selection clauses.
    pub dependency: RetainedDependencyPath,
    /// The single selected symbol immediately after the path on the same physical line.
    pub selected: StringId,
    /// Exact authored span of the selected symbol token.
    pub selected_span: SourceSpan,
}

impl QualifiedReferenceOccurrence {
    /// Remap the retained provider path and selected symbol into a merged string table.
    ///
    /// The path handle is a file-local row index and the spans are source-local byte ranges, so
    /// neither participates in string remapping.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.dependency.remap_string_ids(remap);
        self.selected = remap.get(self.selected);
    }

    /// Remap the retained complete-path identity after its path fork merges.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.dependency.remap_path_ids(remap);
    }

    /// Commit the final build-lifetime source identity without changing any local byte range.
    pub fn commit_source_rebinding(&mut self, file_id: SourceId) {
        self.dependency.commit_source_rebinding(file_id);
        self.selected_span = SourceSpan::new(file_id, self.selected_span.local());
    }
}

/// Recognized occurrences of one scan pass.
pub(crate) struct QualifiedReferenceCollection {
    /// Occurrences in canonical source order.
    pub(crate) occurrences: Vec<QualifiedReferenceOccurrence>,
    /// Path rows owned by an occurrence.
    ///
    /// WHY: the prepared file-reference table must not file these rows as extensionless or
    /// resource file values; one authored path token has exactly one semantic role.
    pub(crate) consumed_path_syntax_ids: Vec<PathSyntaxId>,
    /// Span of the first `as` authored directly after a selected symbol.
    ///
    /// WHAT: an alias in expression position, which is not dependency-clause or expression
    ///       syntax.
    /// WHY: the ordinary header lane reports it as a typed syntax diagnostic. The scanner itself
    ///      owns no diagnostic policy.
    pub(crate) unsupported_alias_span: Option<SourceSpan>,
}

/// Recognize every expression-qualified selection inside the supplied token ranges.
///
/// WHAT: sorts and trims the caller's expression ranges, walks them once and emits one occurrence
///       per `PATH` + same-line `SYMBOL` pair in canonical source order.
/// WHY: ranges arrive per header shell (body, initializer, defaults) rather than in one contiguous
///      span, and trimming keeps a pair from being recorded twice when caller ranges meet.
///
/// Every occurrence validates its authored path through the same `validate_dependency_path`
/// contract an ordinary dependency clause uses, so `@/`, `@./x`, `@../x` and `@x.moth` produce the
/// identical typed diagnostics in expression position.
pub(crate) fn collect_qualified_references(
    canonical: &SourceTokens,
    ranges: impl IntoIterator<Item = TokenRange>,
    file_id: SourceId,
    first_shell_ordinal: u32,
    path_syntax: &PathSyntaxTable,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<QualifiedReferenceCollection, HeaderParseFailure> {
    if canonical.source() != file_id {
        return Err(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "qualified-reference scan source token owner does not match its file identity",
        )
        .into());
    }

    let mut ranges: Vec<TokenRange> = ranges
        .into_iter()
        .filter(|range| !range.is_empty())
        .collect();
    ranges.sort_by_key(|range| range.start().index());

    let mut scanned = Vec::new();
    let mut unsupported_alias_span = None;
    let mut next_unscanned_index = 0usize;

    for range in ranges {
        if range.source() != file_id {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "qualified-reference scan received a range from another source",
            )
            .into());
        }
        let start = range.start().index().max(next_unscanned_index);
        let end = range.end().index();
        if end <= start {
            continue;
        }
        next_unscanned_index = end;

        let start_index = TokenIndex::try_from_index(start).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "qualified-reference scan start exceeded its checked token index domain",
            )
        })?;
        let bounded =
            TokenRange::try_new_for(canonical, start_index, range.end()).map_err(|error| {
                InfrastructureFailure::internal(
                    CompilerStage::HeaderParsing,
                    format!("qualified-reference scan range was invalid: {error:?}"),
                )
            })?;
        let view = TokenFactView::from_source_range(canonical, bounded).map_err(|error| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!("qualified-reference scan view was invalid: {error:?}"),
            )
        })?;

        for index in 0..view.len() {
            let Some(selection) = recognize_qualified_selection(view, index) else {
                continue;
            };
            if unsupported_alias_span.is_none()
                && let Some(follower) = index.checked_add(2).and_then(|follower| view.get(follower))
                && follower.tag == TokenTag::AS
            {
                unsupported_alias_span = Some(SourceSpan::new(file_id, follower.span));
            }
            scanned.push(selection);
        }
    }

    let mut occurrences = Vec::with_capacity(scanned.len());
    for (index, selection) in scanned.into_iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "qualified-reference occurrence count exceeded its checked shell ordinal domain",
            )
        })?;
        let ordinal = first_shell_ordinal.checked_add(index).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "qualified-reference shell ordinal overflowed its file-local identity",
            )
        })?;
        // Retain the same provider-path classification a direct-selection clause carries, so
        // Stage 0 resolves extensionless sources, explicit-extension providers and invalid
        // resource/`.moth` paths through the clause-owned mechanics instead of reclassifying.
        let path_span = SourceSpan::new(file_id, selection.path_span);
        let row = path_syntax.try_path_for_token(selection.path_syntax_id, path_span)?;
        // One authored path token has one validation contract regardless of position: an
        // expression-qualified reference rejects the site root, `.`/`..` segments and authored
        // `.moth` extensions exactly like the same path in a top-level clause.
        validate_dependency_path(row.root, &path_span, path_fork, string_table)
            .map_err(HeaderParseFailure::Diagnostic)?;
        let target = classify_dependency_target(row.root, path_fork, string_table);
        let provider_target = checked_provider_target(row.root, &target, path_fork, string_table)?;
        occurrences.push(QualifiedReferenceOccurrence {
            dependency: RetainedDependencyPath {
                dependency_shell_id: DependencyShellId::new(file_id, ordinal),
                path: row.root,
                path_syntax: selection.path_syntax_id,
                target,
                provider_target,
                local_source_id: None,
                span: path_span,
            },
            selected: selection.selected_name,
            selected_span: SourceSpan::new(file_id, selection.selected_span),
        });
    }

    let consumed_path_syntax_ids = occurrences
        .iter()
        .map(|occurrence| occurrence.dependency.path_syntax)
        .collect();
    Ok(QualifiedReferenceCollection {
        occurrences,
        consumed_path_syntax_ids,
        unsupported_alias_span,
    })
}

/// Append the expression-bearing token ranges owned by one header shell.
///
/// WHAT: collects the header body (retained range or segmented start-body sequence) and every
///       declaration default/initializer range that can contain a value expression.
/// WHY: a qualified reference may appear in a constant initializer, a parameter or field default,
///       a function/start body or a template head. Header shells retain those regions separately,
///       so occurrence recognition needs one bounded range list instead of rescanning the file.
pub(crate) fn push_header_expression_ranges(
    header: &Header,
    canonical: &SourceTokens,
    file_id: SourceId,
    ranges: &mut Vec<TokenRange>,
) -> Result<(), InfrastructureFailure> {
    if header.tokens.source() != file_id {
        return Err(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "retained header range does not match its source token owner",
        ));
    }

    if let Some(sequence) = header.token_sequence {
        let view = canonical.token_sequence(sequence).map_err(|error| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!("retained header token sequence could not be resolved: {error:?}"),
            )
        })?;
        ranges.extend(view.ranges());
    } else if !header.tokens.is_empty() {
        ranges.push(header.tokens);
    }

    match &header.kind {
        HeaderKind::Constant { declaration } => {
            ranges.extend(declaration.initializer_range);
        }
        HeaderKind::Function { signature, .. } => {
            push_signature_default_ranges(&signature.parameters, ranges);
        }
        HeaderKind::Struct { fields, .. } => {
            push_signature_default_ranges(fields, ranges);
        }
        HeaderKind::Choice { variants, .. } => {
            for variant in variants {
                let ChoiceVariantPayloadSyntax::Record { fields } = &variant.payload else {
                    continue;
                };
                push_signature_default_ranges(fields, ranges);
            }
        }
        HeaderKind::Trait { declaration } => {
            for requirement in &declaration.requirements {
                push_signature_default_ranges(&requirement.signature.parameters, ranges);
            }
        }
        HeaderKind::StartFunction
        | HeaderKind::TypeAlias { .. }
        | HeaderKind::ConstTemplate { .. }
        | HeaderKind::TraitConformance { .. }
        | HeaderKind::TraitIncompatibility { .. } => {}
    }

    Ok(())
}

fn push_signature_default_ranges(
    members: &[crate::compiler_frontend::declaration_syntax::signature_members::SignatureMemberSyntax],
    ranges: &mut Vec<TokenRange>,
) {
    for member in members {
        ranges.extend(member.default_range);
    }
}
