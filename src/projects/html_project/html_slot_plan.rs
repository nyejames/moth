//! HTML builder root-slot interpretation.
//!
//! WHAT: turns ordered named const fragments into page metadata, stylesheet links and
//!       deduplicated additional head HTML for one entry page.
//! WHY: the compiler routes direct root inserts to statically registered destinations but does
//!      not know their meaning. This module is the single HTML-builder owner that interprets
//!      those destinations for both JS and Wasm render paths after the root resource union has
//!      been planned, so the two output modes cannot disagree about metadata, stylesheet identity
//!      or head content.

use crate::compiler_frontend::builder_root_slots::ConstFragmentDestination;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::InvalidHtmlSlotContributionReason;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::module_compilation::ResolvedConstFragment;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};
use crate::projects::html_project::diagnostics::capture_html_diagnostic;
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::ops::Range;

/// Builder-interpreted page metadata shared by both HTML output modes.
///
/// WHAT: rendered metadata text, with `None` distinguishing an absent slot from an explicitly
///       empty contribution.
/// WHY: the document shell owns escaping and fallback policy, while slot planning owns source
///       order and structural-url resolution. Keeping rendered values here means a resource piece
///       is resolved once against the final page output context.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HtmlPageMetadata {
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) lang: Option<String>,
    pub(crate) favicon: Option<String>,
    pub(crate) body_style: Option<String>,
}

/// One entry page's complete builder-slot interpretation.
///
/// WHAT: metadata values, final stylesheet `href` values in first-origin order, and additional
///       head HTML whose complete contributions are already deduplicated.
/// WHY: the document shell and both backend render paths consume one plan instead of rescanning
///      fragments or reconstructing slot meaning. Resource preflight still consumes the root
///      resource union, so slot interpretation adds no parallel provenance cache.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HtmlSlotPlan {
    pub(crate) metadata: HtmlPageMetadata,
    pub(crate) stylesheet_hrefs: Vec<String>,
    pub(crate) extra_head_html: String,
}

/// Interpret every named root fragment for the HTML builder.
///
/// Body fragments stay on the ordinary body lane and are not inspected here. Named contributions
/// are processed in the fragment handoff's source order: metadata and head concatenate, while
/// stylesheets keep only the first contribution for each stable resource origin.
pub(crate) fn plan_html_entry_slots(
    const_fragments: &[ResolvedConstFragment],
    structural_url_renderer: &StructuralUrlRenderer<'_>,
    source_domain: Option<&StablePackageIdentity>,
    diagnostic_strings: &mut DiagnosticStringBuilder,
) -> Result<HtmlSlotPlan, PreparedFailure> {
    let mut plan = HtmlSlotPlan::default();
    let mut seen_stylesheet_origins: FxHashSet<&StableResourceOriginId> = FxHashSet::default();
    // Bucket rendered head contributions by hash and compare exact bytes within a bucket, so a
    // hash collision cannot merge distinct content and a duplicate is not retained twice.
    let mut seen_head_contributions: FxHashMap<u64, Vec<Range<usize>>> = FxHashMap::default();

    for fragment in const_fragments {
        let ConstFragmentDestination::Named(slot_name) = &fragment.destination else {
            continue;
        };

        match slot_name.as_str() {
            "title" => append_metadata_value(
                &mut plan.metadata.title,
                &fragment.value,
                structural_url_renderer,
            )?,
            "description" => append_metadata_value(
                &mut plan.metadata.description,
                &fragment.value,
                structural_url_renderer,
            )?,
            "lang" => append_metadata_value(
                &mut plan.metadata.lang,
                &fragment.value,
                structural_url_renderer,
            )?,
            "favicon" => append_metadata_value(
                &mut plan.metadata.favicon,
                &fragment.value,
                structural_url_renderer,
            )?,
            "body_style" => append_metadata_value(
                &mut plan.metadata.body_style,
                &fragment.value,
                structural_url_renderer,
            )?,
            "stylesheets" => {
                let origin = stylesheet_origin(
                    &fragment.value,
                    fragment.span,
                    source_domain,
                    diagnostic_strings,
                )?;
                if seen_stylesheet_origins.insert(origin) {
                    plan.stylesheet_hrefs
                        .push(structural_url_renderer.render_resource_origin(origin)?);
                }
            }
            "head" => {
                let rendered = match &fragment.value {
                    OwnedFoldedString::Text(text) => Cow::Borrowed(text.as_str()),
                    value => Cow::Owned(structural_url_renderer.render_owned(value)?),
                };
                let mut hasher = FxHasher::default();
                rendered.hash(&mut hasher);
                let ranges = seen_head_contributions.entry(hasher.finish()).or_default();
                if !ranges
                    .iter()
                    .any(|range| &plan.extra_head_html[range.start..range.end] == rendered.as_ref())
                {
                    let start = plan.extra_head_html.len();
                    plan.extra_head_html.push_str(&rendered);
                    ranges.push(start..plan.extra_head_html.len());
                }
            }
            other => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "HTML builder received a named root fragment for unregistered destination '{other}'"
                    ),
                )
                .into());
            }
        }
    }

    Ok(plan)
}

/// Append one already-rendered contribution to an optional metadata value.
fn append_metadata_value(
    target: &mut Option<String>,
    value: &OwnedFoldedString,
    structural_url_renderer: &StructuralUrlRenderer<'_>,
) -> Result<(), InfrastructureFailure> {
    if let (Some(existing), OwnedFoldedString::Text(text)) = (&mut *target, value) {
        existing.push_str(text);
    } else {
        let rendered = structural_url_renderer.render_owned(value)?;
        match target {
            Some(existing) => existing.push_str(&rendered),
            None => *target = Some(rendered),
        }
    }
    Ok(())
}

/// Validate one `stylesheets` contribution and return its stable resource origin.
///
/// WHAT: accepts exactly one CSS resource origin surrounded by formatting whitespace only.
/// WHY: stylesheet identity must stay semantic rather than textual. A rendered path String,
///      external URL, site-root expansion or multi-resource value cannot identify one emitted
///      stylesheet, so each is diagnosed at the authored contribution span.
fn stylesheet_origin<'a>(
    value: &'a OwnedFoldedString,
    span: Option<SourceSpan>,
    source_domain: Option<&StablePackageIdentity>,
    diagnostic_strings: &mut DiagnosticStringBuilder,
) -> Result<&'a StableResourceOriginId, PreparedFailure> {
    classify_stylesheet_contribution(value).map_err(|reason| {
        invalid_stylesheet_contribution(reason, span, source_domain, diagnostic_strings)
    })
}

/// Classify one `stylesheets` contribution without constructing a diagnostic.
///
/// WHAT: accepts exactly one CSS resource origin surrounded by formatting whitespace only.
/// WHY: the classification is builder policy with several rejection arms, so it stays a pure
///      function that planning maps to an authored-span diagnostic and tests can exercise
///      each arm without diagnostic plumbing.
fn classify_stylesheet_contribution(
    value: &OwnedFoldedString,
) -> Result<&StableResourceOriginId, InvalidHtmlSlotContributionReason> {
    match value {
        OwnedFoldedString::Text(text) => Err(if text.trim().is_empty() {
            InvalidHtmlSlotContributionReason::MissingResource
        } else {
            InvalidHtmlSlotContributionReason::NotAStaticResource
        }),
        OwnedFoldedString::Pieces(pieces) => {
            let mut resource = None;
            let mut resource_count = 0usize;
            let mut has_site_root = false;
            let mut has_literal_text = false;

            for piece in pieces {
                match piece {
                    OwnedFoldedStringPiece::Text(text) => {
                        has_literal_text |= !text.trim().is_empty();
                    }
                    OwnedFoldedStringPiece::Resource(origin) => {
                        resource_count += 1;
                        if resource.is_none() {
                            resource = Some(origin);
                        }
                    }
                    OwnedFoldedStringPiece::SiteRoot => has_site_root = true,
                }
            }

            if has_site_root {
                return Err(InvalidHtmlSlotContributionReason::SiteRoot);
            }
            if resource_count > 1 {
                return Err(InvalidHtmlSlotContributionReason::MultipleResources);
            }
            if has_literal_text {
                return Err(InvalidHtmlSlotContributionReason::NotAStaticResource);
            }
            let origin = resource.ok_or(InvalidHtmlSlotContributionReason::MissingResource)?;
            if !origin_names_css(origin) {
                return Err(InvalidHtmlSlotContributionReason::NonCssResource);
            }
            Ok(origin)
        }
    }
}

/// Whether one tracked resource origin names a CSS file by its stable logical path.
fn origin_names_css(origin: &StableResourceOriginId) -> bool {
    origin
        .logical_path()
        .as_str()
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("css"))
}

fn invalid_stylesheet_contribution(
    reason: InvalidHtmlSlotContributionReason,
    span: Option<SourceSpan>,
    source_domain: Option<&StablePackageIdentity>,
    diagnostic_strings: &mut DiagnosticStringBuilder,
) -> PreparedFailure {
    let slot = diagnostic_strings.intern("stylesheets");
    let mut diagnostic = project_config::invalid_html_slot_contribution(slot, reason, span);
    if let Some(domain) = source_domain {
        diagnostic = diagnostic
            .with_primary_frozen_identity_handle(FrozenIdentityHandle::for_domain(domain.clone()));
    }
    capture_html_diagnostic(diagnostic, diagnostic_strings.strings(), None, || None)
}

#[cfg(test)]
#[path = "tests/html_slot_plan_tests.rs"]
mod tests;
