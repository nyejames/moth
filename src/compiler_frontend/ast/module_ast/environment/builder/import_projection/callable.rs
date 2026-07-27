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
            if !matches!(function.category, PublicFunctionCategory::ConcreteLocal) {
                continue;
            }

            let (signature, function_type_id, fallible_carrier_type_id) = self
                .project_imported_function_signature(&local_path, &function, string_table)
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
                    signature,
                },
            );

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

    fn project_imported_function_signature(
        &mut self,
        function_path: &InternedPath,
        function: &PublicFunctionSemantics,
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
            FunctionReturn, FunctionSignature, ReturnChannel, ReturnSlot,
        };

        let mut parameters = Vec::with_capacity(function.parameters.len());
        let mut parameter_type_ids = Vec::with_capacity(function.parameters.len());
        for (index, parameter) in function.parameters.iter().enumerate() {
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
        for returned in &function.returns {
            let type_id = self.intern_imported_canonical_type(&returned.type_identity)?;
            let diagnostic_type = diagnostic_type_spelling(type_id, &self.type_environment);
            let mut slot = ReturnSlot::success(FunctionReturn::Value(diagnostic_type));
            slot.type_id = Some(type_id);
            returns.push(slot);
            return_type_ids.push(type_id);
        }

        let error_return = if let Some(error_identity) = &function.error_return {
            let type_id = self.intern_imported_canonical_type(error_identity)?;
            let diagnostic_type = diagnostic_type_spelling(type_id, &self.type_environment);
            returns.push(ReturnSlot {
                value: FunctionReturn::Value(diagnostic_type),
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
