//! Native call and Invoke channel validation. Operation policy owns types; edges own delivery.

use super::HirValidator;
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::casts::evidence::lookup_builtin_evidence;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
};
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::builtins::member_rules::BuiltinMemberRule;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::HirMapOp;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId};
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind,
};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use rustc_hash::FxHashSet;

impl HirValidator<'_> {
    pub(super) fn builtin_error_type(&self) -> Result<TypeId, InfrastructureFailure> {
        self.type_environment
            .type_id_for_canonical_identity(&CanonicalTypeIdentity::Builtin(
                CanonicalBuiltinType::Error,
            ))
            .ok_or_else(|| self.error_with_hir("Invoke requires registered builtin Error type"))
    }

    pub(super) fn validate_call_channels(
        &self,
        target: &CallTarget,
        args: HirValueRange,
        results: impl ExactSizeIterator<Item = LocalId>,
        fallible: bool,
        anchor: Option<HirLocation>,
    ) -> Result<Option<TypeId>, InfrastructureFailure> {
        let args = self.value_ids(args)?;
        for arg in args {
            self.validate_expression(*arg, anchor)?;
        }
        if let CallTarget::External(function_id) = target {
            if let Some(op) = CollectionBuiltinOp::from_external_function_id(*function_id) {
                let receiver = args.first().ok_or_else(|| {
                    self.error_with_hir("Collection builtin call is missing its receiver")
                })?;
                let shape = self
                    .type_environment
                    .collection_shape(self.expression_row(*receiver)?.ty)
                    .ok_or_else(|| {
                        self.error_with_hir(
                            "Collection builtin receiver must have a collection type",
                        )
                    })?;
                if matches!(op, CollectionBuiltinOp::PushFixed) && shape.fixed_capacity.is_none()
                    || matches!(op, CollectionBuiltinOp::PushGrowable)
                        && shape.fixed_capacity.is_some()
                {
                    return Err(self.error_with_hir(
                        "Collection push operation does not match receiver capacity",
                    ));
                }
                let rule = op.rule(
                    shape.element_type,
                    crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                        FixedScalar::I32,
                    ),
                );
                self.validate_builtin_member_slots(rule, &args[1..], results, fallible, anchor)?;
                return if rule.fallible {
                    Ok(Some(self.builtin_error_type()?))
                } else {
                    Ok(None)
                };
            }
            let function = self
                .external_packages
                .get_function_by_id(*function_id)
                .ok_or_else(|| self.error_with_hir("Call references missing external function"))?;
            if function.is_fallible() != fallible {
                return Err(self.error_with_hir(
                    "Call channel compatibility does not match external signature",
                ));
            }
            if args.len() != function.parameters.len() || results.len() != function.returns.len() {
                return Err(self.error_with_hir(
                    "Call argument/result slot arity does not match external signature",
                ));
            }
            for (arg, parameter) in args.iter().zip(&function.parameters) {
                if !self.external_signature_matches(
                    &parameter.language_type,
                    self.expression_row(*arg)?.ty,
                )? {
                    return Err(
                        self.error_with_hir("Call argument type does not match external signature")
                    );
                }
            }
            for (result, expected) in results.zip(&function.returns) {
                self.require_local_id(result)?;
                if !self
                    .external_signature_matches(&expected.value_type, self.local_types[&result])?
                {
                    return Err(self.error_with_hir(
                        "Call result slot type does not match external signature",
                    ));
                }
            }
            return Ok(None);
        }
        let local_function_id = match target {
            CallTarget::Local(function_id) => Some(*function_id),
            CallTarget::Generated(identity) => self
                .module
                .function_ids_by_generated
                .get(identity.as_ref())
                .copied(),
            CallTarget::ModulePrivate(identity) => self
                .module
                .function_ids_by_private_origin
                .get(identity)
                .copied(),
            CallTarget::CrossModule(identity) => {
                self.module.function_ids_by_origin.get(identity).copied()
            }
            CallTarget::External(_) => None,
        };
        let Some(function_id) = local_function_id else {
            let contract = self
                .module
                .call_result_contracts
                .get(target)
                .ok_or_else(|| {
                    self.error_with_hir("Nonlocal call has no resolved result contract")
                })?;
            if contract.error_return.is_some() != fallible {
                return Err(self.error_with_hir(
                    "Call channel compatibility does not match resolved contract",
                ));
            }
            if results.len() != contract.returns.len() {
                return Err(
                    self.error_with_hir("Call result slot arity does not match resolved contract")
                );
            }
            for (result, expected) in results.zip(contract.returns.iter()) {
                self.require_local_id(result)?;
                if self.local_types.get(&result) != Some(expected) {
                    return Err(self
                        .error_with_hir("Call result slot type does not match resolved contract"));
                }
            }
            return Ok(contract.error_return);
        };
        let function = self
            .module
            .functions
            .iter()
            .find(|function| function.id == function_id)
            .ok_or_else(|| self.error_with_hir("Call references missing local function"))?;
        if function.error_return.is_some() != fallible {
            return Err(
                self.error_with_hir("Call channel compatibility does not match function signature")
            );
        }
        if args.len() != function.params.len() || results.len() != function.returns.len() {
            return Err(self.error_with_hir(
                "Call argument/result slot arity does not match function signature",
            ));
        }
        for (arg, parameter) in args.iter().zip(&function.params) {
            if Some(&self.expression_row(*arg)?.ty) != self.local_types.get(parameter) {
                return Err(
                    self.error_with_hir("Call argument type does not match function parameter")
                );
            }
        }
        for (result, expected) in results.zip(function.returns.iter()) {
            self.require_local_id(result)?;
            if self.local_types.get(&result) != Some(expected) {
                return Err(
                    self.error_with_hir("Call result slot type does not match function signature")
                );
            }
        }
        Ok(function.error_return)
    }

    pub(super) fn validate_invoke(
        &self,
        block: BlockId,
        terminator: &HirTerminator,
        anchor: Option<HirLocation>,
    ) -> Result<(), InfrastructureFailure> {
        let HirTerminator::Invoke {
            id,
            operation,
            success,
            success_results: results,
            failure,
            error_result,
            ..
        } = terminator
        else {
            return Err(self.error_with_hir("Invoke validation requires an Invoke terminator"));
        };
        let error = *error_result;
        let mut destinations = FxHashSet::default();
        for result in results {
            if !destinations.insert(*result) {
                return Err(self.error_with_hir("Invoke success destinations must be unique"));
            }
        }
        if destinations.contains(&error) {
            return Err(self.error_with_hir("Invoke success/error destinations must be disjoint"));
        }
        for target in [*success, *failure] {
            self.require_block_id(target)?;
            self.require_same_function_cfg_owner(block, target)?;
        }
        for local in results.iter().copied().chain(std::iter::once(error)) {
            self.require_local_in_function(local, self.block_owner_by_id[&block])?;
            self.require_local_not_parameter_definition(local)?;
        }
        if let HirFallibleOperation::Call { target, args } = operation {
            let expected_error =
                self.validate_call_channels(target, *args, results.iter().copied(), true, anchor)?;
            if let CallTarget::External(function_id) = target
                && CollectionBuiltinOp::from_external_function_id(*function_id).is_none()
            {
                let signature = self
                    .external_packages
                    .get_function_by_id(*function_id)
                    .and_then(|function| function.error_return_type.as_ref())
                    .ok_or_else(|| {
                        self.error_with_hir("Invoke external function has no error slot")
                    })?;
                if !self.external_signature_matches(signature, self.local_types[&error])? {
                    return Err(self.error_with_hir(
                        "Invoke error destination type does not match external signature",
                    ));
                }
            }
            if let Some(expected) = expected_error
                && self.local_types.get(&error) != Some(&expected)
            {
                return Err(self.error_with_hir(
                    "Invoke error destination type does not match function error slot",
                ));
            }
            return Ok(());
        }
        if self.local_types.get(&error) != Some(&self.builtin_error_type()?) {
            return Err(
                self.error_with_hir("Invoke builtin error destination has the wrong error type")
            );
        }
        if let HirFallibleOperation::MapOp { op, receiver, args } = operation {
            return self.validate_map_channels(
                *op,
                *receiver,
                *args,
                results.iter().copied(),
                true,
                anchor,
            );
        }
        let arity = if matches!(operation, HirFallibleOperation::RangeStepFailure { .. }) {
            0
        } else {
            1
        };
        if results.len() != arity {
            return Err(self.error_with_hir("Invoke operation success slot arity is invalid"));
        }
        if arity == 0 {
            return Ok(());
        }
        let result = HirLocalDestination::Define(results[0]);
        let kind = match operation {
            HirFallibleOperation::CastOp { policy, source } => {
                self.validate_expression(*source, anchor)?;
                let target_type = self.local_types[&results[0]];
                self.validate_cast_channels(
                    *policy,
                    self.expression_row(*source)?.ty,
                    target_type,
                )?;
                return Ok(());
            }
            HirFallibleOperation::NumericOp { op, operands } => HirStatementKind::NumericOp {
                op: *op,
                operands: operands.clone(),
                failure_mode: NumericFailureMode::Trap,
                result,
            },
            HirFallibleOperation::FormatFloat { source } => HirStatementKind::FormatFloat {
                source: *source,
                failure_mode: NumericFailureMode::Trap,
                result,
            },
            HirFallibleOperation::ValidateFloat { source } => HirStatementKind::ValidateFloat {
                source: *source,
                failure_mode: NumericFailureMode::Trap,
                result,
            },
            HirFallibleOperation::Call { .. }
            | HirFallibleOperation::MapOp { .. }
            | HirFallibleOperation::RangeStepFailure { .. } => {
                crate::compiler_bug!(
                    CompilerStage::HirConstruction,
                    "Call and MapOp already returned from validate_invoke; RangeStepFailure already returned after the zero-arity check"
                )
            }
        };
        self.validate_statement(
            &HirStatement {
                id: *id,
                kind,
                span: None,
            },
            block,
        )
    }

    pub(super) fn validate_map_channels(
        &self,
        op: HirMapOp,
        receiver: HirValueId,
        args: HirValueRange,
        results: impl ExactSizeIterator<Item = LocalId>,
        fallible: bool,
        anchor: Option<HirLocation>,
    ) -> Result<(), InfrastructureFailure> {
        self.validate_expression(receiver, anchor)?;
        let shape = self
            .type_environment
            .map_shape(self.expression_row(receiver)?.ty)
            .ok_or_else(|| self.error_with_hir("Map operation receiver must have a map type"))?;
        let op = match op {
            HirMapOp::Get => MapBuiltinOp::Get,
            HirMapOp::Contains => MapBuiltinOp::Contains,
            HirMapOp::Set => MapBuiltinOp::Set,
            HirMapOp::Remove => MapBuiltinOp::Remove,
            HirMapOp::Clear => MapBuiltinOp::Clear,
            HirMapOp::Length => MapBuiltinOp::Length,
        };
        let rule = op.rule(
            shape.key_type,
            shape.value_type,
            self.type_environment.builtins().bool,
            crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                FixedScalar::I32,
            ),
        );
        self.validate_builtin_member_slots(rule, self.value_ids(args)?, results, fallible, anchor)
    }

    fn validate_builtin_member_slots(
        &self,
        rule: BuiltinMemberRule<TypeId>,
        args: &[HirValueId],
        mut results: impl ExactSizeIterator<Item = LocalId>,
        fallible: bool,
        anchor: Option<HirLocation>,
    ) -> Result<(), InfrastructureFailure> {
        if rule.fallible != fallible {
            return Err(self.error_with_hir(
                "Builtin operation channel compatibility does not match semantic signature",
            ));
        }
        if args.len() != rule.arguments.len()
            || results.len() != usize::from(rule.success.is_some())
        {
            return Err(self.error_with_hir(
                "Builtin operation argument/result slot arity does not match semantic signature",
            ));
        }
        for (index, arg) in args.iter().enumerate() {
            self.validate_expression(*arg, anchor)?;
            if rule.arguments.get(index) != Some(self.expression_row(*arg)?.ty) {
                return Err(self.error_with_hir(
                    "Builtin operation argument type does not match semantic signature",
                ));
            }
        }
        if let Some(expected) = rule.success {
            // Arity validation above proves there is exactly one result.
            let result = results.next().ok_or_else(|| {
                self.error_with_hir("Builtin operation is missing its result destination")
            })?;
            self.require_local_id(result)?;
            if self.local_types.get(&result) != Some(&expected) {
                return Err(self.error_with_hir(
                    "Builtin operation result type does not match semantic signature",
                ));
            }
        }
        Ok(())
    }

    fn validate_cast_channels(
        &self,
        policy: BuiltinCastPolicyId,
        source: TypeId,
        target: TypeId,
    ) -> Result<(), InfrastructureFailure> {
        let pair = match policy {
            BuiltinCastPolicyId::NumericConversion { source, target } => (
                BuiltinCastTarget::from(source),
                BuiltinCastTarget::from(target),
            ),
            BuiltinCastPolicyId::NumericToString(source) => {
                (BuiltinCastTarget::from(source), BuiltinCastTarget::String)
            }
            BuiltinCastPolicyId::StringToNumeric(target) => {
                (BuiltinCastTarget::String, BuiltinCastTarget::from(target))
            }
            BuiltinCastPolicyId::ByteToU8 => (
                BuiltinCastTarget::Fixed(FixedScalar::Byte),
                BuiltinCastTarget::Fixed(FixedScalar::U8),
            ),
            BuiltinCastPolicyId::U8ToByte => (
                BuiltinCastTarget::Fixed(FixedScalar::U8),
                BuiltinCastTarget::Fixed(FixedScalar::Byte),
            ),
            BuiltinCastPolicyId::BoolToString => {
                (BuiltinCastTarget::Bool, BuiltinCastTarget::String)
            }
            BuiltinCastPolicyId::CharToString => {
                (BuiltinCastTarget::Char, BuiltinCastTarget::String)
            }
            BuiltinCastPolicyId::CharToInt => (
                BuiltinCastTarget::Char,
                BuiltinCastTarget::Fixed(FixedScalar::I32),
            ),
            BuiltinCastPolicyId::StringToError => {
                (BuiltinCastTarget::String, BuiltinCastTarget::Error)
            }
            BuiltinCastPolicyId::ErrorToString => {
                (BuiltinCastTarget::Error, BuiltinCastTarget::String)
            }
            BuiltinCastPolicyId::IntToChar => (
                BuiltinCastTarget::Fixed(FixedScalar::I32),
                BuiltinCastTarget::Char,
            ),
            BuiltinCastPolicyId::StringToBool => {
                (BuiltinCastTarget::String, BuiltinCastTarget::Bool)
            }
            BuiltinCastPolicyId::StringToChar => {
                (BuiltinCastTarget::String, BuiltinCastTarget::Char)
            }
        };
        let row = lookup_builtin_evidence(pair.0, pair.1)
            .ok_or_else(|| self.error_with_hir("Cast policy has no builtin evidence"))?;
        if row.policy != policy || row.fallibility != BuiltinCastFallibility::Fallible {
            return Err(self.error_with_hir("Cast policy has the wrong fallibility"));
        }
        let expected_target = self.cast_target_type(pair.1)?;
        if source != self.cast_target_type(pair.0)? || target != expected_target {
            return Err(self.error_with_hir("Cast policy source/target type does not match policy"));
        }
        Ok(())
    }

    fn external_signature_matches(
        &self,
        signature: &crate::compiler_frontend::external_packages::ExternalSignatureType,
        ty: TypeId,
    ) -> Result<bool, InfrastructureFailure> {
        use crate::compiler_frontend::external_packages::{ExternalAbiType, ExternalSignatureType};
        let builtins = self.type_environment.builtins();
        Ok(match signature {
            ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar)) => {
                ty == crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(
                    *scalar,
                )
            }
            ExternalSignatureType::Abi(ExternalAbiType::Bool) => ty == builtins.bool,
            ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)
            | ExternalSignatureType::StringContent => ty == builtins.string,
            ExternalSignatureType::Abi(ExternalAbiType::Char) => ty == builtins.char,
            ExternalSignatureType::Abi(ExternalAbiType::Inferred) => true,
            ExternalSignatureType::Abi(ExternalAbiType::Void | ExternalAbiType::Handle) => false,
            ExternalSignatureType::BuiltinError => ty == self.builtin_error_type()?,
            ExternalSignatureType::External(expected) => matches!(
                self.type_environment.get(ty),
                Some(crate::compiler_frontend::datatypes::definitions::TypeDefinition::External(definition)) if definition.type_id == *expected
            ),
            ExternalSignatureType::Optional(inner) => {
                match self.type_environment.option_inner_type(ty) {
                    Some(inner_type) => self.external_signature_matches(inner, inner_type)?,
                    None => false,
                }
            }
        })
    }

    fn cast_target_type(&self, target: BuiltinCastTarget) -> Result<TypeId, InfrastructureFailure> {
        let builtins = self.type_environment.builtins();
        Ok(match target {
            BuiltinCastTarget::Bool => builtins.bool,
            BuiltinCastTarget::String => builtins.string,
            BuiltinCastTarget::Char => builtins.char,
            BuiltinCastTarget::Error => self.builtin_error_type()?,
            BuiltinCastTarget::Fixed(scalar) => {
                crate::compiler_frontend::datatypes::ids::builtin_type_ids::fixed_scalar(scalar)
            }
            BuiltinCastTarget::Number(scale) => self.numeric_scalar_type_id(
                crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar::Number(scale),
            )?,
        })
    }
}
