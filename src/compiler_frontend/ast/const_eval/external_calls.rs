//! Compiler-owned evaluators for trusted Core host calls.
//!
//! WHAT: maps registered operation tokens and settled constant arguments to ordinary payloads,
//! then materialises every registered success slot as an ordinary constant value.
//! WHY: external-call semantics belong beside the shared fold engine, never in package spelling or backend code.

use super::{
    ConstStringRequirement, ConstantFoldError, ConstantFoldRefusalReason, concrete_payload_text,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueCandidate, ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionView;
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic,
};
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::external_packages::{
    ConstEvalLeafType, ConstEvalSlotType, TextConstEvalOp,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// Outcome of one trusted call. A folded call pushes one payload per success slot, in slot-table
/// order, into the caller's buffer; a refusal leaves that buffer empty.
pub(super) enum ExternalCallEvaluation {
    Folded,
    Refused(ConstantFoldRefusalReason),
}

/// Evaluates a trusted call into `payloads`, which is cleared on entry. A fold owns one buffer for
/// all of its calls, so evaluation does not allocate a result vector per call.
pub(super) fn evaluate_external_call(
    operation: TextConstEvalOp,
    arguments: &[Option<ConstValueId>],
    values: &ConstValueCandidate<'_>,
    strings: &mut StringTable,
    span: Option<SourceSpan>,
    payloads: &mut Vec<Option<ConstValuePayload>>,
) -> Result<ExternalCallEvaluation, ConstantFoldError> {
    payloads.clear();
    let expected_argument_count = operation.parameter_types().len();
    if arguments.len() != expected_argument_count {
        return Err(CompilerError::compiler_error(
            "Trusted Core Text call arguments disagree with its registered operation.",
        )
        .into());
    }

    let first_value = settled_argument(arguments, 0)?;
    let first_text = match text_argument_id(first_value, values, strings, span)? {
        TextArgument::Concrete(text) => text,
        TextArgument::Refused(reason) => return Ok(ExternalCallEvaluation::Refused(reason)),
    };
    let second_text = if expected_argument_count == 2 {
        let second_value = settled_argument(arguments, 1)?;
        match text_argument_id(second_value, values, strings, span)? {
            TextArgument::Concrete(text) => Some(text),
            TextArgument::Refused(reason) => return Ok(ExternalCallEvaluation::Refused(reason)),
        }
    } else {
        None
    };

    if operation == TextConstEvalOp::Length {
        let length = strings.resolve(first_text).chars().count();
        return checked_text_length_evaluation(length, span, strings, payloads);
    }
    let first = strings.resolve(first_text);
    let payload = match operation {
        TextConstEvalOp::Length => {
            return Err(CompilerError::compiler_error(
                "Text length evaluation was handled before string comparisons.",
            )
            .into());
        }
        TextConstEvalOp::IsEmpty => ConstValuePayload::Bool(first.is_empty()),
        TextConstEvalOp::Contains => {
            let pattern = strings.resolve(second_text.ok_or_else(|| {
                CompilerError::compiler_error("Text contains evaluation lost its pattern argument.")
            })?);
            ConstValuePayload::Bool(first.contains(pattern))
        }
        TextConstEvalOp::StartsWith => {
            let prefix = strings.resolve(second_text.ok_or_else(|| {
                CompilerError::compiler_error(
                    "Text starts-with evaluation lost its prefix argument.",
                )
            })?);
            ConstValuePayload::Bool(first.starts_with(prefix))
        }
        TextConstEvalOp::EndsWith => {
            let suffix = strings.resolve(second_text.ok_or_else(|| {
                CompilerError::compiler_error("Text ends-with evaluation lost its suffix argument.")
            })?);
            ConstValuePayload::Bool(first.ends_with(suffix))
        }
    };

    payloads.push(Some(payload));
    Ok(ExternalCallEvaluation::Folded)
}

/// Materialises a trusted call's success slots as ordinary constant values, in result order, into
/// `ids`, which is cleared first.
///
/// WHAT: a present leaf is stored under its builtin type; an optional slot wraps it once in
/// `OptionSome`. An absent optional slot stores `OptionNone` under the slot's own type.
/// WHY: optional absence is an ordinary successful value, distinct from refusal, zero results
/// and an error channel, so it materialises as a value and never as a failed fold.
pub(super) fn materialise_success_slots(
    slots: &[ConstEvalSlotType],
    result_types: &[TypeId],
    payloads: impl ExactSizeIterator<Item = Option<ConstValuePayload>>,
    original: ExpressionView<'_>,
    provenance: &SyntheticInterfaceProvenance,
    values: &mut ConstValueCandidate<'_>,
    ids: &mut Vec<ConstValueId>,
) -> Result<(), ConstantFoldError> {
    // The registry and the call must agree on the slot count. Registration already checked the
    // descriptor against the signature, so a mismatch here is a compiler bug, not a user error.
    if result_types.len() != slots.len() || payloads.len() != slots.len() {
        return Err(CompilerError::compiler_error(
            "Trusted constant call result slots disagree with its registered signature.",
        )
        .into());
    }

    ids.clear();
    ids.reserve(slots.len());
    for ((slot, result_type), payload) in slots.iter().zip(result_types).zip(payloads) {
        ids.push(materialise_success_slot(
            *slot,
            *result_type,
            payload,
            original,
            provenance,
            values,
        )?);
    }
    Ok(())
}

/// Stores one success slot. `None` is an authored absence and is legal only for an optional slot.
fn materialise_success_slot(
    slot: ConstEvalSlotType,
    result_type: TypeId,
    payload: Option<ConstValuePayload>,
    original: ExpressionView<'_>,
    provenance: &SyntheticInterfaceProvenance,
    values: &mut ConstValueCandidate<'_>,
) -> Result<ConstValueId, ConstantFoldError> {
    let payload = match payload {
        Some(payload) => payload,
        None if slot.optional => {
            return insert_slot_value(
                values,
                original,
                result_type,
                ConstValueKind::Literal,
                ConstValuePayload::OptionNone,
                provenance,
            );
        }
        None => {
            return Err(CompilerError::compiler_error(
                "Trusted constant call reported absence for a non-optional success slot.",
            )
            .into());
        }
    };

    if !payload_has_leaf_kind(&payload, slot.leaf) {
        return Err(CompilerError::compiler_error(
            "Trusted constant call payload disagrees with its registered leaf type.",
        )
        .into());
    }

    if !slot.optional {
        // A non-optional slot's result type is its leaf type. Any other type means the call's
        // result and the registered descriptor disagree.
        if result_type != slot.leaf.type_id() {
            return Err(CompilerError::compiler_error(
                "Trusted constant call result type disagrees with its registered leaf type.",
            )
            .into());
        }
        return insert_slot_value(
            values,
            original,
            result_type,
            ConstValueKind::Literal,
            payload,
            provenance,
        );
    }

    // The leaf is stored under its own builtin type, then wrapped once in `OptionSome` under the
    // slot's optional type. This is the same layering `cast_complete` produces for optional wraps.
    // WHY no interning happens here: `result_type` is the call's registered success type.
    // `matches_signature` admits an optional slot only as `Optional(builtin leaf)`, and the ABI
    // interns that as `Option<leaf>`, so `result_type` already is the wrapper. The materialiser
    // holds no `TypeEnvironment` to build it itself.
    let leaf = insert_slot_value(
        values,
        original,
        slot.leaf.type_id(),
        ConstValueKind::Literal,
        payload,
        provenance,
    )?;
    insert_slot_value(
        values,
        original,
        result_type,
        ConstValueKind::Composite,
        ConstValuePayload::OptionSome(leaf),
        provenance,
    )
}

/// Stores one slot value under `type_id`, merging the call's argument provenance into it.
fn insert_slot_value(
    values: &mut ConstValueCandidate<'_>,
    original: ExpressionView<'_>,
    type_id: TypeId,
    value_kind: ConstValueKind,
    payload: ConstValuePayload,
    provenance: &SyntheticInterfaceProvenance,
) -> Result<ConstValueId, ConstantFoldError> {
    let mut metadata = ConstValueStore::metadata_for_type(original, type_id, value_kind);
    metadata.synthetic_interface_provenance.merge(provenance);
    Ok(values.insert_value(metadata, payload)?)
}

/// Checks that a payload belongs to the builtin leaf a slot declares. Each leaf admits exactly
/// one payload family, so a mismatch is a compiler bug rather than a user-facing refusal.
fn payload_has_leaf_kind(payload: &ConstValuePayload, leaf: ConstEvalLeafType) -> bool {
    match (leaf, payload) {
        (ConstEvalLeafType::Text, ConstValuePayload::String(_))
        | (ConstEvalLeafType::Bool, ConstValuePayload::Bool(_)) => true,
        (ConstEvalLeafType::I32, ConstValuePayload::FixedScalar(value)) => {
            matches!(value.scalar(), FixedScalar::I32)
        }
        _ => false,
    }
}

fn settled_argument(
    arguments: &[Option<ConstValueId>],
    index: usize,
) -> Result<ConstValueId, CompilerError> {
    arguments.get(index).copied().flatten().ok_or_else(|| {
        CompilerError::compiler_error("External call evaluator received an unsettled argument.")
    })
}

enum TextArgument {
    Concrete(StringId),
    Refused(ConstantFoldRefusalReason),
}

fn text_argument_id(
    value: ConstValueId,
    values: &ConstValueCandidate<'_>,
    strings: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<TextArgument, ConstantFoldError> {
    let metadata = values.metadata(value).ok_or_else(|| {
        CompilerError::compiler_error("Settled external-call argument has no constant metadata.")
    })?;
    if metadata.type_id != builtin_type_ids::STRING {
        return Err(CompilerError::compiler_error(
            "Trusted Core Text evaluation received an argument with a non-String type.",
        )
        .into());
    }
    let payload = values.payload(value).ok_or_else(|| {
        CompilerError::compiler_error("Settled external-call argument has no constant payload.")
    })?;
    match concrete_payload_text(payload, span, ConstStringRequirement::ExternalCall, strings) {
        Ok(Some(text)) => Ok(TextArgument::Concrete(text)),
        Ok(None) => Err(CompilerError::compiler_error(
            "Trusted Core Text evaluation received a String value without text payload.",
        )
        .into()),
        Err(diagnostic) => Ok(TextArgument::Refused(
            ConstantFoldRefusalReason::StructuralTextUnavailable(diagnostic),
        )),
    }
}

fn checked_text_length_evaluation(
    length: usize,
    span: Option<SourceSpan>,
    strings: &mut StringTable,
    payloads: &mut Vec<Option<ConstValuePayload>>,
) -> Result<ExternalCallEvaluation, ConstantFoldError> {
    let Ok(length) = i32::try_from(length) else {
        let operation = strings.intern("text.length");
        let diagnostic = CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::EvaluationOutputBoundExceeded,
            Some(operation),
            span,
        );
        return Ok(ExternalCallEvaluation::Refused(
            ConstantFoldRefusalReason::EvaluationOutputBoundExceeded(diagnostic),
        ));
    };
    let value = FixedScalarValue::signed(FixedScalar::I32, i64::from(length)).ok_or_else(|| {
        CompilerError::compiler_error("Checked Text length did not fit its I32 payload.")
    })?;
    payloads.push(Some(ConstValuePayload::FixedScalar(value)));
    Ok(ExternalCallEvaluation::Folded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::compiler_messages::DiagnosticPayload;

    #[test]
    fn checked_text_length_evaluation_refuses_values_above_i32_max() {
        let mut strings = StringTable::new();
        let mut payloads = Vec::new();
        let evaluation = checked_text_length_evaluation(
            i32::MAX as usize + 1,
            None,
            &mut strings,
            &mut payloads,
        )
        .expect("bounded length evaluation should return a refusal outcome");

        let ExternalCallEvaluation::Refused(
            ConstantFoldRefusalReason::EvaluationOutputBoundExceeded(diagnostic),
        ) = evaluation
        else {
            panic!("checked Text length should report the output-bound refusal");
        };
        assert!(matches!(
            diagnostic.payload,
            DiagnosticPayload::CompileTimeEvaluationError {
                reason: CompileTimeEvaluationErrorReason::EvaluationOutputBoundExceeded,
                ..
            }
        ));
        assert!(
            payloads.is_empty(),
            "a refused call must leave no partial payloads behind"
        );
    }
}
