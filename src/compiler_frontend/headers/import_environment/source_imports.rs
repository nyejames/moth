//! Source import registration and source receiver auto-imports.
//!
//! WHAT: registers imports that resolve to source file declarations (same-module, cross-module,
//! source-backed package) and auto-imports receiver methods when a nominal receiver type is imported.
//! WHY: source imports follow public-export visibility rules that differ from external package imports,
//! so they deserve their own focused registration path.
//! MUST NOT: register external package symbols or build namespace records.

use super::{
    FileVisibility, ImportEnvironmentBuilder, ImportEnvironmentError, SourceDeclarationTarget,
    SourceImportAccess, VisibleNameBinding, VisibleNameRegistry,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidReceiverDeclarationReason,
};
use crate::compiler_frontend::headers::import_environment::diagnostics;
use crate::compiler_frontend::headers::module_symbols::PublicExportTarget;
use crate::compiler_frontend::headers::parse_file_headers::FileImport;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Boxed diagnostic result for source import registration.
///
/// WHAT: gives source import registration one small error boundary.
/// WHY: local-name derivation is already boxed, so registration can propagate it directly
///      and adapt the plain visible-name registry once.
type SourceImportResult<T> = Result<T, ImportEnvironmentError>;

impl<'a> ImportEnvironmentBuilder<'a> {
    /// Auto-import receiver methods for a nominal type from the file where it is declared.
    ///
    /// WHAT: when a struct or choice type is imported, all receiver methods declared in the same
    ///       file whose receiver matches that type become visible through the receiver catalog.
    /// WHY: receiver methods travel with their receiver type on the same import surface.
    pub(super) fn auto_import_receiver_methods_for_type(
        &self,
        file_visibility: &mut FileVisibility,
        nominal_type_path: &InternedPath,
        target_file: &InternedPath,
        access: &SourceImportAccess,
    ) {
        let Some(receiver_type_name) = nominal_type_path.name() else {
            return;
        };

        // Walk receiver_method_paths directly and match by canonical source file.
        // WHY: header parsing records only the parsed receiver name, not semantic
        // receiver identity. Keeping the small scan here avoids a premature
        // header-level index while preserving the same-file nominal rule at the
        // import-preparation boundary.
        for path in &self.module_symbols.receiver_method_paths {
            if self.module_symbols.receiver_method_receiver_names.get(path)
                != Some(&receiver_type_name)
            {
                continue;
            }

            if self
                .module_symbols
                .canonical_source_by_symbol_path
                .get(path)
                .is_some_and(|source_file| source_file == target_file)
                && let Some(name) = path.name()
            {
                let is_visible =
                    self.receiver_type_visible_for_method_surface(nominal_type_path, access);

                if is_visible {
                    Self::add_visible_receiver_method(
                        file_visibility,
                        name,
                        path,
                        SourceLocation::default(),
                    );
                }
            }
        }
    }

    /// Whether the importer and the target of an import are in the same module or source-backed package.
    ///
    /// WHAT: same-module and same-package imports see all authored declarations by default;
    /// cross-module/cross-package imports must go through public surfaces.
    /// WHY: boundary membership, rather than declaration flags, is the gate for same-module
    /// visibility.
    pub(super) fn is_internal_import(
        &self,
        importer_file: &InternedPath,
        symbol_path: &InternedPath,
    ) -> bool {
        let Some(target_file) = self
            .module_symbols
            .canonical_source_by_symbol_path
            .get(symbol_path)
        else {
            return false;
        };

        self.source_files_share_import_boundary(importer_file, target_file)
    }

    /// Whether receiver methods can travel with a source type imported through this access path.
    ///
    /// WHAT: methods travel with the receiver type, not with an independent method export.
    /// Internal imports and direct source exports have already proven the type is visible. Public
    /// imports through public surfaces must expose the receiver type through that public surface.
    pub(super) fn receiver_type_visible_for_method_surface(
        &self,
        nominal_type_path: &InternedPath,
        access: &SourceImportAccess,
    ) -> bool {
        match access {
            SourceImportAccess::Internal | SourceImportAccess::DirectSourceExport => true,
            SourceImportAccess::PublicExport { exported_entries } => {
                exported_entries.iter().any(|entry| {
                    matches!(
                        &entry.target,
                        PublicExportTarget::Source {
                            path,
                            ..
                        } if path == nominal_type_path
                    )
                })
            }
        }
    }

    pub(super) fn register_source_import(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        symbol_path: &InternedPath,
        import: &FileImport,
        access: SourceImportAccess,
    ) -> SourceImportResult<()> {
        let local_name = self.derive_import_local_name(import)?;

        if let Some(symbol_name) = symbol_path.name() {
            self.emit_alias_case_warning_if_needed(import, symbol_name);
        }

        let is_type_alias = self.module_symbols.type_alias_paths.contains(symbol_path);
        let is_trait = self.module_symbols.trait_paths.contains(symbol_path);
        let is_receiver_method = self
            .module_symbols
            .receiver_method_paths
            .contains(symbol_path);

        if is_receiver_method {
            // Source-authored receiver methods are not independently importable or aliasable.
            // They travel with their receiver type's visibility.
            return Err(Box::new(CompilerDiagnostic::invalid_receiver_declaration(
                InvalidReceiverDeclarationReason::ReceiverMethodImportOrExportNotAllowed,
                import.location.clone(),
            ))
            .into());
        }

        // Check export requirement after the source receiver-method guard so explicit method
        // imports report the Phase 5 receiver policy instead of an incidental export failure.
        if matches!(&access, SourceImportAccess::DirectSourceExport) {
            let is_importable = self
                .module_symbols
                .importable_source_symbol_paths
                .contains(symbol_path);
            if !is_importable {
                return Err(Box::new(diagnostics::not_exported_by_source_file(
                    symbol_path,
                    import.location.clone(),
                ))
                .into());
            }
        }

        file_visibility
            .visible_declaration_paths
            .insert(symbol_path.clone());

        let binding = if is_type_alias {
            VisibleNameBinding::TypeAlias {
                canonical_path: symbol_path.clone(),
            }
        } else if is_trait {
            VisibleNameBinding::Trait {
                canonical_path: symbol_path.clone(),
            }
        } else {
            VisibleNameBinding::SourceImport {
                canonical_path: symbol_path.clone(),
            }
        };

        registry.register(local_name, binding, Some(import.location.clone()))?;

        if is_type_alias {
            file_visibility.visible_type_alias_names.insert(
                local_name,
                SourceDeclarationTarget::Local(symbol_path.clone()),
            );
        } else if is_trait {
            file_visibility.visible_trait_names.insert(
                local_name,
                SourceDeclarationTarget::Local(symbol_path.clone()),
            );
        } else {
            file_visibility.visible_source_names.insert(
                local_name,
                SourceDeclarationTarget::Local(symbol_path.clone()),
            );
        }

        // Importing a nominal receiver type auto-imports visible receiver methods
        // for that type from the same declaration surface.
        if self.module_symbols.nominal_type_paths.contains(symbol_path)
            && let Some(target_file) = self
                .module_symbols
                .canonical_source_by_symbol_path
                .get(symbol_path)
        {
            self.auto_import_receiver_methods_for_type(
                file_visibility,
                symbol_path,
                target_file,
                &access,
            );
        }

        Ok(())
    }
}
