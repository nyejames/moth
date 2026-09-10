//! External import registration.
//!
//! WHAT: registers imports that resolve to external package symbols.
//! WHY: external package imports use stable IDs rather than source paths, while receiver method
//! syntax remains source-owned and compiler-owned rather than external-package metadata.
//! MUST NOT: register source declarations or build source namespace records.

use super::BindingEnvironmentError;
use super::{BindingEnvironmentBuilder, FileVisibility, VisibleNameBinding, VisibleNameRegistry};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::dependency_clause_syntax::DependencyAlias;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::identifier_policy::ensure_not_keyword_shadow_identifier;
use crate::compiler_frontend::symbols::string_interning::StringId;
/// Result for external import registration.
///
/// Diagnosed failures remain plain `CompilerDiagnostic` values and
/// infrastructure failures remain typed in `BindingEnvironmentError`.
type ExternalImportResult<T> = Result<T, BindingEnvironmentError>;

/// Registration facts for one external symbol binding.
///
/// WHAT: bundles the source clause and selected-name facts needed to publish one stable external
///       symbol ID into file visibility.
/// WHY: external registration owns this data shape, keeping callers from growing repetitive
///      parameter lists as diagnostics and alias metadata travel together.
pub(super) struct ExternalImportInput<'a> {
    pub(super) symbol_name: StringId,
    pub(super) local_name: StringId,
    pub(super) source_span: Option<SourceSpan>,
    pub(super) local_alias: Option<&'a DependencyAlias>,
    pub(super) symbol_id: ExternalSymbolId,
}

impl<'a> BindingEnvironmentBuilder<'a> {
    pub(super) fn register_external_import(
        &mut self,
        file_visibility: &mut FileVisibility,
        registry: &mut VisibleNameRegistry,
        input: ExternalImportInput<'_>,
    ) -> ExternalImportResult<()> {
        let ExternalImportInput {
            symbol_name,
            local_name,
            source_span,
            local_alias,
            symbol_id,
        } = input;

        let local_name_span = local_alias.map_or(source_span, |alias| Some(alias.span));
        ensure_not_keyword_shadow_identifier(local_name, local_name_span, self.string_table)?;

        self.emit_alias_case_warning_if_needed(local_alias, symbol_name);

        registry.register(
            local_name,
            VisibleNameBinding::ExternalImport { symbol_id },
            local_name_span,
        )?;

        file_visibility
            .visible_external_symbols
            .insert(local_name, symbol_id);

        if let Some(span) = local_name_span {
            file_visibility
                .visible_external_symbol_spans
                .insert(local_name, span);
        }

        Ok(())
    }
}
