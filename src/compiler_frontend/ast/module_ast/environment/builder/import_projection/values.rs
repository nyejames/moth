//! Imported aliases, constants, defaults, and folded values.

use super::*;

/// Services needed to invert one stable folded value into a fresh AST environment.
///
/// WHAT: keeps canonical-type interning, the active type environment and the active TIR store
/// explicit while allowing imported declarations and generated materialisation to share one
/// folded-value inverse projection.
/// WHY: a second recursive value converter would make defaults and constants diverge between
/// provider dependencies and generated sidecars.
pub(crate) trait FoldedValueMaterialiser {
    fn intern_canonical_type(
        &mut self,
        identity: &crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity,
        string_table: &mut StringTable,
    ) -> Result<TypeId, CompilerError>;

    fn type_environment(&self) -> &TypeEnvironment;

    fn template_ir_store(
        &self,
    ) -> Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>>;
}

impl<'context, 'services> FoldedValueMaterialiser
    for AstModuleEnvironmentBuilder<'context, 'services>
{
    fn intern_canonical_type(
        &mut self,
        identity: &crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity,
        _string_table: &mut StringTable,
    ) -> Result<TypeId, CompilerError> {
        self.intern_imported_canonical_type(identity)
    }

    fn type_environment(&self) -> &TypeEnvironment {
        &self.type_environment
    }

    fn template_ir_store(
        &self,
    ) -> Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>> {
        Rc::clone(&self.context.template_ir_store)
    }
}

impl<'context, 'services> AstModuleEnvironmentBuilder<'context, 'services> {
    pub(in crate::compiler_frontend::ast::module_ast::environment::builder) fn project_imported_alias_declarations(
        &mut self,
    ) -> Result<(), CompilerError> {
        let imported = self
            .binding_environment
            .imported_declarations_by_local_path
            .clone();

        for (local_path, origin) in imported {
            let Some(record) = self
                .binding_environment
                .imported_declarations_by_origin
                .get(&origin)
                .cloned()
            else {
                continue;
            };
            let PublicDeclarationSemantics::TransparentAlias(alias) = record.semantics else {
                continue;
            };
            let type_id = self.intern_imported_canonical_type(&alias.target_type_identity)?;
            Rc::make_mut(&mut self.resolved_type_aliases_by_path).insert(
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
            .binding_environment
            .imported_declarations_by_local_path
            .clone();
        for (local_path, origin) in imported {
            let Some(record) = self
                .binding_environment
                .imported_declarations_by_origin
                .get(&origin)
                .cloned()
            else {
                continue;
            };
            let PublicDeclarationSemantics::Constant(constant) = record.semantics else {
                continue;
            };
            let declaration =
                self.project_imported_constant(local_path, &constant, string_table)?;
            let declaration_path = declaration.id.clone();
            Rc::make_mut(&mut self.declaration_table)
                .append_for_construction(declaration)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported constant declaration path was registered more than once",
                    )
                })?;
            self.push_module_constant_path(declaration_path);
        }
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
        materialize_public_folded_value(self, folded, expected_type_id, string_table)
    }
}

/// Rebuild one stable folded value in the active module-local type and TIR environment.
pub(crate) fn materialize_public_folded_value<M: FoldedValueMaterialiser>(
    materialiser: &mut M,
    folded: &PublicFoldedValue,
    expected_type_id: TypeId,
    string_table: &mut StringTable,
) -> Result<Expression, CompilerError> {
    let kind = match folded {
        PublicFoldedValue::Int(value) => ExpressionKind::Int(*value),
        PublicFoldedValue::Float(value) => ExpressionKind::Float(value.value()),
        PublicFoldedValue::Bool(value) => ExpressionKind::Bool(*value),
        PublicFoldedValue::Char(value) => ExpressionKind::Char(*value),
        PublicFoldedValue::String(value) => ExpressionKind::StringSlice(string_table.intern(value)),
        PublicFoldedValue::ConstTemplate(template) => {
            let template_ir_store = materialiser.template_ir_store();
            ExpressionKind::Template(Box::new(materialize_public_const_template(
                template,
                &template_ir_store,
                string_table,
                Default::default(),
            )?))
        }
        PublicFoldedValue::Collection(values) => {
            let element_type_id = materialiser
                .type_environment()
                .collection_shape(expected_type_id)
                .map(|shape| shape.element_type)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded collection value has a non-collection canonical type",
                    )
                })?;
            let mut items = Vec::with_capacity(values.len());
            for value in values {
                items.push(materialize_public_folded_value(
                    materialiser,
                    value,
                    element_type_id,
                    string_table,
                )?);
            }
            ExpressionKind::Collection(items)
        }
        PublicFoldedValue::Record(fields) => {
            let definitions = materialiser
                .type_environment()
                .fields_for(expected_type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded record value has a canonical type without fields",
                    )
                })?
                .to_vec();
            let projected =
                materialize_public_folded_fields(materialiser, fields, &definitions, string_table)?;
            ExpressionKind::StructInstance(projected)
        }
        PublicFoldedValue::Choice {
            type_identity,
            variant_name,
            fields,
        } => {
            let choice_type_id = materialiser.intern_canonical_type(type_identity, string_table)?;
            if choice_type_id != expected_type_id {
                return Err(CompilerError::compiler_error(
                    "Imported folded choice value disagrees with its declared canonical type",
                ));
            }
            let variants = materialiser
                .type_environment()
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
            let projected = materialize_public_folded_fields(
                materialiser,
                fields,
                field_definitions,
                string_table,
            )?;
            let nominal_path = materialiser
                .type_environment()
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
            let start = materialize_public_folded_value(
                materialiser,
                start,
                builtin_type_ids::INT,
                string_table,
            )?;
            let end = materialize_public_folded_value(
                materialiser,
                end,
                builtin_type_ids::INT,
                string_table,
            )?;
            ExpressionKind::Range(Box::new(start), Box::new(end))
        }
        PublicFoldedValue::OptionSome(value) => {
            let inner_type_id = materialiser
                .type_environment()
                .option_inner_type(expected_type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Imported folded option value has a non-option canonical type",
                    )
                })?;
            let inner =
                materialize_public_folded_value(materialiser, value, inner_type_id, string_table)?;
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
        diagnostic_type_spelling(expected_type_id, materialiser.type_environment()),
        ValueMode::ImmutableReference,
    ))
}

fn materialize_public_folded_fields<M: FoldedValueMaterialiser>(
    materialiser: &mut M,
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
            value: materialize_public_folded_value(
                materialiser,
                &field.value,
                definition.type_id,
                string_table,
            )?,
        });
    }
    Ok(projected)
}

/// Rebuilds one stable const-template value in the active module-local TIR store.
///
/// Imported constants and generated sidecars share this inverse projection so neither path
/// carries a provider or declaring-module TIR identity into a fresh AST compilation.
pub(in crate::compiler_frontend::ast) fn materialize_public_const_template(
    template: &PublicConstTemplate,
    store_handle: &Rc<RefCell<crate::compiler_frontend::ast::templates::tir::TemplateIrStore>>,
    string_table: &mut StringTable,
    location: crate::compiler_frontend::tokenizer::tokens::SourceLocation,
) -> Result<Template, CompilerError> {
    let mut store = store_handle.borrow_mut();
    let root =
        materialize_public_const_template_in_store(template, &mut store, string_table, &location)?;

    Ok(Template {
        tir_reference: TemplateTirReference {
            root,
            phase: TemplateTirPhase::Finalized,
            context: TemplateViewContext::default(),
        },
        location,
    })
}

fn materialize_public_const_template_in_store(
    template: &PublicConstTemplate,
    store: &mut crate::compiler_frontend::ast::templates::tir::TemplateIrStore,
    string_table: &mut StringTable,
    location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
) -> Result<crate::compiler_frontend::ast::templates::tir::TemplateIrId, CompilerError> {
    let mut children = Vec::with_capacity(template.pieces.len());

    for piece in &template.pieces {
        let node = match piece {
            PublicConstTemplatePiece::Text(text) => {
                let text_id = string_table.intern(text);
                store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::Text {
                        text: text_id,
                        byte_len: text.len(),
                        origin: TemplateSegmentOrigin::Head,
                    },
                    location.clone(),
                ))
            }
            PublicConstTemplatePiece::Slot(slot) => {
                let placeholder =
                    materialize_public_const_template_slot(slot, store, string_table, location)?;
                store.push_node(TemplateIrNode::new(
                    TemplateIrNodeKind::Slot { placeholder },
                    location.clone(),
                ))
            }
        };
        children.push(node);
    }

    let root = store.push_node(TemplateIrNode::new(
        TemplateIrNodeKind::Sequence { children },
        location.clone(),
    ));
    let kind = match &template.kind {
        PublicConstTemplateKind::Wrapper => TemplateType::String,
        PublicConstTemplateKind::SlotInsert(key) => {
            TemplateType::SlotInsert(materialize_public_slot_key(key, string_table))
        }
    };
    let summary = summarize_existing_root(store, root)?;
    let mut template_ir = TemplateIr::new(root, Style::default(), kind, summary, location.clone());
    let conditional_wrappers = materialize_public_wrapper_references(
        &template.conditional_child_wrappers,
        store,
        string_table,
        location,
    )?;
    if !conditional_wrappers.is_empty() {
        template_ir.conditional_child_wrapper_set =
            Some(store.push_or_reuse_wrapper_set(conditional_wrappers));
    }

    Ok(store.push_template(template_ir))
}

fn materialize_public_const_template_slot(
    slot: &PublicConstTemplateSlot,
    store: &mut crate::compiler_frontend::ast::templates::tir::TemplateIrStore,
    string_table: &mut StringTable,
    location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
) -> Result<TirSlotPlaceholder, CompilerError> {
    let applied = materialize_public_wrapper_references(
        &slot.applied_child_wrappers,
        store,
        string_table,
        location,
    )?;
    let child =
        materialize_public_wrapper_references(&slot.child_wrappers, store, string_table, location)?;
    let applied_set = (!applied.is_empty()).then(|| store.push_or_reuse_wrapper_set(applied));
    let child_set = (!child.is_empty()).then(|| store.push_or_reuse_wrapper_set(child));

    Ok(TirSlotPlaceholder::with_wrapper_sets(
        materialize_public_slot_key(&slot.key, string_table),
        store.next_slot_occurrence_id(),
        location.clone(),
        applied_set,
        child_set,
        slot.skip_parent_child_wrappers,
    ))
}

fn materialize_public_wrapper_references(
    wrappers: &[PublicConstTemplate],
    store: &mut crate::compiler_frontend::ast::templates::tir::TemplateIrStore,
    string_table: &mut StringTable,
    location: &crate::compiler_frontend::tokenizer::tokens::SourceLocation,
) -> Result<Vec<TemplateWrapperReference>, CompilerError> {
    let mut references = Vec::with_capacity(wrappers.len());
    for wrapper in wrappers {
        let root =
            materialize_public_const_template_in_store(wrapper, store, string_table, location)?;
        references.push(TemplateWrapperReference::new(
            root,
            TemplateTirPhase::Finalized,
            TemplateViewContext::default(),
        ));
    }
    Ok(references)
}

fn materialize_public_slot_key(
    key: &PublicTemplateSlotKey,
    string_table: &mut StringTable,
) -> SlotKey {
    match key {
        PublicTemplateSlotKey::Default => SlotKey::Default,
        PublicTemplateSlotKey::Named(name) => SlotKey::Named(string_table.intern(name)),
        PublicTemplateSlotKey::Positional(position) => SlotKey::Positional(*position),
    }
}
