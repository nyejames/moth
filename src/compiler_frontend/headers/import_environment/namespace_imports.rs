//! Namespace import registration and namespace record construction.
//!
//! WHAT: resolves bare imports into namespace records, validates public export boundaries for namespace
//! imports, and builds field-access-only records from source files, public export surfaces, and
//! external packages.
//! WHY: namespace imports are structurally different from grouped imports: they expose a record
//! surface rather than individual symbols, so their registration and record building is separate.
//! External package records are recursive so multi-component symbol paths such as `io.input.new`
//! are represented in header/import visibility.
//! MUST NOT: register grouped imports or perform AST semantic validation.

use super::public_export_resolution::effective_module_boundary_path;
use super::{
    FileVisibility, ImportEnvironmentBuilder, NamespaceRecord, NamespaceRecordSource,
    NamespaceTypeMember, NamespaceValueMember, ResolvedNamespaceTarget, SourceDeclarationTarget,
    SourceImportAccess, VisibleNameBinding, VisibleNameRegistry,
};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ImportPublicSurfaceType};
use crate::compiler_frontend::external_packages::{ExternalSymbolId, ExternalSymbolPath};
use crate::compiler_frontend::headers::import_environment::diagnostics;
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationKind, PublicExportEntry, PublicExportTarget,
};
use crate::compiler_frontend::headers::parse_file_headers::FileImport;
use crate::compiler_frontend::keywords::is_valid_identifier;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::{FxHashMap, FxHashSet};

/// Boxed diagnostic result for namespace import registration and record construction.
///
/// WHAT: gives namespace registration and recursive record construction one small error boundary.
/// WHY: record insertion and privacy checks propagate structured diagnostics through the
///      same connected family without carrying the large value inline at every return.
type NamespaceImportResult<T> = Result<T, Box<CompilerDiagnostic>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SymbolKind {
    Value,
    Type,
}

/// Helper that walks an `ExternalSymbolPath` and inserts symbols into a `NamespaceRecord`
/// tree, creating child namespace records for intermediate components.
///
/// WHAT: turns a flat list of package surface paths into a nested namespace record tree.
/// WHY: the registry stores symbols by full path, but namespace records are trees keyed by
/// component name. A dedicated inserter keeps the descent, child creation, and duplicate
/// detection in one place.
struct ExternalNamespaceRecordInserter<'a> {
    string_table: &'a mut StringTable,
    location: &'a SourceLocation,
}

impl<'a> ExternalNamespaceRecordInserter<'a> {
    fn insert(
        &mut self,
        record: &mut NamespaceRecord,
        symbol_path: &ExternalSymbolPath,
        symbol_id: ExternalSymbolId,
        surface_path: &InternedPath,
    ) -> NamespaceImportResult<()> {
        self.insert_at(record, symbol_path.components(), 0, symbol_id, surface_path)
    }

    fn insert_at(
        &mut self,
        record: &mut NamespaceRecord,
        components: &[String],
        index: usize,
        symbol_id: ExternalSymbolId,
        surface_path: &InternedPath,
    ) -> NamespaceImportResult<()> {
        let component = &components[index];
        let name_id = self.string_table.intern(component);
        let child_surface_path = surface_path.join_str(component, self.string_table);
        let is_leaf = index == components.len() - 1;

        if is_leaf {
            // A namespace slot is exclusive: a leaf cannot share its name with
            // either a child namespace or the other leaf kind at this level.
            if record.child_namespaces.contains_key(&name_id)
                || record.value_members.contains_key(&name_id)
                || record.type_members.contains_key(&name_id)
            {
                return Err(Box::new(
                    CompilerDiagnostic::duplicate_import_surface_member(
                        child_surface_path,
                        name_id,
                        self.location.clone(),
                    ),
                ));
            }

            match symbol_id {
                ExternalSymbolId::Function(function_id) => {
                    record.value_members.insert(
                        name_id,
                        NamespaceValueMember::ExternalSymbol(ExternalSymbolId::Function(
                            function_id,
                        )),
                    );
                }
                ExternalSymbolId::Constant(constant_id) => {
                    record.value_members.insert(
                        name_id,
                        NamespaceValueMember::ExternalSymbol(ExternalSymbolId::Constant(
                            constant_id,
                        )),
                    );
                }
                ExternalSymbolId::Type(type_id) => {
                    record.type_members.insert(
                        name_id,
                        NamespaceTypeMember::ExternalSymbol(ExternalSymbolId::Type(type_id)),
                    );
                }
            }
            return Ok(());
        }

        // Intermediate components become child namespaces. They cannot share a name with
        // a value or type member at the same level.
        if record.value_members.contains_key(&name_id) || record.type_members.contains_key(&name_id)
        {
            return Err(Box::new(
                CompilerDiagnostic::duplicate_import_surface_member(
                    child_surface_path.clone(),
                    name_id,
                    self.location.clone(),
                ),
            ));
        }

        let child_source = record.record_source.clone();
        let child_record = record
            .child_namespaces
            .entry(name_id)
            .or_insert_with(|| NamespaceRecord::empty(child_source));

        self.insert_at(
            child_record,
            components,
            index + 1,
            symbol_id,
            &child_surface_path,
        )
    }
}

impl<'a> ImportEnvironmentBuilder<'a> {
    /// Build and register a namespace import record.
    pub(super) fn register_namespace_import(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        import: &FileImport,
        source_file: &InternedPath,
        namespace_target: ResolvedNamespaceTarget,
    ) -> NamespaceImportResult<()> {
        let local_name = self.derive_namespace_name(import)?;
        let source_namespace_access =
            if let ResolvedNamespaceTarget::SourceFile(file_path) = &namespace_target {
                Some(
                    self.source_namespace_receiver_access(file_path, source_file)
                        .unwrap_or(SourceImportAccess::Internal),
                )
            } else {
                None
            };

        let record = match namespace_target {
            ResolvedNamespaceTarget::SourceFile(ref file_path) => {
                self.validate_namespace_source_boundary(file_path, import, source_file)?;
                match source_namespace_access.as_ref() {
                    Some(SourceImportAccess::PublicExport { exported_entries }) => self
                        .build_public_export_namespace_record(
                            file_path,
                            exported_entries,
                            &import.location,
                        )?,
                    _ => self.build_source_namespace_record(file_path, &import.location)?,
                }
            }
            ResolvedNamespaceTarget::ExternalPackage { package_path } => {
                self.build_external_namespace_record(package_path, &import.location)?
            }
        };

        let record_source = match &namespace_target {
            ResolvedNamespaceTarget::SourceFile(file_path) => {
                NamespaceRecordSource::SourceFile(file_path.clone())
            }
            ResolvedNamespaceTarget::ExternalPackage { package_path } => {
                NamespaceRecordSource::ExternalPackage(*package_path)
            }
        };

        registry.register(
            local_name,
            VisibleNameBinding::NamespaceRecord {
                record_source: record_source.clone(),
            },
            Some(import.location.clone()),
        )?;

        file_visibility
            .visible_namespace_records
            .insert(local_name, record);

        // Namespace imports make source receiver methods from the imported surface visible
        // through the receiver catalog under their original names. External package functions
        // are namespace value members only; external packages do not expose receiver methods.
        match &namespace_target {
            ResolvedNamespaceTarget::SourceFile(file_path) => {
                if let Some(access) = source_namespace_access.as_ref() {
                    self.add_source_namespace_receiver_methods(
                        file_visibility,
                        file_path,
                        access,
                        import.location.clone(),
                    );
                }
            }
            ResolvedNamespaceTarget::ExternalPackage { .. } => {}
        }

        Ok(())
    }

    fn add_source_namespace_receiver_methods(
        &self,
        file_visibility: &mut FileVisibility,
        namespace_file: &InternedPath,
        access: &SourceImportAccess,
        location: SourceLocation,
    ) {
        match access {
            SourceImportAccess::PublicExport { exported_entries } => {
                for entry in exported_entries {
                    let PublicExportTarget::Source(type_path) = &entry.target else {
                        continue;
                    };

                    if !self.module_symbols.nominal_type_paths.contains(type_path) {
                        continue;
                    }

                    let Some(target_file) = self
                        .module_symbols
                        .canonical_source_by_symbol_path
                        .get(type_path)
                    else {
                        continue;
                    };

                    self.auto_import_receiver_methods_for_type(
                        file_visibility,
                        type_path,
                        target_file,
                        access,
                    );
                }
            }

            SourceImportAccess::Internal | SourceImportAccess::DirectSourceExport => {
                if let Some(declared_paths) = self
                    .module_symbols
                    .declared_paths_by_file
                    .get(namespace_file)
                {
                    for path in declared_paths {
                        if self.module_symbols.receiver_method_paths.contains(path)
                            && let Some(name) = path.name()
                        {
                            Self::add_visible_receiver_method(
                                file_visibility,
                                name,
                                path,
                                location.clone(),
                            );
                        }
                    }
                }
            }
        }
    }

    /// Receiver-method access model for a source namespace import.
    ///
    /// WHAT: internal source namespaces retain all receiver methods from that file. Public export
    /// namespaces expose receiver-call visibility through exported receiver types, never through
    /// explicit method fields.
    fn source_namespace_receiver_access(
        &self,
        namespace_file: &InternedPath,
        importer_file: &InternedPath,
    ) -> Option<SourceImportAccess> {
        if self.source_files_share_import_boundary(importer_file, namespace_file) {
            return Some(SourceImportAccess::Internal);
        }

        self.public_export_entries_for_source_file(namespace_file)
            .map(|exported_entries| SourceImportAccess::PublicExport { exported_entries })
    }

    /// Public export entries for a concrete root source file, if this source is a root.
    fn public_export_entries_for_source_file(
        &self,
        source_file: &InternedPath,
    ) -> Option<FxHashSet<PublicExportEntry>> {
        for (package_prefix, root_file) in &self.module_symbols.source_package_root_files {
            if root_file == source_file {
                return self
                    .module_symbols
                    .source_package_public_exports
                    .get(package_prefix)
                    .cloned();
            }
        }

        let module_root = self
            .module_symbols
            .file_module_membership
            .get(source_file)?;
        self.module_symbols
            .module_root_public_exports
            .get(module_root)
            .cloned()
    }

    /// Resolve a bare import that names a public export namespace.
    ///
    /// WHAT: `import @package` and cross-module `import @module` expose the target prepared root
    /// file surface as a namespace record.
    /// WHY: namespace imports must obey the same public export boundary as grouped imports.
    pub(super) fn resolve_public_export_namespace_target(
        &mut self,
        import: &FileImport,
        source_file: &InternedPath,
    ) -> Option<ResolvedNamespaceTarget> {
        let components = import.provider.path.as_components();
        if components.is_empty() {
            return None;
        }

        if let Some(target) = self.resolve_source_package_public_export(components, source_file) {
            return Some(target);
        }

        self.resolve_module_root_public_export(&import.provider.path, source_file)
    }

    fn resolve_source_package_public_export(
        &mut self,
        components: &[StringId],
        source_file: &InternedPath,
    ) -> Option<ResolvedNamespaceTarget> {
        if components.len() != 1 {
            return None;
        }

        let package_prefix = self.string_table.resolve(components[0]).to_owned();
        if !self
            .module_symbols
            .source_package_public_exports
            .contains_key(&package_prefix)
        {
            return None;
        }

        let importer_package = self.module_symbols.file_package_membership.get(source_file);
        if importer_package.map(String::as_str) == Some(package_prefix.as_str()) {
            return None;
        }

        let root_file = self
            .module_symbols
            .source_package_root_files
            .get(&package_prefix)?
            .clone();

        self.module_symbols
            .module_file_paths
            .contains(&root_file)
            .then_some(ResolvedNamespaceTarget::SourceFile(root_file))
    }

    fn resolve_module_root_public_export(
        &mut self,
        import_path: &InternedPath,
        source_file: &InternedPath,
    ) -> Option<ResolvedNamespaceTarget> {
        let effective_path = effective_module_boundary_path(
            source_file,
            import_path,
            &self.module_symbols.file_module_membership,
            &self.module_symbols.module_root_boundaries,
        );

        for boundary in &self.module_symbols.module_root_boundaries {
            if effective_path != boundary.import_prefix {
                continue;
            }

            let importer_root = self.module_symbols.file_module_membership.get(source_file);
            if importer_root == Some(&boundary.module_root) {
                return None;
            }

            if self
                .module_symbols
                .module_file_paths
                .contains(&boundary.root_file)
            {
                return Some(ResolvedNamespaceTarget::SourceFile(
                    boundary.root_file.clone(),
                ));
            }
        }

        None
    }

    /// Enforce public export privacy for concrete source-file namespace imports.
    fn validate_namespace_source_boundary(
        &mut self,
        target_file: &InternedPath,
        import: &FileImport,
        source_file: &InternedPath,
    ) -> NamespaceImportResult<()> {
        if let Some(target_package) = self
            .module_symbols
            .file_package_membership
            .get(target_file)
            .cloned()
        {
            let importer_package = self.module_symbols.file_package_membership.get(source_file);
            if importer_package.map(String::as_str) != Some(target_package.as_str()) {
                if self
                    .module_symbols
                    .source_package_root_files
                    .get(&target_package)
                    .is_some_and(|root_file| target_file == root_file)
                {
                    return Ok(());
                }

                let public_surface_name_id = self.string_table.intern(&target_package);
                return Err(Box::new(diagnostics::not_exported_by_public_surface(
                    &import.provider.path,
                    public_surface_name_id,
                    ImportPublicSurfaceType::SourcePackage,
                    import.location.clone(),
                )));
            }
        }

        let importer_root = self.module_symbols.file_module_membership.get(source_file);
        let target_root = self.module_symbols.file_module_membership.get(target_file);

        let (Some(importer_root), Some(target_root)) = (importer_root, target_root) else {
            return Ok(());
        };

        if importer_root == target_root {
            return Ok(());
        }

        if self
            .module_root_public_surface_file(target_root)
            .is_some_and(|root_file| target_file == &root_file)
        {
            return Ok(());
        }

        if self
            .module_symbols
            .module_root_public_exports
            .contains_key(target_root)
        {
            return Err(Box::new(diagnostics::cross_module_import_not_exported(
                &import.provider.path,
                import.location.clone(),
            )));
        }

        Err(Box::new(diagnostics::missing_module_root_public_surface(
            &import.provider.path,
            import.location.clone(),
        )))
    }

    fn module_root_public_surface_file(&self, module_root: &InternedPath) -> Option<InternedPath> {
        self.module_symbols
            .module_root_boundaries
            .iter()
            .find(|boundary| boundary.module_root == *module_root)
            .map(|boundary| boundary.root_file.clone())
    }

    /// Derive the local namespace name from an import, validating the default name.
    pub(super) fn derive_namespace_name(
        &mut self,
        import: &FileImport,
    ) -> NamespaceImportResult<StringId> {
        match import.alias {
            Some(alias) => Ok(alias),
            None => {
                let stem = import
                    .provider
                    .path
                    .name()
                    .map(|n| self.string_table.resolve(n).to_owned())
                    .unwrap_or_default();
                let stem = stem.strip_suffix(".js").unwrap_or(&stem);
                if stem.is_empty() || !is_valid_identifier(stem) {
                    return Err(Box::new(
                        CompilerDiagnostic::invalid_namespace_default_name(
                            import.provider.path.clone(),
                            import.location.clone(),
                        ),
                    ));
                }
                Ok(self.string_table.intern(stem))
            }
        }
    }

    /// Build a namespace record from a source file's visible declarations.
    fn build_source_namespace_record(
        &self,
        file_path: &InternedPath,
        location: &SourceLocation,
    ) -> NamespaceImportResult<NamespaceRecord> {
        let mut value_members = FxHashMap::default();
        let mut type_members = FxHashMap::default();

        let symbol_paths = self
            .module_symbols
            .declared_paths_by_file
            .get(file_path)
            .cloned()
            .unwrap_or_default();

        for symbol_path in symbol_paths {
            // Skip compiler-owned synthetic declarations (e.g. implicit start function).
            if !self
                .module_symbols
                .importable_source_symbol_paths
                .contains(&symbol_path)
            {
                continue;
            }

            // Receiver methods are not callable as namespace-record value fields.
            // They remain visible only through the receiver catalog.
            if self
                .module_symbols
                .receiver_method_paths
                .contains(&symbol_path)
                || self.module_symbols.trait_paths.contains(&symbol_path)
            {
                continue;
            }

            let Some(name) = symbol_path.name() else {
                continue;
            };

            let kind = self.classify_symbol_kind(&symbol_path);
            match kind {
                SymbolKind::Type => {
                    type_members.insert(
                        name,
                        NamespaceTypeMember::SourceDeclaration(SourceDeclarationTarget::Local(
                            symbol_path,
                        )),
                    );
                }
                SymbolKind::Value => {
                    value_members.insert(
                        name,
                        NamespaceValueMember::SourceDeclaration(SourceDeclarationTarget::Local(
                            symbol_path,
                        )),
                    );
                }
            }
        }

        self.check_duplicate_namespace_members(file_path, &value_members, &type_members, location)?;

        Ok(NamespaceRecord {
            value_members,
            type_members,
            child_namespaces: FxHashMap::default(),
            record_source: NamespaceRecordSource::SourceFile(file_path.clone()),
        })
    }

    /// Build a namespace record from a root file's explicit public export entries.
    ///
    /// WHAT: namespace imports of a root file expose the same public API as grouped imports,
    /// including import-only roots and grouped re-export aliases. Receiver methods remain
    /// receiver-call-only and are registered through `add_source_namespace_receiver_methods`.
    fn build_public_export_namespace_record(
        &self,
        root_file: &InternedPath,
        exported_entries: &FxHashSet<PublicExportEntry>,
        location: &SourceLocation,
    ) -> NamespaceImportResult<NamespaceRecord> {
        let mut value_members = FxHashMap::default();
        let mut type_members = FxHashMap::default();

        for entry in exported_entries {
            match &entry.target {
                PublicExportTarget::Source(symbol_path) => {
                    if self
                        .module_symbols
                        .receiver_method_paths
                        .contains(symbol_path)
                        || self.module_symbols.trait_paths.contains(symbol_path)
                    {
                        continue;
                    }

                    match self.classify_symbol_kind(symbol_path) {
                        SymbolKind::Type => {
                            type_members.insert(
                                entry.export_name,
                                NamespaceTypeMember::SourceDeclaration(
                                    SourceDeclarationTarget::Local(symbol_path.clone()),
                                ),
                            );
                        }
                        SymbolKind::Value => {
                            value_members.insert(
                                entry.export_name,
                                NamespaceValueMember::SourceDeclaration(
                                    SourceDeclarationTarget::Local(symbol_path.clone()),
                                ),
                            );
                        }
                    }
                }

                PublicExportTarget::External(symbol_id) => match symbol_id {
                    ExternalSymbolId::Function(_) | ExternalSymbolId::Constant(_) => {
                        value_members.insert(
                            entry.export_name,
                            NamespaceValueMember::ExternalSymbol(*symbol_id),
                        );
                    }
                    ExternalSymbolId::Type(_) => {
                        type_members.insert(
                            entry.export_name,
                            NamespaceTypeMember::ExternalSymbol(*symbol_id),
                        );
                    }
                },
            }
        }

        self.check_duplicate_namespace_members(root_file, &value_members, &type_members, location)?;

        Ok(NamespaceRecord {
            value_members,
            type_members,
            child_namespaces: FxHashMap::default(),
            record_source: NamespaceRecordSource::SourceFile(root_file.clone()),
        })
    }

    /// Build a namespace record from an external package's symbols.
    ///
    /// WHAT: turns the package's path-to-ID surface maps into a recursive namespace record.
    ///   One-component paths become direct value/type members; multi-component paths create
    ///   child namespace records down to the leaf.
    /// WHY: external packages are the first surface that needs nested namespace visibility
    ///   (`io.input.new`). Keeping the tree in the import environment lets later phases walk
    ///   dotted paths without rebuilding the surface.
    /// BOUNDARY: source and public export namespace records remain shallow; this recursive build is
    ///   only for external package surfaces.
    pub(super) fn build_external_namespace_record(
        &mut self,
        package_path: StringId,
        location: &SourceLocation,
    ) -> NamespaceImportResult<NamespaceRecord> {
        let mut record =
            NamespaceRecord::empty(NamespaceRecordSource::ExternalPackage(package_path));

        let package_path_str = self.string_table.resolve(package_path).to_owned();
        let Some(package) = self
            .external_package_registry
            .get_package(&package_path_str)
        else {
            return Ok(record);
        };

        let surface_path = InternedPath::from_components(vec![package_path]);
        let mut inserter = ExternalNamespaceRecordInserter {
            string_table: self.string_table,
            location,
        };

        for (path, function_id) in package.function_symbol_ids() {
            inserter.insert(
                &mut record,
                path,
                ExternalSymbolId::Function(*function_id),
                &surface_path,
            )?;
        }

        for (path, type_id) in package.type_symbol_ids() {
            inserter.insert(
                &mut record,
                path,
                ExternalSymbolId::Type(*type_id),
                &surface_path,
            )?;
        }

        for (path, constant_id) in package.constant_symbol_ids() {
            inserter.insert(
                &mut record,
                path,
                ExternalSymbolId::Constant(*constant_id),
                &surface_path,
            )?;
        }

        Ok(record)
    }

    /// Classify a symbol path as a type or value member for namespace records.
    fn classify_symbol_kind(&self, symbol_path: &InternedPath) -> SymbolKind {
        if self.module_symbols.type_alias_paths.contains(symbol_path) {
            return SymbolKind::Type;
        }
        if self.module_symbols.nominal_type_paths.contains(symbol_path) {
            return SymbolKind::Type;
        }
        if let Some(metadata) = self
            .module_symbols
            .generic_declarations_by_path
            .get(symbol_path)
        {
            match metadata.kind {
                GenericDeclarationKind::Struct | GenericDeclarationKind::Choice => {
                    return SymbolKind::Type;
                }
                GenericDeclarationKind::Function => return SymbolKind::Value,
            }
        }
        // Non-generic functions and constants are value members.
        SymbolKind::Value
    }

    /// Check for same-spelling value/type collisions within one namespace surface.
    ///
    /// WHAT: source and public export namespace records are shallow, so this only needs to detect
    /// a value member and a type member with the same name. External package records use
    /// `ExternalNamespaceRecordInserter`, which already rejects namespace/value/type slot
    /// collisions while building the tree.
    fn check_duplicate_namespace_members(
        &self,
        surface_path: &InternedPath,
        value_members: &FxHashMap<StringId, NamespaceValueMember>,
        type_members: &FxHashMap<StringId, NamespaceTypeMember>,
        location: &SourceLocation,
    ) -> NamespaceImportResult<()> {
        for name in value_members.keys() {
            if type_members.contains_key(name) {
                return Err(Box::new(
                    CompilerDiagnostic::duplicate_import_surface_member(
                        surface_path.clone(),
                        *name,
                        location.clone(),
                    ),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/namespace_imports_tests.rs"]
mod tests;
