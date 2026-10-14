//! Struct field type resolution and canonical folded-default validation.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, resolve_diagnostic_type_to_type_id_checked,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use super::resolve_named_signature_type;

pub(crate) enum StructFieldResolutionError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<CompilerError>),
}

impl From<CompilerDiagnostic> for StructFieldResolutionError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        StructFieldResolutionError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for StructFieldResolutionError {
    fn from(error: CompilerError) -> Self {
        StructFieldResolutionError::Infrastructure(Box::new(error))
    }
}

impl From<Box<CompilerError>> for StructFieldResolutionError {
    fn from(error: Box<CompilerError>) -> Self {
        StructFieldResolutionError::Infrastructure(error)
    }
}

// ----------------------------------
//  Struct field type resolution
// ----------------------------------

/// Resolve all declared struct field types against visible declarations.
pub(crate) fn resolve_struct_field_types(
    struct_path: &PathId,
    fields: &[Declaration],
    type_resolution_context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<Vec<Declaration>, StructFieldResolutionError> {
    let resolved_fields =
        resolve_struct_field_type_shells(fields, type_resolution_context, string_table)?;
    validate_struct_field_defaults(&resolved_fields, type_resolution_context)?;

    validate_resolved_field_parent_paths(struct_path, &resolved_fields, path_fork)?;

    Ok(resolved_fields)
}

/// Resolve struct field shell types for constant-time constructor parsing.
///
/// WHAT: turns parsed field type annotations into semantic `TypeId`s.
/// WHY: constructors need canonical field types independently of the receiving owner's folded
/// defaults. Default content lives in the phase constant store, never a field expression tree.
pub(crate) fn resolve_struct_constructor_shell_types(
    struct_path: &PathId,
    fields: &[Declaration],
    type_resolution_context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<Vec<Declaration>, StructFieldResolutionError> {
    let resolved_fields =
        resolve_struct_field_type_shells(fields, type_resolution_context, string_table)?;

    validate_resolved_field_parent_paths(struct_path, &resolved_fields, path_fork)?;

    Ok(resolved_fields)
}

fn validate_resolved_field_parent_paths(
    struct_path: &PathId,
    resolved_fields: &[Declaration],
    path_fork: &PathInternerFork,
) -> Result<(), StructFieldResolutionError> {
    if resolved_fields.is_empty() {
        return Ok(());
    }

    for field in resolved_fields {
        let Some(parent) = path_fork.parent(field.id) else {
            return Err(CompilerError::compiler_error(
                "Resolved struct field is missing its parent struct path.",
            )
            .into());
        };

        if parent != *struct_path {
            return Err(CompilerError::compiler_error(
                "Resolved struct field parent does not match the enclosing struct declaration.",
            )
            .into());
        }
    }

    Ok(())
}

fn resolve_struct_field_type_shells(
    fields: &[Declaration],
    type_resolution_context: &mut TypeResolutionContext<'_>,
    string_table: &mut StringTable,
) -> Result<Vec<Declaration>, StructFieldResolutionError> {
    // WHY: Struct fields must enter AST/HIR in fully resolved nominal form so later
    // phases do not carry unresolved `NamedType` placeholders.
    let mut resolved_fields = Vec::with_capacity(fields.len());
    for field in fields {
        let mut resolved_field = field.to_owned();

        resolved_field.diagnostic_type = resolve_named_signature_type(
            &field.diagnostic_type,
            field.binding_span,
            type_resolution_context,
            string_table,
        )?;

        let type_environment = &mut *type_resolution_context.type_environment;

        resolved_field.type_id = resolve_diagnostic_type_to_type_id_checked(
            &resolved_field.diagnostic_type,
            type_environment,
            resolved_field.binding_span,
        )?;

        resolved_fields.push(resolved_field);
    }

    Ok(resolved_fields)
}

/// The strict member pass has folded every authored struct default to canonical `Default` content,
/// so an unfolded `Initializer` here is a compiler invariant failure. Type resolution also validates
/// that each canonical handle belongs to the phase's constant owner.
fn validate_struct_field_defaults(
    resolved_fields: &[Declaration],
    context: &TypeResolutionContext<'_>,
) -> Result<(), StructFieldResolutionError> {
    for field in resolved_fields {
        if field.value.initializer().is_some() {
            return Err(CompilerError::compiler_error(
                "resolved struct field default was not folded by the strict struct pass",
            )
            .into());
        }
        let Some(default) = field.value.default() else {
            continue;
        };
        context.const_values.metadata(default).ok_or_else(|| {
            CompilerError::compiler_error(
                "resolved struct field default is outside its constant owner",
            )
        })?;
    }
    Ok(())
}
