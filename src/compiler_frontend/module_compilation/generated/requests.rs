//! Canonicalising one concrete generic request from requester AST facts.
//!
//! WHAT: projects a deferred generic instantiation into a stable `GeneratedFunctionIdentity`,
//!       installs the substituted callable contract into the requester's AST, and resolves the
//!       stable origins its type arguments and evidence depend on.
//! WHY:  request identity must be stable across modules and builds, so it is built from canonical
//!       type and evidence identities rather than donor-local handles. This is compiler semantics
//!       over compiler state; the build system only ever sees the finished identity.

use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstantiationRequest, GenericFunctionTemplate, ModuleMaterialisationPreparation,
    bootstrap_call_summary_from_signature, concrete_argument_mapping,
    substitute_function_signature,
};
use crate::compiler_frontend::ast::{Ast, AstImportedFunctionContract};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalEvidenceIdentity, CanonicalTypeIdentity, CanonicalTypeProjectionContext,
    ExportedGenericParameterIdentity, GenericParameterOriginResolver, NominalOriginResolver,
    project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, NominalTypeId};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;
use crate::compiler_frontend::semantic_identity::{GeneratedFunctionIdentity, OriginTypeId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::FxHashMap;

struct GeneratedRequestNominalOrigins<'a> {
    type_environment: &'a TypeEnvironment,
}

impl NominalOriginResolver for GeneratedRequestNominalOrigins<'_> {
    fn resolve_nominal_origin(
        &self,
        nominal_id: NominalTypeId,
    ) -> Result<OriginTypeId, CompilerError> {
        let type_id = self
            .type_environment
            .type_id_for_nominal_id(nominal_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request references a nominal type without a local type identity",
                )
            })?;
        let canonical_identity = self
            .type_environment
            .canonical_identity_for_type_id(type_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request nominal type has no canonical identity",
                )
            })?;
        let CanonicalTypeIdentity::SourceNominal(origin) = canonical_identity else {
            return Err(CompilerError::compiler_error(
                "Generated request nominal type has a non-source canonical identity",
            ));
        };

        Ok(origin.clone())
    }
}

struct GeneratedRequestGenericParameters<'a> {
    type_environment: &'a TypeEnvironment,
    templates: &'a FxHashMap<InternedPath, GenericFunctionTemplate>,
    string_table: &'a StringTable,
}

impl GenericParameterOriginResolver for GeneratedRequestGenericParameters<'_> {
    fn resolve_generic_parameter_origin(
        &self,
        parameter_id: GenericParameterId,
    ) -> Result<ExportedGenericParameterIdentity, CompilerError> {
        let type_id = self
            .type_environment
            .type_id_for_generic_parameter(parameter_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request references a generic parameter without a local type handle",
                )
            })?;
        if let Some(CanonicalTypeIdentity::GenericParameter(identity)) = self
            .type_environment
            .canonical_identity_for_type_id(type_id)
        {
            return Ok(identity.clone());
        }
        for template in self.templates.values() {
            let Some(parameter_list) = self
                .type_environment
                .generic_parameters(template.generic_parameter_list_id)
            else {
                continue;
            };
            let Some((position, parameter)) = parameter_list
                .parameters
                .iter()
                .enumerate()
                .find(|(_, parameter)| parameter.id == parameter_id)
            else {
                continue;
            };
            let owner = template.generic_parameter_owner.clone().ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request references a private generic parameter across a generated boundary",
                )
            })?;
            return Ok(ExportedGenericParameterIdentity::new(
                owner,
                position as u32,
                self.string_table.resolve(parameter.name).to_owned(),
            ));
        }
        Err(CompilerError::compiler_error(format!(
            "Generated request references generic parameter {parameter_id:?} as TypeId({}) without a stable exported identity",
            type_id.0
        )))
    }
}

/// One generated request after canonicalisation, ready for the module transaction.
#[derive(Clone)]
pub(crate) struct CanonicalGeneratedRequest {
    pub(crate) identity: GeneratedFunctionIdentity,
    pub(crate) function_name: Option<StringId>,
    pub(crate) call_location: SourceLocation,
}

pub(crate) fn install_generated_request_contracts(
    requests: &[GenericFunctionInstantiationRequest],
    materialisation_context: &ModuleMaterialisationPreparation,
    templates: &FxHashMap<InternedPath, GenericFunctionTemplate>,
    external_registry: &ExternalPackageRegistry,
    module_ast: &mut Ast,
) -> Result<Vec<CanonicalGeneratedRequest>, CompilerError> {
    let mut identities = Vec::with_capacity(requests.len());
    for request in requests {
        let template = templates.get(&request.key.function_path).ok_or_else(|| {
            CompilerError::compiler_error(
                "Deferred generic request has no requester-local generic contract",
            )
        })?;
        let declaration_identity = template.declaration_identity.clone().ok_or_else(|| {
            CompilerError::compiler_error(
                "Deferred generic request template has no stable declaration identity",
            )
        })?;
        if let Some(request_identity) = request.declaration_identity.as_ref()
            && request_identity != &declaration_identity
        {
            return Err(CompilerError::compiler_error(
                "Deferred generic request declaration identity disagrees with its template",
            ));
        }

        let canonical_type_arguments = {
            let nominal_origins = GeneratedRequestNominalOrigins {
                type_environment: &materialisation_context.type_environment,
            };
            let generic_parameter_origins = GeneratedRequestGenericParameters {
                type_environment: &materialisation_context.type_environment,
                templates,
                string_table: &materialisation_context.string_table,
            };
            let projection_context = CanonicalTypeProjectionContext::new(
                &nominal_origins,
                &generic_parameter_origins,
                external_registry,
            );
            let mut canonical_type_arguments = Vec::with_capacity(request.key.type_arguments.len());
            for type_id in request.key.type_arguments.iter().copied() {
                canonical_type_arguments.push(project_type_id_to_canonical_identity(
                    type_id,
                    &materialisation_context.type_environment,
                    &projection_context,
                )?);
            }
            canonical_type_arguments
        };
        let identity = GeneratedFunctionIdentity::new(
            declaration_identity,
            canonical_type_arguments.into_boxed_slice(),
            canonicalize_generated_request_evidence(
                request,
                materialisation_context,
                external_registry,
            )?,
        );

        let mapping = concrete_argument_mapping(
            template.generic_parameter_list_id,
            request.key.type_arguments.as_ref(),
            &module_ast.type_environment,
        )
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Deferred generic request does not match its projected parameter list",
            )
        })?;
        let signature = substitute_function_signature(
            &template.signature,
            &mapping,
            &mut module_ast.type_environment,
        );
        let fallible_carrier_type_id = signature.error_return_type_id().map(|error_type_id| {
            let success_type_id = match signature.success_return_type_ids().as_slice() {
                [] => crate::compiler_frontend::datatypes::builtin_type_ids::NONE,
                [single] => *single,
                many => module_ast.type_environment.intern_tuple(many.to_vec()),
            };
            module_ast
                .type_environment
                .intern_fallible_carrier(success_type_id, error_type_id)
        });
        let summary = bootstrap_call_summary_from_signature(&signature);
        identities.push(CanonicalGeneratedRequest {
            identity: identity.clone(),
            function_name: request.key.function_path.name(),
            call_location: request.call_location.clone(),
        });
        let contract = AstImportedFunctionContract {
            target: SourceFunctionTarget::Generated {
                identity,
                local_path: request.instance_path.clone(),
            },
            summary,
            fallible_carrier_type_id,
        };

        if module_ast
            .imported_functions_by_local_path
            .insert(request.instance_path.clone(), contract)
            .is_some()
        {
            return Err(CompilerError::compiler_error(
                "Generated request path collides with another imported or generated callable",
            ));
        }
    }

    Ok(identities)
}

fn canonicalize_generated_request_evidence(
    request: &GenericFunctionInstantiationRequest,
    materialisation_context: &ModuleMaterialisationPreparation,
    external_registry: &ExternalPackageRegistry,
) -> Result<Box<[CanonicalEvidenceIdentity]>, CompilerError> {
    if request.evidence.is_empty() {
        return Ok(Box::new([]));
    }

    let nominal_origins = GeneratedRequestNominalOrigins {
        type_environment: &materialisation_context.type_environment,
    };
    let generic_parameter_origins = GeneratedRequestGenericParameters {
        type_environment: &materialisation_context.type_environment,
        templates: materialisation_context.generic_function_templates(),
        string_table: &materialisation_context.string_table,
    };
    let projection_context = CanonicalTypeProjectionContext::new(
        &nominal_origins,
        &generic_parameter_origins,
        external_registry,
    );
    let mut canonical_evidence = Vec::with_capacity(request.evidence.len());
    for evidence_id in request.evidence.iter().copied() {
        let evidence = materialisation_context
            .trait_evidence_environment()
            .get(evidence_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request retained a missing requester-local evidence selection",
                )
            })?;
        let target_type_identity = project_type_id_to_canonical_identity(
            evidence.target_type_id,
            &materialisation_context.type_environment,
            &projection_context,
        )?;
        let trait_identity = materialisation_context
            .trait_environment()
            .canonical_identity_for_id(evidence.trait_id)
            .cloned()
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Generated request evidence has no stable canonical trait identity",
                )
            })?;
        canonical_evidence.push(CanonicalEvidenceIdentity::new(
            target_type_identity,
            trait_identity,
        ));
    }

    Ok(canonical_evidence.into_boxed_slice())
}
