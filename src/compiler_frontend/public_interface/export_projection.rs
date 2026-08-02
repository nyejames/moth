//! Pre-AST direct-export identity seed for the directly-defined public surface.
//!
//! WHAT: owns the one construction path that turns already-bound, sorted declaration shells and
//!       the header-built public export metadata into the transient [`DirectExportSeed`] at the
//!       semantic compilation boundary. The seed carries only the validated module origin, the
//!       separate free-namespace [`ExportBinding`] values and the transient public nominal-type
//!       origin lookup the post-AST callable seed table needs to resolve receiver paths. It is
//!       the immediate consumer of `StableModuleOriginIdentity`: the module origin becomes the
//!       exporting-module and declaration-origin component of every recorded binding.
//! WHY: the compiler design overview requires public-interface facts to be built once from
//!      retained header facts at the semantic boundary, never by reparsing source or scanning
//!      HIR/AST/backend output. Keeping this construction in one narrow compiler-semantic module
//!      keeps stage ownership clear: the headers own declaration-shell discovery, this module
//!      owns stable export-origin projection, and the declaration-record projection in
//!      `direct_projection` owns the final public semantics.
//!
//! ## Pre-AST projection
//!
//! Free export bindings and the public nominal-type origin index are projected from bound, sorted
//! header shells before AST construction ([`build_direct_export_seed`]). The seed carries these
//! facts plus the module origin so the post-AST callable seed table can be built at one boundary
//! where the resolved public type-root table, receiver entries, generic-template classification and
//! stable export origins are all available. Receiver-method callable identity is not carried by
//! the seed; it lives in the transient callable seed table built from the resolved root table and
//! this seed's nominal-type origin index.
//!
//! The seed is destructured and consumed by [`PublicInterfaceDraftBuilder`](super::direct_projection::PublicInterfaceDraftBuilder):
//! the module origin and export bindings move into the [`PublicInterfaceDraft`](super::model::PublicInterfaceDraft)
//! and the nominal-type origin index is dropped before the draft boundary. It is not a durable
//! aggregate surface: no renamed `DefinedPublicExportOrigins` component survives past the builder.
//!
//! ## Scope
//!
//! Only declarations defined directly in the active module root's public surface are recorded
//! as directly-defined exports: free functions, nominal structs, choices, transparent aliases,
//! constants and traits. Receiver methods are attached to their exported receiver type's surface
//! rather than becoming free namespace bindings. Same-module re-exports from the root's `export:`
//! block that target private-file declarations are joined alongside directly-defined exports.
//! Cross-module re-exports from provider interfaces are joined during recursive closure at
//! publication time.
//!
//! The declaration-kind set recorded here matches the directly-defined public export surface
//! owned by `headers::public_exports` and `ast::module_ast::environment::public_surface`. Those
//! owners keep their own stage-local predicates because their file-role semantics differ
//! (export-capable roots versus the active module root alone); this module's projection is
//! narrower because imported-module-root headers belong to another module's component.

use super::SourceProviderImportSet;
use super::model::{
    PublicBindingExport, PublicDiagnosticLocation, PublicExportDiagnosticProvenance,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::module_symbols::{
    ModuleSymbols, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginConstantId, OriginDeclarationId, OriginFunctionId, OriginTraitId,
    OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity,
};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::{FxHashMap, FxHashSet};

/// Pre-AST seed of the directly-defined public export identity facts.
///
/// WHAT: carries the free-namespace export bindings, the public nominal-type origin index and the
///       validated module origin projected from bound, sorted header shells. The module origin is
///       the table-resolved active root origin, not a loose argument.
/// WHY: free bindings and nominal-type origins depend only on header shells and are safe to
///      project before AST construction. The seed keeps the pre-AST projection and the module
///      origin in one named place so the post-AST callable seed table builder joins them with the
///      resolved public type-root table at one boundary without managing loose values.
#[derive(Debug, PartialEq)]
pub(crate) struct DirectExportSeed {
    module_origin: StableModuleOriginIdentity,
    export_bindings: Vec<ExportBinding>,
    export_diagnostic_provenance: Vec<PublicExportDiagnosticProvenance>,
    binding_exports: Vec<PublicBindingExport>,
    public_nominal_type_origins: FxHashMap<InternedPath, OriginTypeId>,
}

pub(crate) struct DirectExportSeedParts {
    pub(crate) module_origin: StableModuleOriginIdentity,
    pub(crate) export_bindings: Vec<ExportBinding>,
    pub(crate) export_diagnostic_provenance: Vec<PublicExportDiagnosticProvenance>,
    pub(crate) binding_exports: Vec<PublicBindingExport>,
}

impl DirectExportSeed {
    /// Construct the pre-AST seed from the already-built, deterministically ordered bindings,
    /// the public nominal-type origin index and the validated active module origin.
    ///
    /// Compiler-internal: the projection owner assembles the inputs in the documented
    /// deterministic order before calling this. Focused tests build the seed directly to feed the
    /// public-interface draft builder.
    pub(crate) fn new(
        module_origin: StableModuleOriginIdentity,
        export_bindings: Vec<ExportBinding>,
        public_nominal_type_origins: FxHashMap<InternedPath, OriginTypeId>,
    ) -> Self {
        Self {
            module_origin,
            export_bindings,
            export_diagnostic_provenance: Vec::new(),
            binding_exports: Vec::new(),
            public_nominal_type_origins,
        }
    }

    fn with_export_diagnostic_provenance(
        mut self,
        export_diagnostic_provenance: Vec<PublicExportDiagnosticProvenance>,
    ) -> Self {
        self.export_diagnostic_provenance = export_diagnostic_provenance;
        self
    }

    fn with_binding_exports(mut self, binding_exports: Vec<PublicBindingExport>) -> Self {
        self.binding_exports = binding_exports;
        self
    }

    /// The stable origin of the module that owns these directly defined exports.
    pub(crate) fn module_origin(&self) -> &StableModuleOriginIdentity {
        &self.module_origin
    }

    /// The free-namespace export bindings for directly defined public declarations, in
    /// deterministic order. Excludes receiver methods.
    pub(crate) fn export_bindings(&self) -> &[ExportBinding] {
        &self.export_bindings
    }

    /// Authored source provenance for directly-defined public export spellings, in deterministic
    /// public-name order. This is diagnostic metadata, not part of semantic export identity.
    #[cfg(test)]
    pub(crate) fn export_diagnostic_provenance(&self) -> &[PublicExportDiagnosticProvenance] {
        &self.export_diagnostic_provenance
    }

    /// The transient public nominal-type origin index used by the post-AST callable seed table
    /// builder to resolve receiver paths to stable [`OriginTypeId`] values.
    pub(crate) fn public_nominal_type_origins(&self) -> &FxHashMap<InternedPath, OriginTypeId> {
        &self.public_nominal_type_origins
    }

    /// Consume the seed, moving the module origin, export bindings and nominal-type origin index
    /// out so the declaration-record projection and the draft can take ownership.
    ///
    /// The only production consumer is [`PublicInterfaceDraftBuilder::build`](super::direct_projection::PublicInterfaceDraftBuilder::build),
    /// which calls this after the borrowing projections finish so the module origin and export
    /// bindings move into the draft and the nominal-type origin index is dropped.
    pub(crate) fn into_parts(self) -> DirectExportSeedParts {
        let Self {
            module_origin,
            export_bindings,
            export_diagnostic_provenance,
            binding_exports,
            public_nominal_type_origins: _,
        } = self;

        DirectExportSeedParts {
            module_origin,
            export_bindings,
            export_diagnostic_provenance,
            binding_exports,
        }
    }
}

/// Build the pre-AST seed of the directly-defined public export identity facts.
///
/// WHAT: projects the sorted declaration shells and header-built public export metadata into free
///       export bindings and the public nominal-type origin index. The active root's owning
///       stable module origin is resolved from the per-file `SourceModuleOriginTable` using the
///       retained active root `FileId`, not from a loose module-origin argument. It reads no
///       source text, tokens, HIR, AST or backend output. The caller retains the seed
///       only on overall semantic success; a diagnosed module exposes no seed.
/// WHY: the semantic compilation boundary already holds the bound, sorted declaration shells and
///      the public export metadata, so stable export origins are projected here once rather than
///      reconstructed by a later stage. Resolving the origin from the table validates that every
///      directly-defined public declaration belongs to one unique active module origin, instead
///      of trusting a single loose argument.
pub(crate) fn build_direct_export_seed(
    source_module_origins: &SourceModuleOriginTable,
    active_root_file_id: FileId,
    sorted_headers: &[Header],
    module_symbols: &ModuleSymbols,
    source_provider_imports: &SourceProviderImportSet<'_>,
    external_registry: &ExternalPackageRegistry,
    string_table: &StringTable,
) -> Result<DirectExportSeed, CompilerError> {
    let active_origin =
        resolve_active_module_origin(source_module_origins, active_root_file_id, sorted_headers)?;

    let (export_bindings, export_diagnostic_provenance) = collect_free_export_bindings(
        source_module_origins,
        &active_origin,
        sorted_headers,
        module_symbols,
        source_provider_imports,
        string_table,
    )?;
    let public_nominal_type_origins = index_public_nominal_type_origins(
        &active_origin,
        &export_bindings,
        sorted_headers,
        module_symbols,
        string_table,
    )?;
    let binding_exports = collect_binding_exports(
        &active_origin,
        module_symbols,
        source_provider_imports,
        external_registry,
        string_table,
    )?;

    Ok(
        DirectExportSeed::new(active_origin, export_bindings, public_nominal_type_origins)
            .with_export_diagnostic_provenance(export_diagnostic_provenance)
            .with_binding_exports(binding_exports),
    )
}

fn collect_binding_exports(
    module_origin: &StableModuleOriginIdentity,
    module_symbols: &ModuleSymbols,
    source_provider_imports: &SourceProviderImportSet<'_>,
    external_registry: &ExternalPackageRegistry,
    string_table: &StringTable,
) -> Result<Vec<PublicBindingExport>, CompilerError> {
    // Synthetic single-file compilation does not construct a Stage 0 module namespace and never
    // publishes a provider interface. Binding-backed re-exports belong to canonical graph jobs,
    // whose prepared symbols always carry module-root membership.
    if module_symbols.file_module_membership.is_empty() {
        return Ok(Vec::new());
    }

    let has_direct_binding_export = module_symbols
        .module_root_public_exports
        .values()
        .chain(module_symbols.source_package_public_exports.values())
        .flatten()
        .any(|entry| matches!(entry.target, PublicExportTarget::External(_)));
    if !has_direct_binding_export && source_provider_imports.is_empty() {
        return Ok(Vec::new());
    }

    let active_module_root = resolve_active_module_root_membership(module_symbols)?;
    let active_root_source = resolve_active_root_source(module_symbols, active_module_root)?;
    let mut entries = Vec::new();

    if let Some(root_entries) = module_symbols
        .module_root_public_exports
        .get(active_module_root)
    {
        entries.extend(root_entries);
    }
    if let Some(package_prefix) = module_symbols
        .file_package_membership
        .get(active_root_source)
        && let Some(package_entries) = module_symbols
            .source_package_public_exports
            .get(package_prefix)
    {
        entries.extend(package_entries);
    }

    let mut exports = Vec::new();
    for entry in entries {
        let target = match &entry.target {
            PublicExportTarget::External(symbol_id) => external_registry
                .canonical_symbol_identity(*symbol_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "binding re-export construction: external symbol {:?} has no canonical package identity",
                        symbol_id
                    ))
                })?,
            PublicExportTarget::Source(target_path) => {
                let Some(provider_interface) = source_provider_imports.resolve_reexport(
                    active_root_source,
                    target_path,
                    string_table,
                ) else {
                    continue;
                };
                let imported_name = target_path.name_str(string_table).ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "binding re-export construction: provider target {:?} has no imported name",
                        target_path
                    ))
                })?;
                let Some(binding) = provider_interface.binding_export(imported_name) else {
                    continue;
                };
                binding.target.clone()
            }
        };

        exports.push(PublicBindingExport {
            exporting_module: module_origin.clone(),
            public_name: string_table.resolve(entry.export_name).to_owned(),
            target,
        });
    }

    exports.sort_by(|left, right| left.public_name.cmp(&right.public_name));
    exports.dedup_by(|left, right| left == right);
    for pair in exports.windows(2) {
        if pair[0].public_name == pair[1].public_name {
            return Err(CompilerError::compiler_error(format!(
                "binding re-export construction: duplicate public export name '{}' in module {:?}",
                pair[0].public_name, module_origin
            )));
        }
    }
    Ok(exports)
}

/// Resolve the one active module origin from the per-file source-origin table.
///
/// WHAT: resolves the active root's owning origin by looking up `active_root_file_id` in the
///       `SourceModuleOriginTable`. The active root must have an owning project-module origin;
///       an unowned active root is an internal failure, not a fallback to a loose argument. This
///       validation runs even when the module has zero directly-defined public exports, so the
///       empty seed still carries a validated active origin. Every directly-defined public
///       header must carry a retained `file_id` and its table origin must equal the active root
///       origin, catching any header that does not belong to the active module root.
/// WHY: the projection must not trust a loose module-origin argument. The table makes the owning
///      origin a per-file fact derived from the graph, so the projection validates the active
///      root's origin instead of assuming it. Removing the canonical-path fallback ensures every
///      declaration identity enters through the same retained file identity, not a path-derived
///      guess.
fn resolve_active_module_origin(
    source_module_origins: &SourceModuleOriginTable,
    active_root_file_id: FileId,
    sorted_headers: &[Header],
) -> Result<StableModuleOriginIdentity, CompilerError> {
    let active_origin = source_module_origins
        .origin_for(active_root_file_id)?
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "defined public export-origin construction: the active root (file id {}) has no owning module origin in the source module origin table",
                active_root_file_id.0
            ))
        })?
        .clone();

    for header in sorted_headers {
        if !is_directly_defined_public_export(header) {
            continue;
        }

        // Preparation sets `file_id` on every prepared Moth file's tokens, so a
        // directly-defined public header without one is an internal invariant violation, not a
        // path-resolution fallback case.
        let file_id = header.tokens.file_id.ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "defined public export-origin construction: a directly-defined public header has no retained file identity (logical path: {:?})",
                header.source_file
            ))
        })?;

        let header_origin = source_module_origins
            .origin_for(file_id)?
            .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "defined public export-origin construction: a directly-defined public header's source file (file id {}) has no owning module origin",
                file_id.0
            ))
        })?;

        if header_origin != &active_origin {
            return Err(CompilerError::compiler_error(format!(
                "defined public export-origin construction: a directly-defined public header's owning module origin ({:?}) does not match the active root origin ({:?})",
                header_origin, active_origin
            )));
        }
    }

    Ok(active_origin)
}

/// Index the stable type origins of directly-defined public nominal types (structs and choices)
/// by canonical declaration path, for receiver-method resolution.
///
/// Receiver methods travel with their receiver type, so a method is part of the public surface
/// only when its receiver type is a directly-defined public nominal type. Canonical paths are
/// unique within a module, so a receiver path resolves to at most one nominal type. The same-file
/// nominal rule (AST-validated) guarantees that a method whose receiver path matches a public type
/// is defined in the same file as that type; this projection runs only on overall semantic success,
/// so the rule already holds.
fn index_public_nominal_type_origins(
    module_origin: &StableModuleOriginIdentity,
    export_bindings: &[ExportBinding],
    sorted_headers: &[Header],
    module_symbols: &ModuleSymbols,
    string_table: &StringTable,
) -> Result<FxHashMap<InternedPath, OriginTypeId>, CompilerError> {
    let mut nominal_type_origins = FxHashMap::default();
    let active_module_root = resolve_optional_active_module_root_membership(module_symbols)?;

    for header in sorted_headers {
        // A directly-defined public declaration always has a defining name: the header parser
        // records one for every authored declaration shell. A missing name here is an impossible
        // metadata gap, not an intentional exclusion, so it must surface as an internal failure
        // rather than silently omitting a public nominal type from the seed.
        let Some(name) = header.tokens.src_path.name_str(string_table) else {
            return Err(CompilerError::compiler_error(format!(
                "defined public export-origin construction: a directly-defined public nominal type header has no resolvable defining name (path: {:?})",
                header.tokens.src_path
            )));
        };

        let category = match &header.kind {
            HeaderKind::Struct { .. } => OriginTypeCategory::Struct,
            HeaderKind::Choice { .. } => OriginTypeCategory::Choice,
            _ => continue,
        };
        let origin = OriginTypeId::new(module_origin.clone(), name.to_owned(), category);
        let is_exported_origin = export_bindings.iter().any(|binding| {
            matches!(binding.origin(), OriginDeclarationId::Type(exported) if exported == &origin)
        });
        if !is_exported_origin {
            continue;
        }
        let belongs_to_active_module = match active_module_root {
            Some(active_module_root) => module_symbols
                .canonical_source_by_symbol_path
                .get(&header.tokens.src_path)
                .and_then(|source| module_symbols.file_module_membership.get(source))
                .is_some_and(|module_root| module_root == active_module_root),
            None => is_directly_defined_public_export(header),
        };
        if !belongs_to_active_module {
            continue;
        }

        nominal_type_origins.insert(header.tokens.src_path.clone(), origin);
    }

    Ok(nominal_type_origins)
}

/// Build the transient stable public source-nominal origin index for the type-surface projection.
///
/// WHAT: maps canonical declaration paths to stable [`OriginTypeId`] values for every
///       `Struct`/`Choice` declaration whose canonical source path is targeted by at least one
///       retained module-root or source-package public export entry, deriving each origin from
///       the header's retained [`FileId`] through the [`SourceModuleOriginTable`]. This mirrors
///       the AST `source_path_is_public_from_root_file` nameability owner: a nominal is public/
///       nameable when a retained public export entry targets its source path. That single rule
///       covers directly-defined active-root public nominal roots, imported project-graph public
///       nominal roots (each targeted by its own module root's public export entry) and
///       privately-authored nominals exposed through a public alias or re-export (a normal-file
///       declaration targeted by a module-root public export entry). It excludes private
///       nominal declarations with no public export target. Active-root nominals resolve to the
///       active module origin; imported project-graph nominals resolve to their defining provider
///       module origin, so a directly-defined public signature or field that references an
///       imported public nominal projects to `SourceNominal(provider_origin)` rather than the
///       active module origin. A source-package header whose `FileId` table entry is `None` (no
///       project-module owner) is deliberately absent from the index: its nominals are not
///       project-graph-owned and must not receive a fabricated origin, and a projected public type
///       that requires one fails through the total nominal resolver with a precise `CompilerError`.
/// WHY: the directly-defined active-root index kept on the seed for receiver-surface
///      finalization excludes imported and alias-target nominals by design, because imported and
///      alias-target receiver surfaces belong to their defining module and must not enter this
///      module's declaration records. Canonical type projection still has to resolve those nominal
///      references, so this expanded index is built once from the already-sorted retained headers,
///      the header-built public export maps and the per-file origin table without re-scanning
///      source. It never invents a second visibility rule from `FileRole` and `export_mode` alone:
///      the public export targeting fact already retained by `headers::public_exports` is the
///      single authority. It is transient: it exists only to feed the projection and is not
///      retained on the seed.
///
/// Rejects a missing `FileId`, an out-of-range table lookup, a duplicate canonical nominal path,
/// a category inconsistency or a conflicting origin explicitly. It never silently overwrites an
/// existing entry.
pub(crate) fn build_public_source_nominal_origin_index(
    source_module_origins: &SourceModuleOriginTable,
    sorted_headers: &[Header],
    module_symbols: &ModuleSymbols,
    string_table: &StringTable,
) -> Result<FxHashMap<InternedPath, OriginTypeId>, CompilerError> {
    let mut origins: FxHashMap<InternedPath, OriginTypeId> = FxHashMap::default();

    for header in sorted_headers {
        if !is_public_export_targeted_nominal_declaration(header, module_symbols) {
            continue;
        }

        // A public export-targeted declaration always carries a defining name recorded by the
        // header parser. A missing name is an impossible metadata gap that must not silently
        // omit a public nominal type from the transient resolver.
        let Some(name) = header.tokens.src_path.name_str(string_table) else {
            return Err(CompilerError::compiler_error(format!(
                "defined public export-origin construction: a public export-targeted nominal type header has no resolvable defining name (path: {:?})",
                header.tokens.src_path
            )));
        };

        let category = match &header.kind {
            HeaderKind::Struct { .. } => OriginTypeCategory::Struct,
            HeaderKind::Choice { .. } => OriginTypeCategory::Choice,
            _ => continue,
        };

        // Preparation assigns a retained FileId to every prepared file's tokens, so a public
        // export-targeted header without one is an internal invariant violation rather than an
        // intentional exclusion.
        let Some(file_id) = header.tokens.file_id else {
            return Err(CompilerError::compiler_error(format!(
                "defined public export-origin construction: a public export-targeted nominal type header has no retained FileId (path: {:?})",
                header.tokens.src_path
            )));
        };

        // A source-package file outside the project module graph has an explicit None owning
        // origin. It is deliberately absent from the index; a projected public type that requires
        // its nominal fails through the total nominal resolver with a precise CompilerError.
        let Some(module_origin) = source_module_origins.origin_for(file_id)? else {
            continue;
        };

        let origin = OriginTypeId::new(module_origin.clone(), name.to_owned(), category);

        if let Some(existing) = origins.get(&header.tokens.src_path) {
            return Err(CompilerError::compiler_error(format!(
                "defined public export-origin construction: a duplicate canonical nominal path resolves to conflicting origins (path: {:?}; existing {:?}, new {:?})",
                header.tokens.src_path, existing, origin
            )));
        }
        origins.insert(header.tokens.src_path.clone(), origin);
    }

    Ok(origins)
}

/// Build the transient stable public source-trait origin index for bound projection.
///
/// WHAT: maps each directly-defined, imported project-graph or public-alias-target trait
///       declaration's canonical path to a stable `OriginTraitId`, so a bound that references an
///       imported or alias-target project-graph trait resolves to that trait's defining provider
///       module origin rather than the active module origin. A source-package header whose
///       `FileId` table entry is `None` (no project-module owner) is deliberately absent from the
///       index: its trait is not project-graph-owned and must not receive a fabricated origin,
///       and a projected public bound that requires one fails through the total bound resolver
///       with a precise `CompilerError`.
///
/// Reuses the shared [`PublicExportTarget::is_source_path`] authority via
/// [`any_retained_public_export_targets_source_path`] so trait origin indexing and nominal
/// origin indexing cannot drift on what a public export targets. It never uses display/path
/// identity fallback.
///
/// Rejects a missing `FileId`, an out-of-range table lookup, a duplicate canonical trait path
/// or a conflicting origin explicitly. It never silently overwrites an existing entry.
pub(crate) fn build_public_source_trait_origin_index(
    source_module_origins: &SourceModuleOriginTable,
    sorted_headers: &[Header],
    module_symbols: &ModuleSymbols,
    string_table: &StringTable,
) -> Result<FxHashMap<InternedPath, OriginTraitId>, CompilerError> {
    let mut origins: FxHashMap<InternedPath, OriginTraitId> = FxHashMap::default();

    for header in sorted_headers {
        if !is_public_export_targeted_trait_declaration(header, module_symbols) {
            continue;
        }

        let Some(name) = header.tokens.src_path.name_str(string_table) else {
            return Err(CompilerError::compiler_error(format!(
                "defined public export-origin construction: a public export-targeted trait header has no resolvable defining name (path: {:?})",
                header.tokens.src_path
            )));
        };

        let Some(file_id) = header.tokens.file_id else {
            return Err(CompilerError::compiler_error(format!(
                "defined public export-origin construction: a public export-targeted trait header has no retained FileId (path: {:?})",
                header.tokens.src_path
            )));
        };

        let Some(module_origin) = source_module_origins.origin_for(file_id)? else {
            continue;
        };

        let origin = OriginTraitId::new(module_origin.clone(), name.to_owned());

        if let Some(existing) = origins.get(&header.tokens.src_path) {
            return Err(CompilerError::compiler_error(format!(
                "defined public export-origin construction: a duplicate canonical trait path resolves to conflicting origins (path: {:?}; existing {:?}, new {:?})",
                header.tokens.src_path, existing, origin
            )));
        }
        origins.insert(header.tokens.src_path.clone(), origin);
    }

    Ok(origins)
}

/// Whether a header is a nominal-type declaration whose canonical source path is targeted by a
/// retained public export entry.
///
/// WHAT: admits a `Struct`/`Choice` declaration when at least one retained module-root or
///       source-package public export entry targets its canonical source path. This mirrors the
///       AST `source_path_is_public_from_root_file` nameability owner using the shared
///       [`PublicExportTarget::is_source_path`] predicate, so origin indexing and nameability
///       cannot drift on what a public export targets. Unlike
///       [`is_directly_defined_public_export`] this does not gate on `FileRole` or `export_mode`:
///       a normal-file declaration with no public export of its own is admitted when a module-root
///       public alias or re-export targets it, and an imported module-root public nominal is
///       admitted because its own module root's public export entry targets it.
fn is_public_export_targeted_nominal_declaration(
    header: &Header,
    module_symbols: &ModuleSymbols,
) -> bool {
    matches!(
        &header.kind,
        HeaderKind::Struct { .. } | HeaderKind::Choice { .. }
    ) && any_retained_public_export_targets_source_path(module_symbols, &header.tokens.src_path)
}

/// Whether a header is a trait declaration whose canonical source path is targeted by a
/// retained public export entry.
///
/// WHAT: admits a `Trait` declaration when at least one retained module-root or source-package
///       public export entry targets its canonical source path. This mirrors the nominal
///       origin index using the shared [`PublicExportTarget::is_source_path`] predicate, so
///       trait origin indexing and nameability cannot drift on what a public export targets.
fn is_public_export_targeted_trait_declaration(
    header: &Header,
    module_symbols: &ModuleSymbols,
) -> bool {
    matches!(&header.kind, HeaderKind::Trait { .. })
        && any_retained_public_export_targets_source_path(module_symbols, &header.tokens.src_path)
}

/// Whether any retained module-root or source-package public export entry targets the given
/// source declaration path.
///
/// WHAT: membership-only scan over the header-built public export maps. Both maps are retained
///       header facts, so this is a header-owned query, not AST nameability policy: the AST owner
///       keeps its per-root-file scoping and builtin visibility, and this index uses the same
///       shared [`PublicExportTarget::is_source_path`] predicate for the entry-target match.
fn any_retained_public_export_targets_source_path(
    module_symbols: &ModuleSymbols,
    path: &InternedPath,
) -> bool {
    module_symbols
        .module_root_public_exports
        .values()
        .any(|entries| {
            entries
                .iter()
                .any(|entry| entry.target.is_source_path(path))
        })
        || module_symbols
            .source_package_public_exports
            .values()
            .any(|entries| {
                entries
                    .iter()
                    .any(|entry| entry.target.is_source_path(path))
            })
}

/// Collect the free-namespace export bindings for directly-defined public declarations and
/// re-exported same-module declarations.
///
/// Receiver methods are excluded here: they are attached to their receiver through the
/// callable seed table (see `build_callable_seed_table`) and must not become independent
/// free namespace bindings.
///
/// Re-exports from the root's `export:` block that target same-module private-file declarations
/// are joined here alongside directly-defined root-file exports. Each re-export's public name
/// comes from the `module_root_public_exports` entry and its origin is derived from the target
/// declaration's header through the `SourceModuleOriginTable`.
fn collect_free_export_bindings(
    source_module_origins: &SourceModuleOriginTable,
    module_origin: &StableModuleOriginIdentity,
    sorted_headers: &[Header],
    module_symbols: &ModuleSymbols,
    source_provider_imports: &SourceProviderImportSet<'_>,
    string_table: &StringTable,
) -> Result<(Vec<ExportBinding>, Vec<PublicExportDiagnosticProvenance>), CompilerError> {
    let mut export_bindings = Vec::new();
    let mut export_diagnostic_provenance = Vec::new();
    let mut seen_public_names: FxHashSet<String> = FxHashSet::default();

    for header in sorted_headers {
        if !is_directly_defined_public_export(header) {
            continue;
        }

        // A directly-defined public authored declaration always has a defining name. A missing
        // name is an impossible metadata gap that must not silently omit a public export from the
        // seed.
        let Some(name) = header.tokens.src_path.name_str(string_table) else {
            return Err(CompilerError::compiler_error(format!(
                "defined public export-origin construction: a directly-defined public declaration header has no resolvable defining name (path: {:?})",
                header.tokens.src_path
            )));
        };

        let Some(origin) =
            free_export_declaration_origin(header, module_origin, module_symbols, name)
        else {
            // The only public declaration that returns `None` after the public check passed is a
            // receiver method, which travels with its receiver surface instead of the free
            // namespace. That exclusion is intentional.
            continue;
        };
        seen_public_names.insert(name.to_owned());
        let public_name = name.to_owned();
        export_bindings.push(ExportBinding::new(
            module_origin.clone(),
            public_name.clone(),
            origin,
        ));
        export_diagnostic_provenance.push(PublicExportDiagnosticProvenance {
            public_name,
            location: portable_source_location(&header.name_location, string_table),
        });
    }

    // Collect re-export bindings from the module root's public export entries. These entries
    // cover declarations from private files re-exported through the root's `export:` block.
    // Directly-defined exports already collected above are skipped by checking the public name.
    let reexport_bindings = collect_reexport_bindings(
        source_module_origins,
        module_origin,
        sorted_headers,
        module_symbols,
        source_provider_imports,
        string_table,
    )?;

    for reexport in reexport_bindings {
        if seen_public_names.insert(reexport.binding.public_name().to_owned()) {
            let public_name = reexport.binding.public_name().to_owned();
            export_bindings.push(reexport.binding);
            if let Some(location) = reexport.provenance {
                export_diagnostic_provenance.push(PublicExportDiagnosticProvenance {
                    public_name,
                    location,
                });
            }
        }
    }

    // Deterministic order independent of hash-map iteration and declaration scheduling: sort by
    // public name, then by declaration category so two bindings can never tie ambiguously.
    export_bindings.sort_by(|left, right| {
        left.public_name().cmp(right.public_name()).then_with(|| {
            declaration_category_rank(left.origin()).cmp(&declaration_category_rank(right.origin()))
        })
    });

    export_diagnostic_provenance.sort_by(|left, right| left.public_name.cmp(&right.public_name));
    Ok((export_bindings, export_diagnostic_provenance))
}

fn portable_source_location(
    location: &SourceLocation,
    string_table: &StringTable,
) -> PublicDiagnosticLocation {
    PublicDiagnosticLocation {
        scope_components: location
            .scope
            .as_components()
            .iter()
            .map(|component| string_table.resolve(*component).to_owned())
            .collect(),
        start_line: location.start_pos.line_number,
        start_column: location.start_pos.char_column,
        end_line: location.end_pos.line_number,
        end_column: location.end_pos.char_column,
    }
}

/// Collect re-export bindings from `module_root_public_exports` and
/// `source_package_public_exports` entries that target same-module source declarations.
///
/// WHAT: iterates the header-built public export maps for the active module root and source
///       packages. Each `PublicExportTarget::Source(path)` entry whose target declaration path
///       belongs to the active module is resolved to an `ExportBinding` with the export name and
///       the declaration's stable origin. `External` targets are deferred to the binding-backed
///       re-export owner.
/// WHY: the `export:` block may re-export declarations from private files within the same module.
/// Those declarations are not in the active root file, so `is_directly_defined_public_export`
/// excludes them. The header-built public export maps already resolved the re-export target, so
/// this function joins them into the export seed without a second source scan.
fn collect_reexport_bindings(
    source_module_origins: &SourceModuleOriginTable,
    module_origin: &StableModuleOriginIdentity,
    sorted_headers: &[Header],
    module_symbols: &ModuleSymbols,
    source_provider_imports: &SourceProviderImportSet<'_>,
    string_table: &StringTable,
) -> Result<Vec<ReexportBinding>, CompilerError> {
    if module_symbols.file_module_membership.is_empty() {
        return Ok(Vec::new());
    }

    let active_module_root = resolve_active_module_root_membership(module_symbols)?;
    let active_root_source = resolve_active_root_source(module_symbols, active_module_root)?;

    // Build a lookup from canonical source path to header so re-export targets can find their
    // declaration header without iterating the full header list for each entry.
    let mut header_by_path: FxHashMap<&InternedPath, &Header> = FxHashMap::default();
    for header in sorted_headers {
        header_by_path.insert(&header.tokens.src_path, header);
    }

    let mut bindings = Vec::new();
    let context = ReexportBindingContext {
        source_module_origins,
        module_origin,
        active_module_root,
        module_symbols,
        header_by_path: &header_by_path,
        source_provider_imports,
        string_table,
    };

    // Collect re-exports from module root public exports. Only entries targeting declarations
    // that belong to the active module are included. Cross-module re-exports from provider
    // interfaces are handled by the recursive closure step during publication.
    if let Some(entries) = module_symbols
        .module_root_public_exports
        .get(active_module_root)
    {
        for entry in entries {
            collect_one_reexport_binding(&mut bindings, active_root_source, entry, &context)?;
        }
    }

    Ok(bindings)
}

struct ReexportBindingContext<'a> {
    source_module_origins: &'a SourceModuleOriginTable,
    module_origin: &'a StableModuleOriginIdentity,
    active_module_root: &'a InternedPath,
    module_symbols: &'a ModuleSymbols,
    header_by_path: &'a FxHashMap<&'a InternedPath, &'a Header>,
    source_provider_imports: &'a SourceProviderImportSet<'a>,
    string_table: &'a StringTable,
}

struct ReexportBinding {
    binding: ExportBinding,
    provenance: Option<PublicDiagnosticLocation>,
}

fn resolve_active_root_source<'a>(
    module_symbols: &'a ModuleSymbols,
    active_module_root: &InternedPath,
) -> Result<&'a InternedPath, CompilerError> {
    module_symbols
        .file_roles_by_source
        .iter()
        .find_map(|(source, role)| {
            (role.is_active_module_root()
                && module_symbols.file_module_membership.get(source) == Some(active_module_root))
                .then_some(source)
        })
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "re-export binding construction: active module-root membership has no active root source",
            )
        })
}

/// Resolve the one module-root membership owned by the active compilation.
///
/// WHAT: joins the header-owned active-root file role to the canonical module-membership table.
/// WHY: retained provider headers may currently carry the same preliminary stable origin as the
/// active root on compatibility compilation paths. Module membership is already the authoritative
/// header-stage boundary, so same-module re-export collection must use it rather than `FileRole`
/// or stable-origin equality alone.
fn resolve_active_module_root_membership(
    module_symbols: &ModuleSymbols,
) -> Result<&InternedPath, CompilerError> {
    resolve_optional_active_module_root_membership(module_symbols)?.ok_or_else(|| {
        CompilerError::compiler_error(
            "re-export binding construction: active root source has no module-root membership",
        )
    })
}

fn resolve_optional_active_module_root_membership(
    module_symbols: &ModuleSymbols,
) -> Result<Option<&InternedPath>, CompilerError> {
    let mut active_module_root = None;

    for (source, role) in &module_symbols.file_roles_by_source {
        if !role.is_active_module_root() {
            continue;
        }
        let Some(module_root) = module_symbols.file_module_membership.get(source) else {
            continue;
        };

        if active_module_root.is_some_and(|existing| existing != module_root) {
            return Err(CompilerError::compiler_error(
                "re-export binding construction: active root sources resolve to more than one module-root membership",
            ));
        }
        active_module_root = Some(module_root);
    }

    Ok(active_module_root)
}

/// Resolve one re-export entry to an `ExportBinding` if it targets a same-module source
/// declaration.
fn collect_one_reexport_binding(
    bindings: &mut Vec<ReexportBinding>,
    exporting_source: &InternedPath,
    entry: &PublicExportEntry,
    context: &ReexportBindingContext<'_>,
) -> Result<(), CompilerError> {
    let PublicExportTarget::Source(target_path) = &entry.target else {
        // External targets are deferred to the binding-backed re-export owner.
        return Ok(());
    };

    // Cross-module re-exports resolve through the immutable provider interface selected by
    // Stage 0. The origin remains provider-owned while this module owns the public alias.
    if let Some(provider_interface) = context.source_provider_imports.resolve_reexport(
        exporting_source,
        target_path,
        context.string_table,
    ) {
        let imported_name = target_path.name_str(context.string_table).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "re-export binding construction: a provider target has no resolvable imported name (path: {:?})",
                target_path
            ))
        })?;
        let Some(provider_origin) = provider_interface.exported_origin(imported_name).cloned()
        else {
            if provider_interface.binding_export(imported_name).is_some() {
                return Ok(());
            }
            return Err(CompilerError::compiler_error(format!(
                "re-export binding construction: completed provider interface {:?} has no public binding '{}' required by target {:?}",
                provider_interface.module_origin, imported_name, target_path
            )));
        };
        let export_name = context.string_table.resolve(entry.export_name).to_owned();
        bindings.push(ReexportBinding {
            binding: ExportBinding::new(
                context.module_origin.clone(),
                export_name,
                provider_origin,
            ),
            provenance: provider_interface
                .export_diagnostic_provenance(imported_name)
                .cloned(),
        });
        return Ok(());
    }

    // Same-module re-exports join the retained local declaration header.
    let Some(header) = context.header_by_path.get(target_path) else {
        return Err(CompilerError::compiler_error(format!(
            "re-export binding construction: source target {:?} has neither a local declaration header nor a completed provider interface",
            target_path
        )));
    };

    let target_belongs_to_active_module = context
        .module_symbols
        .canonical_source_by_symbol_path
        .get(target_path)
        .and_then(|source| context.module_symbols.file_module_membership.get(source))
        .is_some_and(|module_root| module_root == context.active_module_root);
    if !target_belongs_to_active_module {
        return Ok(());
    }

    // Only include declarations whose graph-derived owner is the active module. `FileRole::Normal`
    // is not sufficient here because the retained header set also contains ordinary private files
    // from imported provider modules. Provider declarations remain references to provider
    // interfaces; they must never become consumer-owned direct bindings.
    let file_id = header.tokens.file_id.ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "re-export binding construction: a re-export target declaration has no retained file identity (path: {:?})",
            target_path
        ))
    })?;
    let Some(target_origin) = context.source_module_origins.origin_for(file_id)? else {
        return Ok(());
    };
    if target_origin != context.module_origin {
        return Ok(());
    }

    // The target declaration must have a defining name to build a stable origin.
    let Some(name) = header.tokens.src_path.name_str(context.string_table) else {
        return Err(CompilerError::compiler_error(format!(
            "re-export binding construction: a re-export target declaration has no resolvable defining name (path: {:?})",
            target_path
        )));
    };

    // Determine the declaration category from the header kind and build the origin.
    let origin = reexport_declaration_origin(header, context.module_origin, name)?;

    let export_name = context.string_table.resolve(entry.export_name).to_owned();
    bindings.push(ReexportBinding {
        binding: ExportBinding::new(context.module_origin.clone(), export_name, origin),
        provenance: Some(portable_source_location(
            &header.name_location,
            context.string_table,
        )),
    });

    Ok(())
}

/// Resolve the stable origin for one re-exported declaration from its header kind.
///
/// WHAT: builds the `OriginDeclarationId` from the module origin and the declaration's defining
///       name. Receiver methods are excluded because they travel with their receiver surface.
fn reexport_declaration_origin(
    header: &Header,
    module_origin: &StableModuleOriginIdentity,
    defining_name: &str,
) -> Result<OriginDeclarationId, CompilerError> {
    match &header.kind {
        HeaderKind::Function { .. } => Ok(OriginDeclarationId::Function(
            OriginFunctionId::new_free(module_origin.clone(), defining_name.to_owned()),
        )),
        HeaderKind::Struct { .. } => Ok(OriginDeclarationId::Type(OriginTypeId::new(
            module_origin.clone(),
            defining_name.to_owned(),
            OriginTypeCategory::Struct,
        ))),
        HeaderKind::Choice { .. } => Ok(OriginDeclarationId::Type(OriginTypeId::new(
            module_origin.clone(),
            defining_name.to_owned(),
            OriginTypeCategory::Choice,
        ))),
        HeaderKind::TypeAlias { .. } => Ok(OriginDeclarationId::Type(OriginTypeId::new(
            module_origin.clone(),
            defining_name.to_owned(),
            OriginTypeCategory::TransparentAlias,
        ))),
        HeaderKind::Constant { .. } => Ok(OriginDeclarationId::Constant(OriginConstantId::new(
            module_origin.clone(),
            defining_name.to_owned(),
        ))),
        HeaderKind::Trait { .. } => Ok(OriginDeclarationId::Trait(OriginTraitId::new(
            module_origin.clone(),
            defining_name.to_owned(),
        ))),
        // Non-declaration headers are not valid re-export targets.
        HeaderKind::StartFunction
        | HeaderKind::ConstTemplate { .. }
        | HeaderKind::TraitConformance { .. }
        | HeaderKind::TraitIncompatibility { .. } => Err(CompilerError::compiler_error(format!(
            "re-export binding construction: a re-export target is a non-declaration header kind (path: {:?})",
            header.tokens.src_path
        ))),
    }
}

/// Resolve the stable origin for one directly-defined public free-namespace export declaration,
/// or `None` when the header is not a directly-defined public export.
///
/// Returns `None` for receiver methods (handled by the receiver-surface path), private
/// declarations, imported-module-root declarations, the implicit start function, const templates
/// and trait relations that are not declarations.
fn free_export_declaration_origin(
    header: &Header,
    module_origin: &StableModuleOriginIdentity,
    module_symbols: &ModuleSymbols,
    defining_name: &str,
) -> Option<OriginDeclarationId> {
    if !is_directly_defined_public_export(header) {
        return None;
    }

    match &header.kind {
        HeaderKind::Function { .. } => {
            // A public function in the active module root's export surface is a free namespace
            // export unless it is a receiver method, which travels with its receiver type's
            // surface instead.
            if module_symbols
                .receiver_method_paths
                .contains(&header.tokens.src_path)
            {
                None
            } else {
                Some(OriginDeclarationId::Function(OriginFunctionId::new_free(
                    module_origin.clone(),
                    defining_name.to_owned(),
                )))
            }
        }
        HeaderKind::Struct { .. } => Some(OriginDeclarationId::Type(OriginTypeId::new(
            module_origin.clone(),
            defining_name.to_owned(),
            OriginTypeCategory::Struct,
        ))),
        HeaderKind::Choice { .. } => Some(OriginDeclarationId::Type(OriginTypeId::new(
            module_origin.clone(),
            defining_name.to_owned(),
            OriginTypeCategory::Choice,
        ))),
        HeaderKind::TypeAlias { .. } => Some(OriginDeclarationId::Type(OriginTypeId::new(
            module_origin.clone(),
            defining_name.to_owned(),
            OriginTypeCategory::TransparentAlias,
        ))),
        HeaderKind::Constant { .. } => Some(OriginDeclarationId::Constant(OriginConstantId::new(
            module_origin.clone(),
            defining_name.to_owned(),
        ))),
        HeaderKind::Trait { .. } => Some(OriginDeclarationId::Trait(OriginTraitId::new(
            module_origin.clone(),
            defining_name.to_owned(),
        ))),
        HeaderKind::StartFunction
        | HeaderKind::ConstTemplate { .. }
        | HeaderKind::TraitConformance { .. }
        | HeaderKind::TraitIncompatibility { .. } => None,
    }
}

/// Whether a header is a public declaration authored directly in the active module root.
///
/// WHAT: narrows the export surface to the active module's own declarations. The active module
///       root is the file being compiled; imported-module-root headers belong to another
///       module's surface and are excluded. Only declarations marked public by a strict
///       `export:` block are admitted. The declaration-kind gate is the shared
///       [`HeaderKind::is_authored_public_export_declaration`] owner so the header, AST and
///       semantic-origin public-export predicates cannot drift; this predicate keeps the
///       stage-local file-role and export-mode policy (active module root plus explicit `Public`
///       mode), which is narrower than the header and AST predicates that also accept
///       imported-module-root declarations. Non-declaration headers (start function, const
///       templates, trait relations) are rejected here and again by the category match in
///       `free_export_declaration_origin`.
fn is_directly_defined_public_export(header: &Header) -> bool {
    header.file_role.is_active_module_root()
        && header.export_mode.is_public()
        && header.kind.is_authored_public_export_declaration()
}

/// A deterministic rank for an exported declaration category, used only as a sort tiebreaker.
fn declaration_category_rank(origin: &OriginDeclarationId) -> u8 {
    match origin {
        OriginDeclarationId::Function(_) => 0,
        OriginDeclarationId::Type(_) => 1,
        OriginDeclarationId::Constant(_) => 2,
        OriginDeclarationId::Trait(_) => 3,
    }
}
