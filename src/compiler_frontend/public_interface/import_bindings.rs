//! Build-resolved source-provider inputs consumed by header interface binding.
//!
//! WHAT: associates one retained import shell in one consumer source file with the immutable
//! public semantic interface selected by Stage 0, keyed directly by the shell identity.
//! WHY: the compiler binds names and semantic facts, while the build system owns graph and
//! namespace resolution. This narrow borrowed input keeps build-local `ModuleId` values out of
//! the compiler and prevents header binding from probing the filesystem or comparing path
//! components to rediscover a provider.

use super::model::PublicSemanticInterface;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::identity::ImportShellId;

use rustc_hash::FxHashMap;

pub(crate) struct SourceProviderImport<'a> {
    /// Retained shell this binding belongs to. `None` marks a builder-selected implicit template
    /// scope provider, which has no authored import shell.
    pub(crate) import_shell_id: Option<ImportShellId>,
    /// Import prefix of the completed source package, used only for implicit template scope.
    pub(crate) import_prefix: Option<&'a str>,
    /// Marks a provider selected by the active builder for `.mtf` implicit scope.
    ///
    /// Explicit source imports remain ordinary provider bindings. The build system sets this
    /// flag only for capability-selected providers so header binding does not infer implicit
    /// visibility from a package name or from another file's explicit import.
    pub(crate) implicit_template_scope: bool,
    pub(crate) interface: &'a PublicSemanticInterface,
}

#[derive(Default)]
pub(crate) struct SourceProviderImportSet<'a> {
    imports: Vec<SourceProviderImport<'a>>,
    /// Transient direct lookup by retained shell identity.
    ///
    /// The map is built once per module when the set is constructed and dropped after the
    /// module's bound inputs are built; it is never a durable semantic artefact.
    by_shell_id: FxHashMap<ImportShellId, usize>,
}

impl<'a> SourceProviderImportSet<'a> {
    pub(crate) fn new(imports: Vec<SourceProviderImport<'a>>) -> Self {
        let mut by_shell_id = FxHashMap::default();
        for (index, binding) in imports.iter().enumerate() {
            if let Some(import_shell_id) = binding.import_shell_id {
                by_shell_id.insert(import_shell_id, index);
            }
        }

        Self {
            imports,
            by_shell_id,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.imports.is_empty()
    }

    /// Admit immutable provider interfaces only when every binding target resolves locally.
    ///
    /// This runs before header binding so malformed successful provider state remains an internal
    /// compiler failure rather than degrading into a source import diagnostic.
    pub(crate) fn validate_binding_targets(
        &self,
        external_registry: &ExternalPackageRegistry,
    ) -> Result<(), CompilerError> {
        for interface in self.interfaces() {
            interface.validate_binding_targets(external_registry)?;
        }

        Ok(())
    }

    /// Resolve the completed provider selected for one retained import shell.
    pub(crate) fn resolve(
        &self,
        import_shell_id: ImportShellId,
    ) -> Option<&'a PublicSemanticInterface> {
        self.by_shell_id
            .get(&import_shell_id)
            .map(|index| self.imports[*index].interface)
    }

    /// Resolve the completed provider selected for one grouped public re-export shell.
    pub(crate) fn resolve_reexport(
        &self,
        import_shell_id: ImportShellId,
    ) -> Option<&'a PublicSemanticInterface> {
        self.resolve(import_shell_id)
    }

    pub(super) fn interfaces(&self) -> impl Iterator<Item = &'a PublicSemanticInterface> + '_ {
        self.imports.iter().map(|binding| binding.interface)
    }

    /// Iterate over source packages selected by the builder for `.mtf` implicit scope.
    ///
    /// WHAT: exposes the provider prefix together with its completed interface so header binding
    ///       can register every capability-selected constant surface.
    /// WHY: implicit template scope is builder capability metadata, not a hard-coded `@html`
    ///       special case or a side effect of explicit imports.
    pub(crate) fn implicit_template_scope_interfaces(
        &self,
    ) -> impl Iterator<Item = (&str, &'a PublicSemanticInterface)> + '_ {
        self.imports.iter().filter_map(|binding| {
            if !binding.implicit_template_scope {
                return None;
            }

            binding
                .import_prefix
                .map(|prefix| (prefix, binding.interface))
        })
    }
}
