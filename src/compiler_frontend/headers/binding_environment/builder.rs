//! Binding environment builder implementation.
//!
//! WHAT: constructs per-file visibility maps by registering same-file declarations,
//!        prelude/builtin names, and resolved dependencies.
//! WHY: the builder holds mutable state across all files and performs the heavy lifting
//!      of dependency resolution; keeping it separate from the entry-point orchestration
//!      makes the module structure easier to navigate.

use crate::builder_surface::SourceFileKind;
use crate::builder_surface::definition::TEMPLATE_PRELUDE_SOURCE_PACKAGE_PREFIX;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::ImportPublicSurfaceType;
use crate::compiler_frontend::compiler_messages::InvalidReceiverDeclarationReason;
use crate::compiler_frontend::compiler_messages::NameNamespace;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::ExternalSymbolCategory;
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::dependency_clause_syntax::{
    DependencyAlias, RetainedDependencyPath,
};
use crate::compiler_frontend::headers::module_symbols::{ModuleSymbols, PublicExportEntry};
use crate::compiler_frontend::headers::parse_file_headers::RetainedDependencyClause;
use crate::compiler_frontend::headers::types::DependencySelection;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::public_interface::{
    ProviderInterfaceId, PublicDeclarationSemantics, ResolvedDependencyClause,
    SourceProviderDependencySet,
};
use crate::compiler_frontend::semantic_identity::OriginDeclarationId;
use crate::compiler_frontend::source::{SourceDatabase, SourceKind, SourceSpan};
use crate::compiler_frontend::source_packages::root_file::{
    dependency_path_references_config_file, dependency_path_references_legacy_support_marker,
};
use crate::compiler_frontend::symbols::identifier_policy::ensure_not_keyword_shadow_identifier;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use rustc_hash::{FxHashMap, FxHashSet};
use std::fmt::Debug;
use std::sync::Arc;

use super::external_imports::ExternalImportInput;
use super::source_dependencies::SourceDependencyInput;
use super::{
    BindingEnvironmentError, DependencyTargetResolutionInput, ExternalPackageSymbolLookup,
    ExternalPackageSymbolResolutionInput, FileVisibility, HeaderBindingEnvironment,
    ModuleBoundaryCheckInput, NamespaceRecord, NamespaceRecordSource,
    NamespaceTargetResolutionInput, NamespaceTypeMember, NamespaceValueMember,
    PublicExportLookupResult, PublicExportResolutionInput, ReceiverMethodVisibility,
    ResolvedDependencyTarget, ResolvedNamespaceTarget, ResolvedQualifiedReference,
    ResolvedQualifiedTarget, SourceDeclarationTarget, SourceDependencyAccess, SourceFunctionTarget,
    SourcePackageBoundaryCheckInput, VisibleNameBinding, VisibleNameRegistry,
    check_alias_case_warning, check_module_boundary, check_source_package_boundary,
    has_explicit_moth_extension, resolve_dependency_target, resolve_external_package_symbol,
    resolve_namespace_target, resolve_public_export_boundary,
};

/// Result for the binding-environment builder family.
///
/// WHAT: carries ordinary user-facing dependency diagnostics separately from internal
///       successful-interface invariant failures.
/// WHY: dependency resolution passes structured diagnostics through several recursive helpers, and
///      provider agreement failures must reach the `InfrastructureFailure` lane without first degrading
///      into a source diagnostic.
type BuilderResult<T> = Result<T, BindingEnvironmentError>;

/// One direct-selection target after resolution but before any consumer-local publication.
///
/// WHAT: distinguishes a provider export, a source declaration with its visibility requirement,
///       and an external package symbol.
/// WHY: an ordinary dependency clause registers every target as a visible local name, while a
///      qualified occurrence records the same target without publishing a name. Sharing the
///      resolved target keeps both callers on one visibility, boundary and provider check.
#[derive(Clone)]
enum ResolvedSelectedTarget {
    ProviderDeclaration {
        provider_id: ProviderInterfaceId,
        public_name: StringId,
        origin: OriginDeclarationId,
        local_path: PathId,
    },
    Source {
        symbol_path: PathId,
        access: SourceDependencyAccess,
    },
    External {
        symbol_id: ExternalSymbolId,
    },
}

/// Inputs for one provider declaration binding.
///
/// WHAT: keeps the provider-owned name, consumer-local identity and authored provenance together
/// while the builder records one dependency-bound declaration surface.
/// WHY: the direct provider path and public provider-selection path share the same registration
/// operation; a named input prevents either caller from growing a positional argument list.
struct ProviderDeclarationBindingInput<'a> {
    public_name: StringId,
    origin: &'a OriginDeclarationId,
    local_name: StringId,
    local_alias: Option<&'a DependencyAlias>,
    local_path: &'a PathId,
    source_span: Option<SourceSpan>,
    provider_id: ProviderInterfaceId,
}

/// Insert one provider fact only when every publisher of the same key agrees.
///
/// WHAT: declaration origins, evidence identities and concrete summary origins are stable
///       semantic keys. A malformed successful artefact may publish one key from more than one
///       provider; first-provider-wins would make the accepted facts depend on dependency order.
/// WHY: this is an internal successful-interface invariant, so disagreement is a
///      `InfrastructureFailure`, never a user-facing source diagnostic.
pub(super) fn insert_agreed<K, V>(
    table: &mut FxHashMap<K, V>,
    key: K,
    value: &V,
    fact_class: &str,
) -> Result<(), InfrastructureFailure>
where
    K: std::hash::Hash + Eq + Debug,
    V: Eq + Debug + Clone,
{
    match table.entry(key) {
        std::collections::hash_map::Entry::Occupied(existing) => {
            if existing.get() != value {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HeaderParsing,
                    format!(
                        "provider {fact_class} {:?} disagrees across dependency providers",
                        existing.key()
                    ),
                ));
            }
        }
        std::collections::hash_map::Entry::Vacant(slot) => {
            // Clone the candidate only when the key is vacant; occupied agreement just borrows.
            slot.insert(value.clone());
        }
    }
    Ok(())
}

pub(crate) struct BindingEnvironmentBuilder<'a> {
    pub(super) module_symbols: &'a ModuleSymbols,
    pub(super) external_package_registry: &'a ExternalPackageRegistry,
    pub(super) external_dependency_resolution_table: &'a ExternalImportResolutionTable,
    pub(super) source_provider_dependencies: &'a SourceProviderDependencySet<'a>,
    pub(super) source_files: &'a SourceDatabase,
    pub(super) string_table: &'a mut StringTable,
    pub(super) path_fork: &'a mut PathInternerFork,
    pub(super) environment: HeaderBindingEnvironment,
    pub(super) warnings: crate::compiler_frontend::compiler_messages::DiagnosticBag,
    /// Provider IDs whose closed semantics have already been imported into the module-wide
    /// header environment.
    ///
    /// WHAT: the first shell referencing a provider registers its declarations, evidence and
    ///       summaries once; later direct-selection, namespace, receiver and implicit-template dependencies
    ///       reuse those tables.
    pub(super) provider_semantics_registered: FxHashSet<ProviderInterfaceId>,
}

impl<'a> BindingEnvironmentBuilder<'a> {
    // ------------------------------
    //  Dependency-binding helpers
    // ------------------------------

    /// Emit an alias-case warning when an explicit alias changes leading case.
    pub(super) fn emit_alias_case_warning_if_needed(
        &mut self,
        alias: Option<&DependencyAlias>,
        symbol_name: StringId,
    ) {
        let Some(alias) = alias else {
            return;
        };
        if let Some(warning) = check_alias_case_warning(alias, self.string_table, symbol_name) {
            self.warnings.push(warning);
        }
    }

    /// Whether two source files share the same non-public dependency boundary.
    ///
    /// WHAT: source-backed package members, same module-root members, and files in the implicit entry
    /// module can see each other's ordinary source declarations directly.
    /// WHY: direct-selection source dependencies and namespace dependencies both need the same boundary answer
    /// before deciding whether receiver methods may travel with the dependency-bound surface.
    pub(super) fn source_files_share_dependency_boundary(
        &self,
        consumer_file: &PathId,
        target_file: &PathId,
    ) -> bool {
        // A declaration authored directly in an active root file is internal only to that file.
        // Neither a sibling root nor an ordinary helper shares its dependency boundary, so this
        // answer gates direct selections, receiver-method travel and namespace records before AST.
        if !self
            .module_symbols
            .authored_declarations_visible_from(consumer_file, target_file)
        {
            return false;
        }

        let consumer_package = self
            .module_symbols
            .file_package_membership
            .get(consumer_file);
        let target_package = self.module_symbols.file_package_membership.get(target_file);
        if consumer_package == target_package && consumer_package.is_some() {
            return true;
        }

        let consumer_module = self
            .module_symbols
            .file_module_membership
            .get(consumer_file);
        let target_module = self.module_symbols.file_module_membership.get(target_file);
        if consumer_module == target_module && consumer_module.is_some() {
            return true;
        }

        let consumer_has_explicit_module = consumer_package.is_some() || consumer_module.is_some();
        let target_has_explicit_module = target_package.is_some() || target_module.is_some();

        !consumer_has_explicit_module && !target_has_explicit_module
    }

    /// Validate a Stage 0-resolved same-module edge before binding uses the retained path.
    ///
    /// Stage 0 stores the final compiler `SourceId` on the retained dependency so semantic
    /// binding can verify that the edge still belongs to this module's prepared source slots.
    /// Dependency clauses and expression-qualified occurrences share this check. Binding
    /// continues to resolve the authored symbol path for visibility, but it must fail closed if
    /// a stale or provider-owned identity is attached to a local source edge.
    fn validate_local_source_edge(
        &mut self,
        dependency: &RetainedDependencyPath,
    ) -> BuilderResult<()> {
        let Some(local_source_id) = dependency.local_source_id else {
            return Ok(());
        };

        if matches!(
            dependency.target,
            crate::compiler_frontend::headers::dependency_target::DependencyTargetKind::ExternalProvider {
                ..
            }
        ) {
            return Err(InfrastructureFailure::internal(CompilerStage::HeaderParsing,
                "external provider dependency retained a local source identity",
            )
            .into());
        }

        let Some(source_record) = self.source_files.get(local_source_id) else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!(
                    "retained local dependency references missing source identity {}",
                    local_source_id.index()
                ),
            )
            .into());
        };
        if !matches!(source_record.kind, Some(SourceKind::Compiler(_))) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!(
                    "retained local dependency references non-compiler source identity {}",
                    local_source_id.index()
                ),
            )
            .into());
        }
        if !self
            .module_symbols
            .source_ids_by_source
            .values()
            .any(|source_id| *source_id == local_source_id)
        {
            return Err(InfrastructureFailure::internal(CompilerStage::HeaderParsing, format!(
                "retained local dependency identity {} is outside the prepared module source slots",
                local_source_id.index()
            ))
            .into());
        }

        let Some(namespace_target) = resolve_namespace_target(NamespaceTargetResolutionInput {
            dependency_path: &dependency.path,
            module_file_paths: &self.module_symbols.module_file_paths,
            external_package_registry: self.external_package_registry,
            string_table: self.string_table,
            path_fork: &*self.path_fork,
        }) else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "retained local dependency path does not resolve to a prepared source file",
            )
            .into());
        };
        let ResolvedNamespaceTarget::SourceFile(target_path) = namespace_target else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "retained local dependency path resolves to a non-source namespace target",
            )
            .into());
        };
        let Some(target_source_id) = self.module_symbols.source_ids_by_source.get(&target_path)
        else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "retained local dependency target path has no prepared source identity",
            )
            .into());
        };
        if *target_source_id != local_source_id {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "retained local dependency identity disagrees with its resolved source path",
            )
            .into());
        }

        Ok(())
    }

    pub(super) fn build_file_visibility(
        &mut self,
        source_file: &PathId,
        selection_table: &[DependencySelection],
    ) -> BuilderResult<()> {
        let mut file_visibility = FileVisibility::default();
        let mut registry = VisibleNameRegistry::new();

        // Runtime start-body bindings are not declarations, so they never pass through the
        // visible-name registry. Carry their authored spans for constant-context diagnostics.
        if let Some(binding_spans) = self
            .module_symbols
            .start_body_binding_spans_by_file
            .get(source_file)
        {
            file_visibility.start_body_binding_spans = binding_spans.clone();
        }

        // Reserve compiler-owned core cast trait names before any source
        // declarations or dependencies can claim them. This lets the normal visible-
        // name collision path reject aliases, namespace names, and dependency-bound
        // source/export names that would shadow a core cast trait spelling.
        registry.reserve_core_cast_trait_names(self.string_table);
        registry.reserve_parameter_contract_names(self.string_table);

        // 1. Register same-file declarations.
        if let Some(declared_paths) = self.module_symbols.declared_paths_by_file.get(source_file) {
            for path in declared_paths {
                file_visibility
                    .visible_declaration_paths_mut()
                    .insert(*path);

                let Some(name) = self.path_fork.component(*path) else {
                    continue;
                };

                if self.module_symbols.receiver_method_paths.contains(path) {
                    // Source receiver methods are receiver-call-only declarations. They do not
                    // reserve ordinary value/dependency names, because dispatch includes the receiver
                    // type and `method(value)` is diagnosed from the receiver catalog instead.
                    Self::add_visible_receiver_method(
                        &mut file_visibility,
                        name,
                        path,
                        self.module_symbols
                            .declaration_spans_by_symbol_path
                            .get(path)
                            .copied(),
                    );
                    continue;
                }

                let is_type_alias = self.module_symbols.type_alias_paths.contains(path);
                let is_trait = self.module_symbols.trait_paths.contains(path);
                let binding = if is_type_alias {
                    VisibleNameBinding::TypeAlias {
                        canonical_path: *path,
                    }
                } else if is_trait {
                    VisibleNameBinding::Trait {
                        canonical_path: *path,
                    }
                } else {
                    VisibleNameBinding::SameFileDeclaration {
                        declaration_path: *path,
                    }
                };

                let declaration_span = self
                    .module_symbols
                    .declaration_spans_by_symbol_path
                    .get(path)
                    .copied();
                registry.register(name, binding, declaration_span)?;

                if is_type_alias {
                    file_visibility
                        .visible_type_alias_names
                        .insert(name, SourceDeclarationTarget::Local(*path));
                } else if is_trait {
                    file_visibility
                        .visible_trait_names
                        .insert(name, SourceDeclarationTarget::Local(*path));
                } else {
                    file_visibility
                        .visible_source_names
                        .insert(name, SourceDeclarationTarget::Local(*path));
                }
            }
        }

        // 2. Register builtins.
        for path in &self.module_symbols.builtin_visible_symbol_paths {
            file_visibility
                .visible_declaration_paths_mut()
                .insert(*path);
            if let Some(name) = self.path_fork.component(*path) {
                registry.register(name, VisibleNameBinding::Builtin, None)?;
                file_visibility
                    .visible_source_names
                    .insert(name, SourceDeclarationTarget::Local(*path));
            }
        }

        // 3. Register prelude symbols in the registry so dependencies can detect collisions.
        // Mutation: prelude names are compiler-owned fixed symbols interned for name comparison.
        for (prelude_name, symbol_id) in self.external_package_registry.prelude_symbols_by_name() {
            let prelude_name_id = self.string_table.intern(prelude_name);
            registry.register(
                prelude_name_id,
                VisibleNameBinding::Prelude {
                    symbol_id: *symbol_id,
                },
                None,
            )?;
        }

        // 4. Register prelude namespace aliases so they participate in collision detection
        // before explicit dependencies. The alias name points at an external package path, and the
        // resulting visible namespace record is built from the same path as an explicit
        // `@package`.
        for (prelude_name, package_path) in self
            .external_package_registry
            .prelude_namespace_aliases_by_name()
        {
            let prelude_name_id = self.string_table.intern(prelude_name);
            let package_path_id = self.string_table.intern(package_path);
            registry.register(
                prelude_name_id,
                VisibleNameBinding::NamespaceRecord {
                    record_source: NamespaceRecordSource::ExternalPackage(package_path_id),
                },
                None,
            )?;
        }

        // 5. Resolve and register explicit dependencies.
        if let Some(dependencies) = self
            .module_symbols
            .file_dependency_clauses_by_source
            .get(source_file)
        {
            for dependency in dependencies {
                self.validate_local_source_edge(&dependency.dependency)?;
                let selections = dependency
                    .selections(selection_table)
                    .map_err(BindingEnvironmentError::Internal)?;
                // Reject direct dependencies of retired support markers and canonical config
                // files. Canonical `@` root references are already caught by the path parser's
                // LeadingAtInPathComponent rejection. The retired support spelling and config
                // files need this later check because `+` and `config` are valid path component
                // characters.
                if dependency_path_references_legacy_support_marker(
                    dependency.dependency.path,
                    &*self.path_fork,
                    self.string_table,
                ) || dependency_path_references_config_file(
                    dependency.dependency.path,
                    &*self.path_fork,
                    self.string_table,
                ) {
                    return Err(super::diagnostics::direct_special_file_dependency(
                        &dependency.dependency.path,
                        Some(dependency.dependency.span),
                    )
                    .into());
                }

                if !selections.is_empty() {
                    // Direct selections use one provider shell, then resolve each selected name
                    // against that provider or source public surface.
                    self.resolve_and_register_selection_bindings(
                        &mut file_visibility,
                        &mut registry,
                        dependency,
                        selections,
                        source_file,
                    )?;
                    add_frontend_counter(FrontendCounter::BoundSelectedNameCount, selections.len());
                } else {
                    // An empty selection list is a namespace dependency.
                    self.resolve_and_register_namespace_binding(
                        &mut file_visibility,
                        &mut registry,
                        dependency,
                        source_file,
                    )?;
                    add_frontend_counter(FrontendCounter::BoundNamespaceClauseCount, 1);
                }
            }
        }

        // 5b. Resolve expression-qualified occurrences through the same direct-selection target
        // resolution. The resolved target joins the per-occurrence lookup without publishing a
        // visible local name, so occurrences never collide with declarations or dependency names.
        self.bind_qualified_reference_occurrences(&mut file_visibility, source_file)?;

        // 6. Inject unshadowed prelude symbols into visible maps.
        // Prelude entries that are still registered as Prelude were not shadowed by dependencies
        // or declarations with different targets.
        for (prelude_name, symbol_id) in self.external_package_registry.prelude_symbols_by_name() {
            let prelude_name_id = self.string_table.intern(prelude_name);
            if let Some(VisibleNameBinding::Prelude {
                symbol_id: registered_id,
            }) = registry.get(prelude_name_id)
                && registered_id == symbol_id
            {
                file_visibility
                    .visible_external_symbols
                    .insert(prelude_name_id, *symbol_id);
            }
        }

        // 7. Inject unshadowed prelude namespace aliases into visible namespace records.
        // Aliases that are still registered as a namespace record with the same external
        // package target were not shadowed by same-file declarations, builtins, or dependencies
        // of a different target. Explicit dependencies of the same package already insert an
        // equivalent record, so we skip when the local name is already present.
        for (prelude_name, package_path) in self
            .external_package_registry
            .prelude_namespace_aliases_by_name()
        {
            let prelude_name_id = self.string_table.intern(prelude_name);
            let package_path_id = self.string_table.intern(package_path);
            if let Some(VisibleNameBinding::NamespaceRecord {
                record_source: NamespaceRecordSource::ExternalPackage(registered_package_path_id),
            }) = registry.get(prelude_name_id)
                && registered_package_path_id == &package_path_id
                && !file_visibility
                    .visible_namespace_records
                    .contains_key(&prelude_name_id)
            {
                let mut record = self.build_external_namespace_record(package_path_id, None)?;
                record.authored_clause_path = Some(package_path_id);
                file_visibility
                    .visible_namespace_records
                    .insert(prelude_name_id, record);
            }
        }

        // 8. Add Moth template's compiler-standard constant prelude.
        // WHY: `.mtf` bodies are synthetic constant initializers, so the official `@html`
        //      helper constants must join the same file-local visibility maps as authored
        //      constants without a user-visible dependency record.
        self.register_moth_template_constant_prelude(
            &mut file_visibility,
            &mut registry,
            source_file,
        )?;

        // Sort receiver method paths for deterministic lookup ordering.
        // WHY: same method name from different sources must resolve consistently
        //      across compilations; lexicographic order by function path is stable.
        for paths in file_visibility.visible_receiver_methods.values_mut() {
            paths.sort_by(|a, b| {
                let a_str = self.path_fork.render_portable(
                    *a.target.local_path(),
                    self.string_table,
                    &mut Vec::new(),
                );
                let b_str = self.path_fork.render_portable(
                    *b.target.local_path(),
                    self.string_table,
                    &mut Vec::new(),
                );
                a_str.cmp(&b_str)
            });
        }

        self.environment
            .file_visibility_by_source
            .insert(*source_file, Arc::new(file_visibility));
        Ok(())
    }

    /// Register one provider's closed semantics in the module-wide environment exactly once.
    ///
    /// WHAT: the first shell referencing a provider ID stores its declaration records, evidence
    ///       records and concrete call summaries in the shared tables; later shells resolve the
    ///       same records by origin and identity without cloning the provider payloads again.
    /// WHY: repeated per-shell full-interface projection made binding quadratic in shells and
    ///      duplicated every provider fact per alias.
    pub(super) fn register_provider_semantics_once(
        &mut self,
        provider_id: ProviderInterfaceId,
    ) -> BuilderResult<()> {
        if !self.provider_semantics_registered.insert(provider_id) {
            return Ok(());
        }

        let interface = self.source_provider_dependencies.interface(provider_id)?;

        for provider_declaration in &interface.declarations {
            insert_agreed(
                &mut self.environment.imported_declarations_by_origin,
                provider_declaration.origin.clone(),
                provider_declaration,
                "declaration origin",
            )?;
        }

        for evidence in &interface.reusable_evidence {
            insert_agreed(
                &mut self.environment.imported_evidence_by_identity,
                evidence.identity.clone(),
                evidence,
                "evidence identity",
            )?;
        }

        for summary in &interface.concrete_call_summaries {
            insert_agreed(
                &mut self.environment.imported_call_summaries_by_origin,
                summary.origin.clone(),
                &summary.summary,
                "concrete summary origin",
            )?;
        }

        Ok(())
    }

    /// Register one declaration exported by a completed source-provider interface.
    ///
    /// WHAT: joins a provider public name to its stable declaration origin, then records the
    ///       consumer-local path and all derived receiver/call facts.
    /// WHY: direct provider dependencies and public-surface provider re-exports must share this one
    ///       provider-owned binding operation; neither path may reinterpret a provider name as a
    ///       source path in the consumer module.
    fn register_provider_declaration_binding(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        input: ProviderDeclarationBindingInput<'_>,
    ) -> BuilderResult<()> {
        let ProviderDeclarationBindingInput {
            public_name,
            origin,
            local_name,
            local_alias,
            local_path,
            source_span,
            provider_id,
        } = input;

        let local_name_span = local_alias.map_or(source_span, |alias| Some(alias.span));
        ensure_not_keyword_shadow_identifier(local_name, local_name_span, self.string_table)?;

        let view = self
            .source_provider_dependencies
            .binding_view(provider_id)?;
        let declaration = view.declaration(origin).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!(
                    "provider interface {:?} has no declaration record for exported origin {:?}",
                    provider_id, origin
                ),
            )
        })?;

        self.emit_alias_case_warning_if_needed(local_alias, public_name);

        let binding = match &declaration.semantics {
            PublicDeclarationSemantics::TransparentAlias(_) => VisibleNameBinding::TypeAlias {
                canonical_path: *local_path,
            },
            PublicDeclarationSemantics::Trait(_) => VisibleNameBinding::Trait {
                canonical_path: *local_path,
            },
            _ => VisibleNameBinding::SourceDependency {
                canonical_path: *local_path,
            },
        };

        registry.register(local_name, binding, local_name_span)?;
        file_visibility
            .visible_declaration_paths_mut()
            .insert(*local_path);

        let target = SourceDeclarationTarget::Imported {
            origin: origin.clone(),
            local_path: *local_path,
        };
        match &declaration.semantics {
            PublicDeclarationSemantics::TransparentAlias(_) => {
                file_visibility
                    .visible_type_alias_names
                    .insert(local_name, target);
            }
            PublicDeclarationSemantics::Trait(_) => {
                file_visibility
                    .visible_trait_names
                    .insert(local_name, target);
            }
            _ => {
                file_visibility
                    .visible_source_names
                    .insert(local_name, target);
            }
        }

        self.environment
            .imported_declarations_by_local_path
            .insert(*local_path, origin.clone());

        let receiver_methods = match &declaration.semantics {
            PublicDeclarationSemantics::Struct(structure) => Some(&structure.receiver_methods),
            PublicDeclarationSemantics::Choice(choice) => Some(&choice.receiver_methods),
            _ => None,
        };
        if let Some(receiver_methods) = receiver_methods {
            self.register_imported_receiver_methods(
                file_visibility,
                local_path,
                receiver_methods,
                provider_id,
                local_name_span,
            )?;
        }

        if let crate::compiler_frontend::semantic_identity::OriginDeclarationId::Function(
            function_origin,
        ) = origin
            && view.concrete_call_summary(function_origin).is_some()
        {
            self.environment.imported_functions_by_local_path.insert(
                *local_path,
                super::ImportedFunctionContract {
                    target: SourceFunctionTarget::Imported {
                        origin: function_origin.clone(),
                        local_path: *local_path,
                    },
                },
            );
        }

        Ok(())
    }

    fn register_source_provider_namespace_binding(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        dependency: &RetainedDependencyClause,
        resolved_clause: ResolvedDependencyClause,
    ) -> BuilderResult<()> {
        debug_assert_eq!(
            resolved_clause.shell, dependency.dependency.dependency_shell_id,
            "resolved provider clause must retain its authored shell identity"
        );
        let provider_id = resolved_clause.provider;
        self.register_provider_semantics_once(provider_id)?;
        let view = self
            .source_provider_dependencies
            .binding_view(provider_id)?;
        let interface = self.source_provider_dependencies.interface(provider_id)?;
        let namespace_span = match &dependency.binding {
            crate::compiler_frontend::headers::types::DependencyBindingSyntax::Namespace {
                alias: Some(alias),
            } => Some(alias.span),
            crate::compiler_frontend::headers::types::DependencyBindingSyntax::Namespace {
                alias: None,
            } => Some(dependency.dependency.span),
            crate::compiler_frontend::headers::types::DependencyBindingSyntax::DirectSelections {
                ..
            } => None,
        };

        let mut record = NamespaceRecord::empty(NamespaceRecordSource::SourceFile(
            dependency.dependency.path,
        ));
        for binding in &interface.export_bindings {
            let declaration = view.declaration(binding.origin()).ok_or_else(|| {
                super::diagnostics::missing_dependency_target(
                    &dependency.dependency.path,
                    Some(dependency.dependency.span),
                )
            })?;
            let name = self.string_table.intern(binding.public_name());
            let local_path = self
                .path_fork
                .try_intern_child(dependency.dependency.path, name)
                .ok_or_else(token_lexical::logical_path_capacity)?;
            let target = SourceDeclarationTarget::Imported {
                origin: binding.origin().clone(),
                local_path,
            };

            match &declaration.semantics {
                PublicDeclarationSemantics::Struct(structure) => {
                    record
                        .type_members
                        .insert(name, NamespaceTypeMember::SourceDeclaration(target.clone()));
                    self.register_imported_receiver_methods(
                        file_visibility,
                        &local_path,
                        &structure.receiver_methods,
                        provider_id,
                        namespace_span,
                    )?;
                }
                PublicDeclarationSemantics::Choice(choice) => {
                    record
                        .type_members
                        .insert(name, NamespaceTypeMember::SourceDeclaration(target.clone()));
                    self.register_imported_receiver_methods(
                        file_visibility,
                        &local_path,
                        &choice.receiver_methods,
                        provider_id,
                        namespace_span,
                    )?;
                }
                PublicDeclarationSemantics::TransparentAlias(_) => {
                    record
                        .type_members
                        .insert(name, NamespaceTypeMember::SourceDeclaration(target.clone()));
                }
                PublicDeclarationSemantics::Function(_) => {
                    record.value_members.insert(
                        name,
                        NamespaceValueMember::SourceDeclaration(target.clone()),
                    );
                    if let crate::compiler_frontend::semantic_identity::OriginDeclarationId::Function(
                        function_origin,
                    ) = binding.origin()
                        && view.concrete_call_summary(function_origin).is_some()
                    {
                        self.environment.imported_functions_by_local_path.insert(
                            local_path,
                            super::ImportedFunctionContract {
                                target: SourceFunctionTarget::Imported {
                                    origin: function_origin.clone(),
                                    local_path,
                                },
                            },
                        );
                    }
                }
                PublicDeclarationSemantics::Constant(_) => {
                    record.value_members.insert(
                        name,
                        NamespaceValueMember::SourceDeclaration(target.clone()),
                    );
                }
                PublicDeclarationSemantics::Trait(_) => continue,
            }

            self.environment
                .imported_declarations_by_local_path
                .insert(local_path, binding.origin().clone());
        }

        for binding in &interface.binding_exports {
            let Some(symbol_id) = self
                .external_package_registry
                .resolve_canonical_symbol(&binding.target)
            else {
                return Err(super::diagnostics::missing_dependency_target(
                    &dependency.dependency.path,
                    Some(dependency.dependency.span),
                )
                .into());
            };
            let name = self.string_table.intern(&binding.public_name);
            match binding.target.category {
                ExternalSymbolCategory::Function | ExternalSymbolCategory::Constant => {
                    record
                        .value_members
                        .insert(name, NamespaceValueMember::ExternalSymbol(symbol_id));
                }
                ExternalSymbolCategory::Type => {
                    record
                        .type_members
                        .insert(name, NamespaceTypeMember::ExternalSymbol(symbol_id));
                }
            }
        }
        let local_name = self.derive_namespace_name(dependency)?;

        registry.register(
            local_name,
            VisibleNameBinding::NamespaceRecord {
                record_source: record.record_source.clone(),
            },
            namespace_span,
        )?;
        file_visibility
            .visible_namespace_records
            .insert(local_name, record);
        Ok(())
    }

    pub(super) fn register_imported_receiver_methods(
        &mut self,
        file_visibility: &mut FileVisibility,
        imported_type_path: &PathId,
        methods: &[crate::compiler_frontend::public_interface::PublicReceiverMethodSemantics],
        provider_id: ProviderInterfaceId,
        dependency_span: Option<SourceSpan>,
    ) -> BuilderResult<()> {
        self.register_provider_semantics_once(provider_id)?;

        for method in methods {
            let method_name = self
                .string_table
                .intern(method.method_origin.defining_name());
            let method_path = self
                .path_fork
                .try_intern_child(*imported_type_path, method_name)
                .ok_or_else(token_lexical::logical_path_capacity)?;
            let target = SourceFunctionTarget::Imported {
                origin: method.method_origin.clone(),
                local_path: method_path,
            };

            let methods = file_visibility
                .visible_receiver_methods
                .entry(method_name)
                .or_default();
            if !methods
                .iter()
                .any(|method| method.target.local_path() == target.local_path())
            {
                methods.push(ReceiverMethodVisibility {
                    target: target.clone(),
                    span: dependency_span,
                });
            }

            if self
                .environment
                .imported_call_summaries_by_origin
                .contains_key(&method.method_origin)
            {
                self.environment
                    .imported_functions_by_local_path
                    .insert(method_path, super::ImportedFunctionContract { target });
            }
        }

        Ok(())
    }

    /// Register the compiler-standard flat constant prelude for one Moth template file.
    ///
    /// WHAT: makes the official `@html` package's public compile-time constants visible by bare
    ///       name in every `.mtf` body. The constants travel through the ordinary visible-name
    ///       registry, so a genuine collision with another binding is still diagnosed.
    /// WHY: template helper ergonomics are a fixed language contract, not a builder capability
    ///      list. Only compile-time constants enter the prelude; runtime values, functions and
    ///      types stay unavailable. A binding set without a prelude provider (the standalone
    ///      single-source template service, which has no builder surface) simply has no prelude.
    fn register_moth_template_constant_prelude(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        source_file: &PathId,
    ) -> BuilderResult<()> {
        if !self.is_moth_template_source_file(source_file) {
            return Ok(());
        }

        self.remove_moth_template_generated_self_constants(file_visibility, registry, source_file);

        let Some(provider_id) = self
            .source_provider_dependencies
            .template_prelude_provider()
        else {
            return Ok(());
        };

        let mut prelude_constants = Vec::new();
        self.collect_html_template_prelude_constants(provider_id, &mut prelude_constants)?;

        for (name, path, span) in prelude_constants {
            registry.register(
                name,
                VisibleNameBinding::SourceDependency {
                    canonical_path: path,
                },
                span,
            )?;
            file_visibility.visible_declaration_paths_mut().insert(path);
            file_visibility
                .visible_source_names
                .insert(name, SourceDeclarationTarget::Local(path));
        }

        Ok(())
    }

    fn remove_moth_template_generated_self_constants(
        &self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        source_file: &PathId,
    ) {
        let Some((content_name, content_path)) = file_visibility
            .visible_source_names
            .iter()
            .find_map(|(name, path)| {
                if self.string_table.resolve(*name) != "content" {
                    return None;
                }

                if !self
                    .module_symbols
                    .constant_paths
                    .contains(path.local_path())
                {
                    return None;
                }

                if self.symbol_origin_matches_source(path.local_path(), source_file) {
                    Some((*name, *path.local_path()))
                } else {
                    None
                }
            })
        else {
            return;
        };

        file_visibility
            .visible_declaration_paths_mut()
            .remove(&content_path);
        registry.remove_same_file_declaration(content_name, &content_path);
        if file_visibility
            .visible_source_names
            .get(&content_name)
            .is_some_and(|target| target.local_path() == &content_path)
        {
            file_visibility.visible_source_names.remove(&content_name);
        }
    }

    fn collect_html_template_prelude_constants(
        &mut self,
        provider_id: ProviderInterfaceId,
        prelude_constants: &mut Vec<(StringId, PathId, Option<SourceSpan>)>,
    ) -> BuilderResult<()> {
        let prefix = TEMPLATE_PRELUDE_SOURCE_PACKAGE_PREFIX;
        self.register_provider_semantics_once(provider_id)?;
        let view = self
            .source_provider_dependencies
            .binding_view(provider_id)?;
        let interface = self.source_provider_dependencies.interface(provider_id)?;

        // When a source-package root is prepared in the current module (test fixtures), the
        // header-built public-export map contains the entries. Production packages expose the
        // same surface through a completed provider interface.
        if let Some(entries) = self
            .module_symbols
            .source_package_public_exports
            .get(prefix)
            .filter(|entries| !entries.is_empty())
        {
            self.collect_constant_exports(entries, prelude_constants);
            return Ok(());
        }

        for binding in &interface.export_bindings {
            let Some(origin) = view.exported_origin(binding.public_name()).cloned() else {
                continue;
            };
            let Some(declaration) = self
                .environment
                .imported_declarations_by_origin
                .get(&origin)
            else {
                continue;
            };

            if !matches!(
                declaration.semantics,
                PublicDeclarationSemantics::Constant(_)
            ) {
                continue;
            }

            let name_id = self.string_table.intern(binding.public_name());

            // The synthetic path serves as the consumer-local identity for this cross-module
            // constant. Include the provider prefix so the prelude's identity stays distinct
            // from local declarations in collision diagnostics.
            let package_name = self.string_table.intern(prefix);
            let synthetic_path = self
                .path_fork
                .try_intern_components(&[package_name, name_id])
                .ok_or_else(token_lexical::logical_path_capacity)?;
            self.environment
                .imported_declarations_by_local_path
                .entry(synthetic_path)
                .or_insert_with(|| origin);
            // Provider declaration provenance does not cross the interface boundary. This
            // compiler-generated prelude binding therefore carries no authored span.
            prelude_constants.push((name_id, synthetic_path, None));
        }

        Ok(())
    }

    fn collect_constant_exports(
        &self,
        entries: &FxHashSet<PublicExportEntry>,
        prelude_constants: &mut Vec<(StringId, PathId, Option<SourceSpan>)>,
    ) {
        for entry in entries {
            let Some(path) = entry.target.source_path() else {
                continue;
            };

            if !self.module_symbols.constant_paths.contains(path) {
                continue;
            }

            let span = self
                .module_symbols
                .declaration_spans_by_symbol_path
                .get(path)
                .copied();
            prelude_constants.push((entry.export_name, *path, span));
        }
    }

    fn symbol_origin_matches_source(&self, symbol_path: &PathId, source_file: &PathId) -> bool {
        let Some(origin) = self
            .module_symbols
            .canonical_source_by_symbol_path
            .get(symbol_path)
        else {
            return false;
        };

        if origin == source_file {
            return true;
        }

        // `canonical_source_by_symbol_path` stores `Header::canonical_source_file`, which interns
        // the OS path when one exists. The module-file key is the logical interned path, so the
        // first comparison fails and this join recovers same-file identity.
        self.module_symbols
            .source_record(source_file, self.source_files)
            .and_then(|record| record.canonical_os_path.as_ref())
            .is_some_and(|canonical_path| {
                self.path_fork
                    .render_native(*origin, self.string_table, &mut Vec::new())
                    == canonical_path.as_ref()
            })
    }

    fn is_moth_template_source_file(&self, source_file: &PathId) -> bool {
        matches!(
            self.module_symbols
                .source_record(source_file, self.source_files)
                .and_then(|record| record.kind),
            Some(SourceKind::Compiler(SourceFileKind::MothTemplate))
        )
    }

    /// Resolve and register one clause's direct selections.
    ///
    /// WHAT: every selection goes through the shared target resolution, then the clause publishes
    ///       its consumer-local names through the ordinary registration lanes.
    /// WHY: direct dependency clauses and expression-qualified occurrences must agree on
    ///      visibility, root-local isolation, package/module boundaries and provider origin; only
    ///      local-name publication may differ.
    fn resolve_and_register_selection_bindings(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        dependency: &RetainedDependencyClause,
        selections: &[DependencySelection],
        source_file: &PathId,
    ) -> BuilderResult<()> {
        for selection in selections {
            let target =
                self.resolve_selected_target(&dependency.dependency, selection, source_file)?;
            self.register_selected_target(file_visibility, registry, selection, target)?;
        }

        Ok(())
    }

    /// Resolve one direct-selection target through the existing dependency lanes.
    ///
    /// WHAT: checks an authored provider shell first, then a registered provider-backed file,
    ///       then a virtual package symbol, then the source public-export and ordinary source
    ///       target path. It performs every visibility, boundary and declaration-origin check the
    ///       clause form performs.
    /// WHY: a qualified occurrence is a single-use direct selection. Sharing this operation keeps
    ///      the occurrence lane from inventing a second visibility or provider policy while the
    ///      caller decides whether to publish a local name.
    fn resolve_selected_target(
        &mut self,
        dependency: &RetainedDependencyPath,
        selection: &DependencySelection,
        source_file: &PathId,
    ) -> BuilderResult<ResolvedSelectedTarget> {
        let target = self.resolve_selected_target_inner(dependency, selection, source_file)?;
        if let ResolvedSelectedTarget::Source { symbol_path, .. } = &target
            && self
                .module_symbols
                .receiver_method_paths
                .contains(symbol_path)
        {
            // Source-authored receiver methods are not independently bindable or aliasable.
            // They travel with their receiver type's visibility. Both direct clauses and
            // expression-qualified occurrences must reject this selection here, before either
            // caller can publish or record a free-standing target.
            return Err(generics_traits_api::invalid_receiver_declaration(
                InvalidReceiverDeclarationReason::ReceiverMethodImportOrExportNotAllowed,
                Some(selection.source_span),
            )
            .into());
        }
        Ok(target)
    }

    /// Resolve one selection without the shared receiver-method policy.
    ///
    /// WHY: provider, package and source routes each return a target, so the receiver-method
    ///      rejection belongs in the outer wrapper after every route has produced a final source
    ///      path. Keeping the inner function policy-free prevents one route from forgetting it.
    fn resolve_selected_target_inner(
        &mut self,
        dependency: &RetainedDependencyPath,
        selection: &DependencySelection,
        source_file: &PathId,
    ) -> BuilderResult<ResolvedSelectedTarget> {
        let selected_path = self
            .path_fork
            .try_intern_child(dependency.path, selection.source_name)
            .ok_or_else(token_lexical::logical_path_capacity)?;

        if let Some(resolved_clause) = self
            .source_provider_dependencies
            .resolve_clause(dependency.dependency_shell_id)
        {
            return self.resolve_provider_selected_target(
                resolved_clause.provider,
                selection.source_name,
                &selected_path,
                selection,
            );
        }

        if let Some(target) = self.resolve_provider_backed_selected_target(
            dependency,
            selection,
            &selected_path,
            source_file,
        )? {
            return Ok(target);
        }

        if let Some(target) =
            self.resolve_external_package_selected_target(&selected_path, selection)?
        {
            return Ok(target);
        }

        self.resolve_public_or_source_selected_target(selection, source_file, &selected_path)
    }

    /// Resolve one selection against a completed provider interface.
    ///
    /// WHAT: accepts a provider declaration origin or a provider binding export, reporting the
    ///       same public-surface diagnostic a direct-selection clause reports for a missing name.
    /// WHY: `local_path` is the consumer-local path the AST uses for imported declarations; the
    ///      provider public name may differ from the authored selection on a re-exported surface.
    fn resolve_provider_selected_target(
        &mut self,
        provider_id: ProviderInterfaceId,
        provider_name: StringId,
        local_path: &PathId,
        selection: &DependencySelection,
    ) -> BuilderResult<ResolvedSelectedTarget> {
        self.register_provider_semantics_once(provider_id)?;
        let view = self
            .source_provider_dependencies
            .binding_view(provider_id)?;
        let interface = self.source_provider_dependencies.interface(provider_id)?;
        let provider_name_text = self.string_table.resolve(provider_name);

        if let Some(origin) = view.exported_origin(provider_name_text).cloned() {
            // Preserve the successful-interface invariant: an exported origin must carry its
            // declaration record before either a clause or an occurrence may bind it.
            if view.declaration(&origin).is_none() {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HeaderParsing,
                    format!(
                        "provider interface {:?} has no declaration record for exported origin {:?}",
                        provider_id, origin
                    ),
                )
                .into());
            }

            return Ok(ResolvedSelectedTarget::ProviderDeclaration {
                provider_id,
                public_name: provider_name,
                origin,
                local_path: *local_path,
            });
        }

        if let Some(binding) = view.binding_export(provider_name_text) {
            let Some(symbol_id) = self
                .external_package_registry
                .resolve_canonical_symbol(&binding.target)
            else {
                return Err(super::provider_public_surface_diagnostic(
                    local_path,
                    interface,
                    Some(selection.source_span),
                    self.string_table,
                )
                .into());
            };
            return Ok(ResolvedSelectedTarget::External { symbol_id });
        }

        Err(super::provider_public_surface_diagnostic(
            local_path,
            interface,
            Some(selection.source_span),
            self.string_table,
        )
        .into())
    }

    /// Resolve one selection against a registered provider-backed external file.
    fn resolve_provider_backed_selected_target(
        &self,
        dependency: &RetainedDependencyPath,
        selection: &DependencySelection,
        selected_path: &PathId,
        source_file: &PathId,
    ) -> BuilderResult<Option<ResolvedSelectedTarget>> {
        let Some((resolved, remaining)) =
            self.find_provider_resolution_from_retained_target(source_file, dependency)?
        else {
            return Ok(None);
        };

        // A retained provider root must match the provider prefix exactly. Any remaining path
        // component is a malformed provider identity, not a selected source name.
        if !remaining.is_empty() {
            return Err(
                imports::direct_symbol_path_import(dependency.path, Some(dependency.span)).into(),
            );
        }

        let package = self
            .external_package_registry
            .get_package_by_id(resolved.package_id);
        let Some(package) = package else {
            return Err(super::diagnostics::missing_dependency_target(
                &dependency.path,
                Some(dependency.span),
            )
            .into());
        };

        let symbol_id = self
            .lookup_external_symbol_id_by_name(&package.path, selection.source_name)
            .ok_or_else(|| {
                super::diagnostics::missing_dependency_target(
                    selected_path,
                    Some(selection.source_span),
                )
            })?;

        Ok(Some(ResolvedSelectedTarget::External { symbol_id }))
    }

    /// Resolve one selection against virtual external-package metadata.
    fn resolve_external_package_selected_target(
        &mut self,
        selected_path: &PathId,
        selection: &DependencySelection,
    ) -> BuilderResult<Option<ResolvedSelectedTarget>> {
        match resolve_external_package_symbol(ExternalPackageSymbolResolutionInput {
            dependency_path: selected_path,
            external_package_registry: self.external_package_registry,
            string_table: self.string_table,
            path_fork: &*self.path_fork,
        }) {
            ExternalPackageSymbolLookup::Found { symbol_id } => {
                Ok(Some(ResolvedSelectedTarget::External { symbol_id }))
            }
            ExternalPackageSymbolLookup::PackageFoundSymbolMissing {
                package_path,
                symbol_name,
                candidates,
            } => Err(super::diagnostics::missing_package_symbol(
                symbol_name,
                package_path,
                candidates,
                Some(selection.source_span),
            )
            .into()),
            ExternalPackageSymbolLookup::NoMatch => Ok(None),
        }
    }

    /// Publish one already-resolved target as a clause's consumer-local binding.
    fn register_selected_target(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        selection: &DependencySelection,
        target: ResolvedSelectedTarget,
    ) -> BuilderResult<()> {
        match target {
            ResolvedSelectedTarget::ProviderDeclaration {
                provider_id,
                public_name,
                origin,
                local_path,
            } => self.register_provider_declaration_binding(
                file_visibility,
                registry,
                ProviderDeclarationBindingInput {
                    public_name,
                    origin: &origin,
                    local_name: selection.local_name(),
                    local_alias: selection.local_alias(),
                    local_path: &local_path,
                    source_span: Some(selection.source_span),
                    provider_id,
                },
            ),
            ResolvedSelectedTarget::Source {
                symbol_path,
                access,
            } => self.register_source_dependency(
                file_visibility,
                registry,
                SourceDependencyInput {
                    symbol_path: &symbol_path,
                    local_name: selection.local_name(),
                    source_span: Some(selection.source_span),
                    local_alias: selection.local_alias(),
                    access,
                },
            ),
            ResolvedSelectedTarget::External { symbol_id } => self.register_external_import(
                file_visibility,
                registry,
                ExternalImportInput {
                    symbol_name: selection.source_name,
                    local_name: selection.local_name(),
                    source_span: Some(selection.source_span),
                    local_alias: selection.local_alias(),
                    symbol_id,
                },
            ),
        }
    }

    /// Resolve one selection through public-export boundaries and ordinary source targets.
    fn resolve_public_or_source_selected_target(
        &mut self,
        selection: &DependencySelection,
        source_file: &PathId,
        selected_path: &PathId,
    ) -> BuilderResult<ResolvedSelectedTarget> {
        let dependency_bindable_symbol_paths =
            &self.module_symbols.dependency_bindable_source_symbol_paths;

        let mut public_export_input = PublicExportResolutionInput {
            consumer_file: source_file,
            header_path: selected_path,
            source_package_public_exports: &self.module_symbols.source_package_public_exports,
            file_package_membership: &self.module_symbols.file_package_membership,
            module_root_public_exports: &self.module_symbols.module_root_public_exports,
            file_module_membership: &self.module_symbols.file_module_membership,
            module_root_boundaries: &self.module_symbols.module_root_boundaries,
            string_table: self.string_table,
            path_fork: &mut *self.path_fork,
        };

        if let Some(public_export_result) =
            resolve_public_export_boundary(&mut public_export_input)?
        {
            match public_export_result {
                PublicExportLookupResult::ExportedSource {
                    path,
                    exported_entries,
                } => {
                    return Ok(ResolvedSelectedTarget::Source {
                        symbol_path: path,
                        access: SourceDependencyAccess::PublicExport { exported_entries },
                    });
                }
                PublicExportLookupResult::ExportedProviderSelection {
                    selection: provider_selection,
                    source_name: provider_source_name,
                    diagnostic_path,
                    ..
                } => {
                    let provider_id = self
                        .source_provider_dependencies
                        .resolve_reexport(provider_selection.shell)
                        .ok_or_else(|| {
                            InfrastructureFailure::internal(CompilerStage::HeaderParsing, format!(
                                "public provider selection {:?} has no resolved provider for diagnostic path {:?}",
                                provider_selection, diagnostic_path
                            ))
                        })?;
                    return self.resolve_provider_selected_target(
                        provider_id,
                        provider_source_name,
                        selected_path,
                        selection,
                    );
                }
                PublicExportLookupResult::ExportedExternal { symbol_id } => {
                    return Ok(ResolvedSelectedTarget::External { symbol_id });
                }
                PublicExportLookupResult::NotExported {
                    public_surface_name,
                    public_surface_type,
                } => {
                    let public_surface_name_id = self.string_table.intern(&public_surface_name);
                    let diagnostic_public_surface_type = match public_surface_type {
                        super::public_export_resolution::PublicExportSurfaceType::SourcePackage => {
                            ImportPublicSurfaceType::SourcePackage
                        }
                        super::public_export_resolution::PublicExportSurfaceType::ModuleRoot => {
                            ImportPublicSurfaceType::ModuleRoot
                        }
                    };
                    return Err(super::diagnostics::not_exported_by_public_surface(
                        selected_path,
                        public_surface_name_id,
                        diagnostic_public_surface_type,
                        Some(selection.source_span),
                    )
                    .into());
                }
                PublicExportLookupResult::NotAPublicExportBoundary => {}
            }
        }

        let target = resolve_dependency_target(DependencyTargetResolutionInput {
            dependency_path: selected_path,
            span: Some(selection.source_span),
            module_file_paths: &self.module_symbols.module_file_paths,
            dependency_bindable_symbol_paths,
            external_package_registry: self.external_package_registry,
            string_table: self.string_table,
            path_fork: &*self.path_fork,
        })?;

        match target {
            ResolvedDependencyTarget::Source {
                symbol_path,
                access,
            } => {
                if let Some(target_file) = self
                    .module_symbols
                    .canonical_source_by_symbol_path
                    .get(&symbol_path)
                {
                    if let Some(diagnostic) = super::diagnostics::cross_file_root_local_declaration(
                        self.module_symbols,
                        source_file,
                        target_file,
                        &symbol_path,
                        Some(selection.source_span),
                    ) {
                        return Err(diagnostic.into());
                    }

                    check_source_package_boundary(SourcePackageBoundaryCheckInput {
                        consumer_file: source_file,
                        target_file,
                        requested_path: selected_path,
                        span: Some(selection.source_span),
                        file_package_membership: &self.module_symbols.file_package_membership,
                        source_package_root_files: &self.module_symbols.source_package_root_files,
                        string_table: self.string_table,
                    })?;
                    check_module_boundary(ModuleBoundaryCheckInput {
                        consumer_file: source_file,
                        target_file,
                        symbol_path: &symbol_path,
                        span: Some(selection.source_span),
                        file_module_membership: &self.module_symbols.file_module_membership,
                        module_root_public_exports: &self.module_symbols.module_root_public_exports,
                    })?;
                }

                let effective_requirement =
                    if self.is_internal_dependency(source_file, &symbol_path) {
                        SourceDependencyAccess::Internal
                    } else {
                        access
                    };

                Ok(ResolvedSelectedTarget::Source {
                    symbol_path,
                    access: effective_requirement,
                })
            }
            ResolvedDependencyTarget::External { symbol_id } => {
                Ok(ResolvedSelectedTarget::External { symbol_id })
            }
        }
    }

    /// Bind every retained expression-qualified occurrence authored by one file.
    ///
    /// WHAT: resolves each occurrence through the shared direct-selection target path and records
    ///       the result in the per-occurrence lookup. Imported targets join provider provenance,
    ///       but no selected name is inserted into any visible-name map or registry.
    /// WHY: occurrence semantics must be exactly direct-selection semantics minus local-name
    ///       publication; recording the resolved target here lets AST and Stage 3 consume it
    ///       without re-resolving a name or reopening provider state.
    fn bind_qualified_reference_occurrences(
        &mut self,
        file_visibility: &mut FileVisibility,
        source_file: &PathId,
    ) -> BuilderResult<()> {
        let Some(occurrences) = self
            .module_symbols
            .qualified_references_by_source
            .get(source_file)
        else {
            return Ok(());
        };

        // A repeated provider selection resolves and registers provenance once per file. The
        // cache holds the final target so cache hits skip path resolution, namespace checks and
        // provider registration.
        let mut resolved_by_selection: FxHashMap<(PathId, StringId), ResolvedQualifiedTarget> =
            FxHashMap::default();

        for occurrence in occurrences {
            // Occurrences travel through the same Stage 0-resolved edge as dependency clauses.
            // Validate it before the memo can answer from another occurrence's resolution.
            self.validate_local_source_edge(&occurrence.dependency)?;
            let memo_key = (occurrence.dependency.path, occurrence.selected);
            let target = if let Some(target) = resolved_by_selection.get(&memo_key) {
                target.clone()
            } else {
                let selection = DependencySelection {
                    source_name: occurrence.selected,
                    source_span: occurrence.selected_span,
                    local_alias: None,
                };
                let selected_target =
                    self.resolve_selected_target(&occurrence.dependency, &selection, source_file)?;
                let target = match selected_target {
                    ResolvedSelectedTarget::ProviderDeclaration {
                        provider_id,
                        origin,
                        local_path,
                        ..
                    } => {
                        self.register_imported_qualified_target(
                            file_visibility,
                            provider_id,
                            &origin,
                            local_path,
                            occurrence.selected,
                            occurrence.selected_span,
                        )?;
                        ResolvedQualifiedTarget::Imported { origin, local_path }
                    }
                    ResolvedSelectedTarget::Source { symbol_path, .. } => {
                        // A type alias or trait is dependency-bindable as a type, but a value
                        // expression can never consume it. Report the ordinary lane's namespace
                        // diagnostic here, at the selected span, instead of recording a target
                        // AST cannot project into its declaration table.
                        if self.module_symbols.type_alias_paths.contains(&symbol_path) {
                            return Err(names_declarations::namespace_misuse(
                                occurrence.selected,
                                NameNamespace::Value,
                                NameNamespace::Type,
                                Some(occurrence.selected_span),
                            )
                            .into());
                        }
                        if self.module_symbols.trait_paths.contains(&symbol_path) {
                            return Err(generics_traits_api::trait_name_used_as_type(
                                occurrence.selected,
                                Some(occurrence.selected_span),
                            )
                            .into());
                        }
                        ResolvedQualifiedTarget::Source {
                            declaration_path: symbol_path,
                        }
                    }
                    ResolvedSelectedTarget::External { symbol_id } => {
                        ResolvedQualifiedTarget::External { symbol_id }
                    }
                };
                resolved_by_selection.insert(memo_key, target.clone());
                target
            };

            let key = (
                occurrence.selected_span.source(),
                occurrence.dependency.path_syntax,
            );
            // Binding construction holds the only handle, so `make_mut` completes the map in
            // place instead of copying it per inserted occurrence.
            let previous = Arc::make_mut(&mut self.environment.qualified_reference_targets).insert(
                key,
                ResolvedQualifiedReference {
                    target,
                    selected_span: occurrence.selected_span,
                },
            );
            if previous.is_some() {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HeaderParsing,
                    "two qualified-reference occurrences share one authored path row",
                )
                .into());
            }
        }

        Ok(())
    }

    /// Register the semantic provenance an imported qualified target needs for AST projection.
    ///
    /// WHAT: records the consumer-local path to origin mapping, receiver methods for nominal
    ///       targets and the concrete call contract for functions, exactly as a direct-selection
    ///       clause does.
    /// WHY: AST projects imported declarations from this provenance. Skipping it would make an
    ///       occurrence-selected function or struct unusable even though binding proved it. The
    ///       selected name itself is still never published.
    fn register_imported_qualified_target(
        &mut self,
        file_visibility: &mut FileVisibility,
        provider_id: ProviderInterfaceId,
        origin: &OriginDeclarationId,
        local_path: PathId,
        selected: StringId,
        selected_span: SourceSpan,
    ) -> BuilderResult<()> {
        let view = self
            .source_provider_dependencies
            .binding_view(provider_id)?;
        let declaration = view.declaration(origin).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!(
                    "provider interface {:?} has no declaration record for exported origin {:?}",
                    provider_id, origin
                ),
            )
        })?;

        // Provider aliases and traits are valid direct-selection *type* bindings, but an
        // expression-qualified selection is always a value. Match the ordinary value lane's
        // diagnostics before recording provenance the AST declaration table cannot project.
        match &declaration.semantics {
            PublicDeclarationSemantics::TransparentAlias(_) => {
                return Err(names_declarations::namespace_misuse(
                    selected,
                    NameNamespace::Value,
                    NameNamespace::Type,
                    Some(selected_span),
                )
                .into());
            }
            PublicDeclarationSemantics::Trait(_) => {
                return Err(generics_traits_api::trait_name_used_as_type(
                    selected,
                    Some(selected_span),
                )
                .into());
            }
            PublicDeclarationSemantics::Function(_)
            | PublicDeclarationSemantics::Struct(_)
            | PublicDeclarationSemantics::Choice(_)
            | PublicDeclarationSemantics::Constant(_) => {}
        }

        insert_agreed(
            &mut self.environment.imported_declarations_by_local_path,
            local_path,
            origin,
            "qualified-reference declaration path",
        )?;

        let receiver_methods = match &declaration.semantics {
            PublicDeclarationSemantics::Struct(structure) => Some(&structure.receiver_methods),
            PublicDeclarationSemantics::Choice(choice) => Some(&choice.receiver_methods),
            _ => None,
        };
        if let Some(receiver_methods) = receiver_methods {
            self.register_imported_receiver_methods(
                file_visibility,
                &local_path,
                receiver_methods,
                provider_id,
                Some(selected_span),
            )?;
        }

        if let OriginDeclarationId::Function(function_origin) = origin
            && view.concrete_call_summary(function_origin).is_some()
        {
            self.environment.imported_functions_by_local_path.insert(
                local_path,
                super::ImportedFunctionContract {
                    target: SourceFunctionTarget::Imported {
                        origin: function_origin.clone(),
                        local_path,
                    },
                },
            );
        }

        Ok(())
    }

    fn resolve_and_register_namespace_binding(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        dependency: &RetainedDependencyClause,
        source_file: &PathId,
    ) -> BuilderResult<()> {
        // Reject explicit `.moth` extension in dependency paths.
        if has_explicit_moth_extension(
            &dependency.dependency.path,
            self.string_table,
            &*self.path_fork,
        ) {
            return Err(imports::explicit_moth_extension(
                dependency.dependency.path,
                Some(dependency.dependency.span),
            )
            .into());
        }

        if let Some(resolved_clause) = self
            .source_provider_dependencies
            .resolve_clause(dependency.dependency.dependency_shell_id)
        {
            return self.register_source_provider_namespace_binding(
                file_visibility,
                registry,
                dependency,
                resolved_clause,
            );
        }

        // Check for provider-backed bare dependency.
        if let Some(resolved) = self.resolve_provider_backed_namespace_binding(
            file_visibility,
            registry,
            dependency,
            source_file,
        )? {
            return Ok(resolved);
        }

        // Try namespace resolution first. Public-surface namespaces must be checked before
        // concrete file/package resolution so `@module` exposes the module root's public
        // surface, not a private implementation path or a missing direct symbol.
        let namespace_target = self
            .resolve_public_export_namespace_target(dependency, source_file)?
            .or_else(|| {
                resolve_namespace_target(NamespaceTargetResolutionInput {
                    dependency_path: &dependency.dependency.path,
                    module_file_paths: &self.module_symbols.module_file_paths,
                    external_package_registry: self.external_package_registry,
                    string_table: self.string_table,
                    path_fork: &*self.path_fork,
                })
            });

        if let Some(target) = namespace_target {
            return self.register_namespace_binding(
                file_visibility,
                registry,
                dependency,
                source_file,
                target,
            );
        }

        // Namespace resolution failed. Try normal target resolution to detect
        // direct symbol-path dependencies that are now invalid.
        let target = resolve_dependency_target(DependencyTargetResolutionInput {
            dependency_path: &dependency.dependency.path,
            span: Some(dependency.dependency.span),
            module_file_paths: &self.module_symbols.module_file_paths,
            dependency_bindable_symbol_paths: &self
                .module_symbols
                .dependency_bindable_source_symbol_paths,
            external_package_registry: self.external_package_registry,
            string_table: self.string_table,
            path_fork: &*self.path_fork,
        })?;

        // If normal resolution succeeds for a bare dependency, it's a direct symbol-path dependency.
        match target {
            ResolvedDependencyTarget::Source { symbol_path, .. } => Err(
                imports::direct_symbol_path_import(symbol_path, Some(dependency.dependency.span))
                    .into(),
            ),
            ResolvedDependencyTarget::External { .. } => Err(imports::direct_symbol_path_import(
                dependency.dependency.path,
                Some(dependency.dependency.span),
            )
            .into()),
        }
    }
}
