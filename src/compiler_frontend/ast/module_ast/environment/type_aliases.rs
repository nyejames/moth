//! Type alias target resolution.
//!
//! WHAT: resolves parsed type-alias targets against the canonical module `TypeEnvironment`.
//! WHY: type aliases are compile-time-only type metadata; their targets must be fully resolved
//! before function signatures and struct fields are resolved.
//!
//! ## Cycle handling
//!
//! Type alias cycles (e.g. `A as B` + `B as A`) are detected by dependency sorting, because
//! `create_header` collects named-type dependency edges from alias targets just like from struct
//! fields and constant type annotations. Self-reference (`A as A`) also creates a self-loop edge.

use crate::compiler_frontend::ast::module_ast::environment::builder::AstModuleEnvironmentBuilder;
use crate::compiler_frontend::ast::type_resolution::{
    ResolvedTypeAnnotation, resolve_diagnostic_type_to_type_id_opt, resolve_type,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidDeclarationReason};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::declaration_syntax::type_syntax::parsed_ref_to_data_type;
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::rc::Rc;

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    /// Resolve all type alias targets in sorted-header order.
    ///
    /// WHAT: iterates sorted headers, resolving each `TypeAlias` target against already-resolved
    /// aliases and visible declarations.
    /// WHY: dependency sorting guarantees that when we reach an alias, all its dependencies have
    /// already been processed.
    pub(in crate::compiler_frontend::ast) fn resolve_type_aliases(
        &mut self,
        sorted_headers: &[Header],
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for header in sorted_headers {
            let HeaderKind::TypeAlias { target } = &header.kind else {
                continue;
            };

            let visibility = self
                .binding_environment
                .visibility_for(&header.source_file)
                .map_err(|error| self.error_messages(error, string_table))?
                .clone();

            let resolved_target = {
                let mut type_resolution_context =
                    self.type_resolution_context_for(&visibility, None);

                // Alias declarations are resolved before nominal members and constants.
                // Store the parsed target together with its resolved diagnostic spelling and
                // canonical TypeId so alias metadata has a single owner.
                let data_type_target = parsed_ref_to_data_type(target);

                resolve_type(
                    &data_type_target,
                    &header.name_location,
                    &mut type_resolution_context,
                    string_table,
                )
            }
            .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;

            // Reject aliases to external opaque types for Alpha.
            // WHAT: external types are opaque and cannot be aliased by user code.
            // WHY: aliases to opaque types would let user code pretend it owns a nominal type
            //     that it cannot construct or field-access, leading to confusing semantics.
            if let DataType::External { type_id } = &resolved_target {
                let type_name = self
                    .context
                    .external_package_registry
                    .get_type_by_id(*type_id)
                    .map(|def| def.name.to_string())
                    .unwrap_or_else(|| "external".to_string());

                return Err(self.diagnostic_messages(
                    CompilerDiagnostic::invalid_declaration(
                        InvalidDeclarationReason::ExternalTypeAlias {
                            type_name: string_table.intern(&type_name),
                        },
                        header.tokens.src_path.name(),
                        header.name_location.clone(),
                    ),
                    string_table,
                ));
            }

            let type_id = resolve_diagnostic_type_to_type_id_opt(
                &resolved_target,
                &mut self.type_environment,
            );

            Rc::make_mut(&mut self.resolved_type_aliases_by_path).insert(
                header.tokens.src_path.to_owned(),
                ResolvedTypeAnnotation {
                    source_ref: target.clone(),
                    diagnostic_type: resolved_target,
                    type_id,
                },
            );
        }

        Ok(())
    }
}
