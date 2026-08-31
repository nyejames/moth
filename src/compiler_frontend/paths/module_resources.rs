//! Module-local dense resource origins.
//!
//! WHAT: interns one record per stable resource origin the module resolves, and issues the dense
//! `ResourceId` handle that structural values carry in place of a repeated stable origin.
//! WHY: one origin is usually named several times in a module. Storing the stable origin once and
//! referring to it by a dense handle keeps resource-heavy modules from paying for every repeat.
//!
//! This table owns identity only. It deliberately holds no record of *where* a resource is used:
//! executable uses are owned by HIR block and function link facts, and non-HIR uses are owned by
//! the fragment and exported folded-value metadata they already travel with. Those owners know
//! which function or fragment a use belongs to, which is what exact liveness needs and what a
//! module-wide use list cannot express.
//!
//! `ResourceId` is module-local. It is always read back through the table that issued it and never
//! crosses a module or generated-sidecar boundary; public interfaces carry
//! `StableResourceOriginId` instead. Those handles are consequently never remerged or renumbered:
//! nothing merges or renumbers a `ModuleResourceTable`, so a merge that moves HIR carrying
//! `ResourceId`s into another table's domain is compiler corruption, and such a case must convert
//! through `StableResourceOriginId` first. The table owns no `PathBuf`, output path or rendered
//! URL.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::paths::file_references::ResourceSourceId;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use rustc_hash::{FxHashMap, FxHashSet};

/// Dense module-local handle for one resolved resource origin.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ResourceId(u32);

impl ResourceId {
    fn from_index(index: usize) -> Self {
        Self(index as u32)
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

/// One resolved resource origin and the location that first introduced it.
///
/// The location is diagnostic context only. It is deliberately outside
/// [`StableResourceOriginId`], so two files in one module that name the same resource intern one
/// origin and the first authored location wins.
#[derive(Clone, Debug)]
pub(crate) struct ModuleResourceOrigin {
    pub(crate) origin: StableResourceOriginId,
    pub(crate) first_authored_location: SourceLocation,
}

/// One compiler-owned semantic resource origin paired with its Stage 0 physical source.
///
/// The association is emitted while AST interns the origin. Build publication consumes this
/// opaque source identity directly instead of reconstructing it from the origin's path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceSourceAssociation {
    pub(crate) origin: StableResourceOriginId,
    pub(crate) source: ResourceSourceId,
}

/// Every resource origin one module resolved.
///
/// Origins are appended in resolution order. That order is deterministic for a given module, but
/// it is not the module's lexical source order: declaration headers are topologically sorted
/// before AST processing. Document, block and source order belong to TIR and HIR link facts.
#[derive(Debug, Default)]
pub(crate) struct ModuleResourceTable {
    origins: Vec<ModuleResourceOrigin>,
    by_origin: FxHashMap<StableResourceOriginId, ResourceId>,
    source_association_keys: FxHashSet<(StableResourceOriginId, ResourceSourceId)>,
    source_associations: Vec<ResourceSourceAssociation>,
}

impl ModuleResourceTable {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Intern one stable origin and return its dense handle.
    ///
    /// A repeated origin returns the existing handle and keeps the first authored location, so
    /// origin data is stored once however many times the module names the resource.
    pub(crate) fn intern_origin(
        &mut self,
        origin: StableResourceOriginId,
        first_authored_location: SourceLocation,
    ) -> ResourceId {
        if let Some(&existing) = self.by_origin.get(&origin) {
            return existing;
        }

        let resource = ResourceId::from_index(self.origins.len());

        self.by_origin.insert(origin.clone(), resource);
        self.origins.push(ModuleResourceOrigin {
            origin,
            first_authored_location,
        });
        add_frontend_counter(FrontendCounter::ResourceOriginCount, 1);

        resource
    }

    /// Intern one origin and retain its explicit Stage 0 source association.
    ///
    /// The source ID is opaque to AST and is carried unchanged to the build publication boundary.
    /// Distinct source IDs for one origin are retained so the boundary can diagnose that
    /// disagreement instead of silently choosing one.
    pub(crate) fn intern_origin_with_source(
        &mut self,
        origin: StableResourceOriginId,
        source: ResourceSourceId,
        first_authored_location: SourceLocation,
    ) -> ResourceId {
        let resource = self.intern_origin(origin.clone(), first_authored_location);
        if self
            .source_association_keys
            .insert((origin.clone(), source))
        {
            self.source_associations
                .push(ResourceSourceAssociation { origin, source });
        }
        resource
    }

    /// Read one origin row through a fallible boundary.
    ///
    /// A handle past the end of the table means a `ResourceId` reached a table that never issued
    /// it, which is compiler corruption rather than an authoring mistake.
    pub(crate) fn try_origin(
        &self,
        resource: ResourceId,
    ) -> Result<&ModuleResourceOrigin, CompilerError> {
        self.origins.get(resource.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "resource handle {} is outside a module resource table of {} origins",
                resource.0,
                self.origins.len()
            ))
        })
    }

    /// Every explicit compiler-produced origin/source association, in interning order.
    pub(crate) fn resource_source_associations(&self) -> &[ResourceSourceAssociation] {
        &self.source_associations
    }
    /// Every resolved origin currently owned by this module, in interning order.
    ///
    /// This is the module-owned identity view used by focused table diagnostics.
    #[allow(dead_code)] // focused table diagnostics inspect current ownership
    pub(crate) fn origins(&self) -> &[ModuleResourceOrigin] {
        &self.origins
    }
    /// Whether this module currently owns no resolved resource origins.
    #[allow(dead_code)] // focused table diagnostics inspect current ownership
    pub(crate) fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }
    /// Remap source-location string IDs after this table's owning module joins a string table.
    ///
    /// Resource identity and dense handles are module-local and never change; only diagnostic
    /// provenance carries interned IDs across the remap boundary.
    pub(crate) fn remap_string_ids(
        &mut self,
        remap: &crate::compiler_frontend::symbols::string_interning::StringIdRemap,
    ) {
        for origin in &mut self.origins {
            origin.first_authored_location.remap_string_ids(remap);
        }
    }
}

#[cfg(test)]
#[path = "tests/module_resources_tests.rs"]
mod tests;
