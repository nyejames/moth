//! Reference and place-lowering helpers for HIR expressions.
//!
//! WHAT: lowers AST nodes that identify storage locations, field paths, and module constants.
//! WHY: HIR must distinguish assignable places from value expressions before later alias and
//! mutation analysis can reason about them.

#[cfg(test)]
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};

use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValuePayload};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::generic_identity_bridge::TypeIdentityKey;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{FieldId, FunctionId, LocalId, StructId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::return_hir_transformation_error;

use super::LoweredExpression;

impl<'a> HirBuilder<'a> {
    // WHAT: converts an AST node that semantically yields a value into HIR expression form.
    // WHY: some runtime AST containers store expressions as general nodes, and HIR still needs a
    //      single value-producing lowering path for them.
    #[cfg(test)]
    pub(crate) fn lower_ast_node_as_expression(
        &mut self,
        node: &AstNode,
    ) -> Result<LoweredExpression, CompilerError> {
        match &node.kind {
            NodeKind::ExpressionStatement(expr) => {
                if self.expression_needs_current_block_lowering(expr) {
                    let value = self.lower_expression_value_to_current_block(expr)?;
                    return Ok(LoweredExpression {
                        prelude: vec![],
                        value,
                    });
                }
                self.lower_expression(expr)
            }

            _ => {
                return_hir_transformation_error!(
                    format!("AST node is not an expression: {:?}", node.kind),
                    self.hir_error_location(&node.location)
                )
            }
        }
    }

    // WHAT: resolves an AST node into a concrete HIR place for loads, stores, and copies.
    // WHY: place lowering must distinguish between value-producing expressions and assignable
    //      storage locations before later borrow and mutation analysis runs.
    #[cfg(test)]
    pub(crate) fn lower_ast_node_to_place(
        &mut self,
        node: &AstNode,
    ) -> Result<(Vec<HirStatement>, HirPlace), CompilerError> {
        match &node.kind {
            NodeKind::ExpressionStatement(expr) => match &expr.kind {
                ExpressionKind::Reference(name) => {
                    if let Some(local) = self.locals_by_name.get(name).copied() {
                        return Ok((vec![], HirPlace::Local(local)));
                    }

                    // Field/index lowering requires a place. Module constants are lowered as
                    // rvalues, so materialize them into a temporary local when referenced in
                    // place-position expressions (for example `format.center`).
                    let lowered =
                        self.lower_reference_expression(name, expr.type_id, &node.location)?;
                    if let HirExpressionKind::Load(place) = &lowered.value.kind {
                        return Ok((lowered.prelude, place.to_owned()));
                    }

                    let temp_local =
                        self.allocate_temp_local(lowered.value.ty, Some(node.location.to_owned()))?;
                    let assign_statement = HirStatement {
                        id: self.allocate_node_id(),
                        kind: HirStatementKind::Assign {
                            target: HirPlace::Local(temp_local),
                            value: lowered.value,
                        },
                        location: node.location.to_owned(),
                    };

                    self.side_table
                        .map_statement(&node.location, &assign_statement);

                    let mut prelude = lowered.prelude;
                    prelude.push(assign_statement);
                    Ok((prelude, HirPlace::Local(temp_local)))
                }

                _ => {
                    let lowered = if self.expression_needs_current_block_lowering(expr) {
                        LoweredExpression {
                            prelude: vec![],
                            value: self.lower_expression_value_to_current_block(expr)?,
                        }
                    } else {
                        self.lower_expression(expr)?
                    };

                    if let HirExpressionKind::Load(place) = &lowered.value.kind {
                        return Ok((lowered.prelude, place.to_owned()));
                    }

                    let temp_local =
                        self.allocate_temp_local(lowered.value.ty, Some(node.location.to_owned()))?;
                    let assign_statement = HirStatement {
                        id: self.allocate_node_id(),
                        kind: HirStatementKind::Assign {
                            target: HirPlace::Local(temp_local),
                            value: lowered.value,
                        },
                        location: node.location.to_owned(),
                    };
                    self.side_table
                        .map_statement(&node.location, &assign_statement);

                    let mut prelude = lowered.prelude;
                    prelude.push(assign_statement);
                    Ok((prelude, HirPlace::Local(temp_local)))
                }
            },

            _ => {
                return_hir_transformation_error!(
                    format!("Cannot lower AST node to HIR place: {:?}", node.kind),
                    self.hir_error_location(&node.location)
                )
            }
        }
    }

    // WHAT: resolves a frontend place expression into a concrete HIR place.
    // WHY: copy expressions and assignment targets now carry narrow `PlaceExpression` values
    //      instead of broad `AstNode` fragments.
    pub(crate) fn lower_place_expression_to_hir_place(
        &mut self,
        place: &PlaceExpression,
    ) -> Result<(Vec<HirStatement>, HirPlace), CompilerError> {
        match &place.kind {
            PlaceExpressionKind::Local(name) => {
                if let Some(local) = self.locals_by_name.get(name).copied() {
                    return Ok((vec![], HirPlace::Local(local)));
                }

                // Field/index lowering requires a place. Module constants are lowered as
                // rvalues, so materialize them into a temporary local when referenced in
                // place-position expressions.
                let lowered =
                    self.lower_reference_expression(name, place.type_id, &place.location)?;
                if let HirExpressionKind::Load(hir_place) = &lowered.value.kind {
                    return Ok((lowered.prelude, hir_place.to_owned()));
                }

                let temp_local =
                    self.allocate_temp_local(lowered.value.ty, Some(place.location.to_owned()))?;
                let assign_statement = HirStatement {
                    id: self.allocate_node_id(),
                    kind: HirStatementKind::Assign {
                        target: HirPlace::Local(temp_local),
                        value: lowered.value,
                    },
                    location: place.location.to_owned(),
                };

                self.side_table
                    .map_statement(&place.location, &assign_statement);

                let mut prelude = lowered.prelude;
                prelude.push(assign_statement);
                Ok((prelude, HirPlace::Local(temp_local)))
            }

            PlaceExpressionKind::Field { base, field } => {
                let (prelude, base_place) = self.lower_place_expression_to_hir_place(base)?;
                let field_id = self.resolve_field_id_for_base_place_or_error(
                    &base_place,
                    *field,
                    &place.location,
                )?;

                Ok((
                    prelude,
                    HirPlace::Field {
                        base: Box::new(base_place),
                        field: field_id,
                    },
                ))
            }
        }
    }

    // WHAT: lowers an expression-owned field access into a HIR value or const-record expansion.
    // WHY: field access is now owned by `ExpressionKind::FieldAccess` rather than a statement node.
    pub(crate) fn lower_field_access_expression(
        &mut self,
        base: &Expression,
        field: StringId,
        result_type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        if let Some(lowered) = self.try_lower_const_record_field_access_expression(
            base,
            field,
            result_type_id,
            location,
        )? {
            return Ok(lowered);
        }

        let region = self.current_region_or_error(location)?;
        let (prelude, place) = self.lower_expression_to_place(base, field, location)?;
        let ty = self.lower_type_id(result_type_id, location)?;

        Ok(LoweredExpression {
            prelude,
            value: self.make_expression(
                location,
                HirExpressionKind::Load(place),
                ty,
                ValueKind::Place,
                region,
            ),
        })
    }

    // WHAT: lowers a value expression to a HIR place for field projection.
    // WHY: field access requires a place; if the base is not already a place it is materialized
    //      into a temporary local.
    fn lower_expression_to_place(
        &mut self,
        base: &Expression,
        field: StringId,
        location: &SourceLocation,
    ) -> Result<(Vec<HirStatement>, HirPlace), CompilerError> {
        let lowered = self.lower_expression(base)?;

        if let HirExpressionKind::Load(base_place) = &lowered.value.kind {
            let field_id =
                self.resolve_field_id_for_base_place_or_error(base_place, field, location)?;
            return Ok((
                lowered.prelude,
                HirPlace::Field {
                    base: Box::new(base_place.to_owned()),
                    field: field_id,
                },
            ));
        }

        let temp_local = self.allocate_temp_local(lowered.value.ty, Some(location.to_owned()))?;
        let assign_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::Assign {
                target: HirPlace::Local(temp_local),
                value: lowered.value,
            },
            location: location.to_owned(),
        };
        self.side_table.map_statement(location, &assign_statement);

        let mut prelude = lowered.prelude;
        prelude.push(assign_statement);

        let field_id = self.resolve_field_id_for_base_place_or_error(
            &HirPlace::Local(temp_local),
            field,
            location,
        )?;

        Ok((
            prelude,
            HirPlace::Field {
                base: Box::new(HirPlace::Local(temp_local)),
                field: field_id,
            },
        ))
    }

    fn try_lower_const_record_field_access_expression(
        &mut self,
        base: &Expression,
        field: StringId,
        result_type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<Option<LoweredExpression>, CompilerError> {
        if let Some(base_value_id) = self.const_store_value_for_expression(base) {
            let base_metadata = self
                .module_const_values
                .metadata(base_value_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "HIR const-record base points outside the folded-value store",
                    )
                })?;
            if base_metadata.const_record_state != ConstRecordState::ConstRecord {
                return Ok(None);
            }

            let Some(field_value_id) = self.module_const_values.field_value(base_value_id, field)
            else {
                return_hir_transformation_error!(
                    format!(
                        "Const record field '{}' was not present during HIR field lowering",
                        self.string_table.resolve(field)
                    ),
                    self.hir_error_location(location)
                );
            };

            let metadata = self
                .module_const_values
                .metadata(field_value_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "HIR const-record field points outside the folded-value store",
                    )
                })?;
            if metadata.const_record_state == ConstRecordState::ConstRecord {
                return_hir_transformation_error!(
                    "HIR invariant: nested const-record field access still produces a record",
                    self.hir_error_location(location)
                );
            }

            let mut value = self.lower_const_store_expression(field_value_id, location)?;
            value.ty = self.lower_type_id(result_type_id, location)?;
            value.region = self.current_region_or_error(location)?;
            return Ok(Some(LoweredExpression {
                prelude: vec![],
                value,
            }));
        }

        if base.is_const_record_value() {
            return_hir_transformation_error!(
                "HIR invariant: const-record field access reached HIR without a folded store binding",
                self.hir_error_location(location)
            );
        }

        Ok(None)
    }

    fn const_store_value_for_expression(&self, expression: &Expression) -> Option<ConstValueId> {
        match &expression.kind {
            ExpressionKind::Reference(name) => self.module_constants_by_name.get(name).copied(),
            ExpressionKind::FieldAccess { base, field } => {
                let base_value_id = self.const_store_value_for_expression(base)?;
                self.module_const_values.field_value(base_value_id, *field)
            }
            _ => None,
        }
    }

    pub(super) fn lower_reference_expression(
        &mut self,
        name: &InternedPath,
        type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        let region = self.current_region_or_error(location)?;
        let ty = self.lower_type_id(type_id, location)?;

        if let Some(local_id) = self.locals_by_name.get(name).copied() {
            return Ok(LoweredExpression {
                prelude: vec![],
                value: self.make_expression(
                    location,
                    HirExpressionKind::Load(HirPlace::Local(local_id)),
                    ty,
                    ValueKind::Place,
                    region,
                ),
            });
        }

        if let Some(mut constant_value) =
            self.try_lower_module_constant_reference(name, location)?
        {
            // Preserve the type expected by the AST reference expression while reusing
            // the constant's lowered value shape.
            constant_value.ty = ty;
            constant_value.region = region;

            return Ok(LoweredExpression {
                prelude: vec![],
                value: constant_value,
            });
        }

        return_hir_transformation_error!(
            format!(
                "Unresolved local '{}' during HIR expression lowering",
                self.symbol_name_for_diagnostics(name)
            ),
            self.hir_error_location(location)
        )
    }

    fn try_lower_module_constant_reference(
        &mut self,
        name: &InternedPath,
        location: &SourceLocation,
    ) -> Result<Option<HirExpression>, CompilerError> {
        let Some(value_id) = self.module_constants_by_name.get(name).copied() else {
            return Ok(None);
        };

        let metadata = self.module_const_values.metadata(value_id).ok_or_else(|| {
            CompilerError::compiler_error(
                "HIR module constant name index points outside the folded-value store",
            )
        })?;

        // INVARIANT: helper-only templates are excluded before HIR. A wrapper may retain a
        // public template payload, but its store row also carries the folded string used here.
        if matches!(
            self.module_const_values.payload(value_id),
            Some(ConstValuePayload::Template { folded: None, .. })
        ) {
            return_hir_transformation_error!(
                format!(
                    "HIR invariant: non-renderable template constant '{}' reached HIR expression lowering",
                    self.symbol_name_for_diagnostics(name)
                ),
                self.hir_error_location(location)
            );
        }

        // INVARIANT: const-record runtime use should have been rejected in AST.
        // If a const record reaches HIR reference lowering, push validation earlier
        // instead of converting this into a user diagnostic here.
        if metadata.const_record_state == ConstRecordState::ConstRecord {
            return_hir_transformation_error!(
                format!(
                    "HIR invariant: const record '{}' reached HIR reference lowering without field access",
                    self.symbol_name_for_diagnostics(name)
                ),
                self.hir_error_location(location)
            );
        }

        Ok(Some(self.lower_const_store_expression(value_id, location)?))
    }

    // WHAT: resolves a function path through the HIR declaration table.
    // WHY: expression lowering should fail with a structured HIR error instead of assuming AST
    //      declaration registration stayed in sync.
    pub(crate) fn resolve_function_id_or_error(
        &self,
        name: &InternedPath,
        location: &SourceLocation,
    ) -> Result<FunctionId, CompilerError> {
        let Some(function_id) = self.functions_by_name.get(name).copied() else {
            return_hir_transformation_error!(
                format!(
                    "Unresolved function '{}' during HIR expression lowering",
                    self.symbol_name_for_diagnostics(name)
                ),
                self.hir_error_location(location)
            );
        };

        Ok(function_id)
    }

    pub(crate) fn resolve_call_target_or_error(
        &self,
        name: &InternedPath,
        location: &SourceLocation,
    ) -> Result<crate::compiler_frontend::external_packages::CallTarget, CompilerError> {
        use crate::compiler_frontend::external_packages::CallTarget;
        use crate::compiler_frontend::headers::binding_environment::SourceFunctionTarget;

        if let Some(function_id) = self.functions_by_name.get(name).copied() {
            return Ok(CallTarget::Local(function_id));
        }
        if let Some(contract) = self.imported_functions_by_name.get(name) {
            return match &contract.target {
                SourceFunctionTarget::Imported { origin, .. } => {
                    Ok(CallTarget::CrossModule(origin.clone()))
                }
                SourceFunctionTarget::Generated { identity, .. } => {
                    Ok(CallTarget::Generated(identity.clone()))
                }
                SourceFunctionTarget::ModulePrivate { identity, .. } => {
                    Ok(CallTarget::ModulePrivate(identity.clone()))
                }
                SourceFunctionTarget::Local(_) => Err(CompilerError::compiler_error(
                    "Imported function contract carried a local function target",
                )),
            };
        }

        return_hir_transformation_error!(
            format!(
                "Unresolved function '{}' during HIR expression lowering",
                self.symbol_name_for_diagnostics(name)
            ),
            self.hir_error_location(location)
        );
    }

    // WHAT: resolves a field path within one nominal struct declaration.
    // WHY: field access lowering must use declaration-time IDs so later passes can reason about
    //      fields without path scans.
    pub(crate) fn resolve_field_id_or_error(
        &self,
        struct_id: StructId,
        field_name: &InternedPath,
        location: &SourceLocation,
    ) -> Result<FieldId, CompilerError> {
        let Some(field_id) = self
            .fields_by_struct_and_name
            .get(&(struct_id, field_name.to_owned()))
            .copied()
        else {
            return_hir_transformation_error!(
                format!(
                    "Field '{}' is not registered for struct {:?}",
                    self.symbol_name_for_diagnostics(field_name),
                    struct_id
                ),
                self.hir_error_location(location)
            );
        };

        Ok(field_id)
    }

    pub(crate) fn resolve_struct_id_from_nominal_path(
        &self,
        nominal_path: &InternedPath,
        location: &SourceLocation,
    ) -> Result<StructId, CompilerError> {
        let Some(struct_id) = self.structs_by_name.get(nominal_path).copied() else {
            return_hir_transformation_error!(
                format!(
                    "Unresolved struct '{}' during HIR lowering",
                    self.symbol_name_for_diagnostics(nominal_path)
                ),
                self.hir_error_location(location)
            );
        };

        Ok(struct_id)
    }

    fn resolve_field_id_for_base_place_or_error(
        &mut self,
        base_place: &HirPlace,
        field_name: StringId,
        location: &SourceLocation,
    ) -> Result<FieldId, CompilerError> {
        let struct_id = self.resolve_struct_id_for_place_or_error(base_place, location)?;
        let Some(struct_path) = self.side_table.struct_name_path(struct_id) else {
            return_hir_transformation_error!(
                format!(
                    "Struct {:?} is missing a side-table path binding",
                    struct_id
                ),
                self.hir_error_location(location)
            );
        };

        let field_path = struct_path.append(field_name);

        self.resolve_field_id_or_error(struct_id, &field_path, location)
    }

    fn resolve_struct_id_for_place_or_error(
        &mut self,
        place: &HirPlace,
        location: &SourceLocation,
    ) -> Result<StructId, CompilerError> {
        let ty = self.resolve_place_type_id_or_error(place, location)?;
        let path = match self.type_environment.get(ty).cloned() {
            Some(TypeDefinition::Struct(def)) => Some(def.path),
            Some(TypeDefinition::GenericInstance(instance))
                if self
                    .type_environment
                    .struct_definition(instance.base)
                    .is_some() =>
            {
                let Some(nominal_path) = self.type_environment.nominal_path_by_id(instance.base)
                else {
                    return_hir_transformation_error!(
                        "Generic struct instance is missing nominal path metadata",
                        self.hir_error_location(location)
                    );
                };
                let nominal_path = nominal_path.to_owned();
                let Some(TypeIdentityKey::GenericInstance(key)) =
                    self.type_environment.type_id_to_type_identity_key(ty)
                else {
                    return_hir_transformation_error!(
                        "Generic struct instance is missing a canonical key during field access lowering",
                        self.hir_error_location(location)
                    );
                };

                return self.resolve_or_register_generic_struct(&key, &nominal_path, ty, location);
            }
            _ => {
                return_hir_transformation_error!(
                    "Field access base does not resolve to a struct value in this HIR phase",
                    self.hir_error_location(location)
                )
            }
        };
        let Some(path) = path else {
            return_hir_transformation_error!(
                "Field access base is missing nominal struct path metadata",
                self.hir_error_location(location)
            );
        };

        match self.structs_by_name.get(&path).copied() {
            Some(struct_id) => Ok(struct_id),
            None => {
                return_hir_transformation_error!(
                    format!(
                        "Struct '{}' is not registered in HIR builder",
                        path.to_string(self.string_table)
                    ),
                    self.hir_error_location(location)
                )
            }
        }
    }

    fn resolve_place_type_id_or_error(
        &self,
        place: &HirPlace,
        location: &SourceLocation,
    ) -> Result<TypeId, CompilerError> {
        match place {
            HirPlace::Local(local_id) => self.resolve_local_type_id_or_error(*local_id, location),
            HirPlace::Field { field, .. } => self.resolve_field_type_id_or_error(*field, location),
            HirPlace::Index { base, .. } => {
                let base_type = self.resolve_place_type_id_or_error(base, location)?;
                match self.type_environment.collection_element_type(base_type) {
                    Some(element) => Ok(element),
                    None => {
                        return_hir_transformation_error!(
                            "Index access base is not a collection type",
                            self.hir_error_location(location)
                        )
                    }
                }
            }
        }
    }

    fn resolve_local_type_id_or_error(
        &self,
        local_id: LocalId,
        location: &SourceLocation,
    ) -> Result<TypeId, CompilerError> {
        self.local_type_id_or_error(local_id, location)
    }

    fn resolve_field_type_id_or_error(
        &self,
        field_id: FieldId,
        location: &SourceLocation,
    ) -> Result<TypeId, CompilerError> {
        self.field_type_id_or_error(field_id, location)
    }
}
