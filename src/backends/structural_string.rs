//! Concrete structural-string URLs supplied by a builder for one physical variant.
//!
//! WHAT: carries the builder-rendered text for module-local resource handles and site-root pieces
//! into backend lowerers.
//! WHY: backends must lower structural strings without depending on HTML output planning or
//! cloning canonical HIR. The HTML builder owns URL rendering and supplies this small map at the
//! lowering boundary.

use crate::compiler_frontend::paths::module_resources::ResourceId;
use std::collections::HashMap;

/// Concrete text for structural strings in one selected output variant.
#[derive(Debug, Clone, Default)]
pub(crate) struct StructuralStringUrlMap {
    /// URL text keyed by the module-local resource handle carried in HIR.
    pub(crate) resource_urls: HashMap<ResourceId, String>,
    /// URL text for `SiteRoot` pieces, when the consuming builder supplies an origin policy.
    pub(crate) site_root_url: Option<String>,
}
