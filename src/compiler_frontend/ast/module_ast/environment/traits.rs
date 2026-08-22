//! Trait definition resolution during AST environment construction.
//!
//! WHAT: converts header-stage trait shells into resolved trait metadata with stable IDs and
//! semantic requirement `TypeId`s.
//! WHY: AST owns type resolution, so trait requirement signatures are resolved here while the
//! trait subsystem owns the resulting compile-time metadata.

use super::builder::AstModuleEnvironmentBuilder;
use crate::compiler_frontend::ast::module_ast::scope_context::{ContextKind, ScopeContext};
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, SignatureTypeFallbackPolicy,
    function_signature_from_syntax_with_unresolved_types,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::{
    GenericParameterScopeBuildInput, build_generic_parameter_scope, resolve_function_signature,
};
use crate::compiler_frontend::builtins::casts::evidence::{
    builtin_evidence_rows, builtin_evidence_trait_kind_for_row, type_id_for_builtin_target,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::builtins::casts::traits::{
    BUILTIN_CAST_TRAIT_ROWS, builtin_cast_trait_name,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTraitIncompatibilityReason,
};
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, GenericParameterScope, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    FunctionReturnSyntax, FunctionSignatureSyntax, ReturnSlotSyntax, SignatureMemberSyntax,
};
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitRequirement, ResolvedTraitReturn,
    TraitReceiverRequirement, TraitVisibility,
};
use crate::compiler_frontend::traits::environment::{
    CoreTraitKind, TraitEnvironment, requirement_parameter_from_type, trait_this_name,
};
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::{TraitId, TraitRequirementId};
use crate::compiler_frontend::traits::syntax::{
    TraitDeclarationSyntax, TraitReferenceSyntax, TraitRequirementSyntax,
};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;
use std::sync::Arc;

struct TraitRequirementResolutionInput<'a> {
    header: &'a Header,
    declaration: &'a TraitDeclarationSyntax,
    requirement: &'a TraitRequirementSyntax,
    this_name: StringId,
    this_type: TypeId,
    requirement_id: TraitRequirementId,
    generic_parameter_scope: Option<&'a GenericParameterScope>,
}

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    pub(in crate::compiler_frontend::ast) fn resolve_trait_definitions(
        &mut self,
        sorted_headers: &[Header],
        string_table: &mut StringTable,
    ) -> Result<TraitEnvironment, CompilerMessages> {
        let mut trait_environment = TraitEnvironment::new();
        trait_environment.register_core_displayable(&mut self.type_environment, string_table);
        Self::register_core_cast_traits(
            &mut trait_environment,
            &mut self.type_environment,
            string_table,
        )?;
        self.project_imported_trait_declarations(&mut trait_environment, string_table)
            .map_err(|error| self.error_messages(error, string_table))?;

        for header in sorted_headers {
            let HeaderKind::Trait { declaration } = &header.kind else {
                continue;
            };

            let definition = self.resolve_trait_definition(
                header,
                declaration,
                &trait_environment,
                string_table,
            )?;

            if let Some(existing_id) = trait_environment.insert(definition) {
                let Some(existing_definition) = trait_environment.get(existing_id) else {
                    continue;
                };

                return Err(self.diagnostic_messages(
                    CompilerDiagnostic::duplicate_declaration(
                        declaration.name,
                        Some(existing_definition.declaration_location.clone()),
                        declaration.name_location.clone(),
                    ),
                    string_table,
                ));
            }
        }

        self.validate_trait_conformance_references(
            sorted_headers,
            &trait_environment,
            string_table,
        )?;

        self.resolve_trait_incompatibilities(sorted_headers, &mut trait_environment, string_table)?;

        Ok(trait_environment)
    }

    /// Registers the compiler-owned core cast trait definitions.
    ///
    /// WHAT: iterates the static core cast trait table and registers one
    ///      `ResolvedTraitDefinition` per variant with the trait environment,
    ///      then records a `CoreTraitKind::Castable` classifier so the AST
    ///      environment builder can map trait ids to their builtin target
    ///      and fallibility during evidence registration.
    /// WHY: `cast` requires all twelve `CASTABLE_TO_*` and `TRY_CASTABLE_TO_*`
    ///      trait names to resolve without dependency clauses; sharing one registration
    ///      path with `DISPLAYABLE` keeps the trait environment table-driven
    ///      and prevents drift between the catalogue, the trait definitions,
    ///      and the builtin evidence rows.
    pub(in crate::compiler_frontend::ast) fn register_core_cast_traits(
        trait_environment: &mut TraitEnvironment,
        type_environment: &mut crate::compiler_frontend::datatypes::environment::TypeEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for metadata in BUILTIN_CAST_TRAIT_ROWS {
            let Some(success_type) =
                type_id_for_builtin_target(metadata.target, type_environment, string_table)
            else {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Core cast trait target type was not registered before trait metadata.",
                    ),
                    string_table,
                ));
            };

            let fallible = metadata.fallibility == BuiltinCastFallibility::Fallible;
            let error_return_type = if fallible {
                let Some(error_type) = type_id_for_builtin_target(
                    BuiltinCastTarget::Error,
                    type_environment,
                    string_table,
                ) else {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "Builtin Error type was not registered before fallible core cast traits.",
                        ),
                        string_table,
                    ));
                };
                Some(error_type)
            } else {
                None
            };
            let trait_id = trait_environment.register_core_trait(
                type_environment,
                string_table,
                metadata.trait_name,
                metadata.requirement_name,
                success_type,
                error_return_type,
            );
            trait_environment.record_core_trait_kind(
                trait_id,
                CoreTraitKind::Castable {
                    target: metadata.target,
                    fallibility: metadata.fallibility,
                },
            );
        }

        Self::register_core_cast_trait_incompatibility_pairs(trait_environment);

        Ok(())
    }

    /// Registers the automatic incompatibility pairs for core cast traits.
    ///
    /// WHAT: after all twelve core cast traits are registered, groups their
    ///      `TraitId`s by builtin target and records each infallible/fallible
    ///      pair as incompatible in the trait environment.
    /// WHY: the core cast trait table is the single source of truth for both
    ///      the trait names and their targets; deriving the six pairs from the
    ///      table avoids a parallel hand-written list and keeps the catalogue
    ///      consistent.
    fn register_core_cast_trait_incompatibility_pairs(trait_environment: &mut TraitEnvironment) {
        #[derive(Default)]
        struct CoreCastTraitPair {
            infallible: Option<TraitId>,
            fallible: Option<TraitId>,
        }

        let mut trait_ids_by_target: FxHashMap<BuiltinCastTarget, CoreCastTraitPair> =
            FxHashMap::default();

        for metadata in BUILTIN_CAST_TRAIT_ROWS {
            let Some(trait_id) =
                trait_environment.core_trait_id_for_static_name(metadata.trait_name)
            else {
                continue;
            };

            let pair = trait_ids_by_target.entry(metadata.target).or_default();
            match metadata.fallibility {
                BuiltinCastFallibility::Infallible => pair.infallible = Some(trait_id),
                BuiltinCastFallibility::Fallible => pair.fallible = Some(trait_id),
            }
        }

        for pair in trait_ids_by_target.values() {
            if let (Some(infallible), Some(fallible)) = (pair.infallible, pair.fallible) {
                trait_environment.record_incompatible_traits(infallible, fallible);
            }
        }
    }

    /// Registers the compiler-owned builtin evidence rows for every core
    /// cast trait row.
    ///
    /// WHAT: walks the static builtin evidence table and inserts one
    ///      `TraitEvidenceDefinition` with `TraitEvidenceKind::Builtin` for
    ///      every (source, target) row whose trait id resolves through the
    ///      trait environment. Reject rows whose trait id is missing because
    ///      registration order was somehow violated.
    /// WHY: builtin evidence must satisfy static generic-bound checks via
    ///      `builtin_for`, and the registration must be centralized so the
    ///      evidence table is the single source of truth for the initial
    ///      builtin evidence rows.
    pub(in crate::compiler_frontend::ast) fn register_builtin_cast_evidence(
        trait_environment: &TraitEnvironment,
        trait_evidence_environment: &mut TraitEvidenceEnvironment,
        type_environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        use crate::compiler_frontend::traits::evidence::environment::{
            TraitEvidenceDefinition, TraitEvidenceKind,
        };

        for &row in builtin_evidence_rows() {
            let source = row.source;
            let _target = row.target;
            let Some(trait_kind) = builtin_evidence_trait_kind_for_row(row) else {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Builtin cast evidence row did not map to a registered core cast trait.",
                    ),
                    string_table,
                ));
            };
            let trait_name = builtin_cast_trait_name(trait_kind);
            let Some(trait_id) = trait_environment.core_trait_id_for_static_name(trait_name) else {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Core cast trait was not registered before builtin evidence.",
                    ),
                    string_table,
                ));
            };
            let Some(source_type_id) =
                type_id_for_builtin_target(source, type_environment, string_table)
            else {
                return Err(CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(
                        "Builtin cast evidence source type was not registered.",
                    ),
                    string_table,
                ));
            };

            // Source-File/Location are recorded as default for compiler-owned
            // builtin evidence because the source row is internal metadata,
            // not a user-declared conformance.
            let declaration = TraitEvidenceDefinition {
                id: crate::compiler_frontend::traits::ids::TraitEvidenceId(0),
                kind: TraitEvidenceKind::Builtin,
                target_type_id: source_type_id,
                trait_id,
                source_file: crate::compiler_frontend::symbols::interned_path::InternedPath::new(),
                declaration_location:
                    crate::compiler_frontend::tokenizer::tokens::SourceLocation::default(),
                requirements: Vec::new(),
            };
            trait_evidence_environment.insert_builtin(declaration);
        }
        Ok(())
    }

    fn resolve_trait_definition(
        &mut self,
        header: &Header,
        declaration: &TraitDeclarationSyntax,
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<ResolvedTraitDefinition, CompilerMessages> {
        let visibility = Arc::clone(
            self.binding_environment
                .visibility_for(&header.source_file)
                .map_err(|error| self.error_messages(error, string_table))?,
        );

        let this_name = trait_this_name(string_table);
        let this_parameters =
            trait_this_parameter_list(this_name, declaration.name_location.clone());
        let registered_this = self
            .type_environment
            .register_generic_parameter_list(&this_parameters, &FxHashMap::default());
        let Some(this_canonical_id) = registered_this
            .canonical_by_local
            .get(&TypeParameterId(0))
            .copied()
        else {
            return Err(self.error_messages(
                CompilerError::compiler_error(
                    "Trait `This` synthetic type parameter was not registered.",
                ),
                string_table,
            ));
        };
        let this_type = self
            .type_environment
            .intern_generic_parameter(this_canonical_id, this_name);

        let generic_parameter_scope =
            build_generic_parameter_scope(GenericParameterScopeBuildInput {
                generic_parameters: &this_parameters,
                canonical_by_local: Some(&registered_this.canonical_by_local),
                visible_source_bindings: &visibility.visible_source_names,
                visible_type_aliases: &visibility.visible_type_alias_names,
                visible_external_symbols: &visibility.visible_external_symbols,
                declaration_table: self.declaration_table.as_ref(),
                generic_declarations_by_path: &self.module_symbols.generic_declarations_by_path,
                string_table,
            })
            .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;

        let mut requirements = Vec::with_capacity(declaration.requirements.len());
        let mut requirement_locations_by_name = FxHashMap::default();
        let mut next_requirement_id = trait_environment.next_requirement_id();

        for requirement in &declaration.requirements {
            if let Some(first_location) = requirement_locations_by_name
                .insert(requirement.name, requirement.name_location.clone())
            {
                return Err(self.diagnostic_messages(
                    CompilerDiagnostic::duplicate_trait_requirement(
                        declaration.name,
                        requirement.name,
                        first_location,
                        requirement.name_location.clone(),
                    ),
                    string_table,
                ));
            }

            let resolved_requirement = self.resolve_trait_requirement(
                TraitRequirementResolutionInput {
                    header,
                    declaration,
                    requirement,
                    this_name,
                    this_type,
                    requirement_id: next_requirement_id,
                    generic_parameter_scope: generic_parameter_scope.as_ref(),
                },
                string_table,
            )?;
            requirements.push(resolved_requirement);
            next_requirement_id.0 += 1;
        }

        if header.export_mode.is_public() {
            self.validate_exported_trait_surface(
                declaration.name,
                &requirements,
                &header.source_file,
                trait_environment,
                string_table,
            )?;
        }

        Ok(ResolvedTraitDefinition {
            id: trait_environment.next_trait_id(),
            name: declaration.name,
            canonical_path: header.tokens.src_path.clone(),
            source_file: header.source_file.clone(),
            this_type,
            requirements,
            declaration_location: declaration.name_location.clone(),
            visibility: TraitVisibility::Source {
                exported: header.export_mode.is_public(),
            },
        })
    }

    fn resolve_trait_requirement(
        &mut self,
        input: TraitRequirementResolutionInput<'_>,
        string_table: &mut StringTable,
    ) -> Result<ResolvedTraitRequirement, CompilerMessages> {
        let TraitRequirementResolutionInput {
            header,
            declaration,
            requirement,
            this_name,
            this_type,
            requirement_id,
            generic_parameter_scope,
        } = input;

        let visibility = Arc::clone(
            self.binding_environment
                .visibility_for(&header.source_file)
                .map_err(|error| self.error_messages(error, string_table))?,
        );

        let signature_syntax =
            signature_with_trait_this_as_parameter(&requirement.signature, this_name);
        let unresolved_signature = self.unresolved_trait_requirement_signature(
            header,
            declaration,
            &signature_syntax,
            string_table,
        )?;

        let mut type_resolution_context =
            self.type_resolution_context_for(&visibility, generic_parameter_scope);
        let resolved_signature = resolve_function_signature(
            &header.tokens.src_path,
            &unresolved_signature,
            None,
            &mut type_resolution_context,
            string_table,
        )
        .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;

        let Some(first_parameter) = resolved_signature.signature.parameters.first() else {
            return Err(self.diagnostic_messages(
                CompilerDiagnostic::unsupported_trait_feature(
                    declaration.name,
                    string_table.intern("missing This receiver"),
                    requirement.location.clone(),
                ),
                string_table,
            ));
        };

        let receiver = if first_parameter.value.value_mode.is_mutable() {
            TraitReceiverRequirement::Mutable { this_type }
        } else {
            TraitReceiverRequirement::Immutable { this_type }
        };

        let mut parameters = Vec::with_capacity(
            resolved_signature
                .signature
                .parameters
                .len()
                .saturating_sub(1),
        );
        for parameter in resolved_signature.signature.parameters.iter().skip(1) {
            parameters.push(requirement_parameter_from_type(
                parameter.id.clone(),
                parameter.value.value_mode.clone(),
                parameter.value.type_id,
                parameter.value.location.clone(),
            ));
        }

        let mut returns = Vec::with_capacity(resolved_signature.signature.returns.len());
        for return_slot in &resolved_signature.signature.returns {
            let Some(type_id) = return_slot.type_id else {
                return Err(self.diagnostic_messages(
                    CompilerDiagnostic::unsupported_trait_feature(
                        declaration.name,
                        string_table.intern("unresolved requirement return"),
                        requirement.location.clone(),
                    ),
                    string_table,
                ));
            };

            returns.push(ResolvedTraitReturn {
                type_id,
                channel: return_slot.channel,
                location: requirement.location.clone(),
            });
        }

        Ok(ResolvedTraitRequirement {
            id: requirement_id,
            name: requirement.name,
            name_location: requirement.name_location.clone(),
            receiver,
            parameters,
            returns,
            location: requirement.location.clone(),
        })
    }

    fn unresolved_trait_requirement_signature(
        &mut self,
        header: &Header,
        declaration: &TraitDeclarationSyntax,
        signature_syntax: &FunctionSignatureSyntax,
        string_table: &mut StringTable,
    ) -> Result<FunctionSignature, CompilerMessages> {
        let source_file_scope = header.canonical_source_file(string_table);
        let signature_context = ScopeContext::new(
            ContextKind::ConstantHeader,
            header.tokens.src_path.to_owned(),
            Rc::clone(&self.declaration_table),
            Arc::clone(&self.context.external_package_registry),
            vec![],
            0,
            self.context.template_ir_store.clone(),
        )
        .with_style_directives(self.context.style_directives)
        .with_build_profile(self.context.build_profile)
        .with_project_path_resolver(self.context.project_path_resolver.clone())
        .with_path_format_config(self.context.path_format_config.clone())
        .with_template_const_loop_iteration_limit(self.context.template_const_loop_iteration_limit)
        .with_rendered_path_usage_sink(Rc::clone(&self.rendered_path_usages))
        .with_resolved_type_aliases(Rc::new(self.resolved_type_aliases_by_path.clone()))
        .with_generic_declarations(Rc::new(
            self.module_symbols.generic_declarations_by_path.clone(),
        ))
        .with_resolved_struct_fields_by_path(Rc::new(self.resolved_struct_fields_by_path.clone()))
        .with_nominal_type_ids_by_path(Rc::new(self.nominal_type_ids_by_path.clone()))
        .with_source_file_scope(source_file_scope);

        let mut compatibility_cache = TypeCompatibilityCache::new();
        let mut type_interner =
            AstTypeInterner::new(&mut self.type_environment, &mut compatibility_cache);
        let signature = function_signature_from_syntax_with_unresolved_types(
            signature_syntax,
            &header.tokens.path_syntax,
            &signature_context,
            &mut type_interner,
            string_table,
            SignatureTypeFallbackPolicy::StrictCapacity,
        )
        .map_err(|error| self.expression_error_messages(error, string_table))?;
        self.warnings
            .extend(signature_context.take_emitted_warnings());

        if signature.parameters.is_empty() {
            return Err(self.diagnostic_messages(
                CompilerDiagnostic::unsupported_trait_feature(
                    declaration.name,
                    string_table.intern("marker requirement"),
                    declaration.name_location.clone(),
                ),
                string_table,
            ));
        }

        Ok(signature)
    }

    fn validate_trait_conformance_references(
        &self,
        sorted_headers: &[Header],
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for header in sorted_headers {
            let HeaderKind::TraitConformance { conformance } = &header.kind else {
                continue;
            };

            let visibility = self
                .binding_environment
                .visibility_for(&header.source_file)
                .map_err(|error| self.error_messages(error, string_table))?;

            for trait_ref in &conformance.traits {
                self.resolve_visible_trait_reference(
                    trait_ref,
                    visibility,
                    trait_environment,
                    string_table,
                )?;
            }
        }

        Ok(())
    }

    fn resolve_trait_incompatibilities(
        &self,
        sorted_headers: &[Header],
        trait_environment: &mut TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let mut recorded_source_pairs: FxHashSet<(TraitId, TraitId)> = FxHashSet::default();

        for header in sorted_headers {
            let HeaderKind::TraitIncompatibility { incompatibility } = &header.kind else {
                continue;
            };

            let relation_source_file = header.source_file.clone();
            let visibility = Arc::clone(
                self.binding_environment
                    .visibility_for(&header.source_file)
                    .map_err(|error| self.error_messages(error, string_table))?,
            );

            let subject_id = self.resolve_trait_incompatibility_reference(
                &incompatibility.subject,
                &incompatibility.subject,
                &relation_source_file,
                &visibility,
                trait_environment,
                string_table,
            )?;

            for incompatible_trait in &incompatibility.incompatible_traits {
                let incompatible_id = self.resolve_trait_incompatibility_reference(
                    &incompatibility.subject,
                    incompatible_trait,
                    &relation_source_file,
                    &visibility,
                    trait_environment,
                    string_table,
                )?;

                if subject_id == incompatible_id {
                    return Err(self.diagnostic_messages(
                        CompilerDiagnostic::invalid_trait_incompatibility(
                            incompatibility.subject.name,
                            Some(incompatible_trait.name),
                            InvalidTraitIncompatibilityReason::SelfIncompatible,
                            incompatible_trait.location.clone(),
                        ),
                        string_table,
                    ));
                }

                let normalized_pair = if subject_id.0 < incompatible_id.0 {
                    (subject_id, incompatible_id)
                } else {
                    (incompatible_id, subject_id)
                };

                if !recorded_source_pairs.insert(normalized_pair) {
                    return Err(self.diagnostic_messages(
                        CompilerDiagnostic::invalid_trait_incompatibility(
                            incompatibility.subject.name,
                            Some(incompatible_trait.name),
                            InvalidTraitIncompatibilityReason::DuplicateRelation,
                            incompatible_trait.location.clone(),
                        ),
                        string_table,
                    ));
                }

                trait_environment.record_incompatible_traits(subject_id, incompatible_id);

                // Retain the resolved public visibility once, at the same time the
                // symmetric relation is resolved, so the public-interface draft can later
                // attach incompatibilities to direct public trait records without reparsing
                // headers. Only an explicitly public `TRAIT must not TRAIT` header in an
                // export-capable root contributes public-interface metadata; a private
                // relation never enters the draft even when both traits are public.
                if header.file_role.is_export_capable() && header.export_mode.is_public() {
                    trait_environment
                        .record_public_incompatible_traits(subject_id, incompatible_id);
                }
            }
        }

        Ok(())
    }

    fn resolve_trait_incompatibility_reference(
        &self,
        subject: &TraitReferenceSyntax,
        trait_ref: &TraitReferenceSyntax,
        relation_source_file: &crate::compiler_frontend::symbols::interned_path::InternedPath,
        visibility: &crate::compiler_frontend::headers::binding_environment::FileVisibility,
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<TraitId, CompilerMessages> {
        let trait_id = if let Some(path) = visibility.visible_trait_names.get(&trait_ref.name) {
            trait_environment.id_for_path(path)
        } else {
            trait_environment.core_trait_id_for_name(trait_ref.name, string_table)
        };

        let Some(trait_id) = trait_id else {
            return Err(self.diagnostic_messages(
                CompilerDiagnostic::invalid_trait_incompatibility(
                    subject.name,
                    Some(trait_ref.name),
                    InvalidTraitIncompatibilityReason::UnknownTrait,
                    trait_ref.location.clone(),
                ),
                string_table,
            ));
        };

        let Some(definition) = trait_environment.get(trait_id) else {
            return Ok(trait_id);
        };

        // Same-file relations are source-order metadata: a trait must be declared
        // before it can participate in an authored `must not` relation. Imported and
        // core traits are already visible through different mechanisms, so only
        // same-file source definitions need this local ordering check.
        if definition.source_file == *relation_source_file
            && location_starts_after(&definition.declaration_location, &trait_ref.location)
        {
            return Err(self.diagnostic_messages(
                CompilerDiagnostic::invalid_trait_incompatibility(
                    subject.name,
                    Some(trait_ref.name),
                    InvalidTraitIncompatibilityReason::UnknownTrait,
                    trait_ref.location.clone(),
                ),
                string_table,
            ));
        }

        Ok(trait_id)
    }

    pub(in crate::compiler_frontend::ast) fn resolve_visible_trait_reference(
        &self,
        trait_ref: &TraitReferenceSyntax,
        visibility: &crate::compiler_frontend::headers::binding_environment::FileVisibility,
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<TraitId, CompilerMessages> {
        if let Some(path) = visibility.visible_trait_names.get(&trait_ref.name)
            && let Some(id) = trait_environment.id_for_path(path)
        {
            return Ok(id);
        }

        if let Some(id) = trait_environment.core_trait_id_for_name(trait_ref.name, string_table) {
            return Ok(id);
        }

        Err(self.diagnostic_messages(
            CompilerDiagnostic::unknown_trait_name(trait_ref.name, trait_ref.location.clone()),
            string_table,
        ))
    }

    fn validate_exported_trait_surface(
        &mut self,
        trait_name: StringId,
        requirements: &[ResolvedTraitRequirement],
        public_root_file: &crate::compiler_frontend::symbols::interned_path::InternedPath,
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for requirement in requirements {
            for parameter in &requirement.parameters {
                self.validate_public_trait_type(
                    trait_name,
                    parameter.type_id,
                    public_root_file,
                    parameter.location.clone(),
                    trait_environment,
                    string_table,
                )?;
            }

            for return_slot in &requirement.returns {
                self.validate_public_trait_type(
                    trait_name,
                    return_slot.type_id,
                    public_root_file,
                    return_slot.location.clone(),
                    trait_environment,
                    string_table,
                )?;
            }
        }

        Ok(())
    }

    fn validate_public_trait_type(
        &self,
        trait_name: StringId,
        type_id: TypeId,
        public_root_file: &crate::compiler_frontend::symbols::interned_path::InternedPath,
        location: SourceLocation,
        trait_environment: &TraitEnvironment,
        string_table: &StringTable,
    ) -> Result<(), CompilerMessages> {
        let mut visited_types = FxHashSet::default();
        if self.public_type_id_is_nameable(
            type_id,
            public_root_file,
            trait_environment,
            &mut visited_types,
        ) {
            return Ok(());
        }

        Err(self.diagnostic_messages(
            CompilerDiagnostic::trait_private_surface_leak(trait_name, type_id, location),
            string_table,
        ))
    }
}

fn location_starts_after(left: &SourceLocation, right: &SourceLocation) -> bool {
    left.start_pos.line_number > right.start_pos.line_number
        || (left.start_pos.line_number == right.start_pos.line_number
            && left.start_pos.char_column > right.start_pos.char_column)
}

fn trait_this_parameter_list(
    this_name: StringId,
    location: SourceLocation,
) -> GenericParameterList {
    GenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name: this_name,
            location,
            trait_bounds: Vec::new(),
        }],
    }
}

fn signature_with_trait_this_as_parameter(
    signature: &FunctionSignatureSyntax,
    this_name: StringId,
) -> FunctionSignatureSyntax {
    FunctionSignatureSyntax {
        parameters: signature
            .parameters
            .iter()
            .map(|parameter| signature_member_with_trait_this(parameter, this_name))
            .collect(),
        returns: signature
            .returns
            .iter()
            .map(|return_slot| return_slot_with_trait_this(return_slot, this_name))
            .collect(),
    }
}

fn signature_member_with_trait_this(
    member: &SignatureMemberSyntax,
    this_name: StringId,
) -> SignatureMemberSyntax {
    let mut member = member.clone();
    member.type_annotation = parsed_type_with_trait_this(&member.type_annotation, this_name);
    member
}

fn return_slot_with_trait_this(
    return_slot: &ReturnSlotSyntax,
    this_name: StringId,
) -> ReturnSlotSyntax {
    ReturnSlotSyntax {
        value: FunctionReturnSyntax {
            type_annotation: parsed_type_with_trait_this(
                &return_slot.value.type_annotation,
                this_name,
            ),
            location: return_slot.value.location.clone(),
        },
        channel: return_slot.channel,
        location: return_slot.location.clone(),
    }
}

fn parsed_type_with_trait_this(parsed_type: &ParsedTypeRef, this_name: StringId) -> ParsedTypeRef {
    match parsed_type {
        ParsedTypeRef::This { location } => ParsedTypeRef::Named {
            name: this_name,
            location: location.clone(),
        },

        ParsedTypeRef::Applied {
            base,
            arguments,
            location,
        } => ParsedTypeRef::Applied {
            base: Box::new(parsed_type_with_trait_this(base, this_name)),
            arguments: arguments
                .iter()
                .map(|argument| parsed_type_with_trait_this(argument, this_name))
                .collect(),
            location: location.clone(),
        },

        ParsedTypeRef::Collection {
            element,
            location,
            fixed_capacity,
        } => ParsedTypeRef::Collection {
            element: Box::new(parsed_type_with_trait_this(element, this_name)),
            location: location.clone(),
            fixed_capacity: fixed_capacity.clone(),
        },

        ParsedTypeRef::Optional { inner, location } => ParsedTypeRef::Optional {
            inner: Box::new(parsed_type_with_trait_this(inner, this_name)),
            location: location.clone(),
        },

        ParsedTypeRef::Result { ok, err, location } => ParsedTypeRef::Result {
            ok: Box::new(parsed_type_with_trait_this(ok, this_name)),
            err: Box::new(parsed_type_with_trait_this(err, this_name)),
            location: location.clone(),
        },

        _ => parsed_type.clone(),
    }
}

#[cfg(test)]
#[path = "traits_tests.rs"]
mod traits_tests;
