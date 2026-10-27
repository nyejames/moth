//! Declaration and symbol-registration helpers for HIR statement lowering.
//!
//! WHAT: owns top-level declaration registration, module-constant lowering, and local creation.
//! WHY: these paths build the HIR symbol tables and compile-time data pool that later control-flow
//! lowering depends on.
//!
//! ## Diagnostic boundary
//!
//! `InfrastructureFailure::hir_transformation` in this module means an internal
//! HIR transformation or lowering invariant failure only. Normal user-facing source failures
//! must be emitted during AST processing or earlier.
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::source::SourceSpan;

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationValue, NodeKind};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValueId, ConstValueStore, ConstValueVisit,
};
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionStore;
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnChannel};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::TypeId;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::generic_identity_bridge::TypeIdentityKey;
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::constants::{HirConstField, HirConstValue, HirModuleConst};
use crate::compiler_frontend::hir::expression_store::HirVariantFieldRange;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirVariantCarrier, HirVariantField, OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_side_table::{HirLocalOriginKind, HirLocation};
use crate::compiler_frontend::hir::ids::{FieldId, HirValueId, LocalId};
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::HirWriteTarget;
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;

impl<'a> HirBuilder<'a> {
    // WHAT: pre-registers all structs and functions before any HIR body lowering starts.
    // WHY: later statement and expression lowering relies on complete symbol tables for stable ID lookups.
    pub(crate) fn prepare_hir_declarations(
        &mut self,
        ast: &Ast,
    ) -> Result<(), HirConstructionFailure> {
        // Register choices FIRST so struct and function signature lowering can resolve them.
        // WHY: choices are nominal types discovered from AST declarations. Pre-registering
        //      them before expression lowering ensures `resolve_choice_id` is a pure
        //      lookup and never needs lazy creation.
        for choice_def in &ast.choice_definitions {
            self.register_choice_id(&choice_def.nominal_path)?;
        }

        for definition in &ast.imported_struct_definitions {
            self.register_struct_declaration(&definition.nominal_path, &None)?;
        }

        for node in &ast.nodes {
            if let NodeKind::StructDefinition(name, _) = &node.kind {
                self.register_struct_declaration(name, &node.span)?;
            }
        }

        for node in &ast.nodes {
            if let NodeKind::Function(name, signature, _) = &node.kind {
                self.register_function_declaration(name, signature, &node.span)?;
            }
        }

        self.resolve_start_function(ast)
    }

    // WHAT: lowers the AST module-constant pool into HIR's dedicated constant metadata arena.
    // WHY: module constants should remain compile-time data instead of turning into runtime statements.
    pub(crate) fn lower_module_constants(&mut self) -> Result<(), HirConstructionFailure> {
        self.module.module_constants.clear();
        self.module_constants_by_name.clear();

        // WHAT: the store leaves `self` once for the whole pass and returns at the end.
        // WHY: the closure passed to `fold_value` borrows the rest of `HirBuilder` mutably, so
        // the store cannot stay borrowed from `self` while lowering runs. Moving it out once
        // into an owned `Vec`, cloning each constant's `PathId` purely to end the borrow,
        // and then took and restored the store again for every single value.
        let store = std::mem::take(&mut self.module_const_values);
        let result = self.lower_module_constants_from(&store);
        self.module_const_values = store;
        result
    }

    fn lower_module_constants_from(
        &mut self,
        store: &ConstValueStore,
    ) -> Result<(), HirConstructionFailure> {
        for (path, id) in store.path_value_bindings() {
            self.module_constants_by_name.insert(*path, id);
        }

        for row in store.iter_module_constant_views() {
            if !row.metadata.hir_visible {
                continue;
            }

            let const_value = self.lower_const_value_for_module_pool(store, row.id)?;

            let const_id = self.allocate_const_id();
            let const_type = self.lower_type_id(row.metadata.type_id)?;
            let mut path_scratch = Vec::new();
            let name =
                self.path_fork
                    .render_portable(*row.path, self.string_table, &mut path_scratch);

            self.module.module_constants.push(HirModuleConst {
                id: const_id,
                name,
                ty: const_type,
                value: const_value,
            });
        }

        Ok(())
    }

    fn lower_const_value_for_module_pool(
        &mut self,
        store: &ConstValueStore,
        value_id: ConstValueId,
    ) -> Result<HirConstValue, HirConstructionFailure> {
        let mut path_scratch = Vec::new();
        store.fold_value(value_id, &mut |metadata, visit| {
            increment_frontend_counter(FrontendCounter::HirConstValueConversions);
            match visit {
                ConstValueVisit::Number(value) => {
                    self.validate_number_value_type(value, metadata.type_id)?;
                    Ok(HirConstValue::Number(value.clone()))
                }
                ConstValueVisit::FixedScalar(value) => Ok(HirConstValue::FixedScalar(value)),
                ConstValueVisit::Bool(value) => Ok(HirConstValue::Bool(value)),
                ConstValueVisit::Char(value) => Ok(HirConstValue::Char(value)),
                ConstValueVisit::String(value) => match value {
                    ConstStringValue::Text(text) => Ok(HirConstValue::String(
                        self.string_table.resolve(*text).to_owned(),
                    )),

                    // Pieces stay structural in the pool exactly like structural HIR
                    // expressions; physical variant planning owns final URL text.
                    ConstStringValue::Pieces(pieces) => {
                        Ok(HirConstValue::StructuralString {
                            pieces: pieces.clone(),
                        })
                    }
                },
                ConstValueVisit::Collection(values) => Ok(HirConstValue::Collection(values)),
                ConstValueVisit::Record(fields) => Ok(HirConstValue::Record(
                    fields
                        .into_iter()
                        .map(|field| HirConstField {
                            name: self.path_fork.render_portable(
                                *field.name,
                                self.string_table,
                                &mut path_scratch,
                            ),
                            value: field.value,
                        })
                        .collect(),
                )),
                ConstValueVisit::Choice { tag, fields, .. } => Ok(HirConstValue::Choice {
                    tag,
                    fields: fields
                        .into_iter()
                        .map(|field| HirConstField {
                            name: self.path_fork.render_portable(
                                *field.name,
                                self.string_table,
                                &mut path_scratch,
                            ),
                            value: field.value,
                        })
                        .collect(),
                }),
                ConstValueVisit::Range { start, end } => {
                    Ok(HirConstValue::Range(Box::new(start), Box::new(end)))
                }
                ConstValueVisit::Coerced(value) => Ok(value),
                ConstValueVisit::OptionSome(value) => {
                    Ok(HirConstValue::OptionSome(Box::new(value)))
                }
                ConstValueVisit::OptionNone => Ok(HirConstValue::OptionNone),
                ConstValueVisit::Template { folded, .. } => match folded {
                    Some(ConstStringValue::Text(value)) => Ok(HirConstValue::String(
                        self.string_table.resolve(*value).to_owned(),
                    )),

                    // A piece-bearing template fold is the same structural string the
                    // `String` arm above keeps, so it lowers with the same variant.
                    Some(ConstStringValue::Pieces(pieces)) => {
                        Ok(HirConstValue::StructuralString {
                            pieces: pieces.clone(),
                        })
                    }
                    None => Err(InfrastructureFailure::internal(CompilerStage::HirConstruction,
                        "HIR invariant: Template constant reached HIR module-constant lowering before AST materialized it. Non-renderable template.",
                    )
                    .into()),
                },
            }
        })
    }

    /// Lower one module-store value directly to a constant HIR expression.
    ///
    /// WHAT: maps the shared postorder store visitor to HIR's expression vocabulary for
    /// references and const-record field access.
    /// WHY: module constants are already folded; HIR must not rebuild an AST expression tree or
    /// recursively lower a cloned declaration.
    pub(crate) fn lower_const_store_expression(
        &mut self,
        value_id: ConstValueId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let store = std::mem::take(&mut self.module_const_values);
        let result = store.fold_value(value_id, &mut |metadata, visit| {
            let region = self.current_region_or_error()?;
            let ty = self.lower_type_id(metadata.type_id)?;
            #[cfg(feature = "benchmark_counters")]
            if matches!(
                visit,
                ConstValueVisit::FixedScalar(_) | ConstValueVisit::Bool(_) | ConstValueVisit::Char(_)
            ) {
                increment_frontend_counter(FrontendCounter::CensusHirScalarPayloadCopiesSampled);
            }
            match visit {
                ConstValueVisit::Number(value) => {
                    #[cfg(feature = "benchmark_counters")]
                    increment_frontend_counter(FrontendCounter::CensusHirNumberPayloadClonesSampled);
                    self.validate_number_value_type(value, metadata.type_id)?;
                    self.make_expression(
                        span,
                        HirExpressionKind::Number(value.clone()),
                        ty,
                        ValueKind::Const,
                        region,
                    )
                }
                ConstValueVisit::FixedScalar(value) => self.make_expression(
                    span,
                    HirExpressionKind::FixedScalar(value),
                    ty,
                    ValueKind::Const,
                    region,
                ),
                ConstValueVisit::Bool(value) => {
                    self.make_expression(span, HirExpressionKind::Bool(value), ty, ValueKind::Const, region)
                }
                ConstValueVisit::Char(value) => {
                    self.make_expression(span, HirExpressionKind::Char(value), ty, ValueKind::Const, region)
                }
                ConstValueVisit::String(value) => match value {
                    ConstStringValue::Text(text) => self.make_expression(
                        span,
                        HirExpressionKind::StringLiteral(
                            self.string_table.resolve(*text).to_owned(),
                        ),
                        ty,
                        ValueKind::Const,
                        region,
                    ),

                    // Pieces stay structural in constant expressions exactly like
                    // `HirExpressionKind::StructuralString` values from template lowering;
                    // physical variant planning owns final URL text.
                    ConstStringValue::Pieces(pieces) => {
                        #[cfg(feature = "benchmark_counters")]
                        increment_frontend_counter(FrontendCounter::CensusHirStructuralPiecesClonesSampled);
                        let pieces = self.module.expressions.append_string_pieces(pieces, *span)?;
                        self.make_expression(
                            span,
                            HirExpressionKind::StructuralString { pieces },
                            ty,
                            ValueKind::Const,
                            region,
                        )
                    }
                },
                ConstValueVisit::Collection(values) => {
                    let values = self.module.expressions.append_values(&values, *span)?;
                    self.make_expression(
                        span,
                        HirExpressionKind::Collection(values),
                        ty,
                        ValueKind::Const,
                        region,
                    )
                }
                ConstValueVisit::Record(fields) => {
                    let struct_id = self.resolve_const_struct_id(metadata.type_id)?;
                    let fields: Vec<(FieldId, HirValueId)> = fields
                        .into_iter()
                        .map(|field| {
                            Ok((
                                self.resolve_field_id_or_error(struct_id, field.name)?,
                                field.value,
                            ))
                        })
                        .collect::<Result<Vec<_>, HirConstructionFailure>>()?;
                    let fields = self.module.expressions.append_struct_fields(&fields, *span)?;
                    self.make_expression(
                        span,
                        HirExpressionKind::StructConstruct { struct_id, fields },
                        ty,
                        ValueKind::Const,
                        region,
                    )
                }
                ConstValueVisit::Choice {
                    nominal_path,
                    tag,
                    fields,
                } => {
                    let choice_id = self.resolve_const_choice_id(
                        nominal_path,
                        metadata.type_id,
                        span,
                    )?;
                    let fields: Vec<HirVariantField> = fields
                        .into_iter()
                        .map(|field| HirVariantField {
                            name: self.path_fork.component(*field.name),
                            value: field.value,
                        })
                        .collect();
                    let fields = self.module.expressions.append_variant_fields(&fields, *span)?;
                    self.make_expression(
                        span,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Choice { choice_id },
                            variant_index: tag,
                            fields,
                        },
                        ty,
                        ValueKind::Const,
                        region,
                    )
                }
                ConstValueVisit::Range { start, end } => self.make_expression(
                    span,
                    HirExpressionKind::Range {
                        start,
                        end,
                    },
                    ty,
                    ValueKind::Const,
                    region,
                ),
                ConstValueVisit::Coerced(value) => Ok(value),
                ConstValueVisit::OptionSome(value) => {
                    // Option payload fields are named `value`, matching runtime
                    // `VariantConstruct` producers and `VariantPayloadGet` readers.
                    let value_name = self.string_table.intern("value");
                    let fields = self.module.expressions.append_variant_fields(
                        &[HirVariantField {
                            name: Some(value_name),
                            value,
                        }],
                        *span,
                    )?;
                    self.make_expression(
                        span,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Option,
                            variant_index: OPTION_SOME_VARIANT_INDEX,
                            fields,
                        },
                        ty,
                        ValueKind::Const,
                        region,
                    )
                }
                ConstValueVisit::OptionNone => self.make_expression(
                    span,
                    HirExpressionKind::VariantConstruct {
                        carrier: HirVariantCarrier::Option,
                        variant_index: 0,
                        fields: HirVariantFieldRange::empty(),
                    },
                    ty,
                    ValueKind::Const,
                    region,
                ),
                ConstValueVisit::Template { folded, .. } => match folded {
                    Some(ConstStringValue::Text(value)) => self.make_expression(
                        span,
                        HirExpressionKind::StringLiteral(
                            self.string_table.resolve(*value).to_owned(),
                        ),
                        ty,
                        ValueKind::Const,
                        region,
                    ),

                    // A piece-bearing template fold is the same structural string the
                    // `String` arm above keeps constant, so it lowers with the same
                    // expression vocabulary as `HirExpressionKind::StructuralString`.
                    Some(ConstStringValue::Pieces(pieces)) => {
                        let pieces = self.module.expressions.append_string_pieces(pieces, *span)?;
                        self.make_expression(
                            span,
                            HirExpressionKind::StructuralString { pieces },
                            ty,
                            ValueKind::Const,
                            region,
                        )
                    }
                    None => Err(InfrastructureFailure::hir_transformation("HIR invariant: Template constant reached HIR module-constant lowering before AST materialized it. Non-renderable template.").into()),
                },
            }
        });
        self.module_const_values = store;
        result
    }

    fn validate_number_value_type(
        &self,
        value: &NumberValue,
        type_id: TypeId,
    ) -> Result<(), HirConstructionFailure> {
        if self.type_environment.number_scale(type_id) != Some(value.scale()) {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "HIR invariant: Dec value scale {} does not match resolved type {type_id:?}",
                value.scale()
            ))
            .into());
        }

        Ok(())
    }

    fn resolve_const_struct_id(
        &mut self,
        type_id: TypeId,
    ) -> Result<crate::compiler_frontend::hir::ids::StructId, HirConstructionFailure> {
        if let Some(TypeIdentityKey::GenericInstance(key)) =
            self.type_environment.type_id_to_type_identity_key(type_id)
        {
            let nominal_path = self
                .type_environment
                .nominal_path(type_id)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::HirConstruction,
                        "HIR const record generic instance has no nominal path",
                    )
                })?
                .to_owned();
            return self.resolve_or_register_generic_struct(&key, &nominal_path, type_id);
        }

        let TypeDefinition::Struct(definition) =
            self.type_environment.get(type_id).ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "HIR const record has unknown type",
                )
            })?
        else {
            return Err(InfrastructureFailure::hir_transformation(
                "HIR invariant: const record value does not carry a struct type",
            )
            .into());
        };
        self.resolve_struct_id_from_nominal_path(&definition.path)
    }

    fn resolve_const_choice_id(
        &mut self,
        nominal_path: &PathId,
        type_id: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<crate::compiler_frontend::hir::ids::ChoiceId, HirConstructionFailure> {
        if let Some(TypeIdentityKey::GenericInstance(key)) =
            self.type_environment.type_id_to_type_identity_key(type_id)
        {
            return self.resolve_or_register_generic_choice(&key, nominal_path, type_id, span);
        }
        self.resolve_choice_id(nominal_path)
    }

    fn register_struct_declaration(
        &mut self,
        name: &PathId,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        if self.structs_by_name.contains_key(name) {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "HIR invariant: duplicate struct declaration '{}' during HIR lowering",
                self.symbol_name_for_diagnostics(name)
            ))
            .into());
        }

        let frontend_type_id = self
            .type_environment
            .nominal_id_for_path(name)
            .and_then(|nominal_id| self.type_environment.type_id_for_nominal_id(nominal_id))
            .ok_or_else(|| {
                crate::compiler_frontend::compiler_errors::InfrastructureFailure::internal(CompilerStage::HirConstruction, format!(
                    "HIR invariant: struct '{}' is not registered in TypeEnvironment during HIR lowering",
                    self.symbol_name_for_diagnostics(name)
                ))
            })?;

        let fields = self
            .type_environment
            .struct_definition_for(frontend_type_id)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    format!(
                        "HIR invariant: nominal type '{}' is not a struct during HIR lowering",
                        self.symbol_name_for_diagnostics(name)
                    ),
                )
            })?
            .fields
            .to_vec();

        let struct_id = self.allocate_struct_id();
        let mut hir_fields = Vec::with_capacity(fields.len());

        for field in &fields {
            // AST guarantees module-wide unique PathId symbols. For struct fields this
            // means each field path must be prefixed by its parent struct path.
            let Some(parent) = self.path_fork.try_parent(field.name) else {
                return Err(InfrastructureFailure::hir_transformation(format!(
                    "HIR invariant: field '{}' has no parent struct path during HIR lowering",
                    self.symbol_name_for_diagnostics(&field.name)
                ))
                .into());
            };

            if parent != *name {
                return Err(InfrastructureFailure::hir_transformation(format!(
                    "HIR invariant: field '{}' is not prefixed by struct '{}'",
                    self.symbol_name_for_diagnostics(&field.name),
                    self.symbol_name_for_diagnostics(name)
                ))
                .into());
            }

            if self
                .fields_by_struct_and_name
                .contains_key(&(struct_id, field.name))
            {
                return Err(InfrastructureFailure::hir_transformation(format!(
                    "HIR invariant: duplicate field '{}' in struct '{}'",
                    self.symbol_name_for_diagnostics(&field.name),
                    self.symbol_name_for_diagnostics(name)
                ))
                .into());
            }

            let field_location = if field.span.is_none() {
                *span
            } else {
                field.span
            };

            let field_type = self.lower_type_id(field.type_id)?;
            let field_id = self.allocate_field_id();

            self.fields_by_struct_and_name
                .insert((struct_id, field.name), field_id);
            self.side_table.bind_field_name(field_id, field.name);
            self.side_table
                .map_ast_to_hir(field_location, HirLocation::Field(field_id));
            self.side_table
                .map_hir_source_span(HirLocation::Field(field_id), field.span);

            hir_fields.push(HirField {
                id: field_id,
                ty: field_type,
            });
        }

        let hir_struct = HirStruct {
            id: struct_id,
            frontend_type_id,
            fields: hir_fields,
        };

        self.structs_by_name.insert(*name, struct_id);
        self.side_table.bind_struct_name(struct_id, *name);
        self.side_table
            .map_ast_to_hir(*span, HirLocation::Struct(struct_id));
        self.side_table
            .map_hir_source_span(HirLocation::Struct(struct_id), *span);
        self.push_struct(hir_struct);

        Ok(())
    }

    fn register_function_declaration(
        &mut self,
        name: &PathId,
        signature: &FunctionSignature,
        span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        if self.functions_by_name.contains_key(name) {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "HIR invariant: duplicate function declaration '{}' during HIR lowering",
                self.symbol_name_for_diagnostics(name)
            ))
            .into());
        }

        let function_id = self.allocate_function_id();

        let success_return_slot_count = signature.success_return_slot_count();
        let success_return_type_ids: Vec<Option<TypeId>> = signature
            .returns
            .iter()
            .filter(|slot| slot.channel == ReturnChannel::Success)
            .map(|slot| slot.type_id)
            .collect();

        let resolved_success_count = success_return_type_ids
            .iter()
            .filter(|id| id.is_some())
            .count();
        if resolved_success_count != success_return_slot_count {
            return Err(InfrastructureFailure::hir_transformation(format!(
                    "HIR invariant: function signature has {} success return slots but {} resolved canonical TypeIds.",
                    success_return_slot_count, resolved_success_count
                )).into());
        }

        let success_return_type_ids: Vec<_> = success_return_type_ids
            .into_iter()
            .flatten()
            .map(|type_id| self.lower_type_id(type_id))
            .collect::<Result<Vec<_>, _>>()?;

        let has_error_return_slot = signature
            .returns
            .iter()
            .any(|slot| slot.channel == ReturnChannel::Error);
        let error_return_type_id = match (has_error_return_slot, signature.error_return_type_id()) {
            (true, Some(type_id)) => Some(self.lower_type_id(type_id)?),
            (true, None) => {
                return Err(InfrastructureFailure::hir_transformation("HIR invariant: function signature has an error return slot without a canonical TypeId.").into());
            }
            (false, Some(_)) => {
                return Err(InfrastructureFailure::hir_transformation("HIR invariant: function signature has an error return TypeId without an error return slot.").into());
            }
            (false, None) => None,
        };

        let region_id = self.allocate_region_id();
        self.push_region(HirRegion::lexical(region_id, None));

        let entry_block_id = self.allocate_block_id();
        let entry_block = crate::compiler_frontend::hir::blocks::HirBlock {
            id: entry_block_id,
            region: region_id,
            locals: vec![],
            statements: vec![],
            terminator: HirTerminator::Uninitialized,
        };

        self.side_table.map_block(*span, &entry_block);
        self.push_block(entry_block);

        let function = HirFunction {
            id: function_id,
            entry: entry_block_id,
            params: vec![],
            returns: success_return_type_ids.into_boxed_slice(),
            error_return: error_return_type_id,
        };

        self.functions_by_name.insert(*name, function_id);
        self.side_table.bind_function_name(function_id, *name);
        self.side_table.map_function(*span, &function);
        self.push_function(function);

        self.module
            .function_provenance
            .insert(function_id, SyntheticInterfaceProvenance::empty());

        Ok(())
    }

    fn resolve_start_function(&mut self, ast: &Ast) -> Result<(), HirConstructionFailure> {
        if !ast.root_role.has_implicit_start() {
            self.module.start_function = None;
            return Ok(());
        }

        let start_component = self.string_table.intern(IMPLICIT_START_FUNC_NAME);
        let start_name = self
            .functions_by_name
            .keys()
            .find(|path| {
                self.path_fork.try_parent(**path) == Some(ast.entry_path)
                    && self.path_fork.try_component(**path) == Some(start_component)
            })
            .copied();

        let Some(start_name) = start_name else {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "HIR invariant: failed to resolve module start function '{}' during HIR lowering",
                IMPLICIT_START_FUNC_NAME
            ))
            .into());
        };

        let start_function = self.functions_by_name[&start_name];
        self.module.start_function = Some(start_function);
        Ok(())
    }
    pub(super) fn lower_parameter_locals(
        &mut self,
        function_id: crate::compiler_frontend::hir::ids::FunctionId,
        signature: &FunctionSignature,
    ) -> Result<(), HirConstructionFailure> {
        for param in &signature.parameters {
            let param_type = self.lower_type_id(param.type_id)?;
            // Authored parameter binding uses the explicit `Declaration.binding_span`
            // (the binding-name anchor), never the default-value/initializer span.
            let local_id = self.allocate_named_local(
                param.id.to_owned(),
                param_type,
                param.value_mode.is_mutable(),
                param.binding_span,
            )?;

            let function = self.function_mut_by_id_or_error(function_id)?;
            function.params.push(local_id);
        }

        Ok(())
    }

    pub(super) fn lower_variable_declaration_statement(
        &mut self,
        expressions: &ExpressionStore,
        variable: &Declaration,
        span: &Option<SourceSpan>,
        statement_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let initializer = variable
            .value
            .initializer()
            .map(|id| expressions.view(id))
            .transpose()?;
        if initializer.is_some_and(|initializer| initializer.is_const_record_value()) {
            if !self.module_constants_by_name.contains_key(&variable.id) {
                return Err(InfrastructureFailure::hir_transformation(format!(
                        "HIR invariant: body-local const record '{}' reached HIR without a folded store binding",
                        self.symbol_name_for_diagnostics(&variable.id)
                    )).into());
            }
            return Ok(());
        }

        if matches!(
            variable.value,
            DeclarationValue::PendingConstant
                | DeclarationValue::Shell(_)
                | DeclarationValue::Default(_)
                | DeclarationValue::StructDefinition(_)
        ) {
            return Err(InfrastructureFailure::hir_transformation(
                "Unresolved or non-runtime declaration owner reached HIR statement lowering",
            )
            .into());
        }
        let local_type = self.lower_type_id(variable.type_id)?;
        // Authored binding: the local carries the explicit `Declaration.binding_span`
        // (the binding-name anchor), never the initializer span. The initializer span
        // belongs to the value expression lowered below.
        let local_id = self.allocate_named_local(
            variable.id.to_owned(),
            local_type,
            variable.value_mode.is_mutable(),
            variable.binding_span,
        )?;

        let Some(initializer) = initializer else {
            return Ok(());
        };
        let value = self.lower_expression_value_to_current_block(initializer)?;

        // Authored initialization carries the statement span, falling back to the
        // initializer span when the statement is identity-free.
        self.emit_statement_kind_with_span(
            crate::compiler_frontend::hir::statements::HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local_id),
                value,
            },
            span,
            statement_span.or(initializer.span()),
        )
    }

    pub(crate) fn allocate_named_local(
        &mut self,
        name: PathId,
        ty: crate::compiler_frontend::datatypes::ids::TypeId,
        mutable: bool,
        span: Option<SourceSpan>,
    ) -> Result<LocalId, HirConstructionFailure> {
        if self.locals_by_name.contains_key(&name) {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "Local '{}' is already declared in this function scope",
                self.symbol_name_for_diagnostics(&name)
            ))
            .into());
        }

        let region = self.current_region_or_error()?;
        let block_id = self.current_block_id_or_error()?;
        let local_id = self.allocate_local_id();

        let local = HirLocal {
            id: local_id,
            ty,
            mutable,
            region,
            span,
        };

        self.side_table.map_local_source(&local);
        self.register_local_in_block(block_id, local)?;

        self.locals_by_name.insert(name, local_id);
        self.side_table.bind_local_name(local_id, name);
        self.side_table
            .bind_local_origin(local_id, HirLocalOriginKind::User, None, None);
        self.side_table
            .map_ast_to_hir(span, HirLocation::Local(local_id));

        Ok(local_id)
    }
}
