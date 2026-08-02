//! Import environment builder implementation.
//!
//! WHAT: constructs per-file visibility maps by registering same-file declarations,
//!        prelude/builtin names, and resolved imports.
//! WHY: the builder holds mutable state across all files and performs the heavy lifting
//!      of import resolution; keeping it separate from the entry-point orchestration
//!      makes the module structure easier to navigate.

use crate::builder_surface::SourceFileKind;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ImportPublicSurfaceType};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::ExternalSymbolCategory;
use crate::compiler_frontend::headers::module_symbols::{
    ModuleSymbols, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::headers::parse_file_headers::FileImport;
use crate::compiler_frontend::public_interface::{
    PublicDeclarationSemantics, SourceProviderImportSet,
};
use crate::compiler_frontend::source_packages::root_file::{
    import_path_references_config_file, import_path_references_support_root_file,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::FxHashSet;

use super::{
    ExternalPackageSymbolLookup, ExternalPackageSymbolResolutionInput, FileVisibility,
    HeaderImportEnvironment, ImportTargetResolutionInput, ModuleBoundaryCheckInput,
    NamespaceRecord, NamespaceRecordSource, NamespaceTargetResolutionInput, NamespaceTypeMember,
    NamespaceValueMember, PublicExportLookupResult, PublicExportResolutionInput,
    ReceiverMethodVisibility, ResolvedImportTarget, SourceDeclarationTarget, SourceFunctionTarget,
    SourceImportAccess, SourcePackageBoundaryCheckInput, VisibleNameBinding, VisibleNameRegistry,
    check_alias_case_warning, check_module_boundary, check_source_package_boundary,
    has_explicit_moth_extension, resolve_external_package_symbol, resolve_import_target,
    resolve_namespace_target, resolve_public_export_boundary,
};

/// Boxed diagnostic result for the import-environment builder family.
///
/// WHAT: gives visibility construction and its local resolution helpers one small error boundary.
/// WHY: import resolution passes structured diagnostics through several recursive helpers
///      without carrying the large value inline at every return.
type BuilderResult<T> = Result<T, Box<CompilerDiagnostic>>;

pub(crate) struct ImportEnvironmentBuilder<'a> {
    pub(super) module_symbols: &'a ModuleSymbols,
    pub(super) external_package_registry: &'a ExternalPackageRegistry,
    pub(super) external_import_resolution_table: &'a ExternalImportResolutionTable,
    pub(super) source_provider_imports: &'a SourceProviderImportSet<'a>,
    pub(super) string_table: &'a mut StringTable,
    pub(super) environment: HeaderImportEnvironment,
    pub(super) warnings: Vec<crate::compiler_frontend::compiler_messages::CompilerDiagnostic>,
}

impl<'a> ImportEnvironmentBuilder<'a> {
    // ------------------------------
    //  Import helper methods
    // ------------------------------

    /// Derive the local binding name for an import.
    pub(super) fn derive_import_local_name(&self, import: &FileImport) -> BuilderResult<StringId> {
        match import.alias {
            Some(alias) => Ok(alias),
            None => match import.provider.path.name() {
                Some(name) => Ok(name),
                None => Err(Box::new(super::diagnostics::missing_import_target_no_path(
                    import.location.clone(),
                ))),
            },
        }
    }

    /// Emit an alias-case warning when an explicit alias changes leading case.
    pub(super) fn emit_alias_case_warning_if_needed(
        &mut self,
        import: &FileImport,
        symbol_name: StringId,
    ) {
        let Some(alias) = import.alias else {
            return;
        };
        if let Some(warning) = check_alias_case_warning(
            &import.alias_location,
            &import.provider.path_location,
            alias,
            symbol_name,
            self.string_table,
        ) {
            self.warnings.push(warning);
        }
    }

    /// Whether two source files share the same non-public import boundary.
    ///
    /// WHAT: source-backed package members, same module-root members, and files in the implicit entry
    /// module can see each other's ordinary source declarations directly.
    /// WHY: grouped source imports and namespace imports both need the same boundary answer
    /// before deciding whether receiver methods may travel with the imported surface.
    pub(super) fn source_files_share_import_boundary(
        &self,
        importer_file: &InternedPath,
        target_file: &InternedPath,
    ) -> bool {
        let importer_package = self
            .module_symbols
            .file_package_membership
            .get(importer_file);
        let target_package = self.module_symbols.file_package_membership.get(target_file);
        if importer_package == target_package && importer_package.is_some() {
            return true;
        }

        let importer_module = self
            .module_symbols
            .file_module_membership
            .get(importer_file);
        let target_module = self.module_symbols.file_module_membership.get(target_file);
        if importer_module == target_module && importer_module.is_some() {
            return true;
        }

        let importer_has_explicit_module = importer_package.is_some() || importer_module.is_some();
        let target_has_explicit_module = target_package.is_some() || target_module.is_some();

        !importer_has_explicit_module && !target_has_explicit_module
    }

    pub(super) fn build_file_visibility(
        &mut self,
        source_file: &InternedPath,
        importable_symbol_paths: &FxHashSet<InternedPath>,
    ) -> BuilderResult<()> {
        let mut file_visibility = FileVisibility::default();
        let mut registry = VisibleNameRegistry::new();

        // Reserve compiler-owned core cast trait names before any source
        // declarations or imports can claim them. This lets the normal visible-
        // name collision path reject aliases, namespace names, and imported
        // source/export names that would shadow a core cast trait spelling.
        registry.reserve_core_cast_trait_names(self.string_table);

        // 1. Register same-file declarations.
        if let Some(declared_paths) = self.module_symbols.declared_paths_by_file.get(source_file) {
            for path in declared_paths {
                file_visibility
                    .visible_declaration_paths
                    .insert(path.clone());

                let Some(name) = path.name() else {
                    continue;
                };

                if self.module_symbols.receiver_method_paths.contains(path) {
                    // Source receiver methods are receiver-call-only declarations. They do not
                    // reserve ordinary value/import names, because dispatch includes the receiver
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

                registry.register(name, binding, Some(SourceLocation::default()))?;

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
                .visible_declaration_paths
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

        // 3. Register prelude symbols in the registry so imports can detect collisions.
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
        // before explicit imports. The alias name points at an external package path, and the
        // resulting visible namespace record is built from the same path as an explicit
        // `import @package`.
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

        // 5. Resolve and register explicit imports.
        if let Some(imports) = self.module_symbols.file_imports_by_source.get(source_file) {
            for import in imports {
                // Reject direct imports of support-root files and canonical config files.
                // Normal `@*.moth` root references are already caught by the path parser's
                // LeadingAtInPathComponent rejection. Support roots and config files need
                // this later check because `+` and `config` are valid path component characters.
                if import_path_references_support_root_file(
                    &import.provider.path,
                    import.from_grouped,
                    self.string_table,
                ) || import_path_references_config_file(
                    &import.provider.path,
                    import.from_grouped,
                    self.string_table,
                ) {
                    return Err(Box::new(super::diagnostics::direct_special_file_import(
                        &import.provider.path,
                        import.location.clone(),
                    )));
                }

                if import.from_grouped {
                    // Grouped imports keep the existing public-surface-to-target resolution flow.
                    self.resolve_and_register_grouped_import(
                        &mut file_visibility,
                        &mut registry,
                        import,
                        source_file,
                        importable_symbol_paths,
                    )?;
                } else {
                    // Bare imports are namespace imports or direct symbol-path imports.
                    self.resolve_and_register_bare_import(
                        &mut file_visibility,
                        &mut registry,
                        import,
                        source_file,
                        importable_symbol_paths,
                    )?;
                }
            }
        }

        // 6. Inject unshadowed prelude symbols into visible maps.
        // Prelude entries that are still registered as Prelude were not shadowed by imports
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
        // package target were not shadowed by same-file declarations, builtins, or imports
        // of a different target. Explicit imports of the same package already insert an
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
        // file-local visibility maps as authored constants without a user-visible import record.
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
            .insert(source_file.clone(), file_visibility);
        Ok(())
    }

    fn register_source_provider_import(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        import: &FileImport,
        interface: &crate::compiler_frontend::public_interface::PublicSemanticInterface,
    ) -> BuilderResult<()> {
        // Retain the provider's complete stable declaration closure once. AST owns projection
        // into consumer-local handles and must never reopen donor syntax for nested types.
        for provider_declaration in &interface.declarations {
            self.environment
                .imported_declarations_by_origin
                .entry(provider_declaration.origin.clone())
                .or_insert_with(|| provider_declaration.clone());
        }
        self.environment
            .imported_reusable_evidence
            .extend(interface.reusable_evidence.iter().cloned());

        let Some(public_name_id) = import.provider.path.name() else {
            return Err(Box::new(super::diagnostics::missing_import_target_no_path(
                import.location.clone(),
            )));
        };
        let public_name = self.string_table.resolve(public_name_id);
        let Some(origin) = interface.exported_origin(public_name).cloned() else {
            if let Some(binding) = interface.binding_export(public_name) {
                let Some(symbol_id) = self
                    .external_package_registry
                    .resolve_canonical_symbol(&binding.target)
                else {
                    return Err(Box::new(
                        self.provider_public_surface_diagnostic(import, interface),
                    ));
                };
                return self.register_external_import(file_visibility, registry, import, symbol_id);
            }
            return Err(Box::new(
                self.provider_public_surface_diagnostic(import, interface),
            ));
        };
        let Some(declaration) = interface.declaration(&origin).cloned() else {
            return Err(Box::new(
                self.provider_public_surface_diagnostic(import, interface),
            ));
        };

        let local_name = self.derive_import_local_name(import)?;
        let local_path = import.provider.path.clone();
        let target = SourceDeclarationTarget::Imported {
            origin: origin.clone(),
            local_path: local_path.clone(),
        };
        let binding = match declaration.semantics {
            PublicDeclarationSemantics::TransparentAlias(_) => VisibleNameBinding::TypeAlias {
                canonical_path: local_path.clone(),
            },
            PublicDeclarationSemantics::Trait(_) => VisibleNameBinding::Trait {
                canonical_path: local_path.clone(),
            },
            _ => VisibleNameBinding::SourceImport {
                canonical_path: local_path.clone(),
            },
        };

        registry.register(local_name, binding, Some(import.location.clone()))?;
        file_visibility
            .visible_declaration_paths
            .insert(local_path.clone());

        match declaration.semantics {
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

        let receiver_methods = match &declaration.semantics {
            PublicDeclarationSemantics::Struct(structure) => {
                Some(structure.receiver_methods.clone())
            }
            PublicDeclarationSemantics::Choice(choice) => Some(choice.receiver_methods.clone()),
            _ => None,
        };
        self.environment
            .imported_declarations_by_local_path
            .insert(local_path.clone(), declaration);

        if let Some(receiver_methods) = receiver_methods {
            self.register_imported_receiver_methods(
                file_visibility,
                &local_path,
                &receiver_methods,
                interface,
                &import.location,
            )?;
        }

        if let crate::compiler_frontend::semantic_identity::OriginDeclarationId::Function(
            function_origin,
        ) = origin
            && let Some(summary) = interface.concrete_call_summary(&function_origin)
        {
            self.environment.imported_functions_by_local_path.insert(
                local_path.clone(),
                super::ImportedFunctionContract {
                    target: super::SourceFunctionTarget::Imported {
                        origin: function_origin,
                        local_path,
                    },
                    summary: summary.clone(),
                },
            );
        }
        Ok(())
    }

    fn provider_public_surface_diagnostic(
        &mut self,
        import: &FileImport,
        interface: &crate::compiler_frontend::public_interface::PublicSemanticInterface,
    ) -> CompilerDiagnostic {
        let module_path = interface.module_origin.logical_module_path();
        let (surface_name, surface_type) = module_path
            .rsplit('/')
            .find(|component| !component.is_empty())
            .map_or_else(
                || {
                    (
                        interface.module_origin.package().name(),
                        ImportPublicSurfaceType::SourcePackage,
                    )
                },
                |name| (name, ImportPublicSurfaceType::ModuleRoot),
            );
        let surface_name = self.string_table.intern(surface_name);

        super::diagnostics::not_exported_by_public_surface(
            &import.provider.path,
            surface_name,
            surface_type,
            import.location.clone(),
        )
    }

    fn register_source_provider_namespace_import(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        import: &FileImport,
        interface: &crate::compiler_frontend::public_interface::PublicSemanticInterface,
    ) -> BuilderResult<()> {
        for provider_declaration in &interface.declarations {
            self.environment
                .imported_declarations_by_origin
                .entry(provider_declaration.origin.clone())
                .or_insert_with(|| provider_declaration.clone());
        }
        self.environment
            .imported_reusable_evidence
            .extend(interface.reusable_evidence.iter().cloned());

        let mut record = NamespaceRecord::empty(NamespaceRecordSource::SourceFile(
            import.provider.path.clone(),
        ));
        for binding in &interface.export_bindings {
            let Some(declaration) = interface.declaration(binding.origin()).cloned() else {
                return Err(Box::new(super::diagnostics::missing_import_target(
                    &import.provider.path,
                    import.location.clone(),
                )));
            };
            let name = self.string_table.intern(binding.public_name());
            let local_path = import.provider.path.append(name);
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
                        interface,
                        &import.location,
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
                        interface,
                        &import.location,
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
                        && let Some(summary) = interface.concrete_call_summary(function_origin)
                    {
                        self.environment.imported_functions_by_local_path.insert(
                            local_path.clone(),
                            super::ImportedFunctionContract {
                                target: SourceFunctionTarget::Imported {
                                    origin: function_origin.clone(),
                                    local_path: local_path.clone(),
                                },
                                summary: summary.clone(),
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
                .insert(local_path, declaration);
        }

        for binding in &interface.binding_exports {
            let Some(symbol_id) = self
                .external_package_registry
                .resolve_canonical_symbol(&binding.target)
            else {
                return Err(Box::new(super::diagnostics::missing_import_target(
                    &import.provider.path,
                    import.location.clone(),
                )));
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

        let local_name = self.derive_namespace_name(import)?;
        registry.register(
            local_name,
            VisibleNameBinding::NamespaceRecord {
                record_source: record.record_source.clone(),
            },
            Some(import.location.clone()),
        )?;
        file_visibility
            .visible_namespace_records
            .insert(local_name, record);
        Ok(())
    }

    fn register_imported_receiver_methods(
        &mut self,
        file_visibility: &mut FileVisibility,
        imported_type_path: &InternedPath,
        methods: &[crate::compiler_frontend::public_interface::PublicReceiverMethodSemantics],
        interface: &crate::compiler_frontend::public_interface::PublicSemanticInterface,
        import_location: &SourceLocation,
    ) -> BuilderResult<()> {
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
                    location: import_location.clone(),
                });

            if let Some(summary) = interface.concrete_call_summary(&method.method_origin) {
                self.environment.imported_functions_by_local_path.insert(
                    method_path,
                    super::ImportedFunctionContract {
                        target,
                        summary: summary.clone(),
                    },
                );
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
        self.collect_implicit_template_scope_constants(&mut implicit_constants);

        // Layer 2: exported constants from the exact same-directory module public surface. Both
        // layers pass through the same visible-name registry, so equal spellings are diagnosed
        // rather than resolved by source-order precedence.
        self.collect_same_directory_public_export_constants(source_file, &mut implicit_constants);

        for (name, path, location) in implicit_constants {
            registry.register(
                name,
                VisibleNameBinding::SourceImport {
                    canonical_path: path.clone(),
                },
                Some(location),
            )?;
            file_visibility
                .visible_declaration_paths
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
            .visible_declaration_paths
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
    ) {
        let providers: Vec<_> = self
            .source_provider_imports
            .implicit_template_scope_interfaces()
            .collect();

        for (prefix, interface) in providers {
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
                let Some(declaration) = interface.declaration(binding.origin()) else {
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
                    .or_insert_with(|| declaration.clone());
                let location = SourceLocation {
                    scope: InternedPath::from_single_str(&format!("@{prefix}"), self.string_table),
                    ..SourceLocation::default()
                };
                implicit_constants.push((name_id, synthetic_path, location));
            }
        }
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
            let PublicExportTarget::Source(path) = &entry.target else {
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

    fn resolve_and_register_grouped_import(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        import: &FileImport,
        source_file: &InternedPath,
        importable_symbol_paths: &FxHashSet<InternedPath>,
    ) -> BuilderResult<()> {
        if let Some(interface) =
            self.source_provider_imports
                .resolve(source_file, import, self.string_table)
        {
            return self.register_source_provider_import(
                file_visibility,
                registry,
                import,
                interface,
            );
        }

        // Check for provider-backed grouped import first.
        if let Some(resolved) = self.resolve_provider_backed_grouped_import(
            file_visibility,
            registry,
            import,
            source_file,
        )? {
            return Ok(resolved);
        }

        if let Some(resolved) = self.resolve_and_register_external_package_grouped_import(
            file_visibility,
            registry,
            import,
        )? {
            return Ok(resolved);
        }

        // Try public-surface resolution first.
        let public_export_input = PublicExportResolutionInput {
            importer_file: source_file,
            header_path: &import.provider.path,
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
                    return self.register_source_import(
                        file_visibility,
                        registry,
                        &path,
                        import,
                        SourceImportAccess::PublicExport { exported_entries },
                    );
                }
                PublicExportLookupResult::ExportedExternal { symbol_id } => {
                    return self.register_external_import(
                        file_visibility,
                        registry,
                        import,
                        symbol_id,
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
                    return Err(Box::new(
                        super::diagnostics::not_exported_by_public_surface(
                            &import.provider.path,
                            public_surface_name_id,
                            diagnostic_public_surface_type,
                            import.location.clone(),
                        ),
                    ));
                }
                PublicExportLookupResult::NotAPublicExportBoundary => {
                    // Fall through to normal target resolution.
                }
            }
        }

        // Normal target resolution.
        let target = resolve_import_target(ImportTargetResolutionInput {
            import_path: &import.provider.path,
            location: &import.location,
            module_file_paths: &self.module_symbols.module_file_paths,
            importable_symbol_paths,
            external_package_registry: self.external_package_registry,
            string_table: self.string_table,
        })?;

        match target {
            ResolvedImportTarget::Source {
                symbol_path,
                access,
            } => {
                if let Some(target_file) = self
                    .module_symbols
                    .canonical_source_by_symbol_path
                    .get(&symbol_path)
                {
                    check_source_package_boundary(SourcePackageBoundaryCheckInput {
                        importer_file: source_file,
                        target_file,
                        requested_path: &import.provider.path,
                        location: import.location.clone(),
                        file_package_membership: &self.module_symbols.file_package_membership,
                        source_package_root_files: &self.module_symbols.source_package_root_files,
                        string_table: self.string_table,
                    })?;
                    check_module_boundary(ModuleBoundaryCheckInput {
                        importer_file: source_file,
                        target_file,
                        symbol_path: &symbol_path,
                        location: import.location.clone(),
                        file_module_membership: &self.module_symbols.file_module_membership,
                        module_root_public_exports: &self.module_symbols.module_root_public_exports,
                    })?;
                }

                let effective_requirement = if self.is_internal_import(source_file, &symbol_path) {
                    SourceImportAccess::Internal
                } else {
                    access
                };

                self.register_source_import(
                    file_visibility,
                    registry,
                    &symbol_path,
                    import,
                    effective_requirement,
                )
            }
            ResolvedImportTarget::External { symbol_id } => {
                self.register_external_import(file_visibility, registry, import, symbol_id)
            }
        }
    }

    /// Resolve grouped virtual-package imports before source public-surface enforcement.
    ///
    /// WHAT: `import @web/canvas { get_canvas }` is parsed as a grouped import whose
    /// individual entry path is `web/canvas/get_canvas`. That path may also look like a
    /// a path into a module public surface if the project has a `web/canvas/@*.moth` root-file shape.
    /// Checking external metadata here keeps virtual packages out of source public-surface privacy
    /// rules while leaving all source imports on normal public-surface-first resolution.
    fn resolve_and_register_external_package_grouped_import(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        import: &FileImport,
    ) -> BuilderResult<Option<()>> {
        if !import.from_grouped {
            return Ok(None);
        }

        match resolve_external_package_symbol(ExternalPackageSymbolResolutionInput {
            import_path: &import.provider.path,
            external_package_registry: self.external_package_registry,
            string_table: self.string_table,
        }) {
            ExternalPackageSymbolLookup::Found { symbol_id } => {
                self.register_external_import(file_visibility, registry, import, symbol_id)?;
                Ok(Some(()))
            }
            ExternalPackageSymbolLookup::PackageFoundSymbolMissing {
                package_path,
                symbol_name,
            } => Err(Box::new(super::diagnostics::missing_package_symbol(
                symbol_name,
                package_path,
                import.location.clone(),
            ))),
            ExternalPackageSymbolLookup::NoMatch => Ok(None),
        }
    }

    fn resolve_and_register_bare_import(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        import: &FileImport,
        source_file: &InternedPath,
        importable_symbol_paths: &FxHashSet<InternedPath>,
    ) -> BuilderResult<()> {
        // Reject explicit `.moth` extension in import paths.
        if has_explicit_moth_extension(&import.provider.path, self.string_table) {
            return Err(Box::new(CompilerDiagnostic::explicit_moth_extension(
                import.provider.path.clone(),
                import.location.clone(),
            )));
        }

        if let Some(interface) =
            self.source_provider_imports
                .resolve(source_file, import, self.string_table)
        {
            return self.register_source_provider_namespace_import(
                file_visibility,
                registry,
                import,
                interface,
            );
        }

        // Check for provider-backed bare import.
        if let Some(resolved) = self.resolve_provider_backed_bare_import(
            file_visibility,
            registry,
            import,
            source_file,
        )? {
            return Ok(resolved);
        }

        // Try namespace resolution first. Public-surface namespaces must be checked before
        // concrete file/package resolution so `import @module` exposes the module root's public
        // surface, not a private implementation path or a missing direct symbol.
        let namespace_target = self
            .resolve_public_export_namespace_target(import, source_file)
            .or_else(|| {
                resolve_namespace_target(NamespaceTargetResolutionInput {
                    import_path: &import.provider.path,
                    module_file_paths: &self.module_symbols.module_file_paths,
                    external_package_registry: self.external_package_registry,
                    string_table: self.string_table,
                })
            });

        if let Some(target) = namespace_target {
            return self.register_namespace_import(
                file_visibility,
                registry,
                import,
                source_file,
                target,
            );
        }

        // Namespace resolution failed. Try normal target resolution to detect
        // direct symbol-path imports that are now invalid.
        let target = resolve_import_target(ImportTargetResolutionInput {
            import_path: &import.provider.path,
            location: &import.location,
            module_file_paths: &self.module_symbols.module_file_paths,
            importable_symbol_paths,
            external_package_registry: self.external_package_registry,
            string_table: self.string_table,
        })?;

        // If normal resolution succeeds for a bare import, it's a direct symbol-path import.
        match target {
            ResolvedImportTarget::Source { symbol_path, .. } => Err(Box::new(
                CompilerDiagnostic::direct_symbol_path_import(symbol_path, import.location.clone()),
            )),
            ResolvedImportTarget::External { .. } => {
                Err(Box::new(CompilerDiagnostic::direct_symbol_path_import(
                    import.provider.path.clone(),
                    import.location.clone(),
                )))
            }
        }
    }
}
