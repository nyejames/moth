//! Type resolution for constants and nominal declarations.
//!
//! WHAT: parses constant values and resolves struct field types in header dependency order.
//! WHY: headers are already dependency-sorted; constants are parsed linearly. Struct defaults
//! can reference constants, so constants are resolved before struct fields.

use super::builder::AstModuleEnvironmentBuilder;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::generic_bounds::{
    GenericBoundEvidenceContext, validate_nominal_generic_bound_evidence,
};
use crate::compiler_frontend::ast::module_ast::environment::constant_resolution::{
    ConstantHeaderParseContext, parse_constant_header_declaration,
};
use crate::compiler_frontend::ast::module_ast::scope_context::{ContextKind, ScopeContext};
use crate::compiler_frontend::ast::statements::functions::{
    SignatureTypeFallbackPolicy, signature_member_to_declaration,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::{
    GenericParameterScopeBuildInput, StructFieldResolutionError, build_generic_parameter_scope,
    collect_type_parameter_ids_from_choice_variants, collect_type_parameter_ids_from_declarations,
    resolve_choice_variant_payload_types, resolve_diagnostic_type_to_type_id_checked,
    resolve_struct_constructor_shell_types, resolve_struct_field_types,
    validate_generic_parameters_used, validate_no_recursive_generic_type,
    validate_no_recursive_runtime_structs,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticPayload,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId};
use crate::compiler_frontend::declaration_syntax::choice::{
    ChoiceVariant, ChoiceVariantPayload, ChoiceVariantPayloadSyntax, ChoiceVariantSyntax,
};
use crate::compiler_frontend::declaration_syntax::signature_members::SignatureMemberSyntax;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use std::sync::Arc;

use crate::compiler_frontend::headers::binding_environment::FileVisibility;
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::instrumentation::{
    AstCounter, add_ast_counter, increment_ast_counter,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::timing_scope_attributed_opt;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;

#[derive(Clone, Copy)]
enum MemberShellSemanticContext {
    StructField,
    ChoicePayloadField,
}

struct NominalBoundSurfaceValidationContext<'a> {
    visibility: &'a FileVisibility,
    trait_environment: &'a TraitEnvironment,
    trait_evidence_environment: &'a TraitEvidenceEnvironment,
}

fn member_shell_diagnostic_for_context(
    diagnostic: CompilerDiagnostic,
    member_context: MemberShellSemanticContext,
) -> CompilerDiagnostic {
    match member_context {
        MemberShellSemanticContext::StructField
            if is_non_constant_struct_default_diagnostic(&diagnostic) =>
        {
            CompilerDiagnostic::invalid_struct_default_value(diagnostic.primary_location.clone())
        }

        MemberShellSemanticContext::StructField
        | MemberShellSemanticContext::ChoicePayloadField => diagnostic,
    }
}

fn is_non_constant_struct_default_diagnostic(diagnostic: &CompilerDiagnostic) -> bool {
    matches!(
        diagnostic.payload,
        DiagnosticPayload::CompileTimeEvaluationError {
            reason: CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant,
            ..
        }
    )
}

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    /// Register struct and choice identities early so later phases (trait definitions,
    /// constant parsing, field resolution) can reference nominal types by canonical `TypeId`.
    ///
    /// WHAT: registers every struct and choice in `TypeEnvironment` with empty members,
    /// then stores unresolved field/variant shells in AST-owned side tables.
    /// WHY: split from member resolution so trait metadata can be built while nominal
    /// identities are already available, without forcing trait definitions to wait for
    /// fully resolved fields.
    pub(in crate::compiler_frontend::ast) fn register_nominal_shells(
        &mut self,
        sorted_headers: &[Header],
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for header in sorted_headers {
            match &header.kind {
                HeaderKind::Struct {
                    fields,
                    generic_parameters,
                } => {
                    let unresolved_fields = self.unresolved_member_syntax_to_declarations(
                        header,
                        fields,
                        MemberShellSemanticContext::StructField,
                        string_table,
                        SignatureTypeFallbackPolicy::AllowUnresolvedCapacity,
                        true,
                    )?;

                    let generic_param_list_id = if generic_parameters.is_empty() {
                        None
                    } else {
                        let registered = self.type_environment.register_generic_parameter_list(
                            generic_parameters,
                            &FxHashMap::default(),
                        );
                        let list_id = registered.list_id;
                        self.generic_parameter_lists_by_path
                            .insert(header.tokens.src_path.clone(), registered);
                        Some(list_id)
                    };

                    let struct_def = StructTypeDefinition {
                        id: NominalTypeId(0),
                        path: header.tokens.src_path.clone(),
                        fields: Box::new([]),
                        generic_parameters: generic_param_list_id,
                        const_record: false,
                    };
                    let (_, struct_type_id) =
                        self.type_environment.register_nominal_struct(struct_def);
                    self.nominal_type_ids_by_path
                        .insert(header.tokens.src_path.clone(), struct_type_id);

                    self.resolved_struct_fields_by_path
                        .insert(header.tokens.src_path.to_owned(), unresolved_fields);

                    self.replace_declaration(Declaration {
                        id: header.tokens.src_path.to_owned(),
                        value: Expression::new(
                            ExpressionKind::NoValue,
                            header.name_location.to_owned(),
                            struct_type_id,
                            DataType::runtime_struct(
                                header.tokens.src_path.to_owned(),
                                struct_type_id,
                            ),
                            ValueMode::ImmutableReference,
                        ),
                    })
                    .map_err(|error| self.error_messages(error, string_table))?;
                }
                HeaderKind::Choice {
                    variants,
                    generic_parameters,
                } => {
                    let generic_param_list_id = if generic_parameters.is_empty() {
                        None
                    } else {
                        let registered = self.type_environment.register_generic_parameter_list(
                            generic_parameters,
                            &FxHashMap::default(),
                        );
                        let list_id = registered.list_id;
                        self.generic_parameter_lists_by_path
                            .insert(header.tokens.src_path.clone(), registered);
                        Some(list_id)
                    };

                    let unresolved_variants = self.unresolved_choice_variants_for_header(
                        header,
                        variants,
                        string_table,
                        SignatureTypeFallbackPolicy::AllowUnresolvedCapacity,
                        true,
                    )?;
                    self.choice_variant_shells_by_path
                        .insert(header.tokens.src_path.to_owned(), unresolved_variants);

                    let choice_def = ChoiceTypeDefinition {
                        id: NominalTypeId(0),
                        path: header.tokens.src_path.clone(),
                        variants: Box::new([]),
                        generic_parameters: generic_param_list_id,
                    };
                    let (_, choice_type_id) =
                        self.type_environment.register_nominal_choice(choice_def);
                    self.nominal_type_ids_by_path
                        .insert(header.tokens.src_path.clone(), choice_type_id);

                    self.replace_declaration(Declaration {
                        id: header.tokens.src_path.to_owned(),
                        value: Expression::new(
                            ExpressionKind::NoValue,
                            header.name_location.to_owned(),
                            choice_type_id,
                            DataType::Choices {
                                nominal_path: header.tokens.src_path.to_owned(),
                                type_id: choice_type_id,
                                generic_instance_key: None,
                            },
                            ValueMode::ImmutableReference,
                        ),
                    })
                    .map_err(|error| self.error_messages(error, string_table))?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Resolves constants and nominal member types in header dependency order.
    ///
    /// WHY: headers are already dependency-sorted; constants are parsed in that order.
    /// Struct defaults require constant-context parsing and dependency visibility gates.
    /// Trait metadata is available so trait names on fields, payloads, and constant declarations
    /// are rejected as static contracts instead of falling through to an unknown-type diagnostic.
    pub(in crate::compiler_frontend::ast) fn resolve_nominal_members_and_constants(
        &mut self,
        sorted_headers: &[Header],
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        // -------------------------------------------------
        //  Resolve constructor shell types for constants
        // -------------------------------------------------
        self.resolve_constructor_shells_for_constants(
            sorted_headers,
            trait_environment,
            string_table,
        )?;

        // -------------------
        //  Resolve constants
        // -------------------
        timing_scope_attributed_opt!(
            _constant_header_guard,
            self.context
                .timing_metric_family
                .constant_header_resolution(),
            self.context.timing_context
        );
        self.resolve_constant_headers(sorted_headers, trait_environment, string_table)?;

        // ----------------------------
        //  Resolve struct field types
        // ----------------------------
        for header in sorted_headers {
            let HeaderKind::Struct {
                generic_parameters,
                fields,
            } = &header.kind
            else {
                continue;
            };

            let visibility = self
                .binding_environment
                .visibility_for(&header.source_file)
                .map_err(|error| self.error_messages(error, string_table))?
                .clone();

            let source_file_scope = header.canonical_source_file(string_table);
            let generic_parameter_scope =
                build_generic_parameter_scope(GenericParameterScopeBuildInput {
                    generic_parameters,
                    canonical_by_local: self
                        .generic_parameter_lists_by_path
                        .get(&header.tokens.src_path)
                        .map(|registered| &registered.canonical_by_local),
                    visible_source_bindings: &visibility.visible_source_names,
                    visible_type_aliases: &visibility.visible_type_alias_names,
                    visible_external_symbols: &visibility.visible_external_symbols,
                    declaration_table: self.declaration_table.as_ref(),
                    generic_declarations_by_path: &self.module_symbols.generic_declarations_by_path,
                    string_table,
                })
                .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;
            // Rebuild member shells after constants are available so fixed-capacity
            // expressions in field types fold into final canonical TypeIds. The
            // earlier shell table is intentionally only a constructor-parsing scaffold.
            let unresolved_fields = self.unresolved_member_syntax_to_declarations(
                header,
                fields,
                MemberShellSemanticContext::StructField,
                string_table,
                SignatureTypeFallbackPolicy::StrictCapacity,
                false,
            )?;
            let template_ir_store = Rc::clone(&self.context.template_ir_store);
            let mut type_resolution_context = self.type_resolution_context_for_with_traits(
                &visibility,
                generic_parameter_scope.as_ref(),
                Some(trait_environment),
            );

            let resolved_fields = resolve_struct_field_types(
                &header.tokens.src_path,
                &unresolved_fields,
                &mut type_resolution_context,
                &template_ir_store,
                string_table,
            )
            .map_err(|error| match error {
                StructFieldResolutionError::Diagnostic(diagnostic) => {
                    self.diagnostic_messages(*diagnostic, string_table)
                }
                StructFieldResolutionError::Infrastructure(error) => {
                    self.error_messages(*error, string_table)
                }
            })?;

            // Write final canonical struct field definitions into the identity-only
            // TypeEnvironment registration.
            let field_definitions =
                self.field_definitions_from_declarations(&resolved_fields, string_table)?;

            if let Some(&type_id) = self.nominal_type_ids_by_path.get(&header.tokens.src_path) {
                self.type_environment
                    .update_struct_fields(type_id, field_definitions);
            }

            // Update the AST-owned shell table with resolved fields so later stages
            // (including constant parsing) see canonical member metadata.
            self.resolved_struct_fields_by_path.insert(
                header.tokens.src_path.to_owned(),
                resolved_fields.to_owned(),
            );

            // Every generic parameter declared on the struct must appear in at least one
            // field type; unused parameters indicate a declaration error.
            let mut used_parameters = FxHashSet::default();
            collect_type_parameter_ids_from_declarations(&resolved_fields, &mut used_parameters);
            validate_generic_parameters_used(
                generic_parameters,
                &used_parameters,
                &header.tokens.src_path,
                &header.name_location,
            )
            .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;

            // Generic structs must not contain recursive field types that reference
            // the struct itself through generic parameters.
            if !generic_parameters.is_empty() {
                for field in &resolved_fields {
                    validate_no_recursive_generic_type(
                        &header.tokens.src_path,
                        &field.value.diagnostic_type,
                        &field.value.location,
                        string_table,
                    )
                    .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;
                }
            }

            // Record the source file that owns this struct for later diagnostic rendering.
            self.struct_source_by_path.insert(
                header.tokens.src_path.to_owned(),
                source_file_scope.to_owned(),
            );
        }

        // --------------------------------------
        //  Resolve choice variant payload types
        // --------------------------------------
        for header in sorted_headers {
            let HeaderKind::Choice {
                generic_parameters,
                variants,
            } = &header.kind
            else {
                continue;
            };

            let source_file_scope = header.canonical_source_file(string_table);
            let visibility = self
                .binding_environment
                .visibility_for(&header.source_file)
                .map_err(|error| self.error_messages(error, string_table))?
                .clone();

            let generic_parameter_scope =
                build_generic_parameter_scope(GenericParameterScopeBuildInput {
                    generic_parameters,
                    canonical_by_local: self
                        .generic_parameter_lists_by_path
                        .get(&header.tokens.src_path)
                        .map(|registered| &registered.canonical_by_local),
                    visible_source_bindings: &visibility.visible_source_names,
                    visible_type_aliases: &visibility.visible_type_alias_names,
                    visible_external_symbols: &visibility.visible_external_symbols,
                    declaration_table: self.declaration_table.as_ref(),
                    generic_declarations_by_path: &self.module_symbols.generic_declarations_by_path,
                    string_table,
                })
                .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;
            // Rebuild payload shells after constants for the same reason as struct
            // fields: final semantic member types must preserve folded fixed capacities.
            let unresolved_variants = self.unresolved_choice_variants_for_header(
                header,
                variants,
                string_table,
                SignatureTypeFallbackPolicy::StrictCapacity,
                false,
            )?;
            let mut type_resolution_context = self.type_resolution_context_for_with_traits(
                &visibility,
                generic_parameter_scope.as_ref(),
                Some(trait_environment),
            );

            let resolved_variants = resolve_choice_variant_payload_types(
                &unresolved_variants,
                &mut type_resolution_context,
                string_table,
            )
            .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;

            // Every generic parameter declared on the choice must appear in at least one
            // variant payload type; unused parameters indicate a declaration error.
            let mut used_parameters = FxHashSet::default();
            collect_type_parameter_ids_from_choice_variants(
                &resolved_variants,
                &mut used_parameters,
            );
            validate_generic_parameters_used(
                generic_parameters,
                &used_parameters,
                &header.tokens.src_path,
                &header.name_location,
            )
            .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;

            // Generic choices must not contain recursive payload types that reference
            // the choice itself through generic parameters.
            if !generic_parameters.is_empty() {
                for variant in &resolved_variants {
                    if let ChoiceVariantPayload::Record { fields } = &variant.payload {
                        for field in fields {
                            validate_no_recursive_generic_type(
                                &header.tokens.src_path,
                                &field.value.diagnostic_type,
                                &field.value.location,
                                string_table,
                            )
                            .map_err(|diagnostic| {
                                self.diagnostic_messages(*diagnostic, string_table)
                            })?;
                        }
                    }
                }
            }

            // Write final canonical choice variant definitions into the identity-only
            // TypeEnvironment registration.
            let mut variant_definitions = Vec::with_capacity(resolved_variants.len());
            for (tag, variant) in resolved_variants.iter().enumerate() {
                let payload = match &variant.payload {
                    ChoiceVariantPayload::Unit => ChoiceVariantPayloadDefinition::Unit,
                    ChoiceVariantPayload::Record { fields } => {
                        let field_definitions =
                            self.field_definitions_from_declarations(fields, string_table)?;
                        ChoiceVariantPayloadDefinition::Record {
                            fields: field_definitions,
                        }
                    }
                };

                variant_definitions.push(ChoiceVariantDefinition {
                    name: variant.id,
                    tag,
                    payload,
                    location: variant.location.clone(),
                });
            }

            let Some(&choice_type_id) = self.nominal_type_ids_by_path.get(&header.tokens.src_path)
            else {
                let error = CompilerError::compiler_error(format!(
                    "Choice '{}' was not registered before resolved variant update",
                    header.tokens.src_path.to_string(string_table)
                ));
                return Err(self.error_messages(error, string_table));
            };

            self.type_environment
                .update_choice_variants(choice_type_id, variant_definitions.into_boxed_slice());

            // Update the AST-owned shell table with resolved variants for later
            // constant constructor parsing and body emission.
            self.choice_variant_shells_by_path.insert(
                header.tokens.src_path.to_owned(),
                resolved_variants.to_owned(),
            );
            self.choice_source_by_path.insert(
                header.tokens.src_path.to_owned(),
                source_file_scope.to_owned(),
            );

            // Replace the placeholder declaration with the resolved choice type.
            self.replace_declaration(Declaration {
                id: header.tokens.src_path.to_owned(),
                value: Expression::new(
                    ExpressionKind::NoValue,
                    header.name_location.to_owned(),
                    choice_type_id,
                    DataType::Choices {
                        nominal_path: header.tokens.src_path.to_owned(),
                        type_id: choice_type_id,
                        generic_instance_key: None,
                    },
                    ValueMode::ImmutableReference,
                ),
            })
            .map_err(|error| self.error_messages(error, string_table))?;
        }

        // ----------------------------
        //  Validate no recursive runtime structs
        // ----------------------------
        // Ensure no runtime struct contains itself as a field type, directly or indirectly.
        // This check runs after all field types are resolved so the full graph is visible.
        validate_no_recursive_runtime_structs(&self.resolved_struct_fields_by_path, string_table)
            .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;

        Ok(())
    }

    /// Resolve declaration-site trait bounds for nominal generic structs and choices.
    ///
    /// WHAT: patches the already-registered canonical generic parameter lists with resolved
    /// `TraitId`s once trait definitions exist.
    /// WHY: nominal identity must be registered before trait signatures resolve, but concrete
    /// generic instantiation later needs the bounds stored on the canonical TypeEnvironment list.
    pub(in crate::compiler_frontend::ast) fn resolve_nominal_generic_bounds(
        &mut self,
        sorted_headers: &[Header],
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for header in sorted_headers {
            let generic_parameters = match &header.kind {
                HeaderKind::Struct {
                    generic_parameters, ..
                }
                | HeaderKind::Choice {
                    generic_parameters, ..
                } => generic_parameters,

                _ => continue,
            };

            if generic_parameters.is_empty() {
                continue;
            }

            let visibility = self
                .binding_environment
                .visibility_for(&header.source_file)
                .map_err(|error| self.error_messages(error, string_table))?
                .clone();
            let resolved_bounds_by_local = self.resolve_generic_parameter_bounds(
                generic_parameters,
                &visibility,
                trait_environment,
                string_table,
            )?;

            if header.export_mode.is_public() {
                let owner_name = header.tokens.src_path.name().ok_or_else(|| {
                    self.error_messages(
                        CompilerError::compiler_error(
                            "Public nominal generic header had no source-path name.",
                        ),
                        string_table,
                    )
                })?;
                self.validate_public_generic_bounds(
                    owner_name,
                    generic_parameters,
                    &resolved_bounds_by_local,
                    &header.source_file,
                    trait_environment,
                    string_table,
                )?;
            }

            if let Some(registered) = self
                .generic_parameter_lists_by_path
                .get(&header.tokens.src_path)
            {
                self.type_environment.update_generic_parameter_bounds(
                    registered.list_id,
                    &resolved_bounds_by_local,
                    &registered.canonical_by_local,
                );
            }
        }

        Ok(())
    }

    /// Validate concrete bounded generic instances on declaration surfaces.
    ///
    /// WHAT: checks aliases, nominal member types, and function signatures after trait evidence
    /// has been validated.
    /// WHY: those surfaces are resolved before receiver methods and conformance evidence exist,
    /// but each concrete `Box of T` still needs visible reusable evidence at its declaration site.
    pub(in crate::compiler_frontend::ast) fn validate_nominal_generic_bound_surfaces(
        &mut self,
        sorted_headers: &[Header],
        trait_environment: &TraitEnvironment,
        trait_evidence_environment: &TraitEvidenceEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for header in sorted_headers {
            let visibility = self
                .binding_environment
                .visibility_for(&header.source_file)
                .map_err(|error| self.error_messages(error, string_table))?
                .clone();
            let validation_context = NominalBoundSurfaceValidationContext {
                visibility: &visibility,
                trait_environment,
                trait_evidence_environment,
            };

            match &header.kind {
                HeaderKind::TypeAlias { .. } => {
                    let Some(annotation) = self
                        .resolved_type_aliases_by_path
                        .get(&header.tokens.src_path)
                    else {
                        continue;
                    };
                    let type_id = match annotation.type_id {
                        Some(type_id) => type_id,
                        None => resolve_diagnostic_type_to_type_id_checked(
                            &annotation.diagnostic_type,
                            &mut self.type_environment,
                            &header.name_location,
                        )
                        .map_err(|diagnostic| {
                            self.diagnostic_messages(*diagnostic, string_table)
                        })?,
                    };

                    self.validate_nominal_generic_bound_type_id(
                        type_id,
                        header.name_location.clone(),
                        &validation_context,
                        string_table,
                    )?;
                }

                HeaderKind::Constant { .. } => {
                    let Some(declaration) =
                        self.declaration_table.get_by_path(&header.tokens.src_path)
                    else {
                        continue;
                    };
                    self.validate_nominal_generic_bound_type_id(
                        declaration.value.type_id,
                        declaration.value.location.clone(),
                        &validation_context,
                        string_table,
                    )?;
                }

                HeaderKind::Struct { .. } => {
                    let Some(fields) = self
                        .resolved_struct_fields_by_path
                        .get(&header.tokens.src_path)
                    else {
                        continue;
                    };
                    for field in fields.clone() {
                        self.validate_nominal_generic_bound_type_id(
                            field.value.type_id,
                            field.value.location,
                            &validation_context,
                            string_table,
                        )?;
                    }
                }

                HeaderKind::Choice { .. } => {
                    let Some(variants) = self
                        .choice_variant_shells_by_path
                        .get(&header.tokens.src_path)
                    else {
                        continue;
                    };
                    for variant in variants.clone() {
                        if let ChoiceVariantPayload::Record { fields } = variant.payload {
                            for field in fields {
                                self.validate_nominal_generic_bound_type_id(
                                    field.value.type_id,
                                    field.value.location,
                                    &validation_context,
                                    string_table,
                                )?;
                            }
                        }
                    }
                }

                HeaderKind::Function { .. } => {
                    let Some(resolved_signature) = self
                        .resolved_function_signatures_by_path
                        .get(&header.tokens.src_path)
                        .cloned()
                    else {
                        continue;
                    };

                    for parameter in resolved_signature.signature.parameters {
                        self.validate_nominal_generic_bound_type_id(
                            parameter.value.type_id,
                            parameter.value.location,
                            &validation_context,
                            string_table,
                        )?;
                    }

                    for return_slot in resolved_signature.signature.returns {
                        if let Some(type_id) = return_slot.type_id {
                            self.validate_nominal_generic_bound_type_id(
                                type_id,
                                header.name_location.clone(),
                                &validation_context,
                                string_table,
                            )?;
                        }
                    }
                }

                _ => {}
            }
        }

        Ok(())
    }

    fn validate_nominal_generic_bound_type_id(
        &self,
        type_id: TypeId,
        location: SourceLocation,
        validation_context: &NominalBoundSurfaceValidationContext<'_>,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let evidence_context = GenericBoundEvidenceContext::from_file_visibility(
            &self.type_environment,
            validation_context.trait_environment,
            validation_context.trait_evidence_environment,
            validation_context.visibility,
            &self.resolved_type_aliases_by_path,
        );

        validate_nominal_generic_bound_evidence(type_id, location, &evidence_context)
            .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))
    }

    /// Resolve struct field and choice variant types needed for constant constructor parsing.
    ///
    /// WHAT: runs a lightweight type-resolution pass over struct fields and choice variant
    /// payloads before constants are evaluated.
    /// WHY: constant initializers may contain struct or choice constructors; those constructors
    /// need resolved member types to validate arity and field compatibility at parse time.
    fn resolve_constructor_shells_for_constants(
        &mut self,
        sorted_headers: &[Header],
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        for header in sorted_headers {
            match &header.kind {
                HeaderKind::Struct {
                    generic_parameters, ..
                } => {
                    let visibility = self
                        .binding_environment
                        .visibility_for(&header.source_file)
                        .map_err(|error| self.error_messages(error, string_table))?
                        .clone();

                    let generic_parameter_scope =
                        build_generic_parameter_scope(GenericParameterScopeBuildInput {
                            generic_parameters,
                            canonical_by_local: self
                                .generic_parameter_lists_by_path
                                .get(&header.tokens.src_path)
                                .map(|registered| &registered.canonical_by_local),
                            visible_source_bindings: &visibility.visible_source_names,
                            visible_type_aliases: &visibility.visible_type_alias_names,
                            visible_external_symbols: &visibility.visible_external_symbols,
                            declaration_table: self.declaration_table.as_ref(),
                            generic_declarations_by_path: &self
                                .module_symbols
                                .generic_declarations_by_path,
                            string_table,
                        })
                        .map_err(|diagnostic| {
                            self.diagnostic_messages(*diagnostic, string_table)
                        })?;

                    let unresolved_fields = self
                        .resolved_struct_fields_by_path
                        .get(&header.tokens.src_path)
                        .cloned()
                        .ok_or_else(|| {
                            self.error_messages(
                                CompilerError::compiler_error(
                                    "Struct constructor shells were not registered before constant resolution.",
                                ),
                                string_table,
                            )
                        })?;

                    let resolved_fields = {
                        let mut type_resolution_context = self
                            .type_resolution_context_for_with_traits(
                                &visibility,
                                generic_parameter_scope.as_ref(),
                                Some(trait_environment),
                            );
                        resolve_struct_constructor_shell_types(
                            &header.tokens.src_path,
                            &unresolved_fields,
                            &mut type_resolution_context,
                            string_table,
                        )
                    }
                    .map_err(|error| match error {
                        StructFieldResolutionError::Diagnostic(diagnostic) => {
                            self.diagnostic_messages(*diagnostic, string_table)
                        }
                        StructFieldResolutionError::Infrastructure(error) => {
                            self.error_messages(*error, string_table)
                        }
                    })?;

                    // Store resolved constructor shell types for constant parsing.
                    self.resolved_struct_fields_by_path
                        .insert(header.tokens.src_path.to_owned(), resolved_fields);
                }

                HeaderKind::Choice {
                    generic_parameters, ..
                } => {
                    let visibility = self
                        .binding_environment
                        .visibility_for(&header.source_file)
                        .map_err(|error| self.error_messages(error, string_table))?
                        .clone();

                    let generic_parameter_scope =
                        build_generic_parameter_scope(GenericParameterScopeBuildInput {
                            generic_parameters,
                            canonical_by_local: self
                                .generic_parameter_lists_by_path
                                .get(&header.tokens.src_path)
                                .map(|registered| &registered.canonical_by_local),
                            visible_source_bindings: &visibility.visible_source_names,
                            visible_type_aliases: &visibility.visible_type_alias_names,
                            visible_external_symbols: &visibility.visible_external_symbols,
                            declaration_table: self.declaration_table.as_ref(),
                            generic_declarations_by_path: &self
                                .module_symbols
                                .generic_declarations_by_path,
                            string_table,
                        })
                        .map_err(|diagnostic| {
                            self.diagnostic_messages(*diagnostic, string_table)
                        })?;

                    let unresolved_variants = self
                        .choice_variant_shells_by_path
                        .get(&header.tokens.src_path)
                        .cloned()
                        .ok_or_else(|| {
                            self.error_messages(
                                CompilerError::compiler_error(
                                    "Choice variant shells were not registered before constant resolution.",
                                ),
                                string_table,
                            )
                        })?;

                    let resolved_variants = {
                        let mut type_resolution_context = self
                            .type_resolution_context_for_with_traits(
                                &visibility,
                                generic_parameter_scope.as_ref(),
                                Some(trait_environment),
                            );
                        resolve_choice_variant_payload_types(
                            &unresolved_variants,
                            &mut type_resolution_context,
                            string_table,
                        )
                    }
                    .map_err(|diagnostic| self.diagnostic_messages(*diagnostic, string_table))?;

                    // Store resolved constructor shell types for constant parsing.
                    self.choice_variant_shells_by_path
                        .insert(header.tokens.src_path.to_owned(), resolved_variants);
                }

                _ => {}
            }
        }

        Ok(())
    }

    fn resolve_constant_headers(
        &mut self,
        sorted_headers: &[Header],
        trait_environment: &TraitEnvironment,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let resolved_type_aliases = Rc::new(self.resolved_type_aliases_by_path.clone());
        let generic_declarations =
            Rc::new(self.module_symbols.generic_declarations_by_path.clone());

        // Constant parsing reads these side tables but does not mutate them. Clone the maps once
        // for the constants pass so const-heavy modules do not rebuild identical Rc payloads for
        // every header.
        let resolved_struct_fields_by_path = Rc::new(self.resolved_struct_fields_by_path.clone());
        let choice_variant_shells_by_path = Rc::new(self.choice_variant_shells_by_path.clone());
        let nominal_type_ids_by_path = Rc::new(self.nominal_type_ids_by_path.clone());
        let trait_environment = Rc::new(trait_environment.clone());

        add_ast_counter(
            AstCounter::ConstantPassSideTableEntriesCloned,
            resolved_type_aliases.len()
                + generic_declarations.len()
                + resolved_struct_fields_by_path.len()
                + choice_variant_shells_by_path.len()
                + nominal_type_ids_by_path.len(),
        );

        for header in sorted_headers {
            let HeaderKind::Constant { .. } = &header.kind else {
                continue;
            };

            let visibility = self
                .binding_environment
                .visibility_for(&header.source_file)
                .map_err(|error| self.error_messages(error, string_table))?;

            let declaration = parse_constant_header_declaration(
                header,
                ConstantHeaderParseContext {
                    top_level_declarations: Rc::clone(&self.declaration_table),
                    module_constants: &self.module_constants,
                    file_visibility: visibility,
                    resolved_type_aliases: Rc::clone(&resolved_type_aliases),
                    generic_declarations_by_path: Rc::clone(&generic_declarations),
                    resolved_struct_fields_by_path: Rc::clone(&resolved_struct_fields_by_path),
                    choice_variant_shells_by_path: Rc::clone(&choice_variant_shells_by_path),
                    type_environment: &mut self.type_environment,
                    nominal_type_ids_by_path: Rc::clone(&nominal_type_ids_by_path),
                    external_package_registry: &self.context.external_package_registry,
                    style_directives: self.context.style_directives,
                    project_path_resolver: self.context.project_path_resolver.clone(),
                    path_format_config: self.context.path_format_config.clone(),
                    template_const_loop_iteration_limit: self
                        .context
                        .template_const_loop_iteration_limit,
                    template_ir_store: self.context.template_ir_store.clone(),
                    build_profile: self.context.build_profile,
                    warnings: &mut self.warnings,
                    rendered_path_usages: self.rendered_path_usages.clone(),
                    string_table,
                    trait_environment: Some(Rc::clone(&trait_environment)),
                },
            )
            .map_err(|error| self.expression_error_messages(error, string_table))?;

            increment_ast_counter(AstCounter::ConstantsResolved);
            increment_ast_counter(AstCounter::ModuleConstantDeclarationClones);

            self.replace_declaration(declaration.clone())
                .map_err(|error| self.error_messages(error, string_table))?;
            self.module_constants.push(declaration);
        }

        Ok(())
    }

    /// Convert parsed signature member syntax into unresolved `Declaration` shells.
    ///
    /// WHAT: produces `Declaration` values for struct fields or choice payload fields
    /// from the shared signature-member parser.
    /// WHY: struct and choice declarations use the same surface syntax for members,
    /// but struct defaults require different diagnostics when they reference non-constant values.
    fn unresolved_member_syntax_to_declarations(
        &mut self,
        header: &Header,
        fields: &[SignatureMemberSyntax],
        member_context: MemberShellSemanticContext,
        string_table: &mut StringTable,
        fallback_policy: SignatureTypeFallbackPolicy,
        emit_warnings: bool,
    ) -> Result<Vec<Declaration>, CompilerMessages> {
        let visibility = self
            .binding_environment
            .visibility_for(&header.source_file)
            .map_err(|error| self.error_messages(error, string_table))?
            .clone();

        let field_context = self.constant_header_scope_context(header, &visibility, string_table);

        // Parse each field inside a temporary scope so that type-resolution errors
        // can be remapped to the appropriate diagnostic for struct defaults vs choice payloads.
        let conversion_result = (|| -> Result<Vec<Declaration>, ExpressionParseError> {
            let mut compatibility_cache = TypeCompatibilityCache::new();
            let mut type_interner =
                AstTypeInterner::new(&mut self.type_environment, &mut compatibility_cache);

            let mut declarations = Vec::with_capacity(fields.len());
            for field in fields {
                let declaration = signature_member_to_declaration(
                    field,
                    &header.tokens.path_syntax,
                    &field_context,
                    &mut type_interner,
                    string_table,
                    fallback_policy,
                )
                .map_err(|error| match error {
                    ExpressionParseError::Diagnostic(diagnostic) => {
                        ExpressionParseError::Diagnostic(Box::new(
                            member_shell_diagnostic_for_context(*diagnostic, member_context),
                        ))
                    }
                    ExpressionParseError::Infrastructure(error) => {
                        ExpressionParseError::Infrastructure(error)
                    }
                })?;
                declarations.push(declaration);
            }

            Ok(declarations)
        })();

        let declarations = conversion_result
            .map_err(|error| self.expression_error_messages(error, string_table))?;

        if emit_warnings {
            self.warnings.extend(field_context.take_emitted_warnings());
        }

        Ok(declarations)
    }

    /// Convert parsed choice variant syntax into `ChoiceVariant` shells with unresolved payloads.
    ///
    /// WHAT: builds `ChoiceVariant` values from header-parsed syntax, keeping payload
    /// field types as unresolved `Declaration` shells.
    /// WHY: choice variants must record their shape early so constructor parsing can
    /// check tag names and arity, while payload type resolution happens later.
    fn unresolved_choice_variants_for_header(
        &mut self,
        header: &Header,
        variants: &[ChoiceVariantSyntax],
        string_table: &mut StringTable,
        fallback_policy: SignatureTypeFallbackPolicy,
        emit_warnings: bool,
    ) -> Result<Vec<ChoiceVariant>, CompilerMessages> {
        let mut resolved_variants = Vec::with_capacity(variants.len());

        for variant in variants {
            let payload = match &variant.payload {
                ChoiceVariantPayloadSyntax::Unit => ChoiceVariantPayload::Unit,

                ChoiceVariantPayloadSyntax::Record { fields } => {
                    let declarations = self.unresolved_member_syntax_to_declarations(
                        header,
                        fields,
                        MemberShellSemanticContext::ChoicePayloadField,
                        string_table,
                        fallback_policy,
                        emit_warnings,
                    )?;
                    ChoiceVariantPayload::Record {
                        fields: declarations,
                    }
                }
            };

            resolved_variants.push(ChoiceVariant {
                id: variant.id,
                payload,
                location: variant.location.clone(),
            });
        }

        Ok(resolved_variants)
    }

    /// Build a `ScopeContext` suitable for parsing constant headers during environment construction.
    ///
    /// WHAT: assembles visibility, type aliases, and struct/choice metadata into a
    /// `ScopeContext` that constant header parsing can use.
    /// WHY: the full `AstModuleEnvironment` is not yet assembled when constants are
    /// resolved, so this helper wires up the pieces that are already available.
    fn constant_header_scope_context(
        &self,
        header: &Header,
        visibility: &FileVisibility,
        string_table: &mut StringTable,
    ) -> ScopeContext {
        let source_file_scope: InternedPath = header.canonical_source_file(string_table);

        ScopeContext::new(
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
        .with_file_visibility(Rc::new(visibility.clone()))
        .with_explicit_compile_time_constants(&self.module_constants)
        .with_resolved_type_aliases(Rc::new(self.resolved_type_aliases_by_path.clone()))
        .with_generic_declarations(Rc::new(
            self.module_symbols.generic_declarations_by_path.clone(),
        ))
        .with_resolved_struct_fields_by_path(Rc::new(self.resolved_struct_fields_by_path.clone()))
        .with_choice_variant_shells_by_path(Rc::new(self.choice_variant_shells_by_path.clone()))
        .with_nominal_type_ids_by_path(Rc::new(self.nominal_type_ids_by_path.clone()))
        .with_source_file_scope(source_file_scope)
    }
}
