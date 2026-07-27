//! Imported aliases, constants, defaults, and folded values.

use super::*;

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    pub(in crate::compiler_frontend::ast::module_ast::environment::builder) fn project_imported_alias_declarations(
        &mut self,
    ) -> Result<(), CompilerError> {
        let imported = self
            .import_environment
            .imported_declarations_by_local_path
            .clone();

        for (local_path, record) in imported {
            let PublicDeclarationSemantics::TransparentAlias(alias) = record.semantics else {
                continue;
            };
            let type_id = self.intern_imported_canonical_type(&alias.target_type_identity)?;
            self.resolved_type_aliases_by_path.insert(
                local_path,
                ResolvedTypeAnnotation {
                    source_ref: ParsedTypeRef::Inferred,
                    diagnostic_type: diagnostic_type_spelling(type_id, &self.type_environment),
                    type_id: Some(type_id),
                },
            );
        }

        Ok(())
    }

    pub(in crate::compiler_frontend::ast::module_ast::environment::builder) fn project_imported_constant_declarations(
        &mut self,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerError> {
        let imported = self
            .import_environment
            .imported_declarations_by_local_path
            .clone();
        let mut declarations = self.declaration_table.iter().cloned().collect::<Vec<_>>();

        for (local_path, record) in imported {
            let PublicDeclarationSemantics::Constant(constant) = record.semantics else {
                continue;
            };
            let declaration =
                self.project_imported_constant(local_path, &constant, string_table)?;
            self.module_constants.push(declaration.clone());
            declarations.push(declaration);
        }

        self.declaration_table = Rc::new(TopLevelDeclarationTable::new(declarations));
        Ok(())
    }

    fn project_imported_constant(
        &mut self,
        local_path: InternedPath,
        constant: &PublicConstantSemantics,
        string_table: &mut StringTable,
    ) -> Result<Declaration, CompilerError> {
        let type_id = self.intern_imported_canonical_type(&constant.type_identity)?;
        let value =
            self.project_imported_folded_value(&constant.folded_value, type_id, string_table)?;
        Ok(Declaration {
            id: local_path,
            value,
        })
    }

    pub(super) fn project_imported_folded_value(
        &mut self,
        folded: &PublicFoldedValue,
        expected_type_id: TypeId,
        string_table: &mut StringTable,
    ) -> Result<Expression, CompilerError> {
        let kind = match folded {
            PublicFoldedValue::Int(value) => ExpressionKind::Int(*value),
            PublicFoldedValue::Float(value) => ExpressionKind::Float(value.value()),
            PublicFoldedValue::Bool(value) => ExpressionKind::Bool(*value),
            PublicFoldedValue::Char(value) => ExpressionKind::Char(*value),
            PublicFoldedValue::String(value) => {
                ExpressionKind::StringSlice(string_table.intern(value))
            }
            PublicFoldedValue::Collection(values) => {
                let element_type_id = self
                    .type_environment
                    .collection_shape(expected_type_id)
                    .map(|shape| shape.element_type)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported folded collection value has a non-collection canonical type",
                        )
                    })?;
                let mut items = Vec::with_capacity(values.len());
                for value in values {
                    items.push(self.project_imported_folded_value(
                        value,
                        element_type_id,
                        string_table,
                    )?);
                }
                ExpressionKind::Collection(items)
            }
            PublicFoldedValue::Record(fields) => {
                let definitions = self
                    .type_environment
                    .fields_for(expected_type_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported folded record value has a canonical type without fields",
                        )
                    })?
                    .to_vec();
                let projected =
                    self.project_imported_folded_fields(fields, &definitions, string_table)?;
                ExpressionKind::StructInstance(projected)
            }
            PublicFoldedValue::Choice {
                type_identity,
                variant_name,
                fields,
            } => {
                let choice_type_id = self.intern_imported_canonical_type(type_identity)?;
                if choice_type_id != expected_type_id {
                    return Err(CompilerError::compiler_error(
                        "Imported folded choice value disagrees with its declared canonical type",
                    ));
                }
                let variants = self
                    .type_environment
                    .variants_for(choice_type_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported folded choice value has a non-choice canonical type",
                        )
                    })?;
                let variant = variants
                    .iter()
                    .find(|variant| string_table.resolve(variant.name) == variant_name)
                    .cloned()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Imported folded choice value names unknown variant '{variant_name}'"
                        ))
                    })?;
                let field_definitions = match &variant.payload {
                    ChoiceVariantPayloadDefinition::Unit => &[][..],
                    ChoiceVariantPayloadDefinition::Record { fields } => fields.as_ref(),
                };
                let projected =
                    self.project_imported_folded_fields(fields, field_definitions, string_table)?;
                let nominal_path = self
                    .type_environment
                    .nominal_path(choice_type_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported folded choice type has no consumer-local nominal path",
                        )
                    })?
                    .clone();
                ExpressionKind::ChoiceConstruct {
                    nominal_path,
                    tag: variant.tag,
                    fields: projected,
                }
            }
            PublicFoldedValue::Range { start, end } => {
                let start =
                    self.project_imported_folded_value(start, builtin_type_ids::INT, string_table)?;
                let end =
                    self.project_imported_folded_value(end, builtin_type_ids::INT, string_table)?;
                ExpressionKind::Range(Box::new(start), Box::new(end))
            }
            PublicFoldedValue::OptionSome(value) => {
                let inner_type_id = self
                    .type_environment
                    .option_inner_type(expected_type_id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Imported folded option value has a non-option canonical type",
                        )
                    })?;
                let inner =
                    self.project_imported_folded_value(value, inner_type_id, string_table)?;
                ExpressionKind::Coerced {
                    value: Box::new(inner),
                    to_type: expected_type_id,
                }
            }
            PublicFoldedValue::OptionNone => ExpressionKind::OptionNone,
        };

        Ok(Expression::new(
            kind,
            Default::default(),
            expected_type_id,
            diagnostic_type_spelling(expected_type_id, &self.type_environment),
            ValueMode::ImmutableReference,
        ))
    }

    fn project_imported_folded_fields(
        &mut self,
        fields: &[PublicFoldedField],
        definitions: &[FieldDefinition],
        string_table: &mut StringTable,
    ) -> Result<Vec<Declaration>, CompilerError> {
        if fields.len() != definitions.len() {
            return Err(CompilerError::compiler_error(
                "Imported folded aggregate value has a field count inconsistent with its canonical type",
            ));
        }

        let mut projected = Vec::with_capacity(fields.len());
        for (field, definition) in fields.iter().zip(definitions) {
            let definition_name = definition
                .name
                .name_str(string_table)
                .ok_or_else(|| CompilerError::compiler_error("Imported field path has no name"))?;
            if definition_name != field.name {
                return Err(CompilerError::compiler_error(format!(
                    "Imported folded aggregate field '{}' does not match canonical field '{definition_name}'",
                    field.name
                )));
            }
            projected.push(Declaration {
                id: definition.name.clone(),
                value: self.project_imported_folded_value(
                    &field.value,
                    definition.type_id,
                    string_table,
                )?,
            });
        }
        Ok(projected)
    }
}
