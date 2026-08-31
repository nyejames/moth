//! AST-owned semantics for value-position file paths.
//!
//! Stage 0 resolves physical targets and publishes one reader-facing view for ordinary modules
//! and frozen persistent-generic bodies. This module interprets that view without touching the
//! filesystem: ordinary content rows use the content file's logical path to reference the
//! synthetic `content` declaration and reuse its interned `StringId`; frozen content rows carry a
//! captured `OwnedFoldedString` and lower it through the shared owned-value inverse projection.
//! In either lane, resource and site-root rows remain structural strings.

use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind;
use crate::compiler_frontend::ast::field_access::reference_expression_from_declaration;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::materialize_owned_folded_string;
use crate::compiler_frontend::ast::module_ast::scope_context::{
    ScopeContext, Stage0ResolvedFileReferenceOutcome,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidExpressionReason};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::headers::synthetic_content_header::content_constant_path;
use crate::compiler_frontend::paths::file_references::PreparedFileReferenceClass;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};
use crate::compiler_frontend::value_mode::ValueMode;

/// Resolve one `TokenKind::Path` through the value-position Stage 0 view.
///
/// Resolution is driven by the immutable view supplied by semantic orchestration. It covers
/// ordinary module rows and frozen persistent-generic rows, so this function never reopens source
/// tables or calls a filesystem path resolver.
pub(crate) fn resolve_file_value(
    path_syntax: PathSyntaxId,
    token_stream: &FileTokens,
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let location = token_stream.current_location();
    let row = token_stream
        .path_syntax
        .try_path_for_token(path_syntax, &location)?;
    let services = context
        .shared
        .file_value_resolution
        .as_ref()
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "value-position file path reached AST without Stage 0 resolution services",
            )
        })?;
    let stage0_resolution_facts = services.stage0_resolution_facts.as_ref().ok_or_else(|| {
        CompilerError::compiler_error(
            "value-position file path reached AST without Stage 0 resolution services",
        )
    })?;
    let resolved = stage0_resolution_facts
        .lookup(context.shared.declaring_file_id, path_syntax)?
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "value-position file path had no matching Stage 0 resolved-reference row",
            )
        })?;

    match resolved.class {
        PreparedFileReferenceClass::Extensionless => Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::ExtensionlessFileValue,
            row.location.clone(),
        )
        .into()),
        PreparedFileReferenceClass::SourceKindNoFileValue => {
            Err(CompilerDiagnostic::invalid_expression(
                InvalidExpressionReason::MothFileHasNoValue,
                row.location.clone(),
            )
            .into())
        }
        PreparedFileReferenceClass::SiteRoot => match resolved.outcome {
            Stage0ResolvedFileReferenceOutcome::NoPhysicalTarget => structural_string(
                vec![ConstStringPiece::SiteRoot],
                row.location.clone(),
                value_mode,
            ),
            Stage0ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => {
                Err(diagnostic.clone().into())
            }
            _ => Err(CompilerError::compiler_error(
                "site-root file reference unexpectedly resolved to a physical target",
            )
            .into()),
        },
        PreparedFileReferenceClass::ContentSource => match resolved.outcome {
            Stage0ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => {
                Err(diagnostic.clone().into())
            }
            Stage0ResolvedFileReferenceOutcome::Content {
                logical_path,
                value,
            } => {
                if let Some(value) = value {
                    let kind = materialize_owned_folded_string(value, string_table, |origin| {
                        Ok(services
                            .module_resources
                            .borrow_mut()
                            .intern_origin(origin.clone(), row.location.clone()))
                    })?;
                    return Ok(Expression::new(
                        kind,
                        row.location.clone(),
                        builtin_type_ids::STRING,
                        DataType::StringSlice,
                        value_mode.clone(),
                    ));
                }

                let logical_path = logical_path.ok_or_else(|| {
                    CompilerError::compiler_error(
                        "ordinary content file reference had no logical source path",
                    )
                })?;
                let content_path = content_constant_path(logical_path, string_table);
                let declaration = context
                    .shared
                    .top_level_declarations
                    .get_by_path(&content_path)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "synthetic content declaration {:?} was not present before file-value resolution",
                            content_path
                        ))
                    })?;
                Ok(reference_expression_from_declaration(
                    declaration,
                    context,
                    type_interner,
                    row.location.clone(),
                ))
            }
            _ => Err(CompilerError::compiler_error(
                "content file reference did not resolve to a content target",
            )
            .into()),
        },
        PreparedFileReferenceClass::ResourceFile => match resolved.outcome {
            Stage0ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => {
                Err(diagnostic.clone().into())
            }
            Stage0ResolvedFileReferenceOutcome::Resource {
                source,
                owner_relative_path,
            } => {
                let module_origin = services.module_origin.clone().ok_or_else(|| {
                    CompilerError::compiler_error(
                        "resource file value reached AST without a stable module origin",
                    )
                })?;
                let origin = StableResourceOriginId::module_owned(
                    module_origin,
                    owner_relative_path.clone(),
                );
                let mut resources = services.module_resources.borrow_mut();
                let resource = match source {
                    Some(source) => {
                        resources.intern_origin_with_source(origin, *source, row.location.clone())
                    }
                    None => resources.intern_origin(origin, row.location.clone()),
                };
                structural_string(
                    vec![ConstStringPiece::Resource(resource)],
                    row.location.clone(),
                    value_mode,
                )
            }
            _ => Err(CompilerError::compiler_error(
                "resource file reference did not resolve to a resource target",
            )
            .into()),
        },
    }
}

fn structural_string(
    pieces: Vec<ConstStringPiece>,
    location: SourceLocation,
    value_mode: &ValueMode,
) -> Result<Expression, ExpressionParseError> {
    Ok(Expression::new(
        ExpressionKind::StructuralString { pieces },
        location,
        builtin_type_ids::STRING,
        DataType::StringSlice,
        value_mode.clone(),
    ))
}
