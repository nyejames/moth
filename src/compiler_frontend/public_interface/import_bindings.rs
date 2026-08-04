//! Build-resolved source-provider inputs consumed by header interface binding.
//!
//! WHAT: associates one retained import shell or implicit template scope with the immutable
//! public semantic interface selected by Stage 0 through one dense provider table. The table
//! assigns a build-local [`ProviderInterfaceId`] per unique completed interface and validates
//! duplicate shells, duplicate implicit scopes and equal-origin interface agreement while it
//! builds.
//! WHY: the compiler binds names and semantic facts, while the build system owns graph and
//! namespace resolution. This narrow borrowed input keeps build-local `ModuleId` values out of
//! the compiler, prevents header binding from probing the filesystem or comparing path
//! components to rediscover a provider, and gives re-export caches and shell bindings one exact
//! provider identity instead of module origins or raw pointer identity.

use super::interface_view::ProviderBindingView;
use super::model::PublicSemanticInterface;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity;
use crate::compiler_frontend::symbols::identity::ImportShellId;

use rustc_hash::FxHashMap;

/// Dense build-local identity of one provider interface inside a module binding operation.
///
/// WHAT: an operation-local handle into [`ProviderInterfaceTable`]. It never enters persistent
/// or public semantic identity; the stable module origin remains the semantic identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ProviderInterfaceId(usize);

impl ProviderInterfaceId {
    #[cfg(test)]
    pub(crate) fn new(index: usize) -> Self {
        Self(index)
    }
}

/// Explicit provider-import state for one binding.
///
/// WHAT: an authored import always carries its retained shell identity, while an implicit
/// template scope provider is selected by the active builder and carries its package prefix.
/// WHY: optional shells, optional prefixes and boolean flags form invalid combinations; the
/// two-arm enum makes every provider input state representable and complete.
#[derive(Debug)]
pub(crate) enum ProviderImportKind<'a> {
    Authored { shell_id: ImportShellId },
    ImplicitTemplate { package_prefix: &'a str },
}

#[derive(Debug)]
pub(crate) struct SourceProviderImport<'a> {
    pub(crate) kind: ProviderImportKind<'a>,
    pub(crate) interface: &'a PublicSemanticInterface,
}

/// One dense provider interface table per module binding operation.
///
/// WHAT: maps `ProviderInterfaceId` to the borrowed completed interface, retained shells and
/// implicit template scopes to provider IDs, and validates equal-origin agreement once per
/// unique origin while it is constructed.
/// WHY: header binding and re-export caches query provider facts by dense identity, so the
/// module never re-runs publication validation or compares origins per shell.
#[derive(Debug, Default)]
pub(crate) struct ProviderInterfaceTable<'a> {
    interfaces: Vec<&'a PublicSemanticInterface>,
    binding_views: Vec<ProviderBindingView<'a>>,
    by_shell: FxHashMap<ImportShellId, ProviderInterfaceId>,
    implicit_by_prefix: FxHashMap<&'a str, ProviderInterfaceId>,
    implicit_providers: Vec<(&'a str, ProviderInterfaceId)>,
    by_origin: FxHashMap<StableModuleOriginIdentity, ProviderInterfaceId>,
}

impl<'a> ProviderInterfaceTable<'a> {
    fn build(imports: &[SourceProviderImport<'a>]) -> Result<Self, CompilerError> {
        let mut table = Self {
            interfaces: Vec::with_capacity(imports.len()),
            binding_views: Vec::with_capacity(imports.len()),
            by_shell: FxHashMap::default(),
            implicit_by_prefix: FxHashMap::default(),
            implicit_providers: Vec::new(),
            by_origin: FxHashMap::default(),
        };

        for import in imports {
            match import.kind {
                ProviderImportKind::Authored { shell_id } => {
                    if table.by_shell.contains_key(&shell_id) {
                        return Err(CompilerError::compiler_error(format!(
                            "source provider input set resolved import shell {:?} more than once",
                            shell_id
                        )));
                    }
                    let provider_id = table.register(import.interface)?;
                    table.by_shell.insert(shell_id, provider_id);
                }
                ProviderImportKind::ImplicitTemplate { package_prefix } => {
                    if table.implicit_by_prefix.contains_key(package_prefix) {
                        return Err(CompilerError::compiler_error(format!(
                            "source provider input set registered implicit template scope @{} more than once",
                            package_prefix
                        )));
                    }
                    let provider_id = table.register(import.interface)?;
                    table.implicit_by_prefix.insert(package_prefix, provider_id);
                    table.implicit_providers.push((package_prefix, provider_id));
                }
            }
        }

        Ok(table)
    }

    /// Register one completed interface, collapsing exact repeats and rejecting equal-origin
    /// disagreement deterministically in either input order.
    fn register(
        &mut self,
        interface: &'a PublicSemanticInterface,
    ) -> Result<ProviderInterfaceId, CompilerError> {
        if let Some(existing_id) = self.by_origin.get(&interface.module_origin).copied() {
            if self.interfaces[existing_id.0] != interface {
                return Err(CompilerError::compiler_error(format!(
                    "provider interface for module origin {:?} disagrees with an equal-origin provider interface",
                    interface.module_origin
                )));
            }
            return Ok(existing_id);
        }

        let provider_id = ProviderInterfaceId(self.interfaces.len());
        self.interfaces.push(interface);
        self.binding_views
            .push(ProviderBindingView::build(interface)?);
        self.by_origin
            .insert(interface.module_origin.clone(), provider_id);
        Ok(provider_id)
    }

    /// The provider ID selected for one retained import shell.
    pub(crate) fn resolve(&self, shell_id: ImportShellId) -> Option<ProviderInterfaceId> {
        self.by_shell.get(&shell_id).copied()
    }

    /// The provider ID selected for one grouped public re-export shell.
    pub(crate) fn resolve_reexport(&self, shell_id: ImportShellId) -> Option<ProviderInterfaceId> {
        self.resolve(shell_id)
    }

    /// Resolve one dense provider identity to its borrowed completed interface.
    ///
    /// Internal dense lookup failures are `CompilerError` invariant failures, never silently
    /// erased into absence.
    pub(crate) fn interface(
        &self,
        provider_id: ProviderInterfaceId,
    ) -> Result<&'a PublicSemanticInterface, CompilerError> {
        self.interfaces.get(provider_id.0).copied().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "provider interface table has no interface for provider id {}",
                provider_id.0
            ))
        })
    }

    /// The one operation-scoped binding view for a provider ID.
    ///
    /// Built once when the provider registers; every shell referencing the provider reuses it.
    pub(crate) fn binding_view(
        &self,
        provider_id: ProviderInterfaceId,
    ) -> Result<&ProviderBindingView<'a>, CompilerError> {
        self.binding_views.get(provider_id.0).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "provider interface table has no binding view for provider id {}",
                provider_id.0
            ))
        })
    }

    /// Iterate builder-selected implicit template scope providers in deterministic registration
    /// order.
    ///
    /// WHAT: exposes the provider prefix together with its provider ID so header binding can
    ///       register every capability-selected constant surface.
    /// WHY: implicit template scope is builder capability metadata, not a hard-coded `@html`
    ///       special case or a side effect of explicit imports.
    pub(crate) fn implicit_template_scope_providers(
        &self,
    ) -> impl Iterator<Item = (&'a str, ProviderInterfaceId)> + '_ {
        self.implicit_providers.iter().copied()
    }

    pub(crate) fn interfaces(&self) -> impl Iterator<Item = &'a PublicSemanticInterface> + '_ {
        self.interfaces.iter().copied()
    }

    /// Iterate every unique provider with its dense provider ID.
    pub(crate) fn providers(
        &self,
    ) -> impl Iterator<Item = (ProviderInterfaceId, &'a PublicSemanticInterface)> + '_ {
        self.interfaces
            .iter()
            .enumerate()
            .map(|(index, interface)| (ProviderInterfaceId(index), *interface))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.interfaces.is_empty()
    }
}

#[derive(Debug, Default)]
pub(crate) struct SourceProviderImportSet<'a> {
    table: ProviderInterfaceTable<'a>,
}

impl<'a> SourceProviderImportSet<'a> {
    pub(crate) fn new(imports: Vec<SourceProviderImport<'a>>) -> Result<Self, CompilerError> {
        Ok(Self {
            table: ProviderInterfaceTable::build(&imports)?,
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Iterate every unique provider interface in registration order.
    #[cfg(test)]
    pub(crate) fn interfaces(&self) -> impl Iterator<Item = &'a PublicSemanticInterface> + '_ {
        self.table.interfaces()
    }

    /// Iterate every unique provider with its dense provider ID.
    pub(crate) fn providers(
        &self,
    ) -> impl Iterator<Item = (ProviderInterfaceId, &'a PublicSemanticInterface)> + '_ {
        self.table.providers()
    }

    /// Admit immutable provider interfaces only when every binding target resolves locally.
    ///
    /// This runs once per provider ID before header binding so malformed successful provider
    /// state remains an internal compiler failure rather than degrading into a source import
    /// diagnostic.
    pub(crate) fn validate_binding_targets(
        &self,
        external_registry: &ExternalPackageRegistry,
    ) -> Result<(), CompilerError> {
        for interface in self.table.interfaces() {
            interface.validate_binding_targets(external_registry)?;
        }

        Ok(())
    }

    /// Resolve the provider ID selected for one retained import shell.
    pub(crate) fn resolve(&self, import_shell_id: ImportShellId) -> Option<ProviderInterfaceId> {
        self.table.resolve(import_shell_id)
    }

    /// Resolve the provider ID selected for one grouped public re-export shell.
    pub(crate) fn resolve_reexport(
        &self,
        import_shell_id: ImportShellId,
    ) -> Option<ProviderInterfaceId> {
        self.table.resolve_reexport(import_shell_id)
    }

    /// Resolve one provider ID to its borrowed completed interface.
    pub(crate) fn interface(
        &self,
        provider_id: ProviderInterfaceId,
    ) -> Result<&'a PublicSemanticInterface, CompilerError> {
        self.table.interface(provider_id)
    }

    /// The one operation-scoped binding view for a provider ID.
    pub(crate) fn binding_view(
        &self,
        provider_id: ProviderInterfaceId,
    ) -> Result<&ProviderBindingView<'a>, CompilerError> {
        self.table.binding_view(provider_id)
    }

    /// Iterate over source packages selected by the builder for `.mtf` implicit scope.
    pub(crate) fn implicit_template_scope_providers(
        &self,
    ) -> impl Iterator<Item = (&'a str, ProviderInterfaceId)> + '_ {
        self.table.implicit_template_scope_providers()
    }
}
