//! Build-resolved source-provider inputs consumed by header interface binding.
//!
//! WHAT: associates one retained import shell in one consumer source file with the immutable
//! public semantic interface selected by Stage 0.
//! WHY: the compiler binds names and semantic facts, while the build system owns graph and
//! namespace resolution. This narrow borrowed input keeps build-local `ModuleId` values out of
//! the compiler and prevents header binding from probing the filesystem or opening provider
//! syntax.

use super::model::PublicSemanticInterface;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::parse_file_headers::FileImport;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(crate) struct SourceProviderImport<'a> {
    pub(crate) importer_source: Vec<String>,
    pub(crate) imported_path: Vec<String>,
    pub(crate) from_grouped: bool,
    pub(crate) interface: &'a PublicSemanticInterface,
}

#[derive(Default)]
pub(crate) struct SourceProviderImportSet<'a> {
    imports: Vec<SourceProviderImport<'a>>,
}

impl<'a> SourceProviderImportSet<'a> {
    pub(crate) fn new(imports: Vec<SourceProviderImport<'a>>) -> Self {
        Self { imports }
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

    pub(crate) fn resolve(
        &self,
        importer_source: &InternedPath,
        import: &FileImport,
        string_table: &StringTable,
    ) -> Option<&'a PublicSemanticInterface> {
        self.imports
            .iter()
            .find(|binding| {
                path_matches_owned_components(
                    importer_source,
                    &binding.importer_source,
                    string_table,
                ) && path_matches_owned_components(
                    &import.provider.path,
                    &binding.imported_path,
                    string_table,
                ) && binding.from_grouped == import.from_grouped
            })
            .map(|binding| binding.interface)
    }

    /// Resolve the completed provider selected for one grouped public re-export target.
    pub(crate) fn resolve_reexport(
        &self,
        exporting_source: &InternedPath,
        target_path: &InternedPath,
        string_table: &StringTable,
    ) -> Option<&'a PublicSemanticInterface> {
        self.imports
            .iter()
            .find(|binding| {
                binding.from_grouped
                    && path_matches_owned_components(
                        exporting_source,
                        &binding.importer_source,
                        string_table,
                    )
                    && path_matches_owned_components(
                        target_path,
                        &binding.imported_path,
                        string_table,
                    )
            })
            .map(|binding| binding.interface)
    }

    pub(super) fn interfaces(&self) -> impl Iterator<Item = &'a PublicSemanticInterface> + '_ {
        self.imports.iter().map(|binding| binding.interface)
    }
}

fn path_matches_owned_components(
    path: &InternedPath,
    expected: &[String],
    string_table: &StringTable,
) -> bool {
    path.as_components().len() == expected.len()
        && path
            .as_components()
            .iter()
            .zip(expected)
            .all(|(component, expected)| string_table.resolve(*component) == expected)
}
