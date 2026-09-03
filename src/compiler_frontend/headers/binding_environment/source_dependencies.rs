//! Source dependency registration and source receiver method binding.
//!
//! WHAT: registers dependency clauses that bind source file declarations (same-module, cross-module,
//! source-backed package) and binds receiver methods with a dependency-bound nominal receiver type.
//! WHY: source dependencies follow public-export visibility rules that differ from external package dependencies,
//! so they deserve their own focused registration path.
//! MUST NOT: register external package symbols or build namespace records.

use super::{
    BindingEnvironmentBuilder, BindingEnvironmentError, FileVisibility, SourceDeclarationTarget,
    SourceDependencyAccess, VisibleNameBinding, VisibleNameRegistry,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidReceiverDeclarationReason,
};
use crate::compiler_frontend::headers::binding_environment::diagnostics;
use crate::compiler_frontend::headers::dependency_clause_syntax::DependencyAlias;
use crate::compiler_frontend::symbols::identifier_policy::ensure_not_keyword_shadow_identifier;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Boxed diagnostic result for source dependency registration.
///
/// WHAT: gives source dependency registration one small error boundary.
/// WHY: local-name derivation is already boxed, so registration can propagate it directly
///      and adapt the plain visible-name registry once.
type SourceDependencyResult<T> = Result<T, BindingEnvironmentError>;

/// Registration facts for one source declaration binding.
///
/// WHAT: bundles the selected path, local name and visibility requirement for one source dependency.
/// WHY: source registration owns the shape shared by direct, public-export and internal source
///      dependencies, leaving callers focused on resolution rather than argument plumbing.
pub(super) struct SourceDependencyInput<'a> {
    pub(super) symbol_path: &'a InternedPath,
    pub(super) local_name: StringId,
    pub(super) source_location: &'a SourceLocation,
    pub(super) local_alias: Option<&'a DependencyAlias>,
    pub(super) access: SourceDependencyAccess,
}

impl<'a> BindingEnvironmentBuilder<'a> {
    /// Bind receiver methods for a nominal type from the file where it is declared.
    ///
    /// WHAT: when a struct or choice type is dependency-bound, all receiver methods declared in the same
    ///       file whose receiver matches that type become visible through the receiver catalog.
    /// WHY: receiver methods travel with their receiver type on the same dependency surface.
    pub(super) fn bind_receiver_methods_for_type(
        &self,
        file_visibility: &mut FileVisibility,
        nominal_type_path: &InternedPath,
        target_file: &InternedPath,
        access: &SourceDependencyAccess,
    ) {
        let Some(receiver_type_name) = nominal_type_path.name() else {
            return;
        };

        // Walk receiver_method_paths directly and match by canonical source file.
        // WHY: header parsing records only the parsed receiver name, not semantic
        // receiver identity. Keeping the small scan here avoids a premature
        // header-level index while preserving the same-file nominal rule at the
        // dependency-preparation boundary.
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

    /// Whether the consumer and the target of a dependency are in the same module or source-backed package.
    ///
    /// WHAT: same-module and same-package dependencies see all authored declarations by default;
    /// cross-module/cross-package dependencies must go through public surfaces.
    /// WHY: boundary membership, rather than declaration flags, is the gate for same-module
    /// visibility.
    pub(super) fn is_internal_dependency(
        &self,
        consumer_file: &InternedPath,
        symbol_path: &InternedPath,
    ) -> bool {
        let Some(target_file) = self
            .module_symbols
            .canonical_source_by_symbol_path
            .get(symbol_path)
        else {
            return false;
        };

        self.source_files_share_dependency_boundary(consumer_file, target_file)
    }

    /// Whether receiver methods can travel with a source type bound through this access path.
    ///
    /// WHAT: methods travel with the receiver type, not with an independent method export.
    /// Internal dependencies and direct source exports have already proven the type is visible. Public
    /// dependencies through public surfaces must expose the receiver type through that public surface.
    pub(super) fn receiver_type_visible_for_method_surface(
        &self,
        nominal_type_path: &InternedPath,
        access: &SourceDependencyAccess,
    ) -> bool {
        match access {
            SourceDependencyAccess::Internal | SourceDependencyAccess::DirectSourceExport => true,
            SourceDependencyAccess::PublicExport { exported_entries } => {
                exported_entries.iter().any(|entry| {
                    entry
                        .target
                        .source_path()
                        .is_some_and(|path| path == nominal_type_path)
                })
            }
        }
    }

    pub(super) fn register_source_dependency(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        input: SourceDependencyInput<'_>,
    ) -> SourceDependencyResult<()> {
        let SourceDependencyInput {
            symbol_path,
            local_name,
            source_location,
            local_alias,
            access,
        } = input;

        let local_name_location = local_alias.map_or(source_location, |alias| &alias.location);
        ensure_not_keyword_shadow_identifier(
            local_name,
            local_name_location.clone(),
            self.string_table,
        )?;

        if let Some(symbol_name) = symbol_path.name() {
            self.emit_alias_case_warning_if_needed(local_alias, symbol_name);
        }

        let is_type_alias = self.module_symbols.type_alias_paths.contains(symbol_path);
        let is_trait = self.module_symbols.trait_paths.contains(symbol_path);
        let is_receiver_method = self
            .module_symbols
            .receiver_method_paths
            .contains(symbol_path);

        if is_receiver_method {
            // Source-authored receiver methods are not independently bindable or aliasable.
            // They travel with their receiver type's visibility.
            return Err(Box::new(CompilerDiagnostic::invalid_receiver_declaration(
                InvalidReceiverDeclarationReason::ReceiverMethodImportOrExportNotAllowed,
                source_location.clone(),
            ))
            .into());
        }

        // Check export requirement after the source receiver-method guard so explicit method
        // dependencies report the Phase 5 receiver policy instead of an incidental export failure.
        if matches!(&access, SourceDependencyAccess::DirectSourceExport) {
            let is_dependency_bindable = self
                .module_symbols
                .dependency_bindable_source_symbol_paths
                .contains(symbol_path);
            if !is_dependency_bindable {
                return Err(Box::new(diagnostics::not_exported_by_source_file(
                    symbol_path,
                    source_location.clone(),
                ))
                .into());
            }
        }

        file_visibility
            .visible_declaration_paths_mut()
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
            VisibleNameBinding::SourceDependency {
                canonical_path: symbol_path.clone(),
            }
        };

        registry.register(local_name, binding, Some(local_name_location.clone()))?;

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

        // Binding a nominal receiver type also binds visible receiver methods
        // for that type from the same declaration surface.
        if self.module_symbols.nominal_type_paths.contains(symbol_path)
            && let Some(target_file) = self
                .module_symbols
                .canonical_source_by_symbol_path
                .get(symbol_path)
        {
            self.bind_receiver_methods_for_type(file_visibility, symbol_path, target_file, &access);
        }

        Ok(())
    }
}
