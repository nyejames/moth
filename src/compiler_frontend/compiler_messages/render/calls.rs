//! Call and return diagnostic text renderers.
//!
//! WHAT: renders arity, argument-shape, mutability-at-call, and return-shape diagnostics.
//! WHY: call/return validation is a distinct frontend boundary with its own payload family.

use super::{
    DiagnosticRenderContext, closest_name_suggestion, diagnostic_type_name, named_value_or_default,
};
use crate::compiler_frontend::compiler_messages::{
    InvalidAssignmentTargetReason, InvalidBuiltinCallReason, InvalidCallShapeReason,
    InvalidCastReason, InvalidCopyTargetReason, InvalidFieldAccessReason, InvalidMultiBindReason,
    InvalidReceiverCallReason, InvalidReturnShapeReason, ReceiverCallKind,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

/// Build a "call context" prefix that names the function being called when available.
///
/// WHAT: many call-shape diagnostics carry the callee name as an optional payload field.
/// WHY: including the function name makes the message immediately actionable instead of
/// leaving the user to cross-reference the source location.
fn call_prefix(callee_name: Option<StringId>, string_table: &StringTable) -> String {
    callee_name
        .map(|name| format!("Call to '{}'", string_table.resolve(name)))
        .unwrap_or_else(|| "Call".to_owned())
}

/// Resolve the parameter name for display, falling back to a 1-based position label.
fn parameter_label(
    parameter_name: Option<StringId>,
    parameter_index: usize,
    string_table: &StringTable,
) -> String {
    parameter_name
        .map(|name| format!("parameter '{}'", string_table.resolve(name)))
        .unwrap_or_else(|| format!("parameter {}", parameter_index + 1))
}

pub(crate) fn invalid_call_shape_message(
    reason: InvalidCallShapeReason,
    callee_name: Option<StringId>,
    string_table: &StringTable,
) -> String {
    let prefix = call_prefix(callee_name, string_table);

    match reason {
        InvalidCallShapeReason::MissingArgument {
            parameter_name,
            parameter_index,
        } => {
            let label = parameter_label(parameter_name, parameter_index, string_table);
            format!("{prefix} is missing an argument for {label}.")
        }
        InvalidCallShapeReason::ExtraPositionalArgument { expected_count } => {
            format!("{prefix} provides more positional arguments than expected (expected {expected_count}).")
        }
        InvalidCallShapeReason::DuplicateArgument {
            parameter_name,
            parameter_index,
        } => {
            let label = parameter_label(parameter_name, parameter_index, string_table);
            format!("{prefix} provides an argument for {label} more than once. Each parameter may only be supplied once.")
        }
        InvalidCallShapeReason::NamedArgumentNotFound {
            name,
            known_parameters,
       } => {
            let arg_name = string_table.resolve(name);
            let suggestion = closest_name_suggestion(arg_name, &known_parameters, string_table);
            let known = known_parameters
               .iter()
                .map(|p| string_table.resolve(*p))
                .collect::<Vec<_>>()
                .join(", ");
            match suggestion {
                Some(suggested) => {
                    format!(
                        "{prefix} has named argument '{arg_name}' which does not match any parameter. Did you mean '{suggested}'? Known parameters: [{known}]."
                    )
                }
                None => {
                    format!(
                        "{prefix} has named argument '{arg_name}' which does not match any parameter. Known parameters: [{known}]."
                    )
                }
            }
        }
        InvalidCallShapeReason::PositionalAfterNamed => {
            "Positional arguments must come before named arguments. Reorder the call so all positional arguments come first.".to_string()
        }
        InvalidCallShapeReason::NamedArgumentsNotSupported => {
            "Named arguments are not supported for this call. Use positional arguments only.".to_string()
        }
        InvalidCallShapeReason::MutableAccessRequired {
            parameter_name,
            parameter_index,
        } => {
            let label = parameter_label(parameter_name, parameter_index, string_table);
            format!("{prefix} requires explicit mutable access for {label}. Prefix the existing mutable place with `~`.")
        }
        InvalidCallShapeReason::MutableAccessNotAllowed {
            parameter_name,
            parameter_index,
        } => {
            let label = parameter_label(parameter_name, parameter_index, string_table);
            format!("{prefix} does not accept mutable access for {label}. Remove the authored `~` from this argument.")
        }
        InvalidCallShapeReason::MutableAccessOnNonPlace {
            parameter_name,
            parameter_index,
        } => {
            let label = parameter_label(parameter_name, parameter_index, string_table);
            format!("{prefix} cannot use `~` on a fresh or computed value for {label}. Remove `~` and pass the value directly.")
        }
        InvalidCallShapeReason::MutableAccessOnImmutablePlace {
            parameter_name,
            parameter_index,
            binding_name,
        }
        | InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
            parameter_name,
            parameter_index,
            binding_name,
        } => {
            immutable_place_mutable_access_message(
                &prefix,
                parameter_label(parameter_name, parameter_index, string_table),
                binding_name,
                string_table,
            )
        }
        InvalidCallShapeReason::ReactiveSourceRequired {
            parameter_name,
            parameter_index,
        } => {
            let label = parameter_label(parameter_name, parameter_index, string_table);
            format!("{prefix} requires an existing reactive source for {label}. Pass a value declared with `$Type` or `$=` instead of an ordinary value.")
        }
    }
}

/// Render the shared immutable-place mutable-access message.
///
/// WHAT: used both for an immutable place passed without `~` and for an authored `~` on an
///       immutable place, since the binding declaration is the real mistake in both cases.
/// WHY: the two reasons differ only in which authored source the primary label points at; the
/// prose guidance is identical.
fn immutable_place_mutable_access_message(
    prefix: &str,
    parameter_label: String,
    binding_name: Option<StringId>,
    string_table: &StringTable,
) -> String {
    match binding_name {
        Some(binding) => {
            let binding_text = string_table.resolve(binding);
            format!(
                "{prefix} requires mutable access for {parameter_label}, but `{binding_text}` is immutable. Declare the binding as mutable, then pass `~{binding_text}`."
            )
        }
        None => {
            format!(
                "{prefix} requires mutable access for {parameter_label}, but this argument comes from an immutable binding or field. The binding must be mutable before it is passed with `~`."
            )
        }
    }
}

pub(crate) fn invalid_return_shape_message(reason: InvalidReturnShapeReason) -> String {
    match reason {
        InvalidReturnShapeReason::BareReturnWithExpectedValues { expected_count } => {
            format!("Bare 'return' in a function that expects {expected_count} return value(s).")
        }
        InvalidReturnShapeReason::ReturnValuesWithBareSignature => {
            "This function has no return signature, so 'return' must be bare.".to_string()
        }
        InvalidReturnShapeReason::TooManyReturnValues { expected_count } => {
            format!(
                "Return provides more values than the function signature expects ({expected_count})."
            )
        }
        InvalidReturnShapeReason::TooFewReturnValues {
            expected_count,
            provided_count,
        } => {
            format!(
                "Return provides {provided_count} value(s), but the function expects {expected_count}."
            )
        }
        InvalidReturnShapeReason::MissingReturnBangValue => {
            "return! requires an error value.".to_string()
        }
        InvalidReturnShapeReason::FunctionMayFallThrough => {
            "This function can fall through without returning on every path. Add a return, a terminal else branch, or an `assert(false)` guard to ensure all reachable paths produce a value.".to_string()
        }
    }
}

pub(crate) fn invalid_assignment_target_message(
    reason: InvalidAssignmentTargetReason,
    target_name: Option<StringId>,
    _target_type: Option<TypeId>,
    field_name: Option<StringId>,
    root_binding_name: Option<StringId>,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let string_table = context.string_table;
    let target_text = named_value_or_default(target_name, string_table, "this target");

    let backtick_name = |name: Option<StringId>, fallback: &str| match name {
        Some(name) => format!("`{}`", string_table.resolve(name)),
        None => fallback.to_owned(),
    };

    match reason {
        InvalidAssignmentTargetReason::TemporaryNotAssignable => {
            "A temporary value cannot be assigned through. Receive it in a mutable binding first, then assign through that binding.".to_string()
        }
        InvalidAssignmentTargetReason::ImmutableBinding => {
            let binding_text = backtick_name(target_name, "this binding");
            format!("Cannot reassign {binding_text} because its binding is immutable. Make the original binding mutable, then reassign it with ordinary `=`.")
        }
        InvalidAssignmentTargetReason::ImmutableFieldRoot => match root_binding_name {
            Some(_) => {
                let field_text = backtick_name(field_name, "this field");
                let root_text = backtick_name(root_binding_name, "the root binding");
                format!("Cannot assign to field {field_text} because root binding {root_text} is immutable. Declare {root_text} as mutable before this assignment.")
            }
            None => {
                "Cannot assign to a field because the root binding is immutable. Declare the root binding as mutable before this assignment.".to_string()
            }
        },
        InvalidAssignmentTargetReason::UnavailableInCatchRecovery => {
            format!(
                "Assignment target {target_text} is unavailable inside catch recovery for the same assignment."
            )
        }
        InvalidAssignmentTargetReason::CollectionGetTargetNotWritable => {
            "Cannot assign through collection `get(...)`. Call `set(index, value)` with explicit `~` access on the collection instead, then recover with `catch` or propagate with `!` inside a compatible `Error!` function.".to_string()
        }
        InvalidAssignmentTargetReason::MapGetTargetNotWritable => {
            "Cannot assign through map `get(...)`. Call `set(key, value)` with explicit `~` access on the map instead, then recover with `catch` or propagate with `!` inside a compatible `Error!` function.".to_string()
        }
        InvalidAssignmentTargetReason::ReadOnlyMapProperty => {
            "Map `length` is a read-only property and cannot be assigned.".to_string()
        }
        InvalidAssignmentTargetReason::ExpectedAssignmentOperator => {
            format!("Expected assignment operator after binding {target_text}.")
        }
        InvalidAssignmentTargetReason::MutableMarkerOnAssignmentTarget => {
            "`~` is not written on assignment targets. Reassignment uses ordinary `=` and requires an already-mutable binding.".to_string()
        }
    }
}

pub(crate) fn invalid_multi_bind_message(
    reason: InvalidMultiBindReason,
    target_name: Option<StringId>,
    string_table: &StringTable,
) -> String {
    let target_text = named_value_or_default(target_name, string_table, "this target");

    match reason {
        InvalidMultiBindReason::ThisTargetReserved => {
            "'this' is reserved for method receiver parameters and cannot be used in multi-bind declarations.".to_string()
        }
        InvalidMultiBindReason::ExpectedTargetName => {
            "Malformed multi-bind target list. Expected a symbol target name.".to_string()
        }
        InvalidMultiBindReason::MissingTargetAfterComma => {
            "A multi-bind comma must be followed by another target. Remove the comma to end the target line, or add the promised target before '='.".to_string()
        }
        InvalidMultiBindReason::MissingAssignmentOperator => {
            "Multi-bind target list is missing a shared '=' assignment operator.".to_string()
        }
        InvalidMultiBindReason::InvalidTokenAfterTarget => {
            "Invalid token after multi-bind target.".to_string()
        }
        InvalidMultiBindReason::MissingRightHandExpression => {
            "Multi-bind statement is missing a right-hand expression after '='.".to_string()
        }
        InvalidMultiBindReason::MultipleRightHandExpressions => {
            "Multi-bind statements accept exactly one right-hand expression.".to_string()
        }
        InvalidMultiBindReason::MutableTargetNeedsExplicitType => {
            format!("Mutable multi-bind target {target_text} requires an explicit type annotation.")
        }
        InvalidMultiBindReason::DuplicateTarget => {
            format!("Duplicate multi-bind target {target_text} in the same target list.")
        }
        InvalidMultiBindReason::UnsupportedRhs => {
            "Multi-bind is only supported for explicit multi-value surfaces. For now, that means multi-return function calls.".to_string()
        }
        InvalidMultiBindReason::ExistingTargetMutableMarker => {
            format!("Existing multi-bind target {target_text} cannot use a mutable marker.")
        }
        InvalidMultiBindReason::ExistingTargetImmutable => {
            format!("Existing multi-bind target {target_text} is immutable and cannot be reassigned.")
        }
        InvalidMultiBindReason::ArityMismatch { expected, found } => {
            format!("Multi-bind arity mismatch: {expected} target(s) but {found} value slot(s). Target count must match returned slot count exactly.")
        }
        InvalidMultiBindReason::RhsNotMultiValue => {
            "Multi-bind right-hand expression must evaluate to a multi-value return pack.".to_string()
        }
    }
}

pub(crate) fn invalid_builtin_call_message(
    reason: InvalidBuiltinCallReason,
    builtin_name: Option<StringId>,
    string_table: &StringTable,
) -> String {
    let builtin_text = named_value_or_default(builtin_name, string_table, "This builtin");

    match reason {
        InvalidBuiltinCallReason::MissingParentheses => {
            format!("{builtin_text} method call is missing '(' before the argument list.")
        }
        InvalidBuiltinCallReason::TakesNoArguments => {
            format!("{builtin_text} method takes no arguments.")
        }
        InvalidBuiltinCallReason::NamedArgumentsNotSupported => {
            "Named arguments are not supported for builtin member calls.".to_string()
        }
        InvalidBuiltinCallReason::UnhandledFallibleCall => {
            format!(
                "Calls to {builtin_text} must be explicitly handled with postfix `!` or `catch`."
            )
        }
        InvalidBuiltinCallReason::DoesNotAcceptMutableAccess => {
            format!("{builtin_text} does not accept explicit mutable access marker '~'.")
        }
        InvalidBuiltinCallReason::CastMissingParentheses => {
            format!("{builtin_text} cast requires parentheses and exactly one argument.")
        }
        InvalidBuiltinCallReason::CastMissingArgument => {
            format!("{builtin_text} cast requires exactly one argument.")
        }
        InvalidBuiltinCallReason::CastTooManyArguments => {
            format!("{builtin_text} cast takes exactly one argument.")
        }
        InvalidBuiltinCallReason::CastMissingClosingParenthesis => {
            format!("Expected ')' after {builtin_text} cast argument.")
        }
        InvalidBuiltinCallReason::MissingArgument => {
            format!("{builtin_text} requires at least one argument.")
        }
        InvalidBuiltinCallReason::TooManyArguments => {
            format!("{builtin_text} takes too many arguments.")
        }
        InvalidBuiltinCallReason::ExpressionPositionNotAllowed => {
            format!("{builtin_text} is a statement and cannot be used in expression position.")
        }
        InvalidBuiltinCallReason::MapLengthIsProperty => {
            "Map `length` is a property, not a method. Use `map.length` without parentheses."
                .to_string()
        }
        InvalidBuiltinCallReason::ScalarConstructorRemoved => {
            format!(
                "{builtin_text}(...) constructor-style conversions are removed. Use `cast` instead."
            )
        }
    }
}

pub(crate) fn invalid_cast_message(
    reason: InvalidCastReason,
    source_type: Option<TypeId>,
    target_type: Option<TypeId>,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let source = source_type
        .map(|type_id| diagnostic_type_name(type_id, context))
        .unwrap_or_else(|| "the source value".to_owned());
    let target = target_type
        .map(|type_id| diagnostic_type_name(type_id, context))
        .unwrap_or_else(|| "the target type".to_owned());

    match reason {
        InvalidCastReason::MissingExplicitTarget => {
            "`cast` requires an explicit builtin target type at the receiving boundary.".to_owned()
        }
        InvalidCastReason::TargetNotBuiltin => {
            format!("`cast` targets must be compiler-supported builtin types; found '{target}'.")
        }
        InvalidCastReason::TargetIsGenericParameter => {
            format!("`cast` cannot infer a generic target such as '{target}'.")
        }
        InvalidCastReason::SameSourceAndTarget => {
            format!("This value already has type '{target}'. Remove `cast`.")
        }
        InvalidCastReason::SourceIsOptional => {
            format!("`cast` does not automatically unwrap optional source values; found '{source}'.")
        }
        InvalidCastReason::OperandIsFallible => {
            "`cast` only handles cast failures. Handle the operand's `Error!` return before casting."
                .to_owned()
        }
        InvalidCastReason::OperandArityMismatch => {
            "`cast` converts exactly one source value. Cast each return slot separately.".to_owned()
        }
        InvalidCastReason::TargetArityMismatch => {
            "`cast` requires exactly one target value. Cast each target slot separately.".to_owned()
        }
        InvalidCastReason::FallibleEvidenceRequiresHandling => {
            "`cast` selected fallible evidence. Use `cast!` or `cast ... catch:`.".to_owned()
        }
        InvalidCastReason::InfallibleEvidenceCannotUseFallibleForm => {
            "`cast!` and `cast ... catch:` are only valid for fallible casts.".to_owned()
        }
        InvalidCastReason::PropagationRequiresErrorReturn => {
            "`cast!` requires the current function to have an `Error!` return slot.".to_owned()
        }
        InvalidCastReason::PropagationAndRecoveryConflict => {
            "`cast!` cannot also use `catch:`. Choose propagation or local recovery.".to_owned()
        }
        InvalidCastReason::BangMustAttachToCast => {
            "The `!` must be attached to `cast` as `cast!`.".to_owned()
        }
        InvalidCastReason::ScalarConstructorRemoved => {
            "Constructor-style scalar conversions are removed. Use `cast` at an explicit typed boundary."
                .to_owned()
        }
        InvalidCastReason::NoEvidence => {
            format!("No cast evidence exists from '{source}' to '{target}'.")
        }
        InvalidCastReason::BuiltinEvidenceNotConstFoldable => {
            "This builtin cast cannot be fully evaluated at compile time yet.".to_owned()
        }
        InvalidCastReason::UserDefinedEvidenceNotConstFoldable => {
            "User-defined cast evidence must be fully evaluable at compile time.".to_owned()
        }
        InvalidCastReason::GenericBoundEvidenceNotConstFoldable => {
            "Generic-bound cast evidence must be fully evaluable at compile time.".to_owned()
        }
        InvalidCastReason::BuiltinCastFailedInConst => {
            "This builtin cast failed while evaluating a compile-time expression.".to_owned()
        }
        InvalidCastReason::CatchHandlerNotConstFoldable => {
            "The `catch` handler for this cast must be fully evaluable at compile time.".to_owned()
        }
    }
}

pub(crate) fn invalid_receiver_call_message(
    reason: InvalidReceiverCallReason,
    receiver_type: Option<StringId>,
    method_name: Option<StringId>,
    receiver_kind: Option<ReceiverCallKind>,
    receiver_binding_name: Option<StringId>,
    string_table: &StringTable,
) -> String {
    // `receiver_type` carries the rendered type label for type-named diagnostics such as
    // `CalledAsFreeFunction`. Receiver-access diagnostics use the simple binding name and
    // receiver kind instead, so the type field is never rendered as an authored value name.
    let receiver_type_text = named_value_or_default(receiver_type, string_table, "this receiver");
    let method_text = named_value_or_default(method_name, string_table, "this method");

    match reason {
        InvalidReceiverCallReason::CalledAsFreeFunction => {
            format!("{method_text} is a receiver method for {receiver_type_text} and cannot be called as a free function.")
        }
        InvalidReceiverCallReason::MustUseParentheses => {
            format!("{method_text} is a receiver method and must be called with parentheses.")
        }
        InvalidReceiverCallReason::ConstRecordNoRuntimeCalls => {
            format!(
                "Const records are data-only and do not support runtime method calls like {method_text}."
            )
        }
        InvalidReceiverCallReason::MutableReceiverMissingMarker => {
            let method = backtick_name(method_name, string_table, "this method");
            match receiver_kind {
                Some(ReceiverCallKind::CollectionBuiltin)
                | Some(ReceiverCallKind::MapBuiltin) => {
                    let kind_noun = receiver_kind_noun(receiver_kind);
                    format!("{method} requires a mutable {kind_noun} receiver. Call it with explicit `~` access.")
                }
                // Source methods name the receiver method form directly. When the simple
                // receiver binding name is known, show the concrete `~name.method(...)` form
                // so the guidance is source-visible rather than an internal placeholder.
                _ => {
                    let binding = backtick_name(receiver_binding_name, string_table, "");
                    if binding.is_empty() {
                        format!("Mutable receiver method {method} requires explicit mutable access. Prefix the receiver with `~`.")
                    } else {
                        // The concrete example uses raw authored names inside one code span so
                        // the rendered form matches Moth source (`~p.move(...)`) instead of
                        // nesting backticks around each token.
                        let raw_binding =
                            raw_name(receiver_binding_name, string_table, "");
                        let raw_method =
                            raw_name(method_name, string_table, "method");
                        format!("Mutable receiver method {method} requires explicit mutable access. Prefix the receiver with `~`, for example `~{raw_binding}.{raw_method}(...)`.")
                    }
                }
            }
        }
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod => render_immutable_receiver(
            method_name,
            receiver_kind,
            receiver_binding_name,
            string_table,
            /*authored_marker*/ false,
        ),
        InvalidReceiverCallReason::NonPlaceReceiverMutableMethod => {
            let method = backtick_name(method_name, string_table, "this method");
            let kind_noun = receiver_kind_noun(receiver_kind);
            match receiver_kind {
                Some(ReceiverCallKind::SourceMethod) | None => format!("Mutable receiver method {method} requires a mutable place receiver. Bind this value in a mutable binding first, then call it with `~`."),
                _ => format!("{method} requires a mutable {kind_noun} receiver. Bind this value in a mutable binding first, then call it with `~`."),
            }
        }
        InvalidReceiverCallReason::MutableMarkerOnImmutableReceiver => render_immutable_receiver(
            method_name,
            receiver_kind,
            receiver_binding_name,
            string_table,
            /*authored_marker*/ true,
        ),
        InvalidReceiverCallReason::MutableMarkerOnNonPlaceReceiver => {
            let method = backtick_name(method_name, string_table, "this method");
            let kind_noun = receiver_kind_noun(receiver_kind);
            match receiver_kind {
                Some(ReceiverCallKind::SourceMethod) | None => format!("`~` accepts only an existing mutable place. Mutable receiver method {method} cannot be called on a temporary value. Bind this value in a mutable binding first, then call it with `~`."),
                _ => format!("`~` accepts only an existing mutable place. {method} requires a mutable {kind_noun} receiver and cannot be called on a temporary value. Bind this value in a mutable binding first, then call it with `~`."),
            }
        }
        InvalidReceiverCallReason::UnneededMutableAccessMarker => {
            let method = backtick_name(method_name, string_table, "this method");
            format!("{method} does not accept an explicit mutable access marker `~`. Remove the `~` from this call.")
        }
        InvalidReceiverCallReason::MutableMarkerOnNonReceiverCall => {
            "Mutable receiver marker `~` is only valid for receiver calls like `~value.method(...)` or `~values.push(...)`."
                .to_string()
        }
        InvalidReceiverCallReason::AmbiguousGenericBoundMethod => {
            format!(
                "{method_text} is provided by more than one generic bound for {receiver_type_text}. Add a more specific bound or rename one of the trait requirements."
            )
        }
    }
}

/// Render an immutable-receiver diagnostic for either the missing-marker or authored-marker case.
///
/// WHAT: shares the kind-aware noun and binding-name wording between the two immutable-receiver
///       reasons, varying only the leading clause by whether `~` was authored.
/// WHY: both reasons name the binding to declare mutable; the authored-marker case additionally
///      explains that `~` accepts only an existing mutable place and points at the marker.
fn render_immutable_receiver(
    method_name: Option<StringId>,
    receiver_kind: Option<ReceiverCallKind>,
    receiver_binding_name: Option<StringId>,
    string_table: &StringTable,
    authored_marker: bool,
) -> String {
    let method = backtick_name(method_name, string_table, "this method");
    let kind_noun = receiver_kind_noun(receiver_kind);
    let binding = backtick_name(receiver_binding_name, string_table, "");
    let (requirement_phrase, access_command, access_gerund) = match receiver_kind {
        Some(ReceiverCallKind::SourceMethod) | None => (
            format!("Mutable receiver method {method} requires a mutable receiver"),
            "call it with `~`",
            "calling it with `~`",
        ),
        _ => (
            format!("{method} requires a mutable {kind_noun} receiver"),
            "call it with explicit `~` access",
            "calling it with explicit `~` access",
        ),
    };

    let declare_phrase = if binding.is_empty() {
        match receiver_kind {
            Some(ReceiverCallKind::SourceMethod) | None => {
                "Declare the binding as mutable".to_string()
            }
            _ => format!("Declare the {kind_noun} binding as mutable"),
        }
    } else {
        format!("Declare {binding} as mutable")
    };

    let state_clause = if binding.is_empty() {
        "the receiver is immutable".to_string()
    } else {
        format!("{binding} is immutable")
    };

    if authored_marker {
        format!(
            "`~` accepts only an existing mutable place. {requirement_phrase}, but {state_clause}. {declare_phrase} before {access_gerund}."
        )
    } else {
        format!(
            "{requirement_phrase}, but {state_clause}. {declare_phrase}, then {access_command}."
        )
    }
}

/// Render a payload name in backticks, falling back to `fallback` when the fact is absent.
///
/// WHAT: receiver-access diagnostics render authored source names (method, binding) as code
///       spans rather than the single-quoted labels used by type-named diagnostics.
/// WHY: the active diagnostics plan specifies source-visible code spans for receiver-access
///      guidance so the rendered example matches authored Moth syntax.
fn backtick_name(name: Option<StringId>, string_table: &StringTable, fallback: &str) -> String {
    name.map(|n| format!("`{}`", string_table.resolve(n)))
        .unwrap_or_else(|| fallback.to_string())
}

/// Resolve a payload name to its raw source spelling, falling back to `fallback` when absent.
///
/// WHAT: used inside a single rendered code span where the name should not carry its own
///       backtick delimiters.
fn raw_name(name: Option<StringId>, string_table: &StringTable, fallback: &str) -> String {
    name.map(|n| string_table.resolve(n).to_string())
        .unwrap_or_else(|| fallback.to_string())
}

/// The rendered noun for a receiver-call kind, used by receiver-access diagnostics.
///
/// WHAT: maps the receiver kind payload fact to the source-facing noun.
/// WHY: collection and map builtins name their receiver kind, while source methods use the
///      "receiver method" phrasing inline, so this helper supplies the kind noun only.
fn receiver_kind_noun(kind: Option<ReceiverCallKind>) -> &'static str {
    match kind {
        Some(ReceiverCallKind::CollectionBuiltin) => "collection",
        Some(ReceiverCallKind::MapBuiltin) => "map",
        Some(ReceiverCallKind::SourceMethod) | None => "receiver",
    }
}

/// Build a "did you mean?" suggestion for a misspelled field name.
///
/// WHAT: uses the same edit-distance heuristic as named-argument suggestions.
/// WHY: typos in field access are one of the most common mistakes; suggesting the
/// correct field name is immediately actionable.
fn field_suggestion(
    field_name: Option<StringId>,
    known_fields: &[StringId],
    string_table: &StringTable,
) -> Option<String> {
    let name = field_name?;
    let name_str = string_table.resolve(name);
    closest_name_suggestion(name_str, known_fields, string_table)
}

/// Build a hint listing available fields when no close suggestion is found.
fn available_fields_hint(known_fields: &[StringId], string_table: &StringTable) -> String {
    if known_fields.is_empty() {
        return String::new();
    }
    let names = known_fields
        .iter()
        .map(|f| string_table.resolve(*f))
        .collect::<Vec<_>>()
        .join(", ");
    format!(" Available: [{names}]")
}

pub(crate) fn invalid_copy_target_message(reason: InvalidCopyTargetReason) -> String {
    match reason {
        InvalidCopyTargetReason::FunctionName => {
            "`copy` requires an existing binding or field projection. A function name is not a copyable value. Call the function and receive the result in a binding first, then copy that binding.".to_string()
        }
        InvalidCopyTargetReason::FunctionCall => {
            "`copy` requires an existing binding or field projection. A call returns a value that must be received in a binding first, then copy that binding.".to_string()
        }
        InvalidCopyTargetReason::NonPlace => {
            "`copy` requires an existing binding or field projection. Bind this value first, then copy that binding.".to_string()
        }
        InvalidCopyTargetReason::MutableMarkerNotAllowed => {
            "`copy` does not take `~`. Copy the existing binding or field projection without the access marker.".to_string()
        }
    }
}

pub(crate) fn invalid_field_access_message(
    reason: InvalidFieldAccessReason,
    field_name: Option<StringId>,
    receiver_type: Option<TypeId>,
    known_fields: &[StringId],
    context: DiagnosticRenderContext<'_>,
) -> String {
    let string_table = context.string_table;
    let field_text = named_value_or_default(field_name, string_table, "this field");
    let receiver_text = receiver_type.map(|type_id| diagnostic_type_name(type_id, context));

    match reason {
        InvalidFieldAccessReason::ExpectedNameAfterDot => {
            // The optional field-name fallback is intentionally unused here: the access ended
            // at the dot, so there is no authored member name to echo back to the user.
            "Expected a field or method name after '.', but this access ends here.".to_owned()
        }
        InvalidFieldAccessReason::FieldNotMethod => {
            format!(
                "{field_text} is a field, not a receiver method. Dot-call syntax is reserved for declared receiver methods."
            )
        }
        InvalidFieldAccessReason::ChoicePayloadMutation => {
            "Choice payload fields are immutable. Mutation is not supported.".to_string()
        }
        InvalidFieldAccessReason::ChoicePayloadDeferred => {
            "Choice payload field access is deferred. Use pattern matching to extract payload fields."
                .to_string()
        }
        InvalidFieldAccessReason::UnknownExternalMember => match receiver_text {
            Some(receiver_text) => {
                format!("Property or method {field_text} not found for external type '{receiver_text}'.")
            }
            None => format!("Property or method {field_text} not found for external type."),
        },
        InvalidFieldAccessReason::UnknownMember => {
            let suggestion = field_suggestion(field_name, known_fields, string_table);
            match receiver_text {
                Some(receiver_text) => match suggestion {
                    Some(suggested) => {
                        format!("Property or method {field_text} not found for '{receiver_text}'. Did you mean '{suggested}'?")
                    }
                    None => {
                        let available = available_fields_hint(known_fields, string_table);
                        format!("Property or method {field_text} not found for '{receiver_text}'.{available}")
                    }
                },
                None => format!("Property or method {field_text} not found."),
            }
        }
    }
}
