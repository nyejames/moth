//! Template and compile-time diagnostic text renderers.
//!
//! WHAT: renders diagnostics for template slot structure and compile-time evaluation failures.
//! WHY: template diagnostics are a broad payload family and keeping them out of `render::mod`
//! makes the render boundary easier to scan.

use super::context::DiagnosticRenderContext;
use super::named_value_or_default;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::CompileTimeEvaluationErrorView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::{
    InvalidTemplateSlotView, InvalidTemplateStructureCode, InvalidTemplateStructureView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::RenderedPayload;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerStage, InvalidTemplateSlotReason,
    InvalidTemplateStructureReason, StoredTemplateHeadConflict, TemplateHeadConflictOwner,
    TemplateHeadConflictReason, TemplateHeadItemKind,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTableResolver};

pub(crate) fn invalid_template_slot_message(
    reason: InvalidTemplateSlotReason,
    slot_name: Option<StringId>,
    insert_name: Option<StringId>,
    string_table: &dyn StringTableResolver,
) -> String {
    let slot_text = named_value_or_default(slot_name, string_table, "this slot");

    match reason {
        InvalidTemplateSlotReason::InsertOutsideParentSlot => {
            "$insert(...) can only be used while filling an immediate parent template that defines matching $slot targets.".to_string()
        }
        InvalidTemplateSlotReason::NamedInsertOutsideParentSlot => {
            named_insert_outside_parent_slot_message(slot_name, insert_name, string_table)
        }
        InvalidTemplateSlotReason::ExtraLooseContentWithoutDefaultSlot => {
            "This template defines positional $slot(n) targets but no default $slot. There is more loose content than positional slots available.".to_string()
        }
        InvalidTemplateSlotReason::LooseContentWithoutDefaultSlot => {
            "This template defines named $slot(...) targets without a default $slot. Loose content is not allowed here; use $insert(\"name\").".to_string()
        }
        InvalidTemplateSlotReason::InsertCannotTargetDefaultSlot => {
            "$insert cannot target the default slot because the parent template does not define $slot.".to_string()
        }
        InvalidTemplateSlotReason::InsertTargetsUnknownNamedSlot => {
            format!("$insert({slot_text}) targets a named slot that does not exist on the immediate parent template.")
        }
        InvalidTemplateSlotReason::InsertTargetsUnknownPositionalSlot => {
            "$insert targets a positional slot that does not exist on the immediate parent template.".to_string()
        }
        InvalidTemplateSlotReason::SlotDefinitionOutsideTemplateBody => {
            "$slot markers are only valid as direct nested templates inside template bodies.".to_string()
        }
    }
}

/// Renders the escaped named-insert refusal.
///
/// WHAT: names the authored insert constant and the slot it targets when both survived parsing.
/// WHY: the authored name is what the reader can search for, while the target slot names the
///      parent template shape that would make the insertion valid.
fn named_insert_outside_parent_slot_message(
    slot_name: Option<StringId>,
    insert_name: Option<StringId>,
    string_table: &dyn StringTableResolver,
) -> String {
    match (insert_name, slot_name) {
        (Some(insert), Some(slot)) => {
            let insert = string_table.resolve(insert);
            let slot = string_table.resolve(slot);
            format!(
                "The {slot} insert '{insert}' needs a parent template that defines the '{slot}' slot, but this template has no parent template."
            )
        }
        (Some(insert), None) => {
            let insert = string_table.resolve(insert);
            format!(
                "The named insert '{insert}' needs a parent template with a matching slot, but this template has no parent template."
            )
        }
        (None, Some(slot)) => {
            let slot = string_table.resolve(slot);
            format!(
                "This named insert needs a parent template that defines the '{slot}' slot, but this template has no parent template."
            )
        }
        (None, None) => "This named insert needs a parent template with a matching slot, but this template has no parent template.".to_string(),
    }
}

/// Renders a template-slot diagnostic from its compact facts.
/// WHY: the reason and optional names are the complete input to `invalid_template_slot_message`.
pub(crate) fn render_invalid_template_slot(
    _diagnostic: &DiagnosticView<'_>,
    facts: &InvalidTemplateSlotView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let message = invalid_template_slot_message(
        facts.reason,
        facts.slot_name,
        facts.insert_name,
        context.string_table,
    );
    let guidance = match facts.reason {
        InvalidTemplateSlotReason::NamedInsertOutsideParentSlot => {
            vec![named_insert_outside_parent_slot_guidance(
                facts.slot_name,
                context.string_table,
            )]
        }
        _ => Vec::new(),
    };
    RenderedPayload { message, guidance }
}

fn named_insert_outside_parent_slot_guidance(
    slot_name: Option<StringId>,
    string_table: &dyn StringTableResolver,
) -> String {
    // A string slice is the documented way to emit a literal square bracket in a template.
    const LITERAL_BRACKETS_EXAMPLE: &str = r#"[: ["[literal]"]]"#;
    match slot_name {
        Some(slot) => {
            let slot = string_table.resolve(slot);
            format!(
                "Wrap the content in a template that defines a '{slot}' slot, or write the brackets as a string slice when you meant literal text, for example {LITERAL_BRACKETS_EXAMPLE}."
            )
        }
        None => format!(
            "Wrap the content in a template that defines the matching slot, or write the brackets as a string slice when you meant literal text, for example {LITERAL_BRACKETS_EXAMPLE}."
        ),
    }
}

pub(crate) fn compile_time_evaluation_error_message(
    reason: CompileTimeEvaluationErrorReason,
    operation: Option<StringId>,
    numeric_type: Option<NumericScalar>,
    string_table: &dyn StringTableResolver,
) -> String {
    let operation_text = named_value_or_default(operation, string_table, "this expression");
    // The numeric domain is carried as a typed fact, so a failure can name the exact type that
    // overflowed instead of leaving the reader to infer it from the expression.
    let numeric_type_text = numeric_type.map(|numeric_type| numeric_type.to_string());
    let numeric_suffix = numeric_type_text
        .map(|numeric_type| format!(" in {numeric_type}"))
        .unwrap_or_default();

    match reason {
        CompileTimeEvaluationErrorReason::IntegerOverflow => {
            format!(
                "Compile-time integer overflow while evaluating {operation_text}{numeric_suffix}."
            )
        }
        CompileTimeEvaluationErrorReason::FloatOverflow => {
            format!(
                "Compile-time float overflow or non-finite result while evaluating {operation_text}{numeric_suffix}."
            )
        }
        CompileTimeEvaluationErrorReason::DecExponentIntegerOverflow => {
            format!(
                "Compile-time integer overflow while evaluating {operation_text}{numeric_suffix}. A Dec power takes an I32 exponent, so the exponent operand is evaluated as I32 arithmetic before the Dec power runs."
            )
        }
        CompileTimeEvaluationErrorReason::DivideByZero => "Cannot divide by zero.".to_string(),
        CompileTimeEvaluationErrorReason::InvalidExponent => {
            format!("Invalid exponent while evaluating {operation_text}.")
        }
        CompileTimeEvaluationErrorReason::InvalidOperatorForType => {
            format!("Cannot perform operation {operation_text} on this type.")
        }
        CompileTimeEvaluationErrorReason::ConstantSelfReference => {
            format!("Constant {operation_text} cannot reference itself in its initializer.")
        }
        CompileTimeEvaluationErrorReason::ConstantNotVisible => {
            format!("Constant {operation_text} is not visible in this file.")
        }
        CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant => {
            format!(
                "Constants can only reference other constants. {operation_text} resolves to a non-constant value."
            )
        }
        CompileTimeEvaluationErrorReason::SameFileForwardConstantReference => {
            format!(
                "Constant initializer references same-file constant {operation_text} before it is declared."
            )
        }
        CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable => {
            format!("Constant {operation_text} is not compile-time resolvable.")
        }
        CompileTimeEvaluationErrorReason::ExternalNonScalarConstantInConstantContext => {
            format!(
                "External constant {operation_text} is not a scalar value and cannot be used in a constant context."
            )
        }
        CompileTimeEvaluationErrorReason::NonCompileTimeFieldInConstantContext => {
            format!(
                "Const coercion requires compile-time field values. {operation_text} is not compile-time constant."
            )
        }
        CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext => {
            "The 'none' literal requires an explicit optional type context.".to_string()
        }
        CompileTimeEvaluationErrorReason::ExternalTypeConstructionNotSupported => {
            format!(
                "Cannot construct external type {operation_text} with a struct literal. External types are opaque and can only be obtained from external function calls."
            )
        }
        CompileTimeEvaluationErrorReason::StructFieldDefaultNotFoldable => {
            format!("Struct field default value {operation_text} is not compile-time resolvable.")
        }
        CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText => {
            format!(
                "Cannot use a resource-bearing or site-root string for {operation_text}. Those pieces have no final text until the build assigns URL contexts. Keep the operation at runtime or use plain text without structural pieces."
            )
        }
        CompileTimeEvaluationErrorReason::EvaluationOutputBoundExceeded => {
            format!(
                "Compile-time evaluation of {operation_text} exceeds its supported output bound."
            )
        }
    }
}

pub(crate) fn compile_time_evaluation_error_suggestion(
    reason: CompileTimeEvaluationErrorReason,
) -> &'static str {
    match reason {
        CompileTimeEvaluationErrorReason::IntegerOverflow
        | CompileTimeEvaluationErrorReason::FloatOverflow => {
            "Use smaller values or avoid compile-time evaluation of large expressions"
        }
        CompileTimeEvaluationErrorReason::DecExponentIntegerOverflow => {
            "Use an exponent expression that fits I32. A Dec power cannot receive an exponent computed beyond I32."
        }
        CompileTimeEvaluationErrorReason::DivideByZero => {
            "Avoid division by zero in compile-time expressions"
        }
        CompileTimeEvaluationErrorReason::InvalidExponent => {
            "Use an exponent that is valid for the numeric operation"
        }
        CompileTimeEvaluationErrorReason::InvalidOperatorForType => {
            "Use an operator that is valid for the operand types"
        }
        CompileTimeEvaluationErrorReason::ConstantSelfReference => {
            "A constant cannot depend on itself. Use a different value or compute it differently."
        }
        CompileTimeEvaluationErrorReason::ConstantNotVisible => {
            "Bind the compile-time constant before using it in this constant initializer."
        }
        CompileTimeEvaluationErrorReason::NonConstantReferenceInConstant => {
            "Only reference constants in constant declarations and const templates. Add `#` to the binding declaration to make it a compile-time constant."
        }
        CompileTimeEvaluationErrorReason::SameFileForwardConstantReference => {
            "Move the referenced constant above this declaration, or bind it from another file."
        }
        CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable => {
            "Constants may only contain compile-time values and constant references."
        }
        CompileTimeEvaluationErrorReason::ExternalNonScalarConstantInConstantContext => {
            "Only scalar external constants (integers, binary floats and Bool) are supported in constant declarations and const templates"
        }
        CompileTimeEvaluationErrorReason::NonCompileTimeFieldInConstantContext => {
            "Use only compile-time values when constructing records or choices for top-level compile-time constants"
        }
        CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext => {
            "Add an explicit optional type annotation, for example `value String? = none`"
        }
        CompileTimeEvaluationErrorReason::ExternalTypeConstructionNotSupported => {
            "Use an external function that returns this type instead"
        }
        CompileTimeEvaluationErrorReason::StructFieldDefaultNotFoldable => {
            "Struct field defaults may only contain compile-time values and constant references."
        }
        CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText => {
            "Use a plain text string for compile-time character operations, or perform the operation at runtime"
        }
        CompileTimeEvaluationErrorReason::EvaluationOutputBoundExceeded => {
            "Use an input whose compile-time result fits the operation's supported range"
        }
    }
}

/// WHAT: renders a compile-time evaluation error from its compact reason and operation.
/// WHY: the reason and operation feed the message, and the reason also selects the guidance.
pub(crate) fn render_compile_time_evaluation_error(
    _diagnostic: &DiagnosticView<'_>,
    facts: &CompileTimeEvaluationErrorView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: compile_time_evaluation_error_message(
            facts.reason,
            facts.operation,
            facts.numeric_type,
            context.string_table,
        ),
        guidance: vec![compile_time_evaluation_error_suggestion(facts.reason).to_owned()],
    }
}

fn unsupported_template_head_type_message(type_name: &str) -> String {
    format!(
        "Template head expressions only accept final scalar or textual values. Found: {}",
        type_name
    )
}

/// Names one side of a rejected head-item pair: a directive with its `$` marker, or `an
/// expression` for a non-directive head item.
fn template_head_item_display(
    name: Option<StringId>,
    string_table: &dyn StringTableResolver,
) -> String {
    match name {
        Some(name) => format!("`${}`", string_table.resolve(name)),
        None => "an expression".to_owned(),
    }
}

/// Names both conflicting items and explains the semantic rule they violate.
fn incompatible_template_head_item_message(
    conflict: StoredTemplateHeadConflict,
    diagnostic: &DiagnosticView<'_>,
    string_table: &dyn StringTableResolver,
) -> String {
    // Directive names are stored incoming first, then earlier. Expression items store no name.
    let mut names = diagnostic.string_ids().iter();
    let mut next_name = |kind: TemplateHeadItemKind| match kind {
        TemplateHeadItemKind::Directive => Some(
            *names
                .next()
                .expect("a directive conflict item stores its interned name"),
        ),
        TemplateHeadItemKind::Expression => None,
    };
    let incoming = template_head_item_display(next_name(conflict.incoming_kind), string_table);
    let earlier = template_head_item_display(next_name(conflict.earlier_kind), string_table);

    let (subject, other) = match conflict.owner {
        TemplateHeadConflictOwner::EarlierItem => (&earlier, &incoming),
        TemplateHeadConflictOwner::IncomingItem => (&incoming, &earlier),
    };

    // A repeated directive is one item appearing twice, not two items that fail to combine.
    // Formatters keep their body-formatting explanation; every other reason already says
    // why the directive may not share the head, so the simple duplicate wording is truthful.
    let repeated_directive = conflict.earlier_kind == TemplateHeadItemKind::Directive
        && conflict.incoming_kind == TemplateHeadItemKind::Directive
        && earlier == incoming;
    if repeated_directive {
        return match conflict.reason {
            TemplateHeadConflictReason::MultipleFormatters => format!(
                "{incoming} cannot appear more than once in a template head because each formatter controls template body formatting. Choose one formatter."
            ),
            _ => format!("{incoming} cannot appear more than once in a template head."),
        };
    }

    match conflict.reason {
        TemplateHeadConflictReason::MultipleFormatters => format!(
            "{earlier} and {incoming} cannot be combined because both control template body formatting. Choose one formatter."
        ),
        TemplateHeadConflictReason::DuplicateExclusiveDirective => {
            format!("{subject} cannot appear more than once in a template head.")
        }
        TemplateHeadConflictReason::DiscardedCommentDirectiveMustBeOnlyMeaningfulItem => format!(
            "{subject} cannot be combined with {other} because a discarded comment directive must be the template's only meaningful head item."
        ),
        TemplateHeadConflictReason::DocumentationDirectiveMustBeOnlyMeaningfulItem => format!(
            "{subject} cannot be combined with {other} because a documentation directive must be the template's only meaningful head item."
        ),
        TemplateHeadConflictReason::SlotDirectiveMustBeExclusive => format!(
            "{subject} cannot be combined with {other} because a slot declaration must be the template head's only meaningful item."
        ),
    }
}

pub(crate) fn invalid_template_structure_message(reason: InvalidTemplateStructureReason) -> String {
    match reason {

        // Template delimiters, nesting and direct child structure.
        InvalidTemplateStructureReason::MissingClosingBracket => {
            "This template is not closed before the end of the file. Add `]`.".to_string()
        }
        InvalidTemplateStructureReason::SlotInHead => {
            "Slot insertions cannot appear in template heads.".to_string()
        }
        InvalidTemplateStructureReason::MissingHandlerBody => {
            "Template handler is missing a body.".to_string()
        }
        InvalidTemplateStructureReason::InvalidChildDirective => {
            "Invalid child directive in template.".to_string()
        }
        InvalidTemplateStructureReason::NestedTemplateNotAllowed => {
            "Nested templates are not allowed here.".to_string()
        }

        // Template-head values and compile-time requirements.
        InvalidTemplateStructureReason::HelperInConstTemplate => {
            "Top-level const templates cannot evaluate to '$insert(...)' helpers.".to_string()
        }
        InvalidTemplateStructureReason::NonFoldableConstTemplate => {
            "Top-level const templates must be fully foldable at compile time.".to_string()
        }
        InvalidTemplateStructureReason::NonFoldableDocComment => {
            "'$doc' comments can only contain compile-time values.".to_string()
        }
        InvalidTemplateStructureReason::FallibleValueInTemplateHead => {
            "Template heads do not implicitly unwrap fallible values. Handle the error before using the success value, with postfix `!` in a compatible fallible function or with `catch` recovery.".to_string()
        }
        InvalidTemplateStructureReason::UnsupportedTypeInTemplateHead { .. } => {
            crate::compiler_bug!(
                CompilerStage::DiagnosticReporting,
                "template-head type details are rendered from the snapshot"
            )
        }
        InvalidTemplateStructureReason::RuntimeTemplateInConst => {
            "Const templates can only capture compile-time templates.".to_string()
        }
        InvalidTemplateStructureReason::RuntimeValueInConstTemplateHead => {
            "Const templates can only capture compile-time values in the template head.".to_string()
        }

        // Head composition and runtime slot routing.
        InvalidTemplateStructureReason::EmptyPathInTemplateHead => {
            "Path token in template head cannot be empty.".to_string()
        }
        InvalidTemplateStructureReason::IncompatibleHeadItem { .. } => {
            crate::compiler_bug!(
                CompilerStage::DiagnosticReporting,
                "template-head conflicts render from their stored conflict fact and item names"
            )
        }
        InvalidTemplateStructureReason::HelperOutsideWrapperSlot => {
            "`$insert(...)` is a template contribution helper, not a standalone value. Keep it inside a template application that receives the contribution.".to_string()
        }
        InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedInsert => {
            "Runtime template control-flow bodies cannot leave unresolved `$insert(...)` helpers.".to_string()
        }

        // Control-flow suffix syntax.
        InvalidTemplateStructureReason::MissingCommaBeforeControlFlowSuffix => {
            "Template control-flow suffixes must be separated from earlier head items with a comma.".to_string()
        }
        InvalidTemplateStructureReason::ControlFlowSuffixNotFinal => {
            "Template control-flow suffixes must be the final item in the template head.".to_string()
        }
        InvalidTemplateStructureReason::MissingTemplateIfCondition => {
            "Template `if` suffix is missing a condition.".to_string()
        }
        InvalidTemplateStructureReason::MissingTemplateLoopHeader => {
            "Template `loop` suffix is missing a Bool condition, collection source or numeric range header."
                .to_string()
        }
        InvalidTemplateStructureReason::UnexpectedTokenAfterControlFlowSuffix => {
            "Unexpected token after template control-flow suffix.".to_string()
        }
        InvalidTemplateStructureReason::TemplateMatchStyleControlFlowUnsupported => {
            "Template `if` heads support Bool conditions and option-present capture only. Use ordinary statement/value `if value is:` blocks for pattern matching outside templates.".to_string()
        }

        // Const-required selectors and loop contents.
        InvalidTemplateStructureReason::TemplateIfConditionNotConst => {
            "This template must be fully evaluated at compile time, so its `if` condition must fold to a Bool.".to_string()
        }
        InvalidTemplateStructureReason::TemplateIfBranchNotConst => {
            "This template must be fully evaluated at compile time, so its `if` body must be a compile-time value even when it is not selected.".to_string()
        }
        InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred => {
            "This template must be fully evaluated at compile time, but the optional value's presence cannot be determined at compile time.".to_string()
        }
        InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst => {
            "This template must be fully evaluated at compile time, so its range-loop bounds must fold to numeric values.".to_string()
        }
        InvalidTemplateStructureReason::TemplateLoopSourceNotConst => {
            "This template must be fully evaluated at compile time, so its collection-loop source must fold to a collection.".to_string()
        }
        InvalidTemplateStructureReason::TemplateLoopConditionNotConst => {
            "This template must be fully evaluated at compile time, so its conditional-loop condition must fold to a Bool.".to_string()
        }
        InvalidTemplateStructureReason::TemplateConditionalLoopConstTrue => {
            "A conditional loop that is true at compile time cannot appear in a template that must be fully evaluated at compile time because it may not terminate.".to_string()
        }
        InvalidTemplateStructureReason::TemplateLoopBodyNotConst => {
            "This template must be fully evaluated at compile time, so its loop body must be a compile-time value for every iteration.".to_string()
        }
        InvalidTemplateStructureReason::TemplateConstLoopExpansionLimitExceeded { limit } => {
            format!(
                "Const template loop expansion is limited to {} iterations.",
                limit
            )
        }
    }
}

/// WHAT: renders a compact template-structure diagnostic from its typed reason and associated facts.
/// WHY: the code rebuilds the reason `invalid_template_structure_message` consumes; type
/// displays come from the frozen display snapshot.
pub(crate) fn render_invalid_template_structure(
    diagnostic: &DiagnosticView<'_>,
    facts: &InvalidTemplateStructureView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let message = match facts.reason {
        InvalidTemplateStructureCode::UnsupportedTypeInTemplateHead => {
            let type_id = facts
                .type_id
                .expect("an unsupported template-head type code stores its type display");
            let type_name = diagnostic.types().display_for_render(
                type_id,
                context.string_table,
                context.path_table(),
            );
            unsupported_template_head_type_message(&type_name)
        }
        InvalidTemplateStructureCode::IncompatibleHeadItem => {
            let conflict = facts
                .head_conflict
                .expect("the incompatible-head-item code stores its conflict");
            incompatible_template_head_item_message(conflict, diagnostic, context.string_table)
        }
        reason => {
            let reason = reason
                .to_reason(None, facts.limit)
                .expect("the template-structure code stores the data its reason names");
            invalid_template_structure_message(reason)
        }
    };
    RenderedPayload {
        message,
        guidance: Vec::new(),
    }
}
