//! Declaration and symbol-registration helpers for HIR statement lowering.
//!
//! WHAT: owns top-level declaration registration, module-constant lowering, and local creation.
//! WHY: these paths build the HIR symbol tables and compile-time data pool that later control-flow
//! lowering depends on.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` / `return_hir_transformation_error!` in this module means an internal
//! HIR transformation or lowering invariant failure only. Normal user-facing source failures
//! must be emitted as `CompilerDiagnostic` from AST or earlier stages.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{Declaration, NodeKind, SourceLocation};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValueId, ConstValueStore, ConstValueVisit,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnChannel};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::TypeId;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::generic_identity_bridge::TypeIdentityKey;
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::constants::{HirConstField, HirConstValue, HirModuleConst};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, HirVariantField,
    OPTION_SOME_VARIANT_INDEX, ValueKind,
};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_side_table::{HirLocalOriginKind, HirLocation};
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    // WHAT: pre-registers all structs and functions before any HIR body lowering starts.
    // WHY: later statement and expression lowering relies on complete symbol tables for stable ID lookups.
    pub(crate) fn prepare_hir_declarations(&mut self, ast: &Ast) -> Result<(), CompilerError> {
        // Register choices FIRST so struct and function signature lowering can resolve them.
        // WHY: choices are nominal types discovered from AST declarations. Pre-registering
        //      them before expression lowering ensures `resolve_choice_id` is a pure
        //      lookup and never needs lazy creation.
        for choice_def in &ast.choice_definitions {
            self.register_choice_id(&choice_def.nominal_path, &SourceLocation::default())?;
        }

        for definition in &ast.imported_struct_definitions {
            self.register_struct_declaration(&definition.nominal_path, &SourceLocation::default())?;
        }

        for node in &ast.nodes {
            if let NodeKind::StructDefinition(name, _) = &node.kind {
                self.register_struct_declaration(name, &node.location)?;
            }
        }

        for node in &ast.nodes {
            if let NodeKind::Function(name, signature, _) = &node.kind {
                self.register_function_declaration(name, signature, &node.location)?;
            }
        }

        self.resolve_start_function(ast)
    }

    // WHAT: lowers the AST module-constant pool into HIR's dedicated constant metadata arena.
    // WHY: module constants should remain compile-time data instead of turning into runtime statements.
    pub(crate) fn lower_module_constants(&mut self) -> Result<(), CompilerError> {
        self.module.module_constants.clear();
        self.module_constants_by_name.clear();

        // WHAT: the store leaves `self` once for the whole pass and returns at the end.
        // WHY: the closure passed to `fold_value` borrows the rest of `HirBuilder` mutably, so
        // the store cannot stay borrowed from `self` while lowering runs. Moving it out once
        // lets the loop read borrowed rows directly - the previous shape collected every row
        // into an owned `Vec`, cloning each constant's `InternedPath` purely to end the borrow,
        // and then took and restored the store again for every single value.
        let store = std::mem::take(&mut self.module_const_values);
        let result = self.lower_module_constants_from(&store);
        self.module_const_values = store;
        result
    }

    fn lower_module_constants_from(
        &mut self,
        store: &ConstValueStore,
    ) -> Result<(), CompilerError> {
        for (path, id) in store.path_value_bindings() {
            self.module_constants_by_name.insert(path.clone(), id);
        }

        for row in store.iter_module_constant_views() {
            if !row.metadata.hir_visible {
                continue;
            }

            let location = row.metadata.location.clone();
            let const_value = self.lower_const_value_for_module_pool(store, row.id)?;

            let const_id = self.allocate_const_id();
            let const_type = self.lower_type_id(row.metadata.type_id, &location)?;

            self.module.module_constants.push(HirModuleConst {
                id: const_id,
                name: row.path.to_string(self.string_table),
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
    ) -> Result<HirConstValue, CompilerError> {
        store.fold_value(value_id, &mut |_, visit| {
            increment_frontend_counter(FrontendCounter::HirConstValueConversions);
            match visit {
                ConstValueVisit::Int(value) => Ok(HirConstValue::Int(value)),
                ConstValueVisit::Float(value) => Ok(HirConstValue::Float(value)),
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
                            name: field.name.to_string(self.string_table),
                            value: field.value,
                        })
                        .collect(),
                )),
                ConstValueVisit::Choice { tag, fields, .. } => Ok(HirConstValue::Choice {
                    tag,
                    fields: fields
                        .into_iter()
                        .map(|field| HirConstField {
                            name: field.name.to_string(self.string_table),
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
                    None => Err(CompilerError::compiler_error(
                        "HIR invariant: Template constant reached HIR module-constant lowering before AST materialized it. Non-renderable template.",
                    )),
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
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let store = std::mem::take(&mut self.module_const_values);
        let result = store.fold_value(value_id, &mut |metadata, visit| {
            let region = self.current_region_or_error(location)?;
            let ty = self.lower_type_id(metadata.type_id, location)?;
            let expression = match visit {
                ConstValueVisit::Int(value) => {
                    self.make_expression(location, HirExpressionKind::Int(value), ty, ValueKind::Const, region)
                }
                ConstValueVisit::Float(value) => self.make_expression(
                    location,
                    HirExpressionKind::Float(value),
                    ty,
                    ValueKind::Const,
                    region,
                ),
                ConstValueVisit::Bool(value) => {
                    self.make_expression(location, HirExpressionKind::Bool(value), ty, ValueKind::Const, region)
                }
                ConstValueVisit::Char(value) => {
                    self.make_expression(location, HirExpressionKind::Char(value), ty, ValueKind::Const, region)
                }
                ConstValueVisit::String(value) => match value {
                    ConstStringValue::Text(text) => self.make_expression(
                        location,
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
                    ConstStringValue::Pieces(pieces) => self.make_expression(
                        location,
                        HirExpressionKind::StructuralString {
                            pieces: pieces.clone(),
                        },
                        ty,
                        ValueKind::Const,
                        region,
                    ),
                },
                ConstValueVisit::Collection(values) => self.make_expression(
                    location,
                    HirExpressionKind::Collection(values),
                    ty,
                    ValueKind::Const,
                    region,
                ),
                ConstValueVisit::Record(fields) => {
                    let struct_id = self.resolve_const_struct_id(metadata.type_id, location)?;
                    let fields = fields
                        .into_iter()
                        .map(|field| {
                            Ok((
                                self.resolve_field_id_or_error(struct_id, field.name, location)?,
                                field.value,
                            ))
                        })
                        .collect::<Result<Vec<_>, CompilerError>>()?;
                    self.make_expression(
                        location,
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
                        location,
                    )?;
                    let fields = fields
                        .into_iter()
                        .map(|field| HirVariantField {
                            name: field.name.name(),
                            value: field.value,
                        })
                        .collect();
                    self.make_expression(
                        location,
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
                    location,
                    HirExpressionKind::Range {
                        start: Box::new(start),
                        end: Box::new(end),
                    },
                    ty,
                    ValueKind::Const,
                    region,
                ),
                ConstValueVisit::Coerced(value) => value,
                ConstValueVisit::OptionSome(value) => {
                    // Option payload fields are named `value`, matching runtime
                    // `VariantConstruct` producers and `VariantPayloadGet` readers.
                    let value_name = self.string_table.intern("value");
                    self.make_expression(
                        location,
                        HirExpressionKind::VariantConstruct {
                            carrier: HirVariantCarrier::Option,
                            variant_index: OPTION_SOME_VARIANT_INDEX,
                            fields: vec![HirVariantField {
                                name: Some(value_name),
                                value,
                            }],
                        },
                        ty,
                        ValueKind::Const,
                        region,
                    )
                }
                ConstValueVisit::OptionNone => self.make_expression(
                    location,
                    HirExpressionKind::VariantConstruct {
                        carrier: HirVariantCarrier::Option,
                        variant_index: 0,
                        fields: Vec::new(),
                    },
                    ty,
                    ValueKind::Const,
                    region,
                ),
                ConstValueVisit::Template { folded, .. } => match folded {
                    Some(ConstStringValue::Text(value)) => self.make_expression(
                        location,
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
                    Some(ConstStringValue::Pieces(pieces)) => self.make_expression(
                        location,
                        HirExpressionKind::StructuralString {
                            pieces: pieces.clone(),
                        },
                        ty,
                        ValueKind::Const,
                        region,
                    ),
                    None => return_hir_transformation_error!(
                        "HIR invariant: Template constant reached HIR module-constant lowering before AST materialized it. Non-renderable template.",
                        self.hir_error_location(location)
                    ),
                },
            };
            Ok(expression)
        });
        self.module_const_values = store;
        result
    }

    fn resolve_const_struct_id(
        &mut self,
        type_id: TypeId,
        location: &SourceLocation,
    ) -> Result<crate::compiler_frontend::hir::ids::StructId, CompilerError> {
        if let Some(TypeIdentityKey::GenericInstance(key)) =
            self.type_environment.type_id_to_type_identity_key(type_id)
        {
            let nominal_path = self
                .type_environment
                .nominal_path(type_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "HIR const record generic instance has no nominal path",
                    )
                })?
                .to_owned();
            return self.resolve_or_register_generic_struct(&key, &nominal_path, type_id, location);
        }

        let TypeDefinition::Struct(definition) = self
            .type_environment
            .get(type_id)
            .ok_or_else(|| CompilerError::compiler_error("HIR const record has unknown type"))?
        else {
            return_hir_transformation_error!(
                "HIR invariant: const record value does not carry a struct type",
                self.hir_error_location(location)
            );
        };
        self.resolve_struct_id_from_nominal_path(&definition.path, location)
    }

    fn resolve_const_choice_id(
        &mut self,
        nominal_path: &InternedPath,
        type_id: TypeId,
        location: &SourceLocation,
    ) -> Result<crate::compiler_frontend::hir::ids::ChoiceId, CompilerError> {
        if let Some(TypeIdentityKey::GenericInstance(key)) =
            self.type_environment.type_id_to_type_identity_key(type_id)
        {
            return self.resolve_or_register_generic_choice(&key, nominal_path, type_id, location);
        }
        self.resolve_choice_id(nominal_path, location)
    }

    fn register_struct_declaration(
        &mut self,
        name: &InternedPath,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        if self.structs_by_name.contains_key(name) {
            return_hir_transformation_error!(
                format!(
                    "HIR invariant: duplicate struct declaration '{}' during HIR lowering",
                    self.symbol_name_for_diagnostics(name)
                ),
                self.hir_error_location(location)
            );
        }

        let frontend_type_id = self
            .type_environment
            .nominal_id_for_path(name)
            .and_then(|nominal_id| self.type_environment.type_id_for_nominal_id(nominal_id))
            .ok_or_else(|| {
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                    "HIR invariant: struct '{}' is not registered in TypeEnvironment during HIR lowering",
                    name.to_string(self.string_table)
                ))
            })?;

        let fields = self
            .type_environment
            .struct_definition_for(frontend_type_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "HIR invariant: nominal type '{}' is not a struct during HIR lowering",
                    name.to_string(self.string_table)
                ))
            })?
            .fields
            .to_vec();

        let struct_id = self.allocate_struct_id();
        let mut hir_fields = Vec::with_capacity(fields.len());

        for field in &fields {
            // AST guarantees module-wide unique InternedPath symbols. For struct fields this
            // means each field path must be prefixed by its parent struct path.
            let Some(parent) = field.name.parent() else {
                return_hir_transformation_error!(
                    format!(
                        "HIR invariant: field '{}' has no parent struct path during HIR lowering",
                        self.symbol_name_for_diagnostics(&field.name)
                    ),
                    self.hir_error_location(location)
                );
            };

            if parent != *name {
                return_hir_transformation_error!(
                    format!(
                        "HIR invariant: field '{}' is not prefixed by struct '{}'",
                        self.symbol_name_for_diagnostics(&field.name),
                        self.symbol_name_for_diagnostics(name)
                    ),
                    self.hir_error_location(location)
                );
            }

            if self
                .fields_by_struct_and_name
                .contains_key(&(struct_id, field.name.to_owned()))
            {
                return_hir_transformation_error!(
                    format!(
                        "HIR invariant: duplicate field '{}' in struct '{}'",
                        self.symbol_name_for_diagnostics(&field.name),
                        self.symbol_name_for_diagnostics(name)
                    ),
                    self.hir_error_location(location)
                );
            }

            let field_location = if field.location == SourceLocation::default() {
                location.clone()
            } else {
                field.location.clone()
            };

            let field_type = self.lower_type_id(field.type_id, &field_location)?;
            let field_id = self.allocate_field_id();

            self.fields_by_struct_and_name
                .insert((struct_id, field.name.to_owned()), field_id);
            self.side_table
                .bind_field_name(field_id, field.name.to_owned());
            self.side_table
                .map_ast_to_hir(&field_location, HirLocation::Field(field_id));
            self.side_table
                .map_hir_source_location(HirLocation::Field(field_id), &field_location);

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

        self.structs_by_name.insert(name.to_owned(), struct_id);
        self.side_table.bind_struct_name(struct_id, name.to_owned());
        self.side_table
            .map_ast_to_hir(location, HirLocation::Struct(struct_id));
        self.side_table
            .map_hir_source_location(HirLocation::Struct(struct_id), location);
        self.push_struct(hir_struct);

        Ok(())
    }

    fn register_function_declaration(
        &mut self,
        name: &InternedPath,
        signature: &FunctionSignature,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        if self.functions_by_name.contains_key(name) {
            return_hir_transformation_error!(
                format!(
                    "HIR invariant: duplicate function declaration '{}' during HIR lowering",
                    self.symbol_name_for_diagnostics(name)
                ),
                self.hir_error_location(location)
            );
        }

        let function_id = self.allocate_function_id();

        let success_returns = signature.success_returns();
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
        if !success_returns.is_empty() && resolved_success_count != success_returns.len() {
            return_hir_transformation_error!(
                format!(
                    "HIR invariant: function signature has {} success return slots but {} resolved canonical TypeIds.",
                    success_returns.len(),
                    resolved_success_count
                ),
                self.hir_error_location(location)
            );
        }

        let success_return_type_ids: Vec<_> = success_return_type_ids
            .into_iter()
            .flatten()
            .map(|type_id| self.lower_type_id(type_id, location))
            .collect::<Result<Vec<_>, _>>()?;

        let error_return_type_id = match (
            signature.error_return(),
            signature.error_return_type_id(),
        ) {
            (Some(_), Some(type_id)) => Some(self.lower_type_id(type_id, location)?),
            (Some(_), None) => {
                return_hir_transformation_error!(
                    "HIR invariant: function signature has an error return slot without a canonical TypeId.",
                    self.hir_error_location(location)
                );
            }
            (None, Some(_)) => {
                return_hir_transformation_error!(
                    "HIR invariant: function signature has an error return TypeId without an error return slot.",
                    self.hir_error_location(location)
                );
            }
            (None, None) => None,
        };

        let success_return_type = match success_return_type_ids.as_slice() {
            [] => self.type_environment.builtins().none,
            [single] => *single,
            multiple => self.type_environment.intern_tuple(multiple.to_vec()),
        };
        let return_type = if let Some(error_type) = error_return_type_id {
            self.type_environment
                .intern_fallible_carrier(success_return_type, error_type)
        } else {
            success_return_type
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

        self.side_table.map_block(location, &entry_block);
        self.push_block(entry_block);

        let function = HirFunction {
            id: function_id,
            entry: entry_block_id,
            params: vec![],
            return_type,
        };

        self.functions_by_name.insert(name.to_owned(), function_id);
        self.side_table
            .bind_function_name(function_id, name.to_owned());
        self.side_table.map_function(location, &function);
        self.push_function(function);

        self.module
            .function_provenance
            .insert(function_id, SyntheticInterfaceProvenance::empty());

        Ok(())
    }

    fn resolve_start_function(&mut self, ast: &Ast) -> Result<(), CompilerError> {
        if !ast.root_role.has_implicit_start() {
            self.module.start_function = None;
            return Ok(());
        }

        let start_name = ast
            .entry_path
            .join_str(IMPLICIT_START_FUNC_NAME, self.string_table);

        let Some(start_function) = self.functions_by_name.get(&start_name).copied() else {
            let error_location = ast
                .nodes
                .first()
                .map(|node| node.location.clone())
                .unwrap_or_default();

            return_hir_transformation_error!(
                format!(
                    "HIR invariant: failed to resolve module start function '{}' during HIR lowering",
                    self.symbol_name_for_diagnostics(&start_name)
                ),
                self.hir_error_location(&error_location)
            );
        };

        self.module.start_function = Some(start_function);
        Ok(())
    }

    pub(super) fn lower_parameter_locals(
        &mut self,
        function_id: crate::compiler_frontend::hir::ids::FunctionId,
        signature: &FunctionSignature,
        fallback_location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        for param in &signature.parameters {
            let param_location = if param.value.location == SourceLocation::default() {
                fallback_location.clone()
            } else {
                param.value.location.clone()
            };

            let param_type = self.lower_type_id(param.value.type_id, &param_location)?;
            let local_id = self.allocate_named_local(
                param.id.to_owned(),
                param_type,
                param.value.value_mode.is_mutable(),
                Some(param_location.clone()),
            )?;
            if let Some(source) = &param.value.reactive_source {
                self.bind_reactive_source_for_local(local_id, source, param_type, &param_location)?;
            }

            let function = self.function_mut_by_id_or_error(function_id, &param_location)?;
            function.params.push(local_id);
        }

        Ok(())
    }

    pub(super) fn lower_variable_declaration_statement(
        &mut self,
        variable: &Declaration,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        if variable.value.is_const_record_value() {
            if !self.module_constants_by_name.contains_key(&variable.id) {
                return_hir_transformation_error!(
                    format!(
                        "HIR invariant: body-local const record '{}' reached HIR without a folded store binding",
                        self.symbol_name_for_diagnostics(&variable.id)
                    ),
                    self.hir_error_location(location)
                );
            }
            return Ok(());
        }

        let source_location = if variable.value.location == SourceLocation::default() {
            location.clone()
        } else {
            variable.value.location.clone()
        };

        let local_type = self.lower_type_id(variable.value.type_id, &source_location)?;
        let local_id = self.allocate_named_local(
            variable.id.to_owned(),
            local_type,
            variable.value.value_mode.is_mutable(),
            Some(source_location.clone()),
        )?;
        if let Some(source) = &variable.value.reactive_source {
            self.bind_reactive_source_for_local(local_id, source, local_type, &source_location)?;
        }

        let value = self.lower_expression_value_to_current_block(&variable.value)?;

        self.emit_statement_kind(
            crate::compiler_frontend::hir::statements::HirStatementKind::Assign {
                target: HirPlace::Local(local_id),
                value,
            },
            location,
        )
    }

    pub(crate) fn allocate_named_local(
        &mut self,
        name: InternedPath,
        ty: crate::compiler_frontend::datatypes::ids::TypeId,
        mutable: bool,
        source_info: Option<SourceLocation>,
    ) -> Result<LocalId, CompilerError> {
        let local_location = source_info.to_owned().unwrap_or_default();

        // AST forbids shadowing and provides module-wide unique symbol paths, so a duplicate
        // path here indicates invalid redeclaration in the current function lowering context.
        if self.locals_by_name.contains_key(&name) {
            return_hir_transformation_error!(
                format!(
                    "Local '{}' is already declared in this function scope",
                    self.symbol_name_for_diagnostics(&name)
                ),
                self.hir_error_location(&local_location)
            );
        }

        let region = self.current_region_or_error(&local_location)?;
        let block_id = self.current_block_id_or_error(&local_location)?;
        let local_id = self.allocate_local_id();

        let local = HirLocal {
            id: local_id,
            ty,
            mutable,
            region,
            source_info,
        };

        self.side_table.map_local_source(&local);
        self.register_local_in_block(block_id, local, &local_location)?;

        self.locals_by_name.insert(name.to_owned(), local_id);
        self.side_table.bind_local_name(local_id, name);
        self.side_table
            .bind_local_origin(local_id, HirLocalOriginKind::User, None, None);
        self.side_table
            .map_ast_to_hir(&local_location, HirLocation::Local(local_id));

        Ok(local_id)
    }
}
