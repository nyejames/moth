//! Construction of the stable identity component for directly-defined public exports.
//!
//! WHAT: owns the one construction path that turns already-bound, sorted declaration shells and
//!       the header-built public export metadata into the immutable
//!       [`DefinedPublicExportOrigins`] component at the semantic compilation boundary. It is the
//!       immediate consumer of `StableModuleOriginIdentity`: the module origin
//!       becomes the exporting-module and declaration-origin component of every recorded binding.
//! WHY: the compiler design overview requires public-interface facts to be built once from
//!      retained header facts at the semantic boundary, never by reparsing source or scanning
//!      HIR/AST/backend output. Keeping this construction in one narrow compiler-semantic module
//!      keeps stage ownership clear: the headers own declaration-shell discovery, this module
//!      owns stable export-origin projection, and later phases own the completed interface.
//!
//! ## Two-phase split
//!
//! Free export bindings and the public nominal-type origin index are projected from bound, sorted
//! header shells before AST construction ([`build_defined_public_export_origin_draft`]). Receiver
//! surface origins are finalized from the resolved AST [`ReceiverMethodCatalog`] after AST receiver
//! validation succeeds ([`DefinedPublicExportOriginDraft::finalize`]). The split exists because the
//! header-stage receiver-name map is best-effort and absent for valid generic receiver methods and
//! invalid receiver types; the resolved catalog carries the real [`ReceiverKey`] (including generic
//! base resolution), so receiver surfaces are complete for valid generics and never preempt AST
//! receiver diagnostics with an infrastructure error.
//!
//! ## Scope
//!
//! Only declarations defined directly in the active module root's public surface are recorded:
//! free functions, nominal structs, choices, transparent aliases, constants and traits. Receiver
//! methods are attached to their exported receiver type's surface rather than becoming free
//! namespace bindings. Source re-exports are absent by construction: the current entry-closure
//! compilation lacks completed provider interfaces, so donor-local source paths are not stable
//! origins. The future completed provider interface owns re-export stable origin and binding.
//!
//! The declaration-kind set recorded here matches the directly-defined public export surface
//! owned by `headers::public_exports` and `ast::module_ast::environment::public_surface`. Those
//! owners keep their own stage-local predicates because their file-role semantics differ
//! (export-capable roots versus the active module root alone); this module's projection is
//! narrower because imported-module-root headers belong to another module's component.

use crate::compiler_frontend::ast::ReceiverMethodCatalog;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::headers::module_symbols::ModuleSymbols;
use crate::compiler_frontend::headers::parse_file_headers::{FileRole, Header, HeaderKind};
use crate::compiler_frontend::semantic_identity::{
    DefinedPublicExportOrigins, ExportBinding, OriginConstantId, OriginDeclarationId,
    OriginFunctionId, OriginTraitId, OriginTypeCategory, OriginTypeId, ReceiverSurfaceOrigins,
    StableModuleOriginIdentity,
};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::FxHashMap;

/// Pre-AST draft of the directly-defined public export identity component.
///
/// WHAT: carries the free-namespace export bindings and the public nominal-type origin index
///       projected from bound, sorted header shells, plus the module origin needed to finalize
///       receiver surface origins after AST receiver validation succeeds. The module origin is
///       the table-resolved active root origin, not a loose argument.
/// WHY: free bindings and nominal-type origins depend only on header shells and are safe to
///      project before AST construction. Receiver surface origins need the resolved
///      [`ReceiverMethodCatalog`] so they are finalized separately after the AST succeeds; the
///      draft keeps the pre-AST projection and the module origin in one named place so the caller
///      does not manage loose values across the AST boundary.
pub(crate) struct DefinedPublicExportOriginDraft {
    module_origin: StableModuleOriginIdentity,
    export_bindings: Vec<ExportBinding>,
    public_nominal_type_origins: FxHashMap<InternedPath, OriginTypeId>,
}

impl DefinedPublicExportOriginDraft {
    /// Construct the pre-AST draft from the already-built, deterministically ordered bindings,
    /// the public nominal-type origin index and the validated active module origin.
    ///
    /// Compiler-internal: the projection owner assembles the inputs in the documented
    /// deterministic order before calling this. Focused tests build the draft directly to feed
    /// the public-interface draft builder.
    pub(crate) fn new(
        module_origin: StableModuleOriginIdentity,
        export_bindings: Vec<ExportBinding>,
        public_nominal_type_origins: FxHashMap<InternedPath, OriginTypeId>,
    ) -> Self {
        Self {
            module_origin,
            export_bindings,
            public_nominal_type_origins,
        }
    }

    /// Finalize the complete [`DefinedPublicExportOrigins`] by projecting receiver surface origins
    /// from the resolved AST receiver catalog.
    ///
    /// WHAT: consumes [`ReceiverMethodCatalog`] entries built from resolved function signatures and
    ///       attaches receiver methods to their receiver's stable type origin. Generic receiver
    ///       methods resolve to the generic nominal base [`ReceiverKey`] and attach to that stable
    ///       receiver origin. Only receiver types that are directly-defined public nominal types in
    ///       the active module root admit a surface; private and imported receiver types remain
    ///       absent.
    /// WHY: after successful AST construction every active-root receiver path selected by the draft
    ///      has a resolved catalog entry, so a missing entry is a proven internal failure. The
    ///      resolved catalog replaces the best-effort header receiver-name map, which was absent for
    ///      valid generic receiver methods and invalid receiver types.
    pub(crate) fn finalize(
        self,
        receiver_catalog: &ReceiverMethodCatalog,
        string_table: &StringTable,
    ) -> Result<DefinedPublicExportOrigins, CompilerError> {
        let receiver_surfaces = collect_receiver_surface_origins(
            &self.module_origin,
            &self.public_nominal_type_origins,
            receiver_catalog,
            string_table,
        )?;

        Ok(DefinedPublicExportOrigins::new(
            self.module_origin,
            self.export_bindings,
            receiver_surfaces,
        ))
    }
}

/// Build the pre-AST draft of the directly-defined public export identity component.
///
/// WHAT: projects the sorted declaration shells and header-built public export metadata into free
///       export bindings and the public nominal-type origin index. The active root's owning
///       stable module origin is resolved from the per-file `SourceModuleOriginTable` using the
///       retained active root `FileId`, not from a loose module-origin argument. It reads no
///       source text, tokens, HIR, AST or backend output. The caller retains the completed
///       component only on overall semantic success; a diagnosed module exposes no component.
/// WHY: the semantic compilation boundary already holds the bound, sorted declaration shells and
///      the public export metadata, so stable export origins are projected here once rather than
///      reconstructed by a later stage. Resolving the origin from the table validates that every
///      directly-defined public declaration belongs to one unique active module origin, instead
///      of trusting a single loose argument.
pub(crate) fn build_defined_public_export_origin_draft(
    source_module_origins: &SourceModuleOriginTable,
    active_root_file_id: FileId,
    sorted_headers: &[Header],
    module_symbols: &ModuleSymbols,
    string_table: &StringTable,
) -> Result<DefinedPublicExportOriginDraft, CompilerError> {
    let active_origin =
        resolve_active_module_origin(source_module_origins, active_root_file_id, sorted_headers)?;

    let public_nominal_type_origins =
        index_public_nominal_type_origins(&active_origin, sorted_headers, string_table)?;

    let export_bindings =
        collect_free_export_bindings(&active_origin, sorted_headers, module_symbols, string_table)?;

    Ok(DefinedPublicExportOriginDraft::new(
        active_origin,
        export_bindings,
        public_nominal_type_origins,
    ))
}

/// Resolve the one active module origin from the per-file source-origin table.
///
/// WHAT: resolves the active root's owning origin by looking up `active_root_file_id` in the
///       `SourceModuleOriginTable`. The active root must have an owning project-module origin;
///       an unowned active root is an internal failure, not a fallback to a loose argument. This
///       validation runs even when the module has zero directly-defined public exports, so the
///       empty component still carries a validated active origin. Every directly-defined public
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
    sorted_headers: &[Header],
    string_table: &StringTable,
) -> Result<FxHashMap<InternedPath, OriginTypeId>, CompilerError> {
    let mut nominal_type_origins = FxHashMap::default();

    for header in sorted_headers {
        if !is_directly_defined_public_export(header) {
            continue;
        }

        // A directly-defined public declaration always has a defining name: the header parser
        // records one for every authored declaration shell. A missing name here is an impossible
        // metadata gap, not an intentional exclusion, so it must surface as an internal failure
        // rather than silently omitting a public nominal type from the component.
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

        nominal_type_origins.insert(
            header.tokens.src_path.clone(),
            OriginTypeId::new(module_origin.clone(), name.to_owned(), category),
        );
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
/// WHY: the directly-defined active-root index kept on the draft for receiver-surface
///      finalization excludes imported and alias-target nominals by design, because imported and
///      alias-target receiver surfaces belong to their defining module and must not enter this
///      module's [`DefinedPublicExportOrigins`]. Canonical type projection still has to resolve
///      those nominal references, so this expanded index is built once from the already-sorted
///      retained headers, the header-built public export maps and the per-file origin table
///      without re-scanning source. It never invents a second visibility rule from `FileRole` and
///      `export_mode` alone: the public export targeting fact already retained by
///      `headers::public_exports` is the single authority. It is transient: it exists only to
///      feed the projection and is not retained on the export component.
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
        // its nominal fails through the total nominal resolver with a precise CompilerError
        // rather than a path/display identity fallback.
        let Some(module_origin) = source_module_origins.origin_for(file_id)? else {
            continue;
        };

        let origin = OriginTypeId::new(module_origin.clone(), name.to_owned(), category);

        // Canonical declaration paths are unique, so a duplicate path is a real conflict. Report
        // both origins (which embed category) so a category inconsistency or conflicting provider
        // origin is explicit, and never silently overwrite the first entry.
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

/// Build the transient expanded public source-trait origin index.
///
/// WHAT: indexes `OriginTraitId` by canonical declaration path for every trait header whose
///       canonical declaration path is targeted by any retained module-root or source-package
///       public export entry. Analogous to [`build_public_source_nominal_origin_index`] but for
///       trait declarations. Directly-defined public traits, imported project-graph traits and
///       private normal-file traits exposed through a public alias or re-export are admitted.
///       Unexported private traits and explicit-`None` unowned source-package traits are absent.
/// WHY: generic-bound projection resolves each source-bound `TraitId` through its source
///      canonical path to a stable `OriginTraitId`, so a bound that references an imported or
///      alias-target project-graph trait resolves to that trait's defining provider module
///      origin rather than the active module origin. A source-package header whose `FileId`
///      table entry is `None` (no project-module owner) is deliberately absent from the index:
///      its trait is not project-graph-owned and must not receive a fabricated origin, and a
///      projected public bound that requires one fails through the total bound resolver with a
///      precise `CompilerError`.
///
/// Reuses the shared [`PublicExportTarget::is_source_path`] authority via
/// [`any_retained_public_export_targets_source_path`] so trait origin indexing and nominal
/// origin indexing cannot drift on what a public export targets. It never uses display/path
/// identity fallback.
///
/// Rejects a missing `FileId`, an out-of-range table lookup, a duplicate canonical trait path
/// or a conflicting origin explicitly. It never silently overwrites an existing entry.
/// `DefinedPublicExportOrigins` free binding and receiver behavior is unchanged; this index is
/// transient only for bound projection.
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

/// Collect the free-namespace export bindings for directly-defined public declarations.
///
/// Receiver methods are excluded here: they are attached to their receiver surface by
/// `collect_receiver_surface_origins` and must not become independent free namespace bindings.
fn collect_free_export_bindings(
    module_origin: &StableModuleOriginIdentity,
    sorted_headers: &[Header],
    module_symbols: &ModuleSymbols,
    string_table: &StringTable,
) -> Result<Vec<ExportBinding>, CompilerError> {
    let mut export_bindings = Vec::new();

    for header in sorted_headers {
        if !is_directly_defined_public_export(header) {
            continue;
        }

        // A directly-defined public authored declaration always has a defining name. A missing
        // name is an impossible metadata gap that must not silently omit a public export from the
        // component.
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

        export_bindings.push(ExportBinding::new(
            module_origin.clone(),
            name.to_owned(),
            origin,
        ));
    }

    // Deterministic order independent of hash-map iteration and declaration scheduling: sort by
    // public name, then by declaration category so two bindings can never tie ambiguously.
    export_bindings.sort_by(|left, right| {
        left.public_name().cmp(right.public_name()).then_with(|| {
            declaration_category_rank(left.origin()).cmp(&declaration_category_rank(right.origin()))
        })
    });

    Ok(export_bindings)
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

/// Collect receiver-method origins attached to exported nominal type surfaces from the resolved
/// AST receiver catalog.
///
/// A receiver method is part of the public surface only when its receiver type is a
/// directly-defined public nominal type in the active module root. The method's stable function
/// origin is built with [`OriginFunctionId::new_receiver`] using the receiver's stable
/// [`OriginTypeId`], so it stays attached to that receiver surface and never becomes an
/// independent free namespace binding.
///
/// The catalog is built from resolved function signatures, so generic receiver methods resolve to
/// the generic nominal base [`ReceiverKey`] and attach to that stable receiver origin. External and
/// builtin receiver keys are impossible in a successfully validated catalog; encountering them is
/// an internal failure that must not coerce to a nominal surface.
fn collect_receiver_surface_origins(
    module_origin: &StableModuleOriginIdentity,
    public_nominal_type_origins: &FxHashMap<InternedPath, OriginTypeId>,
    receiver_catalog: &ReceiverMethodCatalog,
    string_table: &StringTable,
) -> Result<Vec<ReceiverSurfaceOrigins>, CompilerError> {
    let mut methods_by_receiver: FxHashMap<OriginTypeId, Vec<OriginFunctionId>> =
        FxHashMap::default();

    for entry in receiver_catalog.by_function_path.values() {
        let receiver_type = match &entry.receiver {
            ReceiverKey::Struct(path) | ReceiverKey::Choice(path) => {
                // A receiver path not in the public nominal-type index is a private or imported
                // receiver type, so the method is not part of this module's public surface. Private
                // receiver surfaces are deliberately absent and imported-root receiver methods
                // belong to another module's component.
                match public_nominal_type_origins.get(path) {
                    Some(origin) => origin,
                    None => continue,
                }
            }

            ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_) => {
                // A successfully validated source receiver catalog contains only Struct and Choice
                // receiver keys: the AST receiver-method diagnostic owner rejects External and
                // BuiltinScalar receivers before catalog construction completes. Encountering
                // either here is a proven internal failure that must not coerce to a nominal
                // surface.
                return Err(CompilerError::compiler_error(format!(
                    "defined public export-origin construction: a resolved receiver catalog entry carries a non-nominal receiver key ({:?})",
                    entry.receiver
                )));
            }
        };

        // A receiver method path always carries a defining method name. A missing name is an
        // impossible metadata gap that must not silently omit a public receiver method.
        let Some(method_name) = entry.function_path.name_str(string_table) else {
            return Err(CompilerError::compiler_error(format!(
                "defined public export-origin construction: a receiver method path has no resolvable defining name (path: {:?})",
                entry.function_path
            )));
        };

        let method = OriginFunctionId::new_receiver(
            module_origin.clone(),
            method_name.to_owned(),
            receiver_type.clone(),
        );

        methods_by_receiver
            .entry(receiver_type.clone())
            .or_default()
            .push(method);
    }

    // Deterministic order: sort each surface's methods by defining name, then sort surfaces by
    // receiver defining name and category. Externally observed iteration is therefore independent
    // of hash-map iteration and declaration scheduling.
    let mut receiver_surfaces: Vec<ReceiverSurfaceOrigins> = methods_by_receiver
        .into_iter()
        .map(|(receiver, mut methods)| {
            methods.sort_by(|left, right| left.defining_name().cmp(right.defining_name()));
            ReceiverSurfaceOrigins::new(receiver, methods)
        })
        .collect();

    receiver_surfaces.sort_by(|left, right| {
        left.receiver()
            .defining_name()
            .cmp(right.receiver().defining_name())
            .then_with(|| left.receiver().category().cmp(&right.receiver().category()))
    });

    Ok(receiver_surfaces)
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
    header.file_role == FileRole::ActiveModuleRoot
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

#[cfg(test)]
#[path = "tests/defined_public_export_origins_tests.rs"]
mod tests;
