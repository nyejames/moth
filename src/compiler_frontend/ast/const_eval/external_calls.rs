//! Compiler-owned evaluators for trusted single-slot Core host calls.
//!
//! WHAT: maps registered operation tokens and settled constant arguments to ordinary payloads.
//! WHY: external-call semantics belong beside the shared fold engine, never in package spelling or backend code.

use super::{
    ConstStringRequirement, ConstantFoldError, ConstantFoldRefusalReason, concrete_payload_text,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueCandidate, ConstValueId, ConstValuePayload,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::TextConstEvalOp;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

pub(super) enum ExternalCallEvaluation {
    Folded(ConstValuePayload),
    Refused(ConstantFoldRefusalReason),
}

pub(super) fn evaluate_external_call(
    operation: TextConstEvalOp,
    arguments: &[Option<ConstValueId>],
    values: &ConstValueCandidate<'_>,
    strings: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<ExternalCallEvaluation, ConstantFoldError> {
    let expected_argument_count = match operation {
        TextConstEvalOp::Length | TextConstEvalOp::IsEmpty => 1,
        TextConstEvalOp::Contains | TextConstEvalOp::StartsWith | TextConstEvalOp::EndsWith => 2,
    };
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
        return checked_text_length_evaluation(length, span, strings);
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

    Ok(ExternalCallEvaluation::Folded(payload))
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
    Ok(ExternalCallEvaluation::Folded(
        ConstValuePayload::FixedScalar(value),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::compiler_messages::DiagnosticPayload;

    #[test]
    fn checked_text_length_evaluation_refuses_values_above_i32_max() {
        let mut strings = StringTable::new();
        let evaluation = checked_text_length_evaluation(i32::MAX as usize + 1, None, &mut strings)
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
    }
}
