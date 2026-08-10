//! Imported concrete callable signature and carrier projection.

use super::*;

use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::nominal::imported_nominal_path;
use crate::compiler_frontend::canonical_type_identity::GenericDeclarationOrigin;
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    pub(in crate::compiler_frontend::ast::module_ast::environment::builder) fn project_imported_function_declarations(
        &mut self,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let imported = self
            .import_environment
            .imported_declarations_by_local_path
            .clone();

        let mut projected_declarations = self.declaration_table.iter().cloned().collect::<Vec<_>>();

        for (local_path, origin) in imported {
            let Some(record) = self
                .import_environment
                .imported_declarations_by_origin
                .get(&origin)
                .cloned()
            else {
                continue;
            };
            let PublicDeclarationSemantics::Function(function) = &record.semantics else {
                continue;
            };
            let generic_parameter_list_id = match &function.category {
                PublicFunctionCategory::ConcreteLocal => None,
                PublicFunctionCategory::GenericTemplate(descriptor) => self
                    .register_imported_generic_parameters(
                        &descriptor.generic_parameters,
                        string_table,
                    )
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
            };

            let (signature, function_type_id, fallible_carrier_type_id) = self
                .project_imported_callable_signature(
                    &local_path,
                    &function.parameters,
                    &function.returns,
                    function.error_return.as_ref(),
                    string_table,
                )
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
            let diagnostic_type = DataType::Function(Box::new(None), signature.clone());
            let declaration = Declaration {
                id: local_path.clone(),
                value: Expression::new(
                    ExpressionKind::NoValue,
                    Default::default(),
                    function_type_id,
                    diagnostic_type,
                    ValueMode::ImmutableReference,
                ),
            };
            projected_declarations.push(declaration);
            self.resolved_function_signatures_by_path.insert(
                local_path.clone(),
                ResolvedFunctionSignature {
                    receiver: None,
                    signature: signature.clone(),
                },
            );

            if let PublicFunctionCategory::GenericTemplate(_) = &function.category {
                let OriginDeclarationId::Function(origin) = origin else {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "Imported generic function declaration has no function origin",
                        ),
                        string_table,
                    ));
                };
                let generic_parameter_list_id = generic_parameter_list_id.ok_or_else(|| {
                    CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "Imported generic function declaration has no projected parameter list",
                        ),
                        string_table,
                    )
                })?;
                let generic_parameter_owner =
                    GenericDeclarationOrigin::free_function(origin.clone())
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

                self.generic_function_templates_by_path.insert(
                    local_path.clone(),
                    GenericFunctionTemplate {
                        function_path: local_path,
                        source_file: InternedPath::new(),
                        declaration_identity: Some(
                            crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity::Public(origin),
                        ),
                        generic_parameter_owner: Some(generic_parameter_owner),
                        generic_parameter_list_id,
                        signature,
                        body_tokens: None,
                        declaration_location: Default::default(),
                    },
                );
                continue;
            }

            let header_contract = self
                .import_environment
                .imported_functions_by_local_path
                .get(&local_path)
                .ok_or_else(|| {
                    CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "Imported concrete function declaration has no header-stage call contract",
                        ),
                        string_table,
                    )
                })?;
            self.projected_imported_functions_by_local_path.insert(
                local_path,
                AstImportedFunctionContract {
                    target: header_contract.target.clone(),
                    summary: self
                        .import_environment
                        .imported_call_summaries_by_origin
                        .get(&summary_origin(header_contract).map_err(|error| {
                            CompilerMessages::from_error_ref(error, string_table)
                        })?)
                        .cloned()
                        .ok_or_else(|| {
                            CompilerMessages::from_error_ref(
                                CompilerError::compiler_error(
                                    "Imported concrete function has no shared call summary",
                                ),
                                string_table,
                            )
                        })?,
                    fallible_carrier_type_id,
                },
            );
        }

        self.declaration_table = Rc::new(TopLevelDeclarationTable::new(projected_declarations));

        Ok(())
    }

    pub(in crate::compiler_frontend::ast::module_ast::environment::builder) fn project_imported_receiver_method_declarations(
        &mut self,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerMessages> {
        let imported = self
            .import_environment
            .imported_declarations_by_local_path
            .clone();
        let mut imported = imported.into_iter().collect::<Vec<_>>();
        let mut bound_type_origins = imported
            .iter()
            .filter_map(|(_, origin)| match origin {
                OriginDeclarationId::Type(origin) => Some(origin.clone()),
                _ => None,
            })
            .collect::<FxHashSet<_>>();
        for (origin, record) in &self.import_environment.imported_declarations_by_origin {
            let OriginDeclarationId::Type(type_origin) = origin else {
                continue;
            };
            if !self.imported_type_ids_by_origin.contains_key(type_origin)
                || !matches!(
                    record.semantics,
                    PublicDeclarationSemantics::Struct(_) | PublicDeclarationSemantics::Choice(_)
                )
                || !bound_type_origins.insert(type_origin.clone())
            {
                continue;
            }
            // Evidence-only nominal targets have no authored local alias in the generated
            // visibility table. Project their receiver catalog under the deterministic imported
            // nominal path so stable evidence method origins still resolve to a callable path.
            imported.push((
                imported_nominal_path(type_origin, string_table),
                origin.clone(),
            ));
        }

        for (imported_type_path, origin) in imported {
            let OriginDeclarationId::Type(receiver_origin) = &origin else {
                continue;
            };
            let Some(record) = self
                .import_environment
                .imported_declarations_by_origin
                .get(&origin)
                .cloned()
            else {
                continue;
            };
            let (methods, generic_parameter_surfaces) = match &record.semantics {
                PublicDeclarationSemantics::Struct(structure) => {
                    (&structure.receiver_methods, &structure.generic_parameters)
                }
                PublicDeclarationSemantics::Choice(choice) => {
                    (&choice.receiver_methods, &choice.generic_parameters)
                }
                _ => continue,
            };
            let Some(receiver_type_id) = self
                .imported_type_ids_by_origin
                .get(receiver_origin)
                .copied()
            else {
                continue;
            };
            let receiver = match receiver_origin.category() {
                crate::compiler_frontend::semantic_identity::OriginTypeCategory::Struct => {
                    let path = self
                        .type_environment
                        .struct_definition_for(receiver_type_id)
                        .ok_or_else(|| {
                            CompilerMessages::from_error_ref(
                                CompilerError::compiler_error(
                                    "Imported receiver struct has no registered nominal definition",
                                ),
                                string_table,
                            )
                        })?
                        .path
                        .clone();
                    crate::compiler_frontend::datatypes::ReceiverKey::Struct(path)
                }
                crate::compiler_frontend::semantic_identity::OriginTypeCategory::Choice => {
                    let path = self
                        .type_environment
                        .choice_definition_for(receiver_type_id)
                        .ok_or_else(|| {
                            CompilerMessages::from_error_ref(
                                CompilerError::compiler_error(
                                    "Imported receiver choice has no registered nominal definition",
                                ),
                                string_table,
                            )
                        })?
                        .path
                        .clone();
                    crate::compiler_frontend::datatypes::ReceiverKey::Choice(path)
                }
                crate::compiler_frontend::semantic_identity::OriginTypeCategory::TransparentAlias => {
                    continue;
                }
            };

            let receiver_generic_parameter_list_id = self
                .register_imported_generic_parameters(generic_parameter_surfaces, string_table)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

            for method in methods {
                let method_name = string_table.intern(method.method_origin.defining_name());
                let method_path = imported_type_path.append(method_name);
                let (signature, _, fallible_carrier_type_id) = self
                    .project_imported_callable_signature(
                        &method_path,
                        &method.parameters,
                        &method.returns,
                        method.error_return.as_ref(),
                        string_table,
                    )
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                self.resolved_function_signatures_by_path.insert(
                    method_path.clone(),
                    ResolvedFunctionSignature {
                        receiver: Some(receiver.clone()),
                        signature: signature.clone(),
                    },
                );

                if matches!(
                    method.category,
                    PublicReceiverMethodCategory::GenericTemplate
                ) {
                    if self
                        .projected_imported_receiver_methods_by_local_path
                        .insert(method_path.clone(), method.method_origin.clone())
                        .is_some()
                    {
                        return Err(CompilerMessages::from_error_ref(
                            CompilerError::compiler_error(
                                "Imported receiver method path was projected more than once",
                            ),
                            string_table,
                        ));
                    }
                    self.index_imported_receiver_method_path(
                        method.method_origin.clone(),
                        method_path.clone(),
                        string_table,
                    );
                    let generic_parameter_list_id =
                        receiver_generic_parameter_list_id.ok_or_else(|| {
                            CompilerMessages::from_error_ref(
                                CompilerError::compiler_error(
                                    "Imported generic receiver method has no receiver generic parameter list",
                                ),
                                string_table,
                            )
                        })?;
                    let generic_parameter_owner = GenericDeclarationOrigin::nominal_type(
                        receiver_origin.clone(),
                    )
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                    self.generic_function_templates_by_path.insert(
                        method_path,
                        GenericFunctionTemplate {
                            function_path: imported_type_path.append(method_name),
                            source_file: InternedPath::new(),
                            declaration_identity: Some(GeneratedDeclarationIdentity::Public(
                                method.method_origin.clone(),
                            )),
                            generic_parameter_owner: Some(generic_parameter_owner),
                            generic_parameter_list_id,
                            signature,
                            body_tokens: None,
                            declaration_location: Default::default(),
                        },
                    );
                    continue;
                }

                let header_contract = self
                    .import_environment
                    .imported_functions_by_local_path
                    .get(&method_path)
                    .cloned();
                let Some(header_contract) = header_contract else {
                    // A transparent alias or an evidence-only nominal target may carry the
                    // receiver declaration without re-exporting a concrete executable method.
                    // Such a method is not callable through this local path and must not turn a
                    // missing optional projection into an internal publication failure.
                    continue;
                };
                if self
                    .projected_imported_receiver_methods_by_local_path
                    .insert(method_path.clone(), method.method_origin.clone())
                    .is_some()
                {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(
                            "Imported receiver method path was projected more than once",
                        ),
                        string_table,
                    ));
                }
                self.index_imported_receiver_method_path(
                    method.method_origin.clone(),
                    method_path.clone(),
                    string_table,
                );
                self.projected_imported_functions_by_local_path.insert(
                    method_path,
                    AstImportedFunctionContract {
                        target: header_contract.target.clone(),
                        summary: self
                            .import_environment
                            .imported_call_summaries_by_origin
                            .get(&summary_origin(&header_contract).map_err(|error| {
                                CompilerMessages::from_error_ref(error, string_table)
                            })?)
                            .cloned()
                            .ok_or_else(|| {
                                CompilerMessages::from_error_ref(
                                    CompilerError::compiler_error(
                                        "Imported receiver method has no shared call summary",
                                    ),
                                    string_table,
                                )
                            })?,
                        fallible_carrier_type_id,
                    },
                );
            }
        }

        Ok(())
    }

    fn index_imported_receiver_method_path(
        &mut self,
        method_origin: crate::compiler_frontend::semantic_identity::OriginFunctionId,
        method_path: InternedPath,
        string_table: &StringTable,
    ) {
        let replace = self
            .imported_receiver_method_paths_by_origin
            .get(&method_origin)
            .is_none_or(|existing| {
                method_path.to_string(string_table) < existing.to_string(string_table)
            });
        if replace {
            self.imported_receiver_method_paths_by_origin
                .insert(method_origin, method_path);
        }
    }

    fn project_imported_callable_signature(
        &mut self,
        function_path: &InternedPath,
        parameter_surfaces: &[PublicParameterTypeSlot],
        return_surfaces: &[PublicReturnTypeSlot],
        error_surface: Option<&CanonicalTypeIdentity>,
        string_table: &mut StringTable,
    ) -> Result<
        (
            crate::compiler_frontend::ast::statements::functions::FunctionSignature,
            TypeId,
            Option<TypeId>,
        ),
        CompilerError,
    > {
        use crate::compiler_frontend::ast::statements::functions::{
            FunctionSignature, ReturnChannel, ReturnSlot,
        };

        let mut parameters = Vec::with_capacity(parameter_surfaces.len());
        let mut parameter_type_ids = Vec::with_capacity(parameter_surfaces.len());
        for (index, parameter) in parameter_surfaces.iter().enumerate() {
            let type_id = self.intern_imported_canonical_type(&parameter.type_identity)?;
            let diagnostic_type = diagnostic_type_spelling(type_id, &self.type_environment);
            let name = parameter
                .name
                .as_deref()
                .map(|name| string_table.intern(name))
                .unwrap_or_else(|| string_table.intern(&format!("parameter_{index}")));
            let value_mode = match parameter.access {
                PublicCallParameterAccess::Shared => ValueMode::ImmutableReference,
                PublicCallParameterAccess::Mutable => ValueMode::MutableReference,
                PublicCallParameterAccess::Reactive => ValueMode::ImmutableReference,
            };
            let mut value = Expression::new(
                ExpressionKind::NoValue,
                Default::default(),
                type_id,
                diagnostic_type,
                value_mode.clone(),
            );
            if let Some(default) = &parameter.folded_default {
                value = self.project_imported_folded_value(default, type_id, string_table)?;
                value.value_mode = value_mode;
            }
            if parameter.access == PublicCallParameterAccess::Reactive {
                value.reactive_source = Some(ReactiveSource {
                    path: function_path.append(name),
                    kind: ReactiveSourceKind::Parameter,
                });
            }
            parameters.push(Declaration {
                id: function_path.append(name),
                value,
            });
            parameter_type_ids.push(type_id);
        }

        let mut returns = Vec::new();
        let mut return_type_ids = Vec::new();
        for returned in return_surfaces {
            let type_id = self.intern_imported_canonical_type(&returned.type_identity)?;
            let diagnostic_type = diagnostic_type_spelling(type_id, &self.type_environment);
            let mut slot = ReturnSlot::success(diagnostic_type);
            slot.type_id = Some(type_id);
            returns.push(slot);
            return_type_ids.push(type_id);
        }

        let error_return = if let Some(error_identity) = error_surface {
            let type_id = self.intern_imported_canonical_type(error_identity)?;
            let diagnostic_type = diagnostic_type_spelling(type_id, &self.type_environment);
            returns.push(ReturnSlot {
                value: diagnostic_type,
                type_id: Some(type_id),
                reactive_template: None,
                channel: ReturnChannel::Error,
            });
            Some(type_id)
        } else {
            None
        };

        let fallible_carrier_type_id = error_return.map(|error_type_id| {
            let success_type_id = match return_type_ids.as_slice() {
                [] => builtin_type_ids::NONE,
                [single] => *single,
                many => self.type_environment.intern_tuple(many.to_vec()),
            };
            self.type_environment
                .intern_fallible_carrier(success_type_id, error_type_id)
        });
        let function_type_id = self.type_environment.intern_function(FunctionTypeKey {
            parameters: parameter_type_ids.into_boxed_slice(),
            returns: return_type_ids.into_boxed_slice(),
            error_return,
        });
        Ok((
            FunctionSignature {
                parameters,
                returns,
            },
            function_type_id,
            fallible_carrier_type_id,
        ))
    }
}

/// The stable function origin whose shared call summary a header-stage contract references.
fn summary_origin(
    contract: &crate::compiler_frontend::headers::import_environment::ImportedFunctionContract,
) -> Result<crate::compiler_frontend::semantic_identity::OriginFunctionId, CompilerError> {
    match &contract.target {
        crate::compiler_frontend::headers::import_environment::SourceFunctionTarget::Imported {
            origin,
            ..
        } => Ok(origin.clone()),
        _ => Err(CompilerError::compiler_error(
            "Header-stage imported function contract must target an imported origin",
        )),
    }
}
