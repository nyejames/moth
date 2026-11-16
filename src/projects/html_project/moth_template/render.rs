//! Builder-owned rendering of direct Moth template content.
//!
//! WHAT: adds one document's surviving resource origins to the request-wide HTML resource plan
//! and renders structural pieces through the HTML builder's URL renderer at the template's own
//! document context.
//! WHY: resource identity, placement and URL semantics are builder policy. The compiler service
//! returns structural content plus source facts, so this is the one place the direct lane
//! resolves resource and site-root anchors to text. Output-path conflicts belong to the shared
//! plan, not to a per-document planner.

use crate::build_system::output::parse_native_relative_path;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, ResourceDiagnosticSite, ResourceUrlContext, ResourceUseKind,
};
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
use std::path::{Path, PathBuf};

/// The unset site-origin spelling project configuration uses when no origin is configured.
///
/// The direct lane has no site policy, so site-root pieces render the bare site root.
const DIRECT_TEMPLATE_SITE_ORIGIN: &str = "/";

/// Render one document's structural content at its URL context.
///
/// Exact output liveness comes from the folded string: only resource pieces still present in
/// the final content are planned onto the request-wide plan. The module resource table supplies
/// authored locations for those live origins and is not a liveness source. The document context
/// is the request-relative artefact path when the input has one, so nested documents observe
/// URLs from their real parent. A document with a resource piece validates that context as a
/// portable relative path first, because its native source name may be valid identity but not a
/// usable URL parent.
pub(super) fn render_structural_content(
    content: &OwnedFoldedString,
    resources: &ModuleResourceTable,
    document_path: &Path,
    source_path: &Path,
    plan: &mut HtmlResourceOutputPlan,
    diagnostic_strings: &mut DiagnosticStringBuilder,
) -> Result<String, PreparedFailure> {
    // Only a resource piece needs the document context to render a relative URL. Keep plain text
    // and site-root-only documents independent of that URL contract.
    let needs_document_url_context = match content {
        OwnedFoldedString::Text(_) => false,
        OwnedFoldedString::Pieces(pieces) => pieces
            .iter()
            .any(|piece| matches!(piece, OwnedFoldedStringPiece::Resource(_))),
    };
    if needs_document_url_context {
        validate_document_url_context(document_path, source_path)?;
    }
    let context = ResourceUrlContext::PageDocument(document_path.to_path_buf());

    if let OwnedFoldedString::Pieces(pieces) = content {
        for piece in pieces {
            let OwnedFoldedStringPiece::Resource(origin) = piece else {
                continue;
            };
            let Some(interned) = resources
                .origins()
                .iter()
                .find(|interned| interned.origin == *origin)
            else {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "folded resource origin {origin:?} is absent from its module resource table"
                    ),
                )
                .into());
            };
            plan.plan_origin(
                origin.clone(),
                interned
                    .first_authored_span
                    .map(ResourceDiagnosticSite::project),
                context.clone(),
                diagnostic_strings,
                ResourceUseKind::Executable,
            )?;
        }
    }

    let renderer = StructuralUrlRenderer::new(plan, &context, DIRECT_TEMPLATE_SITE_ORIGIN);

    renderer
        .render_owned(content)
        .map_err(PreparedFailure::from)
}

/// Validate a document path that will anchor resource URLs.
///
/// A native source file name may contain a literal backslash or another nonportable component.
/// That name is valid source identity, but it cannot form a relative document URL for resources.
/// Report it through the MTF filesystem input lane, with the canonical source path as `host_path`
/// and the offending document path in `Debug` form so bytes that portable text would rewrite stay
/// visible.
fn validate_document_url_context(
    document_path: &Path,
    source_path: &Path,
) -> Result<(), PreparedFailure> {
    parse_native_relative_path(document_path).map_err(|reason| {
        PreparedFailure::from(InfrastructureFailure::filesystem(
            CompilerStage::SourceLoading,
            source_path,
            format!(
                "Moth template document path {document_path:?} is not a valid portable URL \
                 context: {reason:?}. Resource URLs need a relative document path."
            ),
        ))
    })?;

    Ok(())
}

/// The document URL path every resource URL is written against.
///
/// Request-relative paths keep nested documents in their real parent directory. A single-file
/// request has no request-relative path, so the source filename is the URL context.
pub(super) fn document_url_context(
    relative_path: Option<&Path>,
    source_path: &Path,
) -> Result<PathBuf, InfrastructureFailure> {
    if let Some(relative) = relative_path {
        return Ok(relative.to_path_buf());
    }

    source_path.file_name().map(PathBuf::from).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "direct Moth template source {source_path:?} has no file name; its document \
             URL context needs one"
            ),
        )
    })
}
