//! Binding environment builder implementation.
//!
//! WHAT: constructs per-file visibility maps by registering same-file declarations,
//!        prelude/builtin names, and resolved dependencies.
//! WHY: the builder holds mutable state across all files and performs the heavy lifting
//!      of dependency resolution; keeping it separate from the entry-point orchestration
//!      makes the module structure easier to navigate.

use crate::builder_surface::SourceFileKind;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ImportPublicSurfaceType};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::ExternalSymbolCategory;
use crate::compiler_frontend::headers::dependency_clause_syntax::DependencyAlias;
use crate::compiler_frontend::headers::module_symbols::{ModuleSymbols, PublicExportEntry};
use crate::compiler_frontend::headers::parse_file_headers::RetainedDependencyClause;
use crate::compiler_frontend::headers::types::DependencySelection;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::public_interface::{
    ProviderInterfaceId, PublicDeclarationSemantics, ResolvedDependencyClause,
    SourceProviderDependencySet,
};
use crate::compiler_frontend::source_packages::root_file::{
    dependency_path_references_config_file, dependency_path_references_support_root_file,
};
use crate::compiler_frontend::symbols::identifier_policy::ensure_not_keyword_shadow_identifier;
use crate::compiler_frontend::symbols::identity::DependencySelectionId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};
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
    ResolvedDependencyTarget, SourceDeclarationTarget, SourceDependencyAccess,
    SourceFunctionTarget, SourcePackageBoundaryCheckInput, VisibleNameBinding, VisibleNameRegistry,
    check_alias_case_warning, check_module_boundary, check_source_package_boundary,
    has_explicit_moth_extension, resolve_dependency_target, resolve_external_package_symbol,
    resolve_namespace_target, resolve_public_export_boundary,
};

/// Result for the binding-environment builder family.
///
/// WHAT: carries ordinary user-facing dependency diagnostics separately from internal
///       successful-interface invariant failures.
/// WHY: dependency resolution passes structured diagnostics through several recursive helpers, and
///      provider agreement failures must reach the `CompilerError` lane without first degrading
///      into a source diagnostic.
type BuilderResult<T> = Result<T, BindingEnvironmentError>;

/// Resolution facts for one direct dependency selection.
///
/// WHAT: carries the selected provider path and consumer context through one source-or-external
///       target resolution.
/// WHY: keeps the builder's resolution boundary explicit while avoiding repeated argument lists
///      for path, selection and source-file facts.
struct SelectedDependencyInput<'a> {
    selection: &'a DependencySelection,
    selected_path: &'a InternedPath,
    source_file: &'a InternedPath,
    dependency_bindable_symbol_paths: &'a FxHashSet<InternedPath>,
}

/// Inputs for one provider declaration binding.
///
/// WHAT: keeps the provider-owned name, consumer-local identity and authored provenance together
/// while the builder records one dependency-bound declaration surface.
/// WHY: the direct provider path and public provider-selection path share the same registration
/// operation; a named input prevents either caller from growing a positional argument list.
struct ProviderDeclarationBindingInput<'a> {
    public_name: StringId,
    local_name: StringId,
    local_alias: Option<&'a DependencyAlias>,
    local_path: &'a InternedPath,
    source_location: &'a SourceLocation,
    provider_id: ProviderInterfaceId,
}

/// Inputs for one provider-backed selection reached through a public export boundary.
///
/// WHAT: carries the consumer selection and the provider-selection identity through the one
/// public-provider registration owner.
/// WHY: the shell/name/path tuple is one semantic fact even though the selection's local alias
/// and consumer path are owned by the binding clause.
struct PublicProviderSelectionInput<'a> {
    selection: &'a DependencySelection,
    selected_path: &'a InternedPath,
    provider_selection: DependencySelectionId,
    provider_source_name: StringId,
    diagnostic_path: &'a InternedPath,
}

/// Insert one provider fact only when every publisher of the same key agrees.
///
/// WHAT: declaration origins, evidence identities and concrete summary origins are stable
///       semantic keys. A malformed successful artefact may publish one key from more than one
///       provider; first-provider-wins would make the accepted facts depend on dependency order.
/// WHY: this is an internal successful-interface invariant, so disagreement is a
///      `CompilerError`, never a user-facing source diagnostic.
pub(super) fn insert_agreed<K, V>(
    table: &mut FxHashMap<K, V>,
    key: K,
    value: &V,
    fact_class: &str,
) -> Result<(), CompilerError>
where
    K: std::hash::Hash + Eq + Debug,
    V: Eq + Debug + Clone,
{
    match table.entry(key) {
        std::collections::hash_map::Entry::Occupied(existing) => {
            if existing.get() != value {
                return Err(CompilerError::compiler_error(format!(
                    "provider {fact_class} {:?} disagrees across dependency providers",
                    existing.key()
                )));
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
    pub(super) string_table: &'a mut StringTable,
    pub(super) environment: HeaderBindingEnvironment,
    pub(super) warnings: Vec<crate::compiler_frontend::compiler_messages::CompilerDiagnostic>,
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
        consumer_file: &InternedPath,
        target_file: &InternedPath,
    ) -> bool {
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

    pub(super) fn build_file_visibility(
        &mut self,
        source_file: &InternedPath,
        selection_table: &[DependencySelection],
    ) -> BuilderResult<()> {
        let mut file_visibility = FileVisibility::default();
        let mut registry = VisibleNameRegistry::new();

        // Reserve compiler-owned core cast trait names before any source
        // declarations or dependencies can claim them. This lets the normal visible-
        // name collision path reject aliases, namespace names, and dependency-bound
        // source/export names that would shadow a core cast trait spelling.
        registry.reserve_core_cast_trait_names(self.string_table);

        // 1. Register same-file declarations.
        if let Some(declared_paths) = self.module_symbols.declared_paths_by_file.get(source_file) {
            for path in declared_paths {
                file_visibility
                    .visible_declaration_paths_mut()
                    .insert(path.clone());

                let Some(name) = path.name() else {
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
                        SourceLocation::default(),
                    );
                    continue;
                }

                let is_type_alias = self.module_symbols.type_alias_paths.contains(path);
                let is_trait = self.module_symbols.trait_paths.contains(path);
                let binding = if is_type_alias {
                    VisibleNameBinding::TypeAlias {
                        canonical_path: path.clone(),
                    }
                } else if is_trait {
                    VisibleNameBinding::Trait {
                        canonical_path: path.clone(),
                    }
                } else {
                    VisibleNameBinding::SameFileDeclaration {
                        declaration_path: path.clone(),
                    }
                };

                let declaration_location = self
                    .module_symbols
                    .declaration_locations_by_symbol_path
                    .get(path)
                    .cloned()
                    .unwrap_or_default();
                registry.register(name, binding, Some(declaration_location))?;

                if is_type_alias {
                    file_visibility
                        .visible_type_alias_names
                        .insert(name, SourceDeclarationTarget::Local(path.clone()));
                } else if is_trait {
                    file_visibility
                        .visible_trait_names
                        .insert(name, SourceDeclarationTarget::Local(path.clone()));
                } else {
                    file_visibility
                        .visible_source_names
                        .insert(name, SourceDeclarationTarget::Local(path.clone()));
                }
            }
        }

        // 2. Register builtins.
        for path in &self.module_symbols.builtin_visible_symbol_paths {
            file_visibility
                .visible_declaration_paths_mut()
                .insert(path.clone());
            if let Some(name) = path.name() {
                registry.register(
                    name,
                    VisibleNameBinding::Builtin,
                    Some(SourceLocation::default()),
                )?;
                file_visibility
                    .visible_source_names
                    .insert(name, SourceDeclarationTarget::Local(path.clone()));
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
                let selections = dependency
                    .selections(selection_table)
                    .map_err(BindingEnvironmentError::Internal)?;
                // Reject direct dependencies of support-root files and canonical config files.
                // Normal `@*.moth` root references are already caught by the path parser's
                // LeadingAtInPathComponent rejection. Support roots and config files need
                // this later check because `+` and `config` are valid path component characters.
                if dependency_path_references_support_root_file(
                    &dependency.dependency.path,
                    self.string_table,
                ) || dependency_path_references_config_file(
                    &dependency.dependency.path,
                    self.string_table,
                ) {
                    return Err(Box::new(super::diagnostics::direct_special_file_dependency(
                        &dependency.dependency.path,
                        dependency.location.clone(),
                    ))
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
                let record = self
                    .build_external_namespace_record(package_path_id, &SourceLocation::default())?;
                file_visibility
                    .visible_namespace_records
                    .insert(prelude_name_id, record);
            }
        }

        // 8. Add Moth template's compiler-integrated implicit constant scope.
        // WHY: `.mtf` bodies are synthetic constant initializers, so they need the same
        // file-local visibility maps as authored constants without a user-visible dependency record.
        self.register_implicit_moth_template_constant_scope(
            &mut file_visibility,
            &mut registry,
            source_file,
        )?;

        // Sort receiver method paths for deterministic lookup ordering.
        // WHY: same method name from different sources must resolve consistently
        //      across compilations; lexicographic order by function path is stable.
        for paths in file_visibility.visible_receiver_methods.values_mut() {
            paths.sort_by(|a, b| {
                let a_str = a.target.local_path().to_string(self.string_table);
                let b_str = b.target.local_path().to_string(self.string_table);
                a_str.cmp(&b_str)
            });
        }

        self.environment
            .file_visibility_by_source
            .insert(source_file.clone(), Arc::new(file_visibility));
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
            local_name,
            local_alias,
            local_path,
            source_location,
            provider_id,
        } = input;

        let local_name_location = local_alias.map_or(source_location, |alias| &alias.location);
        ensure_not_keyword_shadow_identifier(
            local_name,
            local_name_location.clone(),
            self.string_table,
        )?;

        let view = self
            .source_provider_dependencies
            .binding_view(provider_id)?;
        let public_name_text = self.string_table.resolve(public_name);
        let origin = view
            .exported_origin(public_name_text)
            .cloned()
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "provider interface {:?} has no declaration export for public name '{}'",
                    provider_id, public_name_text
                ))
            })?;
        let declaration = view.declaration(&origin).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "provider interface {:?} has no declaration record for exported origin {:?}",
                provider_id, origin
            ))
        })?;

        self.emit_alias_case_warning_if_needed(local_alias, public_name);

        let binding = match &declaration.semantics {
            PublicDeclarationSemantics::TransparentAlias(_) => VisibleNameBinding::TypeAlias {
                canonical_path: local_path.clone(),
            },
            PublicDeclarationSemantics::Trait(_) => VisibleNameBinding::Trait {
                canonical_path: local_path.clone(),
            },
            _ => VisibleNameBinding::SourceDependency {
                canonical_path: local_path.clone(),
            },
        };

        registry.register(local_name, binding, Some(local_name_location.clone()))?;
        file_visibility
            .visible_declaration_paths_mut()
            .insert(local_path.clone());

        let target = SourceDeclarationTarget::Imported {
            origin: origin.clone(),
            local_path: local_path.clone(),
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
            .insert(local_path.clone(), origin.clone());

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
                source_location,
            )?;
        }

        if let crate::compiler_frontend::semantic_identity::OriginDeclarationId::Function(
            function_origin,
        ) = origin
            && view.concrete_call_summary(&function_origin).is_some()
        {
            self.environment.imported_functions_by_local_path.insert(
                local_path.clone(),
                super::ImportedFunctionContract {
                    target: SourceFunctionTarget::Imported {
                        origin: function_origin,
                        local_path: local_path.clone(),
                    },
                },
            );
        }

        Ok(())
    }

    /// Resolve and register a provider-backed selection reached through another module's public
    /// export surface.
    ///
    /// WHAT: joins the retained provider shell and provider public name from `ProviderSelection`
    ///       before publishing the consumer-local binding.
    /// WHY: `diagnostic_path` is only authored context. Treating it as a source declaration path
    ///       bypasses the completed provider interface and can bind a coincident local path.
    fn register_public_provider_selection(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        input: PublicProviderSelectionInput<'_>,
    ) -> BuilderResult<()> {
        let PublicProviderSelectionInput {
            selection,
            selected_path,
            provider_selection,
            provider_source_name,
            diagnostic_path,
        } = input;
        let provider_id = self
            .source_provider_dependencies
            .resolve_reexport(provider_selection.shell)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "public provider selection {:?} has no resolved provider for diagnostic path {:?}",
                    provider_selection, diagnostic_path
                ))
            })?;
        self.register_provider_semantics_once(provider_id)?;
        let view = self
            .source_provider_dependencies
            .binding_view(provider_id)?;
        let provider_name = self.string_table.resolve(provider_source_name);
        let local_name = selection.local_name();

        if view.exported_origin(provider_name).is_some() {
            return self.register_provider_declaration_binding(
                file_visibility,
                registry,
                ProviderDeclarationBindingInput {
                    public_name: provider_source_name,
                    local_name,
                    local_alias: selection.local_alias(),
                    local_path: selected_path,
                    source_location: selection
                        .local_alias()
                        .map_or(&selection.source_location, |alias| &alias.location),
                    provider_id,
                },
            );
        }

        if let Some(binding) = view.binding_export(provider_name) {
            let symbol_id = self
                .external_package_registry
                .resolve_canonical_symbol(&binding.target)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "public provider selection {:?} has an unresolvable binding target for '{}'",
                        provider_selection, provider_name
                    ))
                })?;
            return self.register_external_import(
                file_visibility,
                registry,
                ExternalImportInput {
                    symbol_name: provider_source_name,
                    local_name,
                    source_location: &selection.source_location,
                    local_alias: selection.local_alias(),
                    symbol_id,
                },
            );
        }

        Err(CompilerError::compiler_error(format!(
            "public provider selection {:?} has no exported provider member '{}'",
            provider_selection, provider_name
        ))
        .into())
    }

    fn register_source_provider_dependency(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        dependency: &RetainedDependencyClause,
        selections: &[DependencySelection],
        resolved_clause: ResolvedDependencyClause,
    ) -> BuilderResult<()> {
        debug_assert_eq!(
            resolved_clause.shell, dependency.dependency.dependency_shell_id,
            "resolved provider clause must retain its authored shell identity"
        );
        let provider_id = resolved_clause.provider;
        // Register the provider's closed semantics once and reuse the narrow binding view for
        // every selected name in this clause.
        self.register_provider_semantics_once(provider_id)?;
        let view = self
            .source_provider_dependencies
            .binding_view(provider_id)?;
        let interface = self.source_provider_dependencies.interface(provider_id)?;

        for selection in selections {
            let public_name_id = selection.source_name;
            let public_name = self.string_table.resolve(public_name_id);
            if view.exported_origin(public_name).is_some() {
                let local_path = dependency.dependency.path.append(public_name_id);
                self.register_provider_declaration_binding(
                    file_visibility,
                    registry,
                    ProviderDeclarationBindingInput {
                        public_name: public_name_id,
                        local_name: selection.local_name(),
                        local_alias: selection.local_alias(),
                        local_path: &local_path,
                        source_location: selection
                            .local_alias()
                            .map_or(&selection.source_location, |alias| &alias.location),
                        provider_id,
                    },
                )?;
                continue;
            }

            if let Some(binding) = view.binding_export(public_name) {
                let Some(symbol_id) = self
                    .external_package_registry
                    .resolve_canonical_symbol(&binding.target)
                else {
                    return Err(Box::new(
                        self.provider_public_surface_diagnostic(dependency, selection, interface),
                    )
                    .into());
                };
                self.register_external_import(
                    file_visibility,
                    registry,
                    ExternalImportInput {
                        symbol_name: public_name_id,
                        local_name: selection.local_name(),
                        source_location: &selection.source_location,
                        local_alias: selection.local_alias(),
                        symbol_id,
                    },
                )?;
                continue;
            }

            return Err(Box::new(
                self.provider_public_surface_diagnostic(dependency, selection, interface),
            )
            .into());
        }
        Ok(())
    }

    fn provider_public_surface_diagnostic(
        &mut self,
        dependency: &RetainedDependencyClause,
        selection: &DependencySelection,
        interface: &crate::compiler_frontend::public_interface::PublicSemanticInterface,
    ) -> CompilerDiagnostic {
        let selected_path = dependency.dependency.path.append(selection.source_name);

        super::provider_public_surface_diagnostic(
            &selected_path,
            interface,
            selection.source_location.clone(),
            self.string_table,
        )
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

        let mut record = NamespaceRecord::empty(NamespaceRecordSource::SourceFile(
            dependency.dependency.path.clone(),
        ));
        for binding in &interface.export_bindings {
            let declaration = view.declaration(binding.origin()).ok_or_else(|| {
                Box::new(super::diagnostics::missing_dependency_target(
                    &dependency.dependency.path,
                    dependency.location.clone(),
                ))
            })?;
            let name = self.string_table.intern(binding.public_name());
            let local_path = dependency.dependency.path.append(name);
            let target = SourceDeclarationTarget::Imported {
                origin: binding.origin().clone(),
                local_path: local_path.clone(),
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
                        &dependency.location,
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
                        &dependency.location,
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
                            local_path.clone(),
                            super::ImportedFunctionContract {
                                target: SourceFunctionTarget::Imported {
                                    origin: function_origin.clone(),
                                    local_path: local_path.clone(),
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
                return Err(Box::new(super::diagnostics::missing_dependency_target(
                    &dependency.dependency.path,
                    dependency.location.clone(),
                ))
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
            Some(
                dependency
                    .namespace_binding_location()
                    .cloned()
                    .unwrap_or_else(|| dependency.location.clone()),
            ),
        )?;
        file_visibility
            .visible_namespace_records
            .insert(local_name, record);
        Ok(())
    }

    pub(super) fn register_imported_receiver_methods(
        &mut self,
        file_visibility: &mut FileVisibility,
        imported_type_path: &InternedPath,
        methods: &[crate::compiler_frontend::public_interface::PublicReceiverMethodSemantics],
        provider_id: ProviderInterfaceId,
        dependency_location: &SourceLocation,
    ) -> BuilderResult<()> {
        self.register_provider_semantics_once(provider_id)?;

        for method in methods {
            let method_name = self
                .string_table
                .intern(method.method_origin.defining_name());
            let method_path = imported_type_path.append(method_name);
            let target = SourceFunctionTarget::Imported {
                origin: method.method_origin.clone(),
                local_path: method_path.clone(),
            };

            file_visibility
                .visible_receiver_methods
                .entry(method_name)
                .or_default()
                .push(ReceiverMethodVisibility {
                    target: target.clone(),
                    location: dependency_location.clone(),
                });

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

    fn register_implicit_moth_template_constant_scope(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        source_file: &InternedPath,
    ) -> BuilderResult<()> {
        if !self.is_moth_template_source_file(source_file) {
            return Ok(());
        }

        self.remove_moth_template_generated_self_constants(file_visibility, registry, source_file);

        let mut implicit_constants = Vec::new();

        // Layer 1: exported constants from every builder-declared source-backed package surface.
        self.collect_implicit_template_scope_constants(&mut implicit_constants)?;

        // Layer 2: exported constants from the exact same-directory module public surface. Both
        // layers pass through the same visible-name registry, so equal spellings are diagnosed
        // rather than resolved by source-order precedence.
        self.collect_same_directory_public_export_constants(source_file, &mut implicit_constants);

        for (name, path, location) in implicit_constants {
            registry.register(
                name,
                VisibleNameBinding::SourceDependency {
                    canonical_path: path.clone(),
                },
                Some(location),
            )?;
            file_visibility
                .visible_declaration_paths_mut()
                .insert(path.clone());
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
        source_file: &InternedPath,
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
                    Some((*name, path.local_path().clone()))
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

    fn collect_implicit_template_scope_constants(
        &mut self,
        implicit_constants: &mut Vec<(StringId, InternedPath, SourceLocation)>,
    ) -> BuilderResult<()> {
        for (prefix, provider_id) in self
            .source_provider_dependencies
            .implicit_template_scope_providers()
        {
            self.register_provider_semantics_once(provider_id)?;
            let view = self
                .source_provider_dependencies
                .binding_view(provider_id)?;
            let interface = self.source_provider_dependencies.interface(provider_id)?;

            // When a source-package root is prepared in the current module (test fixtures), the
            // header-built public-export map contains the entries. Production packages expose
            // the same surface through a completed provider interface.
            if let Some(entries) = self
                .module_symbols
                .source_package_public_exports
                .get(prefix)
                .filter(|entries| !entries.is_empty())
            {
                self.collect_constant_exports(entries, implicit_constants, None);
                continue;
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

                // The synthetic path serves as the consumer-local identity for this
                // cross-module constant. Include the provider prefix so each selected source
                // package retains a distinct identity in collision diagnostics.
                let package_name = self.string_table.intern(prefix);
                let synthetic_path = InternedPath::from_components(vec![package_name, name_id]);
                self.environment
                    .imported_declarations_by_local_path
                    .entry(synthetic_path.clone())
                    .or_insert_with(|| origin);
                let location = view
                    .export_diagnostic_provenance(binding.public_name())
                    .map(|location| self.remap_provider_diagnostic_location(location))
                    .unwrap_or_else(|| SourceLocation {
                        scope: InternedPath::from_single_str(
                            &format!("@{prefix}"),
                            self.string_table,
                        ),
                        ..SourceLocation::default()
                    });
                implicit_constants.push((name_id, synthetic_path, location));
            }
        }

        Ok(())
    }

    fn collect_same_directory_public_export_constants(
        &mut self,
        source_file: &InternedPath,
        implicit_constants: &mut Vec<(StringId, InternedPath, SourceLocation)>,
    ) {
        let Some(root_file) = self.same_directory_root_file(source_file) else {
            return;
        };

        if let Some(entries) = self
            .source_package_public_exports_for_file(&root_file)
            .cloned()
        {
            self.collect_constant_exports(&entries, implicit_constants, Some(source_file));
        }

        if let Some(entries) = self
            .module_root_public_exports_for_file(&root_file)
            .cloned()
        {
            self.collect_constant_exports(&entries, implicit_constants, Some(source_file));
        }
    }

    fn collect_constant_exports(
        &mut self,
        entries: &FxHashSet<PublicExportEntry>,
        implicit_constants: &mut Vec<(StringId, InternedPath, SourceLocation)>,
        excluded_source_file: Option<&InternedPath>,
    ) {
        for entry in entries {
            let Some(path) = entry.target.source_path() else {
                continue;
            };

            if !self.module_symbols.constant_paths.contains(path) {
                continue;
            }

            if excluded_source_file
                .is_some_and(|source_file| self.symbol_origin_matches_source(path, source_file))
            {
                continue;
            }

            let location = self.source_location_for_symbol(path);
            implicit_constants.push((entry.export_name, path.clone(), location));
        }
    }

    fn source_location_for_symbol(&mut self, symbol_path: &InternedPath) -> SourceLocation {
        if let Some(location) = self
            .module_symbols
            .declaration_locations_by_symbol_path
            .get(symbol_path)
        {
            return location.clone();
        }

        if let Some(source_file) = self
            .module_symbols
            .canonical_source_by_symbol_path
            .get(symbol_path)
        {
            if let Some(canonical_path) = self
                .module_symbols
                .canonical_os_path_by_source
                .get(source_file)
            {
                return SourceLocation::from_path(canonical_path, self.string_table);
            }

            return SourceLocation {
                scope: source_file.clone(),
                ..SourceLocation::default()
            };
        }

        SourceLocation {
            scope: symbol_path.clone(),
            ..SourceLocation::default()
        }
    }

    fn remap_provider_diagnostic_location(
        &mut self,
        location: &crate::compiler_frontend::public_interface::PublicDiagnosticLocation,
    ) -> SourceLocation {
        SourceLocation {
            scope: InternedPath::from_components(
                location
                    .scope_components
                    .iter()
                    .map(|component| self.string_table.intern(component))
                    .collect(),
            ),
            start_pos: CharPosition {
                line_number: location.start_line,
                char_column: location.start_column,
            },
            end_pos: CharPosition {
                line_number: location.end_line,
                char_column: location.end_column,
            },
        }
    }

    fn symbol_origin_matches_source(
        &self,
        symbol_path: &InternedPath,
        source_file: &InternedPath,
    ) -> bool {
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

        let Some(canonical_source_path) = self
            .module_symbols
            .canonical_os_path_by_source
            .get(source_file)
        else {
            return false;
        };

        origin.to_path_buf(self.string_table) == *canonical_source_path
    }

    fn is_moth_template_source_file(&self, source_file: &InternedPath) -> bool {
        let Some(path) = self
            .module_symbols
            .canonical_os_path_by_source
            .get(source_file)
        else {
            return source_file
                .to_path_buf(self.string_table)
                .extension()
                .and_then(|extension| extension.to_str())
                .and_then(SourceFileKind::from_extension)
                == Some(SourceFileKind::MothTemplate);
        };

        path.extension()
            .and_then(|extension| extension.to_str())
            .and_then(SourceFileKind::from_extension)
            == Some(SourceFileKind::MothTemplate)
    }

    fn same_directory_root_file(&self, source_file: &InternedPath) -> Option<InternedPath> {
        let moth_template_directory = self.source_directory(source_file)?;

        self.module_symbols
            .file_roles_by_source
            .iter()
            .find_map(|(candidate_source, role)| {
                if !role.is_export_capable() {
                    return None;
                }

                let candidate_directory = self.source_directory(candidate_source)?;
                if candidate_directory == moth_template_directory {
                    Some(candidate_source.clone())
                } else {
                    None
                }
            })
    }

    fn source_directory(&self, source_file: &InternedPath) -> Option<std::path::PathBuf> {
        if let Some(path) = self
            .module_symbols
            .canonical_os_path_by_source
            .get(source_file)
        {
            return path.parent().map(|parent| parent.to_path_buf());
        }

        source_file
            .parent()
            .map(|parent| parent.to_path_buf(self.string_table))
    }

    fn source_package_public_exports_for_file(
        &self,
        root_file: &InternedPath,
    ) -> Option<&FxHashSet<PublicExportEntry>> {
        let prefix = self
            .module_symbols
            .source_package_root_files
            .iter()
            .find_map(|(prefix, source)| {
                if source == root_file {
                    Some(prefix)
                } else {
                    None
                }
            })?;

        self.module_symbols
            .source_package_public_exports
            .get(prefix)
    }

    fn module_root_public_exports_for_file(
        &self,
        root_file: &InternedPath,
    ) -> Option<&FxHashSet<PublicExportEntry>> {
        let module_root = self.module_symbols.file_module_membership.get(root_file)?;

        self.module_symbols
            .module_root_public_exports
            .get(module_root)
    }

    fn resolve_and_register_selection_bindings(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        dependency: &RetainedDependencyClause,
        selections: &[DependencySelection],
        source_file: &InternedPath,
    ) -> BuilderResult<()> {
        let source_provider_clause = self
            .source_provider_dependencies
            .resolve_clause(dependency.dependency.dependency_shell_id);
        if let Some(resolved_clause) = source_provider_clause {
            return self.register_source_provider_dependency(
                file_visibility,
                registry,
                dependency,
                selections,
                resolved_clause,
            );
        }

        // Check for provider-backed external-file selections first.
        if let Some(resolved) = self.resolve_provider_backed_selection_dependencies(
            file_visibility,
            registry,
            dependency,
            selections,
            source_file,
        )? {
            return Ok(resolved);
        }

        if let Some(resolved) = self.resolve_and_register_external_package_selection_dependencies(
            file_visibility,
            registry,
            dependency,
            selections,
        )? {
            return Ok(resolved);
        }

        for selection in selections {
            let selected_path = dependency.dependency.path.append(selection.source_name);
            self.resolve_and_register_selected_dependency(
                file_visibility,
                registry,
                SelectedDependencyInput {
                    selection,
                    selected_path: &selected_path,
                    source_file,
                    dependency_bindable_symbol_paths: &self
                        .module_symbols
                        .dependency_bindable_source_symbol_paths,
                },
            )?;
        }

        Ok(())
    }

    fn resolve_and_register_selected_dependency(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        input: SelectedDependencyInput<'_>,
    ) -> BuilderResult<()> {
        let SelectedDependencyInput {
            selection,
            selected_path,
            source_file,
            dependency_bindable_symbol_paths,
        } = input;

        let local_name = selection.local_name();

        let public_export_input = PublicExportResolutionInput {
            consumer_file: source_file,
            header_path: selected_path,
            source_package_public_exports: &self.module_symbols.source_package_public_exports,
            file_package_membership: &self.module_symbols.file_package_membership,
            module_root_public_exports: &self.module_symbols.module_root_public_exports,
            file_module_membership: &self.module_symbols.file_module_membership,
            module_root_boundaries: &self.module_symbols.module_root_boundaries,
            string_table: self.string_table,
        };

        if let Some(public_export_result) = resolve_public_export_boundary(&public_export_input) {
            match public_export_result {
                PublicExportLookupResult::ExportedSource {
                    path,
                    exported_entries,
                } => {
                    return self.register_source_dependency(
                        file_visibility,
                        registry,
                        SourceDependencyInput {
                            symbol_path: &path,
                            local_name,
                            source_location: &selection.source_location,
                            local_alias: selection.local_alias(),
                            access: SourceDependencyAccess::PublicExport { exported_entries },
                        },
                    );
                }
                PublicExportLookupResult::ExportedProviderSelection {
                    selection: provider_selection,
                    source_name: provider_source_name,
                    diagnostic_path,
                    ..
                } => {
                    return self.register_public_provider_selection(
                        file_visibility,
                        registry,
                        PublicProviderSelectionInput {
                            selection,
                            selected_path,
                            provider_selection,
                            provider_source_name,
                            diagnostic_path: &diagnostic_path,
                        },
                    );
                }
                PublicExportLookupResult::ExportedExternal { symbol_id } => {
                    return self.register_external_import(
                        file_visibility,
                        registry,
                        ExternalImportInput {
                            symbol_name: selection.source_name,
                            local_name,
                            source_location: &selection.source_location,
                            local_alias: selection.local_alias(),
                            symbol_id,
                        },
                    );
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
                    return Err(Box::new(super::diagnostics::not_exported_by_public_surface(
                        selected_path,
                        public_surface_name_id,
                        diagnostic_public_surface_type,
                        selection.source_location.clone(),
                    ))
                    .into());
                }
                PublicExportLookupResult::NotAPublicExportBoundary => {}
            }
        }

        let target = resolve_dependency_target(DependencyTargetResolutionInput {
            dependency_path: selected_path,
            location: &selection.source_location,
            module_file_paths: &self.module_symbols.module_file_paths,
            dependency_bindable_symbol_paths,
            external_package_registry: self.external_package_registry,
            string_table: self.string_table,
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
                    check_source_package_boundary(SourcePackageBoundaryCheckInput {
                        consumer_file: source_file,
                        target_file,
                        requested_path: selected_path,
                        location: selection.source_location.clone(),
                        file_package_membership: &self.module_symbols.file_package_membership,
                        source_package_root_files: &self.module_symbols.source_package_root_files,
                        string_table: self.string_table,
                    })?;
                    check_module_boundary(ModuleBoundaryCheckInput {
                        consumer_file: source_file,
                        target_file,
                        symbol_path: &symbol_path,
                        location: selection.source_location.clone(),
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

                self.register_source_dependency(
                    file_visibility,
                    registry,
                    SourceDependencyInput {
                        symbol_path: &symbol_path,
                        local_name,
                        source_location: &selection.source_location,
                        local_alias: selection.local_alias(),
                        access: effective_requirement,
                    },
                )?;
            }
            ResolvedDependencyTarget::External { symbol_id } => {
                self.register_external_import(
                    file_visibility,
                    registry,
                    ExternalImportInput {
                        symbol_name: selection.source_name,
                        local_name,
                        source_location: &selection.source_location,
                        local_alias: selection.local_alias(),
                        symbol_id,
                    },
                )?;
            }
        }

        Ok(())
    }

    /// Resolve direct selections from a virtual package before source public-surface enforcement.
    ///
    /// WHAT: the provider root and selected name are combined only for the external-package
    /// registry lookup. The retained clause still owns one shell and one flat selection list.
    /// Checking external metadata here keeps virtual packages out of source public-surface privacy
    /// rules while leaving all source dependencies on normal public-surface-first resolution.
    fn resolve_and_register_external_package_selection_dependencies(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        dependency: &RetainedDependencyClause,
        selections: &[DependencySelection],
    ) -> BuilderResult<Option<()>> {
        if selections.is_empty() {
            return Ok(None);
        }

        let mut found = false;
        for selection in selections {
            let selected_path = dependency.dependency.path.append(selection.source_name);
            match resolve_external_package_symbol(ExternalPackageSymbolResolutionInput {
                dependency_path: &selected_path,
                external_package_registry: self.external_package_registry,
                string_table: self.string_table,
            }) {
                ExternalPackageSymbolLookup::Found { symbol_id } => {
                    found = true;
                    let local_name = selection.local_name();
                    self.register_external_import(
                        file_visibility,
                        registry,
                        ExternalImportInput {
                            symbol_name: selection.source_name,
                            local_name,
                            source_location: &selection.source_location,
                            local_alias: selection.local_alias(),
                            symbol_id,
                        },
                    )?;
                }
                ExternalPackageSymbolLookup::PackageFoundSymbolMissing {
                    package_path,
                    symbol_name,
                } => {
                    return Err(Box::new(super::diagnostics::missing_package_symbol(
                        symbol_name,
                        package_path,
                        selection.source_location.clone(),
                    ))
                    .into());
                }
                ExternalPackageSymbolLookup::NoMatch => {
                    if found {
                        return Err(Box::new(super::diagnostics::missing_dependency_target(
                            &selected_path,
                            selection.source_location.clone(),
                        ))
                        .into());
                    }
                    return Ok(None);
                }
            }
        }

        Ok(found.then_some(()))
    }

    fn resolve_and_register_namespace_binding(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        dependency: &RetainedDependencyClause,
        source_file: &InternedPath,
    ) -> BuilderResult<()> {
        // Reject explicit `.moth` extension in dependency paths.
        if has_explicit_moth_extension(&dependency.dependency.path, self.string_table) {
            return Err(Box::new(CompilerDiagnostic::explicit_moth_extension(
                dependency.dependency.path.clone(),
                dependency.location.clone(),
            ))
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
            .resolve_public_export_namespace_target(dependency, source_file)
            .or_else(|| {
                resolve_namespace_target(NamespaceTargetResolutionInput {
                    dependency_path: &dependency.dependency.path,
                    module_file_paths: &self.module_symbols.module_file_paths,
                    external_package_registry: self.external_package_registry,
                    string_table: self.string_table,
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
            location: &dependency.location,
            module_file_paths: &self.module_symbols.module_file_paths,
            dependency_bindable_symbol_paths: &self
                .module_symbols
                .dependency_bindable_source_symbol_paths,
            external_package_registry: self.external_package_registry,
            string_table: self.string_table,
        })?;

        // If normal resolution succeeds for a bare dependency, it's a direct symbol-path dependency.
        match target {
            ResolvedDependencyTarget::Source { symbol_path, .. } => {
                Err(Box::new(CompilerDiagnostic::direct_symbol_path_import(
                    symbol_path,
                    dependency.location.clone(),
                ))
                .into())
            }
            ResolvedDependencyTarget::External { .. } => {
                Err(Box::new(CompilerDiagnostic::direct_symbol_path_import(
                    dependency.dependency.path.clone(),
                    dependency.location.clone(),
                ))
                .into())
            }
        }
    }
}
