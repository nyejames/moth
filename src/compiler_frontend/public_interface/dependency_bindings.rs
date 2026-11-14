//! Build-resolved source-provider inputs consumed by header interface binding.
//!
//! WHAT: associates one retained dependency shell or one fixed template-prelude package with the
//! immutable public semantic interface selected by Stage 0 through one dense provider set. The
//! set assigns a build-local [`ProviderInterfaceId`] per unique completed interface and validates
//! duplicate shells, duplicate template prelude packages and equal-origin interface agreement
//! while it builds.
//! WHY: the compiler binds names and semantic facts, while the build system owns graph and
//! namespace resolution. This narrow borrowed input keeps build-local `ModuleId` values out of
//! the compiler, prevents header binding from probing the filesystem or comparing path
//! components to rediscover a provider, and gives re-export caches and shell bindings one exact
//! provider identity instead of module origins or raw pointer identity.

use super::interface_view::ProviderBindingView;
use super::model::PublicSemanticInterface;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity;
use crate::compiler_frontend::symbols::identity::DependencyShellId;

use rustc_hash::FxHashMap;

/// Dense build-local identity of one provider interface inside a module binding operation.
///
/// WHAT: an operation-local handle into [`SourceProviderDependencySet`]. It never enters persistent
/// or public semantic identity; the stable module origin remains the semantic identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ProviderInterfaceId(usize);

impl ProviderInterfaceId {
    #[cfg(test)]
    pub(crate) fn new(index: usize) -> Self {
        Self(index)
    }
}

/// One authored dependency clause resolved to its operation-local provider interface.
///
/// WHAT: carries the exact clause shell together with the dense provider identity selected for
///       that shell. Direct selections and namespace dependencies both consume this resolved phase.
/// WHY: a provider interface is not the dependency clause itself; keeping both identities in one
///      value prevents later binding code from treating a provider row as a selected-name identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedDependencyClause {
    pub(crate) shell: DependencyShellId,
    pub(crate) provider: ProviderInterfaceId,
}

/// Explicit provider-dependency state for one binding.
///
/// WHAT: an authored dependency carries its retained clause-shell identity, while each
///       compiler-standard template prelude dependency names the official package it supplies.
/// WHY: optional shells, optional prefixes and boolean flags form invalid combinations; the
///      two-arm enum makes every provider input state representable and complete. The template
///      prelude package set is fixed by compiler policy, not selected by the active builder.
#[derive(Debug)]
pub(crate) enum ProviderDependencyKind {
    Authored { shell: DependencyShellId },
    TemplatePrelude { package_prefix: &'static str },
}

/// One official Core source package supplying the fixed `.mtf` constant prelude.
///
/// WHAT: pairs the fixed package prefix with the dense provider identity of its completed
///       interface.
/// WHY: header binding needs the prefix to select the package's header-built export lane and to
///      keep its synthetic prelude paths distinct from the other provider's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TemplatePreludeProvider {
    pub(crate) package_prefix: &'static str,
    pub(crate) provider: ProviderInterfaceId,
}

#[derive(Debug)]
pub(crate) struct SourceProviderDependency<'a> {
    pub(crate) kind: ProviderDependencyKind,
    pub(crate) interface: &'a PublicSemanticInterface,
}

/// One dense provider interface set per module binding operation.
///
/// WHAT: maps `ProviderInterfaceId` to the borrowed completed interface, retained shells and the
/// fixed template prelude packages to provider IDs, and validates equal-origin agreement once per
/// unique origin while it is constructed.
/// WHY: header binding and re-export caches query provider facts by dense identity, so the
/// module never re-runs publication validation or compares origins per shell.
#[derive(Debug, Default)]
pub(crate) struct SourceProviderDependencySet<'a> {
    interfaces: Vec<&'a PublicSemanticInterface>,
    binding_views: Vec<ProviderBindingView<'a>>,
    by_shell: FxHashMap<DependencyShellId, ProviderInterfaceId>,
    template_prelude: Vec<TemplatePreludeProvider>,
    by_origin: FxHashMap<StableModuleOriginIdentity, ProviderInterfaceId>,
}

impl<'a> SourceProviderDependencySet<'a> {
    pub(crate) fn new(
        dependencies: Vec<SourceProviderDependency<'a>>,
    ) -> Result<Self, InfrastructureFailure> {
        let mut set = Self {
            interfaces: Vec::with_capacity(dependencies.len()),
            binding_views: Vec::with_capacity(dependencies.len()),
            by_shell: FxHashMap::default(),
            template_prelude: Vec::new(),
            by_origin: FxHashMap::default(),
        };

        for dependency in &dependencies {
            match dependency.kind {
                ProviderDependencyKind::Authored { shell } => {
                    if set.by_shell.contains_key(&shell) {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::AstConstruction,
                            format!(
                                "source provider input set resolved dependency shell {:?} more than once",
                                shell
                            ),
                        ));
                    }
                    let provider_id = set.register(dependency.interface)?;
                    set.by_shell.insert(shell, provider_id);
                }
                ProviderDependencyKind::TemplatePrelude { package_prefix } => {
                    set.register_template_prelude(package_prefix, dependency.interface)?;
                }
            }
        }

        Ok(set)
    }

    /// Register one official package as a template prelude provider.
    ///
    /// WHAT: rejects a repeated package prefix and two prefixes that resolve to one completed
    ///       interface.
    /// WHY: the prelude merges exported constants across the registered packages, so a repeated
    ///      prefix or a shared interface would compare one package's exports against themselves
    ///      and hide the real merge state from the consumer.
    fn register_template_prelude(
        &mut self,
        package_prefix: &'static str,
        interface: &'a PublicSemanticInterface,
    ) -> Result<(), InfrastructureFailure> {
        if self
            .template_prelude
            .iter()
            .any(|provider| provider.package_prefix == package_prefix)
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "source provider input set registered the template prelude @{package_prefix} more than once"
                ),
            ));
        }
        let provider_id = self.register(interface)?;
        if let Some(existing) = self
            .template_prelude
            .iter()
            .find(|provider| provider.provider == provider_id)
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "template prelude packages @{} and @{package_prefix} resolved to the same provider interface",
                    existing.package_prefix
                ),
            ));
        }
        self.template_prelude.push(TemplatePreludeProvider {
            package_prefix,
            provider: provider_id,
        });
        Ok(())
    }

    /// Register one completed interface, collapsing exact repeats and rejecting equal-origin
    /// disagreement deterministically in either input order.
    fn register(
        &mut self,
        interface: &'a PublicSemanticInterface,
    ) -> Result<ProviderInterfaceId, InfrastructureFailure> {
        if let Some(existing_id) = self.by_origin.get(&interface.module_origin).copied() {
            let existing = self.interfaces[existing_id.0];
            // The same borrowed completed interface is registered once per authored shell.
            // Pointer equality is the exact-reference fast path: repeated shells for one
            // provider must not rebuild or structurally compare the full interface.
            if std::ptr::eq(existing, interface) {
                return Ok(existing_id);
            }
            if existing != interface {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!(
                        "provider interface for module origin {:?} disagrees with an equal-origin provider interface",
                        interface.module_origin
                    ),
                ));
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

    pub(crate) fn is_empty(&self) -> bool {
        self.interfaces.is_empty()
    }

    /// Iterate every unique provider interface in registration order.
    #[cfg(test)]
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

    /// Admit immutable provider interfaces only when every binding target resolves locally.
    ///
    /// This runs once per provider ID before header binding so malformed successful provider
    /// state remains an internal compiler failure rather than degrading into a source dependency
    /// diagnostic.
    pub(crate) fn validate_binding_targets(
        &self,
        external_registry: &ExternalPackageRegistry,
    ) -> Result<(), InfrastructureFailure> {
        for interface in &self.interfaces {
            interface.validate_binding_targets(external_registry)?;
        }

        Ok(())
    }

    /// Resolve one retained dependency shell to its dense provider interface.
    pub(crate) fn resolve_clause(
        &self,
        shell: DependencyShellId,
    ) -> Option<ResolvedDependencyClause> {
        self.by_shell
            .get(&shell)
            .copied()
            .map(|provider| ResolvedDependencyClause { shell, provider })
    }

    /// The provider ID selected for one public re-export dependency shell.
    pub(crate) fn resolve_reexport(&self, shell: DependencyShellId) -> Option<ProviderInterfaceId> {
        self.resolve_clause(shell).map(|resolved| resolved.provider)
    }

    /// Resolve one dense provider identity to its borrowed completed interface.
    ///
    /// Internal dense lookup failures are `InfrastructureFailure` invariant failures, never silently
    /// erased into absence.
    pub(crate) fn interface(
        &self,
        provider_id: ProviderInterfaceId,
    ) -> Result<&'a PublicSemanticInterface, InfrastructureFailure> {
        self.interfaces.get(provider_id.0).copied().ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "provider interface table has no interface for provider id {}",
                    provider_id.0
                ),
            )
        })
    }

    /// The one operation-scoped binding view for a provider ID.
    ///
    /// Built once when the provider registers; every shell referencing the provider reuses it.
    pub(crate) fn binding_view(
        &self,
        provider_id: ProviderInterfaceId,
    ) -> Result<&ProviderBindingView<'a>, InfrastructureFailure> {
        self.binding_views.get(provider_id.0).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "provider interface table has no binding view for provider id {}",
                    provider_id.0
                ),
            )
        })
    }

    /// The fixed compiler-standard template prelude providers, in canonical input order.
    ///
    /// WHAT: exposes the prelude package prefixes with their provider IDs so header binding can
    ///       merge their public compile-time constants into the flat `.mtf` prelude.
    /// WHY: the packages are chosen by compiler policy, not by a builder capability list or by an
    ///       explicit dependency clause in the template source.
    pub(crate) fn template_prelude_providers(&self) -> &[TemplatePreludeProvider] {
        &self.template_prelude
    }
}
