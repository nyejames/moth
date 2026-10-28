//! AST-owned semantics for value-position file paths.
//!
//! Stage 0 resolves physical targets and publishes one reader-facing view for ordinary modules
//! and frozen persistent-generic bodies. This module interprets that view without touching the
//! filesystem: ordinary content rows use the content file's logical path to reference the
//! synthetic `content` declaration and reuse its interned `StringId`; frozen content rows carry a
//! captured `OwnedFoldedString` and lower it through the shared owned-value inverse projection.
//! In either lane, resource and site-root rows remain structural strings.

use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOp, OwnedExpr,
};
use crate::compiler_frontend::ast::field_access::reference_expression_from_declaration;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::materialize_owned_folded_string;
use crate::compiler_frontend::ast::module_ast::scope_context::{
    ScopeContext, Stage0ResolvedFileReferenceOutcome,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::unsupported_source_file_kind;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::invalid_compile_time_path;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    invalid_expression, invalid_path,
};
use crate::compiler_frontend::compiler_messages::{InvalidExpressionReason, PathKind};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::headers::synthetic_content_header::content_constant_path;
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReferenceFailure,
};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

fn file_reference_failure_draft(failure: ResolvedFileReferenceFailure) -> DiagnosticDraft {
    match failure {
        ResolvedFileReferenceFailure::InvalidCompileTimePath { path, reason, span } => {
            invalid_compile_time_path(path, reason, Some(span))
        }
        ResolvedFileReferenceFailure::InvalidComponent { span } => {
            invalid_path(PathKind::InvalidComponent, Some(span))
        }
        ResolvedFileReferenceFailure::UnsupportedSourceFileKind {
            path,
            extension,
            span,
        } => unsupported_source_file_kind(path, extension, Some(span)),
    }
}

/// Resolve one path token through the value-position Stage 0 view.
///
/// Resolution is driven by the immutable view supplied by semantic orchestration. It covers
/// ordinary module rows and frozen persistent-generic rows, so this function never reopens source
/// tables or calls a filesystem path resolver.
#[allow(
    clippy::too_many_arguments,
    reason = "file-value resolution takes the path syntax, token stream, scope, interner, builders, value mode, string table, and path fork from the parser call site"
)]
pub(crate) fn resolve_file_value(
    path_syntax: PathSyntaxId,
    token_stream: &AstCursor,
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<OwnedExpr, ExpressionParseError> {
    let token_span = token_stream.current_span();
    let span = Some(token_span);
    token_stream
        .path_syntax_table()?
        .try_path_for_token(path_syntax, token_span)?;
    let services = context
        .shared
        .file_value_resolution
        .as_ref()
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "value-position file path reached AST without Stage 0 resolution services",
            )
        })?;
    let stage0_resolution_facts = services.stage0_resolution_facts.as_ref().ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "value-position file path reached AST without Stage 0 resolution services",
        )
    })?;
    let resolved = stage0_resolution_facts
        .lookup(context.shared.declaring_file_id, path_syntax)?
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "value-position file path had no matching Stage 0 resolved-reference row",
            )
        })?;

    match resolved.class {
        PreparedFileReferenceClass::Extensionless => {
            Err(invalid_expression(InvalidExpressionReason::ExtensionlessFileValue, span).into())
        }
        PreparedFileReferenceClass::SourceKindNoFileValue => {
            Err(invalid_expression(InvalidExpressionReason::MothFileHasNoValue, span).into())
        }
        PreparedFileReferenceClass::SiteRoot => match resolved.outcome {
            Stage0ResolvedFileReferenceOutcome::NoPhysicalTarget => structural_string(
                expression_builder,
                const_values,
                vec![ConstStringPiece::SiteRoot],
                span,
                value_mode,
            ),
            Stage0ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => {
                Err(file_reference_failure_draft(diagnostic).into())
            }
            _ => Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "site-root file reference unexpectedly resolved to a physical target",
            )
            .into()),
        },
        PreparedFileReferenceClass::ContentSource => match resolved.outcome {
            Stage0ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => {
                Err(file_reference_failure_draft(diagnostic).into())
            }
            Stage0ResolvedFileReferenceOutcome::Content {
                logical_path,
                value,
            } => {
                if let Some(value) = value {
                    let value = materialize_owned_folded_string(value, string_table, |origin| {
                        Ok(services
                            .module_resources
                            .borrow_mut()
                            .intern_origin(origin.clone(), span))
                    })?;
                    let value = const_values
                        .insert_resolved(
                            builtin_type_ids::STRING,
                            ConstValuePayload::String(value),
                            span,
                        )
                        .map_err(ExpressionParseError::from)?;
                    return Ok(expression_builder.append_expression(
                        ExpressionOp::Constant(value),
                        span,
                        builtin_type_ids::STRING,
                        *value_mode,
                    )?);
                }

                let logical_path = logical_path.as_ref().ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "ordinary content file reference had no logical source path",
                    )
                })?;
                let content_path = content_constant_path(*logical_path, path_fork, string_table)?;
                let declaration = context
                    .shared
                    .top_level_declarations
                    .get_by_path(&content_path)
                    .ok_or_else(|| {
                        InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                            "synthetic content declaration {:?} was not present before file-value resolution",
                            content_path
                        ))
                    })?;
                reference_expression_from_declaration(
                    declaration,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    span,
                )
            }
            _ => Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "content file reference did not resolve to a content target",
            )
            .into()),
        },
        PreparedFileReferenceClass::ResourceFile => match resolved.outcome {
            Stage0ResolvedFileReferenceOutcome::Diagnostic(diagnostic) => {
                Err(file_reference_failure_draft(diagnostic).into())
            }
            Stage0ResolvedFileReferenceOutcome::Resource {
                source,
                owner_relative_path,
            } => {
                let module_origin = services.module_origin.clone().ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "resource file value reached AST without a stable module origin",
                    )
                })?;
                let origin = StableResourceOriginId::module_owned(
                    module_origin,
                    owner_relative_path.clone(),
                );
                let mut resources = services.module_resources.borrow_mut();
                let resource = match source {
                    Some(source) => resources.intern_origin_with_source(origin, *source, span),
                    None => resources.intern_origin(origin, span),
                };
                structural_string(
                    expression_builder,
                    const_values,
                    vec![ConstStringPiece::Resource(resource)],
                    span,
                    value_mode,
                )
            }
            _ => Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "resource file reference did not resolve to a resource target",
            )
            .into()),
        },
    }
}

pub(crate) fn structural_string(
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    pieces: Vec<ConstStringPiece>,
    span: Option<SourceSpan>,
    value_mode: &ValueMode,
) -> Result<OwnedExpr, ExpressionParseError> {
    let value = const_values
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Pieces(pieces)),
            span,
        )
        .map_err(ExpressionParseError::from)?;
    Ok(expression_builder.append_expression(
        ExpressionOp::Constant(value),
        span,
        builtin_type_ids::STRING,
        *value_mode,
    )?)
}
