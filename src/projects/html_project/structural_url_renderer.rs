//! HTML-builder-owned rendering of structural string pieces.
//!
//! WHAT: resolves resource and site-root pieces to text only after output planning has assigned an
//! artefact URL context, turning the configured project origin into the URL that the bare
//! site-root spelling `@/` renders.
//! WHY: resource identity and site-root policy belong to the builder, while HIR and owned folded
//! values must remain structural until this final output boundary. A site-root URL and a resource
//! URL answer different questions, and keeping them apart is what makes both correct: a site-root
//! URL addresses a route rather than a file the build emits, so it is absolute and always carries
//! the origin, while a resource URL is written relative to the artefact that observes it and never
//! carries the origin. The site root names no file: it has no resource origin, owner, byte source
//! or watch interest, and is never checked, copied, hashed, rewritten or included in a resource
//! union.

use crate::backends::structural_string::StructuralStringUrlMap;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::hir::reachability::HirReachability;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, ResourceUrlContext,
};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

/// Renders structural strings for one consuming HTML artefact.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StructuralUrlRenderer<'a> {
    output_plan: &'a HtmlResourceOutputPlan,
    context: &'a ResourceUrlContext,
    site_origin: &'a str,
}

impl<'a> StructuralUrlRenderer<'a> {
    /// Create a renderer for one planned artefact and the builder's configured site origin.
    pub(crate) fn new(
        output_plan: &'a HtmlResourceOutputPlan,
        context: &'a ResourceUrlContext,
        site_origin: &'a str,
    ) -> Self {
        Self {
            output_plan,
            context,
            site_origin,
        }
    }

    /// Render an owned folded string at this artefact boundary.
    pub(crate) fn render_owned(&self, value: &OwnedFoldedString) -> Result<String, CompilerError> {
        match value {
            OwnedFoldedString::Text(text) => Ok(text.clone()),
            OwnedFoldedString::Pieces(pieces) => {
                let mut rendered = String::new();
                for piece in pieces {
                    match piece {
                        OwnedFoldedStringPiece::Text(text) => rendered.push_str(text),
                        OwnedFoldedStringPiece::Resource(origin) => {
                            rendered.push_str(&self.render_resource_origin(origin)?)
                        }
                        OwnedFoldedStringPiece::SiteRoot => {
                            rendered.push_str(&self.render_site_root_url())
                        }
                    }
                }
                Ok(rendered)
            }
        }
    }

    /// Build the concrete map consumed by a JS or Wasm lowerer for one selected module variant.
    pub(crate) fn lowering_map(
        &self,
        resources: &ModuleResourceTable,
        reachability: &HirReachability,
    ) -> Result<Arc<StructuralStringUrlMap>, CompilerError> {
        let mut resource_urls = HashMap::with_capacity(reachability.reachable_resource_uses.len());
        for resource_use in &reachability.reachable_resource_uses {
            let origin = &resources.try_origin(resource_use.resource_id)?.origin;
            let rendered_url = self.render_resource_origin(origin)?;
            resource_urls.insert(resource_use.resource_id, rendered_url);
        }

        let site_root_url = if reachability.reachable_site_root_uses.is_empty() {
            None
        } else {
            Some(self.render_site_root_url())
        };

        Ok(Arc::new(StructuralStringUrlMap {
            resource_urls,
            site_root_url,
        }))
    }

    fn render_resource_origin(
        &self,
        origin: &StableResourceOriginId,
    ) -> Result<String, CompilerError> {
        let record = self
            .output_plan
            .record_for_origin(origin)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "HTML structural URL renderer has no planned output path for resource origin {origin:?}"
                ))
            })?;

        let context_path = self.context.artefact_path();
        let context_segments = portable_segments(context_path, "context artefact")?;
        let Some(context_file) = context_segments.len().checked_sub(1) else {
            return Err(CompilerError::compiler_error(
                "HTML structural URL renderer received an empty context artefact path",
            ));
        };
        let context_parent = &context_segments[..context_file];
        let target_segments = portable_segments(&record.output_path, "resource output")?;

        let common_segments = context_parent
            .iter()
            .zip(&target_segments)
            .take_while(|(context, target)| context == target)
            .count();
        let same_or_descendant = target_segments.starts_with(context_parent);

        let mut relative_segments: Vec<String> = Vec::new();
        relative_segments.extend(std::iter::repeat_n(
            String::from(".."),
            context_parent.len() - common_segments,
        ));
        relative_segments.extend(
            target_segments[common_segments..]
                .iter()
                .map(|segment| encode_url_segment(segment)),
        );

        let relative = relative_segments.join("/");
        if same_or_descendant {
            if relative.is_empty() {
                return Ok(String::from("./"));
            }
            return Ok(format!("./{relative}"));
        }

        Ok(relative)
    }

    /// Render the site-root URL for the builder's configured project origin.
    ///
    /// The result always ends in `/`, so an authored suffix such as `[@/]docs/` composes by
    /// concatenation. Project configuration spells an unset origin `/`, which renders as the bare
    /// site root.
    fn render_site_root_url(&self) -> String {
        let prefix = self.site_origin.trim_end_matches('/');

        if prefix.is_empty() {
            return String::from("/");
        }

        format!("{prefix}/")
    }
}

fn portable_segments<'a>(path: &'a Path, path_kind: &str) -> Result<Vec<&'a str>, CompilerError> {
    let raw = path.to_str().ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "HTML structural URL renderer received a non-UTF-8 {path_kind} path"
        ))
    })?;
    if raw.is_empty() || raw.starts_with('/') || raw.starts_with('\\') {
        return Err(CompilerError::compiler_error(format!(
            "HTML structural URL renderer received an invalid {path_kind} path '{raw}'"
        )));
    }

    let segments = raw.split(['/', '\\']).collect::<Vec<_>>();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return Err(CompilerError::compiler_error(format!(
            "HTML structural URL renderer received an invalid {path_kind} path '{raw}'"
        )));
    }

    Ok(segments)
}

fn encode_url_segment(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
#[path = "tests/structural_url_renderer_tests.rs"]
mod tests;
