//! Expression-qualified reference occurrence recognition.
//!
//! WHAT: recognizes authored `PATH` + same-line `SYMBOL` pairs inside expression-bearing token
//!       ranges and retains one compact fact per occurrence on its prepared source file.
//! WHY: Stage 0 provider discovery, header binding, declaration ordering and AST lowering must
//!      share one syntax fact instead of each re-reading the same tokens. This module decides
//!      syntax shape only: provider validity, visibility, declaration kind and constness remain
//!      later owners' decisions.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::{CommonSyntaxMistakeReason, CompilerStage};
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax;
use crate::compiler_frontend::declaration_syntax::signature_members::SignatureMemberSyntax;
use crate::compiler_frontend::headers::dependency_clause_syntax::RetainedDependencyPath;
use crate::compiler_frontend::headers::dependency_paths::validate_dependency_path;
use crate::compiler_frontend::headers::dependency_target::{
    checked_provider_target, classify_dependency_target,
};
use crate::compiler_frontend::headers::types::{
    Header, HeaderKind, HeaderParseFailure, SyntheticContentPayload,
};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::path_interner::{PathIdRemap, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{SourceTokens, TokenIndex, TokenRange, TokenTag};
use crate::compiler_frontend::utilities::token_scan::{
    TokenFactView, recognize_qualified_selection,
};

/// One expression-bearing token range owned by a prepared header shell.
///
/// WHAT: the canonical authored interval plus whether its expression folds to compile-time
///       content before the declaring shell is prepared.
/// WHY: Stage 0 occurrence recognition scans runtime bodies and compile-time values in one pass,
///      while Stage 3 declaration ordering consumes only the const-required ranges. Retaining the
///      classification on the range lets `collect_qualified_references` stamp each occurrence
///      without rescanning tokens at ordering time.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HeaderExpressionRange {
    /// Index of the header shell that owns this range.
    pub(crate) header_index: usize,
    /// Exact authored token interval.
    pub(crate) range: TokenRange,
    /// Whether this range folds before its declaring shell is prepared.
    pub(crate) const_required: bool,
}

impl HeaderExpressionRange {
    /// Build one compile-time range for an adapter that owns only content syntax.
    ///
    /// WHY: a synthetic template body is entirely const-required, so its prepare path supplies
    ///      ranges with one explicit constructor instead of reconstructing the flag per range.
    pub(crate) fn const_required(header_index: usize, range: TokenRange) -> Self {
        Self {
            header_index,
            range,
            const_required: true,
        }
    }

    fn expression(header_index: usize, range: TokenRange) -> Self {
        Self {
            header_index,
            range,
            const_required: false,
        }
    }
}

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
    /// Index of the header shell that authored this occurrence in source order.
    ///
    /// WHAT: file preparation numbers its own headers from zero; symbol collection rebases the
    ///       index into the module-wide header vector.
    /// WHY: Stage 3 ordering reads occurrences per header without rescanning token ranges.
    pub owning_header_index: usize,
    /// Authored provider path and shell identity, shared with ordinary direct-selection clauses.
    pub dependency: RetainedDependencyPath,
    /// The single selected symbol immediately after the path on the same physical line.
    pub selected: StringId,
    /// Exact authored span of the selected symbol token.
    pub selected_span: SourceSpan,
    /// Whether the producing expression folds before its declaring shell is prepared.
    ///
    /// WHY: Stage 0 provider discovery sees every occurrence, but only a const-required
    ///      occurrence is a declaration-ordering dependency. Runtime function and start bodies
    ///      must never add a Stage 3 edge.
    pub const_required: bool,
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

/// Recognize every expression-qualified selection inside the supplied token ranges.
///
/// WHAT: sorts and trims the caller's expression ranges, walks them once and emits one occurrence
///       per `PATH` + same-line `SYMBOL` pair in canonical source order. Every occurrence records
///       whether its producing range is const-required.
/// WHY: ranges arrive per header shell (body, initializer, defaults) rather than in one contiguous
///      span, and trimming keeps a pair from being recorded twice when caller ranges meet. A
///      const-required range always wins an overlap with a runtime body, so a shared shell cannot
///      hide the Stage 3 declaration dependency.
///
/// Every occurrence validates its authored path through the same `validate_dependency_path`
/// contract an ordinary dependency clause uses, so `@/`, `@./x`, `@../x` and `@x.moth` produce the
/// identical typed diagnostics in expression position. An alias after the selected symbol is the
/// same invalid-`as` syntax diagnostic the clause lane reports, raised here because no later
/// caller can see the authored `as`.
pub(crate) fn collect_qualified_references(
    canonical: &SourceTokens,
    ranges: impl IntoIterator<Item = HeaderExpressionRange>,
    file_id: SourceId,
    first_shell_ordinal: u32,
    path_syntax: &PathSyntaxTable,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<Vec<QualifiedReferenceOccurrence>, HeaderParseFailure> {
    if canonical.source() != file_id {
        return Err(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "qualified-reference scan source token owner does not match its file identity",
        )
        .into());
    }

    let ranges: Vec<HeaderExpressionRange> = ranges
        .into_iter()
        .filter(|range| !range.range.is_empty())
        .collect();
    for range in &ranges {
        if range.range.source() != file_id {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "qualified-reference scan received a range from another source",
            )
            .into());
        }
    }
    let mut ranges = split_runtime_ranges_around_const_required(ranges);
    ranges.sort_by_key(|range| {
        (
            range.range.start().index(),
            !range.const_required,
            range.range.end().index(),
        )
    });

    let mut scanned = Vec::new();
    let mut unsupported_alias_span = None;
    let mut next_unscanned_index = 0usize;

    for expression_range in ranges {
        let start = expression_range
            .range
            .start()
            .index()
            .max(next_unscanned_index);
        let end = expression_range.range.end().index();
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
        let bounded = TokenRange::try_new_for(canonical, start_index, expression_range.range.end())
            .map_err(|error| {
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
            scanned.push((
                expression_range.header_index,
                selection,
                expression_range.const_required,
            ));
        }
    }

    let mut occurrences = Vec::with_capacity(scanned.len());
    for (index, (owning_header_index, selection, const_required)) in scanned.into_iter().enumerate()
    {
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
            owning_header_index,
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
            const_required,
        });
    }

    // Path validation above ran for every selection first, so an invalid path keeps diagnostic
    // precedence over an alias later in the same expression.
    if let Some(alias_span) = unsupported_alias_span {
        return Err(HeaderParseFailure::Diagnostic(
            token_lexical::common_syntax_mistake(
                CommonSyntaxMistakeReason::InvalidAsOperator,
                Some(alias_span),
            ),
        ));
    }

    Ok(occurrences)
}

/// Split runtime ranges so a const-required range always owns the tokens it covers.
///
/// WHAT: merges the const-required ranges and removes their covered intervals from every runtime
///       range before the scan trims overlaps in authored order.
/// WHY: a runtime body that starts earlier must not stamp a shared default as runtime and hide a
///      real Stage 3 declaration edge. Range lists are small and per shell, so the merged-interval
///      pass stays allocation-light and preserves every non-overlapping runtime token.
fn split_runtime_ranges_around_const_required(
    ranges: Vec<HeaderExpressionRange>,
) -> Vec<HeaderExpressionRange> {
    let mut const_ranges: Vec<TokenRange> = ranges
        .iter()
        .filter(|range| range.const_required)
        .map(|range| range.range)
        .collect();
    if const_ranges.is_empty() {
        return ranges;
    }

    const_ranges.sort_by_key(|range| (range.start(), range.end()));
    let mut merged: Vec<TokenRange> = Vec::with_capacity(const_ranges.len());
    for range in const_ranges {
        match merged.last_mut() {
            Some(last) if range.start() <= last.end() => {
                if range.end() > last.end() {
                    *last = TokenRange::new(range.source(), last.start(), range.end())
                        .expect("merging two ordered ranges keeps the interval ordered");
                }
            }
            _ => merged.push(range),
        }
    }

    let mut split = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.const_required {
            split.push(range);
            continue;
        }

        // Every validated range shares this source, so gap construction cannot mix owners.
        let source = range.range.source();
        let mut uncovered_start = range.range.start();
        for covered in &merged {
            if covered.end() <= uncovered_start {
                continue;
            }
            if covered.start() >= range.range.end() {
                break;
            }
            if covered.start() > uncovered_start {
                split.push(HeaderExpressionRange {
                    header_index: range.header_index,
                    range: TokenRange::new(source, uncovered_start, covered.start())
                        .expect("an uncovered gap between ordered ranges stays ordered"),
                    const_required: false,
                });
            }
            uncovered_start = covered.end();
        }
        if uncovered_start < range.range.end() {
            split.push(HeaderExpressionRange {
                header_index: range.header_index,
                range: TokenRange::new(source, uncovered_start, range.range.end())
                    .expect("a trailing uncovered gap keeps the runtime range ordered"),
                const_required: false,
            });
        }
    }

    split
}

/// Append the expression-bearing token ranges owned by one header shell.
///
/// WHAT: appends the header body (retained range or segmented start-body sequence) as
///       ordering-neutral expression syntax, a config-qualified constant initializer for path
///       consumption, and every const-required value position from
///       `push_const_required_expression_ranges`.
/// WHY: a qualified reference may appear in a constant initializer, a parameter or field default,
///       a function/start body or a template head. Header shells retain those regions separately,
///       so occurrence recognition needs one bounded range list instead of rescanning the file.
///       Deriving the compile-time positions from the classifier keeps this wide Stage 0 scan and
///       Stage 3 ordering on one eligibility policy.
pub(crate) fn push_header_expression_ranges(
    header: &Header,
    header_index: usize,
    canonical: &SourceTokens,
    ranges: &mut Vec<HeaderExpressionRange>,
) -> Result<(), InfrastructureFailure> {
    if let Some(sequence) = header.token_sequence {
        let view = canonical.token_sequence(sequence).map_err(|error| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!("retained header token sequence could not be resolved: {error:?}"),
            )
        })?;
        ranges.extend(
            view.ranges()
                .map(|range| HeaderExpressionRange::expression(header_index, range)),
        );
    } else if !header.tokens.is_empty() {
        ranges.push(HeaderExpressionRange::expression(
            header_index,
            header.tokens,
        ));
    }

    // A config-qualified constant initializer is expression syntax for path consumption but not
    // compile-time content: the file-reference classifier must consume its rows so they never
    // become ordinary file values, while Stage 3 ordering must not see it before config
    // validation succeeds. The expression lane never scans a config-owned shell, so this range
    // stays ordering-neutral.
    if let HeaderKind::Constant { declaration } = &header.kind
        && declaration.config_contract.is_some()
        && let Some(range) = declaration.initializer_range
    {
        ranges.push(HeaderExpressionRange::expression(header_index, range));
    }

    push_const_required_expression_ranges(header, header_index, ranges);
    Ok(())
}

/// Append the token ranges whose values fold before their declaring shell is prepared.
///
/// WHAT: exactly the compile-time value positions: a non-config constant initializer, a synthetic
///       Moth-template body, a const-template body, and function parameter, struct field and
///       choice variant record field defaults.
/// WHY: content-source ordering and qualified-reference ordering must recognise one eligibility
///      policy. A const-required expression is semantically bound before ordinary body emission,
///      so it can require a declaration-ordering edge; a runtime body never can. Trait requirement
///      defaults are absent because signature parsing rejects them (MOTH-RULE-0028), so they are
///      never folded or prepared.
pub(crate) fn push_const_required_expression_ranges(
    header: &Header,
    header_index: usize,
    ranges: &mut Vec<HeaderExpressionRange>,
) {
    let mut push = |range: TokenRange| {
        ranges.push(HeaderExpressionRange::const_required(header_index, range));
    };

    match &header.kind {
        HeaderKind::Constant { declaration } => {
            if declaration.config_contract.is_some() {
                return;
            }
            if let Some(range) = declaration.initializer_range {
                push(range);
            } else if matches!(
                header.synthetic_content_payload,
                Some(SyntheticContentPayload::MothTemplate { .. })
            ) && !header.tokens.is_empty()
            {
                // A Moth-template synthetic content constant keeps its body in `Header::tokens`
                // while leaving the declaration initializer range empty.
                push(header.tokens);
            }
        }
        HeaderKind::ConstTemplate { .. } => {
            // Const-template tokens cover the template head and body, including the top-level
            // compile-time fragments folded before body emission.
            if !header.tokens.is_empty() {
                push(header.tokens);
            }
        }
        HeaderKind::Function { signature, .. } => {
            push_signature_default_ranges(&signature.parameters, &mut push);
        }
        HeaderKind::Struct { fields, .. } => {
            push_signature_default_ranges(fields, &mut push);
        }
        HeaderKind::Choice { variants, .. } => {
            for variant in variants {
                let ChoiceVariantPayloadSyntax::Record { fields } = &variant.payload else {
                    continue;
                };
                push_signature_default_ranges(fields, &mut push);
            }
        }
        HeaderKind::StartFunction
        | HeaderKind::TypeAlias { .. }
        | HeaderKind::Trait { .. }
        | HeaderKind::TraitConformance { .. }
        | HeaderKind::TraitIncompatibility { .. } => {}
    }
}

fn push_signature_default_ranges(
    members: &[SignatureMemberSyntax],
    push: &mut impl FnMut(TokenRange),
) {
    for member in members {
        if let Some(range) = member.default_range {
            push(range);
        }
    }
}
