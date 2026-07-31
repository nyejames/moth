//! Imported concrete callable signature and carrier projection.

use super::*;

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

        for (local_path, record) in imported {
            let PublicDeclarationSemantics::Function(function) = record.semantics else {
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
                let OriginDeclarationId::Function(origin) = record.origin else {
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

                self.generic_function_templates_by_path.insert(
                    local_path.clone(),
                    GenericFunctionTemplate {
                        function_path: local_path,
                        source_file: InternedPath::new(),
                        declaration_identity: Some(
                            crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity::Public(origin),
                        ),
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
                    summary: header_contract.summary.clone(),
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

        for (imported_type_path, record) in imported {
            let OriginDeclarationId::Type(receiver_origin) = &record.origin else {
                continue;
            };
            let methods = match &record.semantics {
                PublicDeclarationSemantics::Struct(structure) => &structure.receiver_methods,
                PublicDeclarationSemantics::Choice(choice) => &choice.receiver_methods,
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

            for method in methods {
                if !matches!(method.category, PublicReceiverMethodCategory::ConcreteLocal) {
                    continue;
                }
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
                        signature,
                    },
                );

                let header_contract = self
                    .import_environment
                    .imported_functions_by_local_path
                    .get(&method_path)
                    .ok_or_else(|| {
                        CompilerMessages::from_error_ref(
                            CompilerError::compiler_error(format!(
                                "Imported concrete receiver method '{}' has no header-stage call contract",
                                method.method_origin.defining_name()
                            )),
                            string_table,
                        )
                    })?;
                self.projected_imported_functions_by_local_path.insert(
                    method_path,
                    AstImportedFunctionContract {
                        target: header_contract.target.clone(),
                        summary: header_contract.summary.clone(),
                        fallible_carrier_type_id,
                    },
                );
            }
        }

        Ok(())
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
