//! External fallible call lowering helpers.
//!
//! WHAT: carrier creation and call emission for fallible external function calls.
//! WHY: external calls have a separate metadata path for error types, so their carrier
//! construction is isolated here to keep the main fallible lowering focused on source calls.

use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use super::carrier::EmittedFallibleCarrier;

/// Input struct for lowering a handled external fallible call.
///
/// WHAT: packages the external function ID, arguments, result types, error type, handling policy,
/// and source location into one struct so the lowering helper has a short signature.
/// WHY: external fallible calls have more metadata than source calls; a context struct keeps the
/// call site readable.
pub(crate) struct ExternalFallibleCallLoweringInput<'a> {
    pub(crate) id: crate::compiler_frontend::external_packages::ExternalFunctionId,
    pub(crate) args: &'a [CallArgument],
    pub(crate) result_type_ids: &'a [FrontendTypeId],
    pub(crate) error_type_id: FrontendTypeId,
    pub(crate) handling:
        &'a crate::compiler_frontend::ast::expressions::expression::FallibleExpressionHandling,
    pub(crate) location: &'a SourceLocation,
}

impl<'a> HirBuilder<'a> {
    /// Builds a fallible carrier from explicit success and error type IDs.
    ///
    /// WHAT: interns the fallible carrier type for external calls where the metadata provides the
    /// error type directly instead of deriving it from a user function signature.
    pub(crate) fn fallible_call_carrier_from_slots(
        &mut self,
        result_type_ids: &[FrontendTypeId],
        error_type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<(TypeId, TypeId, TypeId), CompilerError> {
        let ok_type = self.lower_call_result_type(result_type_ids, location)?;
        let err_type = self.lower_type_id(error_type_id, location)?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(ok_type, err_type);

        Ok((carrier_type, ok_type, err_type))
    }

    /// Emits an external fallible call carrier to the current block.
    pub(super) fn emit_external_result_call_carrier_to_current_block(
        &mut self,
        id: crate::compiler_frontend::external_packages::ExternalFunctionId,
        args: &[CallArgument],
        result_type_ids: &[FrontendTypeId],
        error_type_id: FrontendTypeId,
        location: &SourceLocation,
    ) -> Result<EmittedFallibleCarrier, CompilerError> {
        let (carrier_type, ok_type, err_type) =
            self.fallible_call_carrier_from_slots(result_type_ids, error_type_id, location)?;
        let result_local = self.emit_result_call_to_current_block(
            CallTarget::External(id),
            args,
            carrier_type,
            location,
        )?;

        Ok(EmittedFallibleCarrier {
            result_local,
            carrier_type,
            ok_type,
            err_type,
            validate_float_success: self.type_id_is_float(ok_type),
        })
    }
}
