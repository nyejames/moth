//! Declaration and signature diagnostic text renderers.
//!
//! WHAT: renders diagnostics for declaration syntax, function signatures, generic instantiation,
//! and receiver-method declarations.
//! WHY: declarations are resolved before executable body parsing and form their own payload family.

use super::{DiagnosticRenderContext, diagnostic_type_name, token_kind_name};
use crate::compiler_frontend::compiler_messages::{
    InvalidDeclarationReason, InvalidFunctionSignatureReason, InvalidGenericInstantiationReason,
    InvalidReceiverDeclarationReason, InvalidSignatureMemberReason,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

pub(crate) fn invalid_signature_member_message(reason: InvalidSignatureMemberReason) -> String {
    match reason {
        InvalidSignatureMemberReason::ChoicePayloadMutable => {
            "Choice payload fields cannot be marked mutable.".to_string()
        }
        InvalidSignatureMemberReason::ChoicePayloadDefaultValue => {
            "Choice payload fields cannot have default values.".to_string()
        }
        InvalidSignatureMemberReason::CompileTimeParameterDeferred => {
            "The '#' binding marker is not supported in parameter or field lists. Compile-time parameters are deferred.".to_string()
        }
        InvalidSignatureMemberReason::ThisNotAllowed => {
            "'this' is reserved for method receiver parameters and is not valid in this context."
                .to_string()
        }
        InvalidSignatureMemberReason::TrailingComma => {
            "Trailing comma is not allowed in this parameter list.".to_string()
        }
        InvalidSignatureMemberReason::TraitReceiverMustBeThis => {
            "Trait receiver parameters must use 'This' or '~This'.".to_string()
        }
        InvalidSignatureMemberReason::TraitMutableThisOnlyFirstParameter => {
            "'~This' is only valid as the first parameter of a trait requirement.".to_string()
        }
        InvalidSignatureMemberReason::TraitBareThisOnlyReceiver => {
            "Bare `This` is only valid as the first trait receiver parameter. Name non-receiver `This` parameters, for example `other This`."
                .to_string()
        }
        InvalidSignatureMemberReason::TraitRequirementDefaultValue => {
            "Trait requirements cannot declare default parameter values.".to_string()
        }
        InvalidSignatureMemberReason::ReactiveAccessNotAllowed => {
            "`$Type` parameters are only valid in function signatures. Struct fields, choice payloads, and trait requirements use ordinary type annotations.".to_string()
        }
        InvalidSignatureMemberReason::ReactiveParameterDefaultValue => {
            "Reactive parameters require an existing reactive source argument and cannot have default values.".to_string()
        }
        InvalidSignatureMemberReason::MissingDefaultValue => {
            "Expected a default value after '='.".to_string()
        }
    }
}

pub(crate) fn invalid_function_signature_message(
    reason: &InvalidFunctionSignatureReason,
    string_table: &StringTable,
) -> String {
    match reason {
        InvalidFunctionSignatureReason::MissingArrowOrColon { found } => {
            format!(
                "Expected `->` or `:` after function parameters, but found {}.",
                token_kind_name(found, string_table)
            )
        }
        InvalidFunctionSignatureReason::UnexpectedEndAfterParameters => {
            "Function signature ended unexpectedly after parameters.".to_string()
        }
        InvalidFunctionSignatureReason::MissingReturnType => {
            "Function signature is missing a return type after '->'. Add a type followed by ':', or remove '->' for a no-value function.".to_string()
        }
        InvalidFunctionSignatureReason::MissingTraitRequirementReturnType => {
            "Trait requirement is missing a return type after '->'. Add a type, or remove '->' for a no-value requirement.".to_string()
        }
        InvalidFunctionSignatureReason::TrailingCommaInReturns => {
            "Trailing commas are not allowed in function return declarations.".to_string()
        }
        InvalidFunctionSignatureReason::UnexpectedEndAfterComma => {
            "Function return declarations cannot end after a comma.".to_string()
        }
        InvalidFunctionSignatureReason::UnexpectedEndInReturns => {
            "Unexpected end of function signature while parsing return declarations.".to_string()
        }
        InvalidFunctionSignatureReason::MissingColonAfterReturns => {
            "Function return declarations must end with ':'.".to_string()
        }
        InvalidFunctionSignatureReason::UnexpectedArrowInReturns => {
            "Unexpected '->' inside function return declarations.".to_string()
        }
        InvalidFunctionSignatureReason::MissingCommaOrColon { found } => {
            format!(
                "Expected `,` or `:` after function return declaration, found {}.",
                token_kind_name(found, string_table)
            )
        }
        InvalidFunctionSignatureReason::VoidNotAllowed => {
            "Void is not a valid function return declaration.".to_string()
        }
        InvalidFunctionSignatureReason::UnknownReturnAlias { .. } => {
            "Unknown return alias. Alias returns must name a function parameter.".to_string()
        }
        InvalidFunctionSignatureReason::MissingParameterNameInAlias => {
            "Expected a parameter name in an alias return declaration.".to_string()
        }
        InvalidFunctionSignatureReason::DuplicateParameterInAlias => {
            "Duplicate parameter used in the same alias return declaration.".to_string()
        }
        InvalidFunctionSignatureReason::AliasCannotBeError => {
            "Alias return declarations cannot be marked as an error slot in v1.".to_string()
        }
        InvalidFunctionSignatureReason::AliasReturnNotAllowedInTraitRequirement => {
            "Trait requirements cannot use return aliases. Write an explicit return type instead."
                .to_string()
        }
        InvalidFunctionSignatureReason::MultipleErrorReturnSlots => {
            "Function signatures can only declare one distinguished error return slot.".to_string()
        }
        InvalidFunctionSignatureReason::ErrorSlotNotLast => {
            "The error return slot must be the final return slot in v1.".to_string()
        }
        InvalidFunctionSignatureReason::GenericWhereConstraintsUnsupported => {
            "`where` syntax is not part of Moth generic constraints. Use declaration-site bounds such as `type A is TRAIT`."
                .to_string()
        }
    }
}

pub(crate) fn invalid_declaration_message(
    reason: InvalidDeclarationReason,
    name: Option<StringId>,
    string_table: &StringTable,
) -> String {
    let name_text = name
        .map(|name| format!("'{}'", string_table.resolve(name)))
        .unwrap_or_else(|| "This declaration".to_string());

    match reason {
        InvalidDeclarationReason::ReservedBuiltinName => {
            format!("{name_text} is reserved as a builtin language type.")
        }
        InvalidDeclarationReason::ConstantCannotBeMutable => "Constants cannot be mutable.".to_string(),
        InvalidDeclarationReason::ExternalTypeLiteralConstruction => {
            "Cannot construct external type with a struct literal. External types are opaque and can only be obtained from external function calls.".to_string()
        }
        InvalidDeclarationReason::ParameterizedGenericTypeAlias => {
            "Generic type aliases with parameters are not supported. Alias a fully concrete generic instance instead.".to_string()
        }
        InvalidDeclarationReason::UnusedGenericParameter { parameter_name } => {
            format!(
                "Generic parameter '{}' is declared but never used in the public type shape for '{}'.",
                string_table.resolve(parameter_name),
                name_text.trim_matches('\'')
            )
        }
        InvalidDeclarationReason::RecursiveGenericType => {
            format!(
                "Recursive generic types are not supported yet. Generic type '{}' cannot contain itself.",
                name_text.trim_matches('\'')
            )
        }
        InvalidDeclarationReason::RecursiveRuntimeStruct { cycle } => {
            format!("Recursive runtime struct definitions are not supported in v1. Cycle: {cycle}")
        }
        InvalidDeclarationReason::ExternalTypeAlias { type_name } => {
            let type_name_str = string_table.resolve(type_name);
            format!("Cannot create a type alias for external type '{type_name_str}'. External types are opaque and cannot be aliased.")
        }
        InvalidDeclarationReason::InvalidGenericParameterName { parameter_name } => {
            let parameter_name_str = string_table.resolve(parameter_name);
            format!("Invalid generic parameter name '{parameter_name_str}'. Generic parameter names must be PascalCase or a single uppercase letter.")
        }
        InvalidDeclarationReason::DuplicateGenericParameter { parameter_name } => {
            let parameter_name_str = string_table.resolve(parameter_name);
            format!("Duplicate generic parameter '{parameter_name_str}'. Parameter names must be unique.")
        }
        InvalidDeclarationReason::GenericParameterNameCollision { parameter_name } => {
            let parameter_name_str = string_table.resolve(parameter_name);
            format!("Generic parameter '{parameter_name_str}' collides with an existing visible type name.")
        }
        InvalidDeclarationReason::ReservedGenericParameterName { parameter_name } => {
            let parameter_name_str = string_table.resolve(parameter_name);
            format!("Generic parameter '{parameter_name_str}' collides with a builtin type name.")
        }
        InvalidDeclarationReason::GenericTraitsUnsupported => {
            "Generic trait declarations are outside Moth's trait design scope. Trait declarations cannot have generic parameters."
                .to_string()
        }
        InvalidDeclarationReason::InvalidTraitName => {
            format!("{name_text} is not a valid trait name. Trait names must use all-caps identifiers such as 'DISPLAYABLE'.")
        }
        InvalidDeclarationReason::TraitConformanceMissingTrait => {
            "Trait conformance declarations must name at least one trait after 'must'."
                .to_string()
        }
        InvalidDeclarationReason::TraitConformanceSemicolon => {
            "Trait conformance declarations are newline-terminated and must not end with ';'."
                .to_string()
        }
        InvalidDeclarationReason::TraitIncompatibilityMissingTrait => {
            "Trait incompatibility declarations must name at least one trait after 'must not'."
                .to_string()
        }
        InvalidDeclarationReason::TraitIncompatibilitySemicolon => {
            "Trait incompatibility declarations are newline-terminated and must not end with ';'."
                .to_string()
        }
        InvalidDeclarationReason::MissingInitializerExpression => {
            format!("Declaration {name_text} is missing an initializer expression after '='.")
        }
    }
}

pub(crate) fn invalid_generic_instantiation_message(
    type_name: Option<StringId>,
    reason: &InvalidGenericInstantiationReason,
    context: DiagnosticRenderContext<'_>,
) -> String {
    use InvalidGenericInstantiationReason;

    let string_table = context.string_table;
    let type_name_str = type_name
        .map(|n| format!("'{}'", string_table.resolve(n)))
        .unwrap_or_else(|| "This type".to_string());

    match reason {
        InvalidGenericInstantiationReason::WrongArgumentCount { expected, found } => {
            format!(
                "Generic type {type_name_str} expects {expected} type argument{}, but {found} were provided.",
                if *expected == 1 { "" } else { "s" }
            )
        }
        InvalidGenericInstantiationReason::OptionTypeSyntaxNotSupported => {
            "`Option of T` is not Moth type syntax. Use the `T?` optional suffix.".to_string()
        }
        InvalidGenericInstantiationReason::ResultTypeSyntaxNotSupported => {
            "`Result of T, E` is not Moth type syntax. Fallible functions declare a final `E!` error return slot.".to_string()
        }
        InvalidGenericInstantiationReason::TypeDoesNotAcceptArguments => {
            format!("Type {type_name_str} does not accept generic arguments.")
        }
        InvalidGenericInstantiationReason::ExternalTypeArgumentsUnsupported => {
            format!(
                "External package type {type_name_str} cannot be generic. Expose a concrete external type instead."
            )
        }
        InvalidGenericInstantiationReason::MissingTypeArguments => {
            format!("Generic type {type_name_str} requires type arguments.")
        }
        InvalidGenericInstantiationReason::CannotInferArguments { missing_parameters } => {
            let missing = missing_parameters
                .iter()
                .map(|p| string_table.resolve(*p))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Cannot infer type argument(s) for generic type {type_name_str}: {missing}. Provide an explicit type annotation or constructor arguments with concrete types."
            )
        }
        InvalidGenericInstantiationReason::CannotInferFunctionArguments { missing_parameters } => {
            let missing = missing_parameters
                .iter()
                .map(|p| string_table.resolve(*p))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Cannot infer type argument(s) for generic function {type_name_str}: {missing}. Add an immediate receiving-site type annotation or pass arguments that fix the type."
            )
        }
        InvalidGenericInstantiationReason::ConflictingInference {
            subject,
            parameter_name,
            existing_type_id,
            replacement_type_id,
            ..
        } => {
            let parameter = string_table.resolve(*parameter_name);
            let existing_type = diagnostic_type_name(*existing_type_id, context);
            let replacement_type = diagnostic_type_name(*replacement_type_id, context);
            let subject = match subject {
                crate::compiler_frontend::compiler_messages::GenericInferenceSubject::Function => {
                    "generic function"
                }
                crate::compiler_frontend::compiler_messages::GenericInferenceSubject::NominalType => {
                    "generic type"
                }
            };
            format!(
                "Generic parameter '{parameter}' in {subject} {type_name_str} was inferred as both {existing_type} and {replacement_type}."
            )
        }
        InvalidGenericInstantiationReason::MissingTraitEvidence {
            parameter_name,
            trait_name,
            concrete_type_id,
        } => {
            let parameter = string_table.resolve(*parameter_name);
            let trait_name = string_table.resolve(*trait_name);
            let concrete_type = diagnostic_type_name(*concrete_type_id, context);
            format!(
                "Generic function {type_name_str} requires '{parameter}' to satisfy trait '{trait_name}', but {concrete_type} has no visible trait evidence for that bound."
            )
        }
        InvalidGenericInstantiationReason::MissingNominalTraitEvidence {
            parameter_name,
            trait_name,
            concrete_type_id,
        } => {
            let parameter = string_table.resolve(*parameter_name);
            let trait_name = string_table.resolve(*trait_name);
            let concrete_type = diagnostic_type_name(*concrete_type_id, context);
            format!(
                "Generic type {type_name_str} requires '{parameter}' to satisfy trait '{trait_name}', but {concrete_type} has no visible trait evidence for that bound."
            )
        }
        InvalidGenericInstantiationReason::RecursiveFunctionInstantiation => {
            format!(
                "Generic function {type_name_str} recursively instantiates itself, which is deferred for Alpha."
            )
        }
        InvalidGenericInstantiationReason::ExplicitCallTypeArgumentsUnsupported => {
            "Explicit generic call-site type arguments are not supported. Add an ordinary type annotation to the receiving declaration or argument instead.".to_string()
        }
        InvalidGenericInstantiationReason::GenericFunctionValueDeferred => {
            format!(
                "Generic functions cannot be used as values. Call {type_name_str} directly or write a concrete wrapper function."
            )
        }
    }
}

pub(crate) fn invalid_receiver_declaration_message(
    reason: InvalidReceiverDeclarationReason,
    string_table: &StringTable,
) -> String {
    use InvalidReceiverDeclarationReason;

    match reason {
        InvalidReceiverDeclarationReason::UnknownStructTarget => {
            "Receiver method targets an unknown receiver type.".to_string()
        }
        InvalidReceiverDeclarationReason::NonlocalSourceType => {
            "Source-authored receiver methods must be declared in the same file as their user-defined receiver type. Use a free function for values owned by another file or package."
                .to_string()
        }
        InvalidReceiverDeclarationReason::BuiltinScalarType => {
            "Source-authored receiver methods cannot target builtin scalar types. Use a free function for builtin values."
                .to_string()
        }
        InvalidReceiverDeclarationReason::ExternalOpaqueType => {
            "Source-authored receiver methods cannot target external opaque types. Use a free function for values owned by another package."
                .to_string()
        }
        InvalidReceiverDeclarationReason::FieldNameConflict => {
            "Struct declares both a field and a method with the same name.".to_string()
        }
        InvalidReceiverDeclarationReason::DuplicateMethod => {
            "Duplicate receiver method for this receiver and name.".to_string()
        }
        InvalidReceiverDeclarationReason::DuplicateVisibleMethod => {
            "Visible receiver methods collide for the same receiver type and method name."
                .to_string()
        }
        InvalidReceiverDeclarationReason::GenericReceiverType {
            function_name: _,
            type_name: _,
        } => {
            "Receiver methods on instantiated generic receiver types are not supported. Define the method on the generic type declaration using the receiver type's own parameters."
                .to_string()
        }
        InvalidReceiverDeclarationReason::UnsupportedType {
            function_name,
            type_name,
        } => {
            format!(
                "Function '{}' uses unsupported receiver type '{}'. Source-authored receiver methods must target a user-defined struct or choice.",
                string_table.resolve(function_name),
                string_table.resolve(type_name)
            )
        }
        InvalidReceiverDeclarationReason::ReceiverMethodImportOrExportNotAllowed => {
            "Receiver methods are not imported, exported, or aliased independently. Expose the receiver type; its same-file methods are available through receiver-call syntax when the type is visible."
                .to_string()
        }
    }
}
