//! Reference and place-lowering helpers for HIR expressions.
//!
//! WHAT: lowers AST nodes that identify storage locations, field paths, and module constants.
//! WHY: HIR must distinguish assignable places from value expressions before later alias and
//! mutation analysis can reason about them.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;

use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValuePayload};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, ExpressionView, PlaceProjectionView, PlaceView,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::generic_identity_bridge::TypeIdentityKey;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{FieldId, FunctionId, HirValueId, StructId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;

use super::LoweredExpression;

impl<'a> HirBuilder<'a> {
    /// Projects a bound AST place into HIR without retaining its expression domain.
    pub(crate) fn lower_place_expression_to_hir_place(
        &mut self,
        place: PlaceView<'_>,
    ) -> Result<(Vec<HirStatement>, HirPlace), HirConstructionFailure> {
        let metadata = place.metadata();
        let (mut prelude, mut hir_place, mut current_type) =
            if let Some(local) = self.locals_by_name.get(&metadata.root).copied() {
                let ty = self.local_type_id_or_error(local)?;
                (Vec::new(), HirPlace::local(local), ty)
            } else {
                let value_id = self
                    .module_constants_by_name
                    .get(&metadata.root)
                    .copied()
                    .ok_or_else(|| {
                        InfrastructureFailure::internal(
                            CompilerStage::HirConstruction,
                            "Place root has no local or constant binding",
                        )
                    })?;
                let lowered = self.lower_const_store_expression(value_id, &metadata.span)?;
                let ty = self.module.expressions.expression(lowered).ty;
                let local = self.allocate_temp_local(ty, None)?;
                let statement = HirStatement {
                    id: self.allocate_node_id(),
                    kind: HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(local),
                        value: lowered,
                    },
                    span: None,
                };
                self.side_table.map_statement(metadata.span, &statement);
                (vec![statement], HirPlace::local(local), ty)
            };
        for projection in place.projections() {
            match projection {
                PlaceProjectionView::Field { field, result_type } => {
                    let field_id = self.resolve_field_id_for_type_or_error(current_type, field)?;
                    hir_place = hir_place.with_field(
                        field_id,
                        &mut self.module.expressions,
                        metadata.span,
                    )?;
                    current_type = result_type;
                }
                PlaceProjectionView::Index { index, result_type } => {
                    let value = self.lower_child_expression_for_parent(&mut prelude, index)?;
                    hir_place =
                        hir_place.with_index(value, &mut self.module.expressions, metadata.span)?;
                    current_type = result_type;
                }
            }
        }
        Ok((prelude, hir_place))
    }

    // Folded record members use the same constant visitor as every Constant operation.
    pub(crate) fn lower_field_access_expression(
        &mut self,
        base: ExpressionView<'_>,
        field: StringId,
        result_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        if let Some(lowered) =
            self.try_lower_const_record_field_access_expression(base, field, result_type_id, span)?
        {
            return Ok(lowered);
        }

        let region = self.current_region_or_error()?;
        let (prelude, place) = self.lower_expression_to_place(base, field, span)?;
        let ty = self.lower_type_id(result_type_id)?;

        Ok(LoweredExpression {
            prelude,
            value: self.make_expression(
                span,
                HirExpressionKind::Load(place),
                ty,
                ValueKind::Place,
                region,
            )?,
        })
    }

    // WHAT: lowers a value expression to a HIR place for field projection.
    // WHY: field access requires a place; if the base is not already a place it is materialized
    //      into a temporary local.
    fn lower_expression_to_place(
        &mut self,
        base: ExpressionView<'_>,
        field: StringId,
        span: &Option<SourceSpan>,
    ) -> Result<(Vec<HirStatement>, HirPlace), HirConstructionFailure> {
        let lowered = self.lower_expression(base)?;

        let (base_place, base_type) = {
            let row = self.module.expressions.expression(lowered.value);
            let place = match &row.kind {
                HirExpressionKind::Load(place) => Some(*place),
                _ => None,
            };
            (place, row.ty)
        };
        if let Some(base_place) = base_place {
            let field_id = self.resolve_field_id_for_type_or_error(base_type, field)?;
            return Ok((
                lowered.prelude,
                base_place.with_field(field_id, &mut self.module.expressions, *span)?,
            ));
        }

        let temp_local = self.allocate_temp_local(base_type, None)?;
        let assign_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(temp_local),
                value: lowered.value,
            },
            span: None,
        };
        self.side_table.map_statement(*span, &assign_statement);

        let mut prelude = lowered.prelude;
        prelude.push(assign_statement);

        let root_place = HirPlace::local(temp_local);
        let field_id = self.resolve_field_id_for_type_or_error(base_type, field)?;

        Ok((
            prelude,
            root_place.with_field(field_id, &mut self.module.expressions, *span)?,
        ))
    }

    fn try_lower_const_record_field_access_expression(
        &mut self,
        base: ExpressionView<'_>,
        field: StringId,
        result_type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<Option<LoweredExpression>, HirConstructionFailure> {
        if let Some(base_value_id) = self.const_store_value_for_expression(base) {
            let base_metadata = self
                .module_const_values
                .metadata(base_value_id)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::HirConstruction,
                        "HIR const-record base points outside the folded-value store",
                    )
                })?;
            if base_metadata.const_record_state != ConstRecordState::ConstRecord {
                return Ok(None);
            }

            let Some(field_value_id) =
                self.module_const_values
                    .field_value(base_value_id, field, &*self.path_fork)
            else {
                return Err(InfrastructureFailure::hir_transformation(format!(
                    "Const record field '{}' was not present during HIR field lowering",
                    self.string_table.resolve(field)
                ))
                .into());
            };

            let metadata = self
                .module_const_values
                .metadata(field_value_id)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::HirConstruction,
                        "HIR const-record field points outside the folded-value store",
                    )
                })?;
            if metadata.const_record_state == ConstRecordState::ConstRecord {
                return Err(InfrastructureFailure::hir_transformation(
                    "HIR invariant: nested const-record field access still produces a record",
                )
                .into());
            }

            let value = self.lower_const_store_expression(field_value_id, span)?;
            let ty = self.lower_type_id(result_type_id)?;
            let region = self.current_region_or_error()?;
            let value = self.replace_expression_metadata(value, span, Some(ty), Some(region))?;
            return Ok(Some(LoweredExpression {
                prelude: vec![],
                value,
            }));
        }

        if base.is_const_record_value() {
            return Err(InfrastructureFailure::hir_transformation("HIR invariant: const-record field access reached HIR without a folded store binding").into());
        }

        Ok(None)
    }
    fn const_store_value_for_expression(
        &self,
        expression: ExpressionView<'_>,
    ) -> Option<ConstValueId> {
        match expression.operation() {
            ExpressionOperationView::Constant(id) => Some(id),
            ExpressionOperationView::Reference(name) => {
                self.module_constants_by_name.get(&name).copied()
            }
            ExpressionOperationView::FieldAccess { base, field } => {
                let base_value_id = self.const_store_value_for_expression(base)?;
                self.module_const_values
                    .field_value(base_value_id, field, &*self.path_fork)
            }
            _ => None,
        }
    }

    pub(super) fn lower_reference_expression(
        &mut self,
        name: &PathId,
        type_id: FrontendTypeId,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let region = self.current_region_or_error()?;
        let ty = self.lower_type_id(type_id)?;

        if let Some(local_id) = self.locals_by_name.get(name).copied() {
            return Ok(LoweredExpression {
                prelude: vec![],
                value: self.make_expression(
                    span,
                    HirExpressionKind::Load(HirPlace::local(local_id)),
                    ty,
                    ValueKind::Place,
                    region,
                )?,
            });
        }

        if let Some(constant_value) = self.try_lower_module_constant_reference(name, span)? {
            // Preserve the type expected by the AST reference expression while reusing
            // the constant's lowered value shape.
            let constant_value =
                self.replace_expression_metadata(constant_value, span, Some(ty), Some(region))?;

            return Ok(LoweredExpression {
                prelude: vec![],
                value: constant_value,
            });
        }

        Err(InfrastructureFailure::hir_transformation(format!(
            "Unresolved local '{}' during HIR expression lowering",
            self.symbol_name_for_diagnostics(name)
        ))
        .into())
    }

    fn try_lower_module_constant_reference(
        &mut self,
        name: &PathId,
        span: &Option<SourceSpan>,
    ) -> Result<Option<HirValueId>, HirConstructionFailure> {
        let Some(value_id) = self.module_constants_by_name.get(name).copied() else {
            return Ok(None);
        };

        let metadata = self.module_const_values.metadata(value_id).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HirConstruction,
                "HIR module constant name index points outside the folded-value store",
            )
        })?;

        // INVARIANT: helper-only templates are excluded before HIR. A wrapper may retain a
        // public template payload, but its store row also carries the folded string used here.
        if matches!(
            self.module_const_values.payload(value_id),
            Some(ConstValuePayload::Template { folded: None, .. })
        ) {
            return Err(InfrastructureFailure::hir_transformation(format!(
                    "HIR invariant: non-renderable template constant '{}' reached HIR expression lowering",
                    self.symbol_name_for_diagnostics(name)
                )).into());
        }

        // INVARIANT: const-record runtime use should have been rejected in AST.
        // If a const record reaches HIR reference lowering, push validation earlier
        // instead of converting this into a user diagnostic here.
        if metadata.const_record_state == ConstRecordState::ConstRecord {
            return Err(InfrastructureFailure::hir_transformation(format!(
                    "HIR invariant: const record '{}' reached HIR reference lowering without field access",
                    self.symbol_name_for_diagnostics(name)
                )).into());
        }

        Ok(Some(self.lower_const_store_expression(value_id, span)?))
    }

    // WHAT: resolves a function path through the HIR declaration table.
    // WHY: expression lowering should fail with a structured HIR error instead of assuming AST
    //      declaration registration stayed in sync.
    pub(crate) fn resolve_function_id_or_error(
        &self,
        name: &PathId,
    ) -> Result<FunctionId, HirConstructionFailure> {
        let Some(function_id) = self.functions_by_name.get(name).copied() else {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "Unresolved function '{}' during HIR expression lowering",
                self.symbol_name_for_diagnostics(name)
            ))
            .into());
        };

        Ok(function_id)
    }

    pub(crate) fn resolve_call_target_or_error(
        &self,
        name: &PathId,
    ) -> Result<crate::compiler_frontend::external_packages::CallTarget, HirConstructionFailure>
    {
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
                    Ok(CallTarget::Generated(Box::new(identity.clone())))
                }
                SourceFunctionTarget::ModulePrivate { identity, .. } => {
                    Ok(CallTarget::ModulePrivate(identity.clone()))
                }
                SourceFunctionTarget::Local(_) => Err(InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "Imported function contract carried a local function target",
                )
                .into()),
            };
        }

        Err(InfrastructureFailure::hir_transformation(format!(
            "Unresolved function '{}' during HIR expression lowering",
            self.symbol_name_for_diagnostics(name)
        ))
        .into())
    }

    // WHAT: resolves a field path within one nominal struct declaration.
    // WHY: field access lowering must use declaration-time IDs so later passes can reason about
    //      fields without path scans.
    pub(crate) fn resolve_field_id_or_error(
        &self,
        struct_id: StructId,
        field_name: &PathId,
    ) -> Result<FieldId, HirConstructionFailure> {
        let Some(field_id) = self
            .fields_by_struct_and_name
            .get(&(struct_id, field_name.to_owned()))
            .copied()
        else {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "Field '{}' is not registered for struct {:?}",
                self.symbol_name_for_diagnostics(field_name),
                struct_id
            ))
            .into());
        };

        Ok(field_id)
    }

    pub(crate) fn resolve_struct_id_from_nominal_path(
        &self,
        nominal_path: &PathId,
    ) -> Result<StructId, HirConstructionFailure> {
        let Some(struct_id) = self.structs_by_name.get(nominal_path).copied() else {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "Unresolved struct '{}' during HIR lowering",
                self.symbol_name_for_diagnostics(nominal_path)
            ))
            .into());
        };

        Ok(struct_id)
    }

    fn resolve_field_id_for_type_or_error(
        &mut self,
        base_type: TypeId,
        field_name: StringId,
    ) -> Result<FieldId, HirConstructionFailure> {
        let struct_id = self.resolve_struct_id_for_type_or_error(base_type)?;
        let Some(struct_path) = self.side_table.struct_name_path(struct_id) else {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "Struct {:?} is missing a side-table path binding",
                struct_id
            ))
            .into());
        };

        let field_id =
            self.fields_by_struct_and_name
                .iter()
                .find_map(|(&(owner_id, path), &field_id)| {
                    (owner_id == struct_id
                        && self.path_fork.try_parent(path) == Some(struct_path)
                        && self.path_fork.try_component(path) == Some(field_name))
                    .then_some(field_id)
                });

        let Some(field_id) = field_id else {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "Field '{}' is not registered for struct {:?}",
                self.string_table.resolve(field_name),
                struct_id
            ))
            .into());
        };

        Ok(field_id)
    }

    fn resolve_struct_id_for_type_or_error(
        &mut self,
        type_id: TypeId,
    ) -> Result<StructId, HirConstructionFailure> {
        let generic_base = match self.type_environment.get(type_id) {
            Some(TypeDefinition::Struct(def)) => {
                let path = def.path;
                let Some(struct_id) = self.structs_by_name.get(&path).copied() else {
                    return Err(InfrastructureFailure::hir_transformation(format!(
                        "Struct '{}' is not registered in HIR builder",
                        self.symbol_name_for_diagnostics(&path)
                    ))
                    .into());
                };
                return Ok(struct_id);
            }
            Some(TypeDefinition::GenericInstance(instance))
                if self
                    .type_environment
                    .struct_definition(instance.base)
                    .is_some() =>
            {
                instance.base
            }
            _ => {
                return Err(InfrastructureFailure::hir_transformation(
                    "Field access base does not resolve to a struct value in this HIR phase",
                )
                .into());
            }
        };

        let Some(nominal_path) = self
            .type_environment
            .nominal_path_by_id(generic_base)
            .copied()
        else {
            return Err(InfrastructureFailure::hir_transformation(
                "Generic struct instance is missing nominal path metadata",
            )
            .into());
        };
        let Some(TypeIdentityKey::GenericInstance(key)) =
            self.type_environment.type_id_to_type_identity_key(type_id)
        else {
            return Err(InfrastructureFailure::hir_transformation(
                "Generic struct instance is missing a canonical key during field access lowering",
            )
            .into());
        };

        self.resolve_or_register_generic_struct(&key, &nominal_path, type_id)
    }
}
