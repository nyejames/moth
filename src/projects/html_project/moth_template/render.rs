//! Builder-owned rendering of direct Moth template content.
//!
//! WHAT: adds one document's surviving resource origins to the request-wide HTML resource plan
//! and renders structural pieces through the HTML builder's URL renderer at the template's own
//! document context.
//! WHY: resource identity, placement and URL semantics are builder policy. The compiler service
//! returns structural content plus source facts, so this is the one place the direct lane
//! resolves resource and site-root anchors to text. Output-path conflicts belong to the shared
//! plan, not to a per-document planner.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, ResourceUrlContext, ResourceUseKind,
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
/// URLs from their real parent.
pub(super) fn render_structural_content(
    content: &OwnedFoldedString,
    resources: &ModuleResourceTable,
    document_path: &Path,
    plan: &mut HtmlResourceOutputPlan,
    string_table: &mut StringTable,
) -> Result<String, CompilerMessages> {
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
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "folded resource origin {origin:?} is absent from its module resource table"
                    )),
                    string_table,
                ));
            };
            plan.plan_origin(
                origin.clone(),
                interned.first_authored_location.clone(),
                context.clone(),
                string_table,
                ResourceUseKind::Executable,
            )?;
        }
    }

    let renderer = StructuralUrlRenderer::new(plan, &context, DIRECT_TEMPLATE_SITE_ORIGIN);

    renderer
        .render_owned(content)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}

/// The document URL path every resource URL is written against.
///
/// Request-relative paths keep nested documents in their real parent directory. A single-file
/// request has no request-relative path, so the source filename is the URL context.
pub(super) fn document_url_context(
    relative_path: Option<&Path>,
    source_path: &Path,
    string_table: &mut StringTable,
) -> Result<PathBuf, CompilerMessages> {
    if let Some(relative) = relative_path {
        return Ok(relative.to_path_buf());
    }

    source_path.file_name().map(PathBuf::from).ok_or_else(|| {
        CompilerMessages::from_error_ref(
            CompilerError::compiler_error(format!(
                "direct Moth template source {source_path:?} has no file name; its document \
                 URL context needs one"
            )),
            string_table,
        )
    })
}
