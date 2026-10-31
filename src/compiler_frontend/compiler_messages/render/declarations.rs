//! Declaration and signature diagnostic text renderers.
//!
//! WHAT: renders diagnostics for declaration syntax, function signatures, generic instantiation,
//! and receiver-method declarations.
//! WHY: declarations are resolved before executable body parsing and form their own payload family.

use super::{
    DiagnosticRenderContext, RenderedPayload, closest_name_suggestion,
    closest_name_suggestion_with_extra, duplicate_declaration_message, namespace_misuse_message,
    token_kind_name, unknown_name_message,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    GenericInstantiationExtra, GenericInstantiationShape, InvalidFunctionSignatureView,
    InvalidGenericInstantiationView, InvalidReceiverDeclarationCode,
    InvalidReceiverDeclarationView, decode_generic_instantiation_extra,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    DuplicateDeclarationView, IdentifierNamingConventionView, InvalidDeclarationCode,
    InvalidDeclarationView, InvalidSignatureMemberView, InvalidStructDefaultValueView,
    MissingDeclarationInitializerView, ReservedBuiltinNameView, ReservedNameCollisionView,
    ShadowedNameView, TypeUsedAsValueView, UnknownNameView, UnknownTypeNameView,
    UnknownValueNameView, ValueUsedAsTypeView, decode_invalid_declaration_reason,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, InvalidDeclarationReason, InvalidFunctionSignatureReason,
    InvalidSignatureMemberReason, NameNamespace, NamingConvention, ReservedNameOwner,
    TypeUsedAsValueReason, UnknownTypeNameSite,
};
use crate::compiler_frontend::datatypes::display::builtin_type_annotation_spellings;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTableResolver};

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
        InvalidSignatureMemberReason::MissingDefaultValue => {
            "Expected a default value after '='.".to_string()
        }
    }
}

pub(crate) fn render_invalid_signature_member(
    _diagnostic: &DiagnosticView<'_>,
    facts: &InvalidSignatureMemberView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: invalid_signature_member_message(facts.reason),
        guidance: Vec::new(),
    }
}

fn identifier_naming_convention_message(
    name: StringId,
    expected_style: NamingConvention,
    string_table: &dyn StringTableResolver,
) -> String {
    let style_name = match expected_style {
        NamingConvention::CamelCase => "CamelCase",
        NamingConvention::LowercaseWithUnderscores => "lowercase_with_underscores",
        NamingConvention::UppercaseWithUnderscores => "UPPER_CASE_WITH_UNDERSCORES",
        NamingConvention::LowercaseOrUppercaseWithUnderscores => {
            "lowercase_with_underscores or UPPER_CASE_WITH_UNDERSCORES"
        }
    };
    format!(
        "Identifier '{}' should use {}",
        string_table.resolve(name),
        style_name
    )
}

pub(crate) fn render_identifier_naming_convention(
    _diagnostic: &DiagnosticView<'_>,
    facts: &IdentifierNamingConventionView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: identifier_naming_convention_message(
            facts.name,
            facts.expected_style,
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

fn reserved_builtin_name_message(name: StringId, string_table: &dyn StringTableResolver) -> String {
    format!("Reserved builtin name '{}'", string_table.resolve(name))
}

pub(crate) fn render_reserved_builtin_name(
    _diagnostic: &DiagnosticView<'_>,
    facts: &ReservedBuiltinNameView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: reserved_builtin_name_message(facts.name, context.string_table),
        guidance: Vec::new(),
    }
}

fn shadowed_name_message(name: StringId, string_table: &dyn StringTableResolver) -> String {
    format!("Shadowed name '{}'", string_table.resolve(name))
}

pub(crate) fn render_shadowed_name(
    _diagnostic: &DiagnosticView<'_>,
    facts: &ShadowedNameView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: shadowed_name_message(facts.name, context.string_table),
        guidance: Vec::new(),
    }
}

fn reserved_name_collision_message(
    name: StringId,
    reserved_by: ReservedNameOwner,
    string_table: &dyn StringTableResolver,
) -> String {
    let owner = match reserved_by {
        ReservedNameOwner::BuiltinType => "builtin type",
        ReservedNameOwner::Keyword => "keyword",
        ReservedNameOwner::CoreTrait => "core trait",
        ReservedNameOwner::ImplicitStart => "compiler-owned entry point",
        ReservedNameOwner::ParameterContract => "compiler-owned parameter contract",
    };
    format!(
        "Reserved name collision: '{}' is a reserved {}",
        string_table.resolve(name),
        owner
    )
}

pub(crate) fn render_reserved_name_collision(
    _diagnostic: &DiagnosticView<'_>,
    facts: &ReservedNameCollisionView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: reserved_name_collision_message(
            facts.name,
            facts.reserved_by,
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

pub(crate) fn invalid_function_signature_message(
    reason: &InvalidFunctionSignatureReason,
    string_table: &dyn StringTableResolver,
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

/// WHAT: renders an invalid function signature from its reason code and optional token.
/// WHY: the code determines whether the token fact is required.
pub(crate) fn render_invalid_function_signature(
    _diagnostic: &DiagnosticView<'_>,
    facts: &InvalidFunctionSignatureView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let reason = facts
        .code
        .to_reason(facts.found)
        .expect("a function-signature code stores its required token");
    RenderedPayload {
        message: invalid_function_signature_message(&reason, context.string_table),
        guidance: Vec::new(),
    }
}

pub(crate) fn invalid_declaration_message(
    reason: InvalidDeclarationReason,
    name: Option<StringId>,
    string_table: &dyn StringTableResolver,
    cycle_ids: Option<&[StringId]>,
) -> String {
    let name_text = name
        .map(|name| format!("'{}'", string_table.resolve(name)))
        .unwrap_or_else(|| "This declaration".to_string());

    match reason {
        InvalidDeclarationReason::ReservedBuiltinName => {
            format!("{name_text} is reserved as a builtin language type.")
        }
        InvalidDeclarationReason::DeclarationRequiresModuleTopLevel => {
            format!("Declaration {name_text} cannot appear inside an executable body. Declare it at module top level.")
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
            let cycle = cycle_ids.unwrap_or(cycle.as_slice());
            let mut cycle_text = String::new();
            for (index, component) in cycle.iter().enumerate() {
                if index > 0 {
                    cycle_text.push_str(" -> ");
                }
                cycle_text.push_str(string_table.resolve(*component));
            }
            format!("Recursive runtime struct definitions are not supported in v1. Cycle: {cycle_text}")
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

pub(crate) fn render_invalid_declaration(
    diagnostic: &DiagnosticView<'_>,
    facts: &InvalidDeclarationView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let reason = decode_invalid_declaration_reason(facts.code, facts.detail)
        .expect("invalid declaration code stores its required detail");
    let cycle_ids = match facts.code {
        InvalidDeclarationCode::RecursiveRuntimeStruct => Some(diagnostic.string_ids()),
        _ => None,
    };

    RenderedPayload {
        message: invalid_declaration_message(reason, facts.name, context.string_table, cycle_ids),
        guidance: Vec::new(),
    }
}

pub(crate) fn render_invalid_struct_default_value(
    _diagnostic: &DiagnosticView<'_>,
    _facts: &InvalidStructDefaultValueView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: "Invalid struct default value".to_owned(),
        guidance: Vec::new(),
    }
}

fn missing_declaration_initializer_message(
    name: StringId,
    string_table: &dyn StringTableResolver,
) -> String {
    format!(
        "Declaration '{}' requires '=' followed by an initializer expression.",
        string_table.resolve(name)
    )
}

pub(crate) fn render_missing_declaration_initializer(
    _diagnostic: &DiagnosticView<'_>,
    facts: &MissingDeclarationInitializerView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: missing_declaration_initializer_message(facts.name, context.string_table),
        guidance: Vec::new(),
    }
}

pub(crate) fn render_invalid_generic_instantiation(
    diagnostic: &DiagnosticView<'_>,
    facts: &InvalidGenericInstantiationView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    use GenericInstantiationExtra as Extra;
    use GenericInstantiationShape as Shape;

    let extra = decode_generic_instantiation_extra(facts.shape.code(), diagnostic.string_ids())
        .expect("generic-instantiation reason code and extra string ids agree");
    let type_name = match extra {
        Extra::ConflictingInference { type_name, .. } => type_name,
        _ => facts.shape.type_name(),
    };
    let type_name = type_name
        .map(|name| format!("'{}'", context.string_table.resolve(name)))
        .unwrap_or_else(|| "This type".to_owned());
    let message = match (&facts.shape, extra) {
        (
            Shape::WrongArgumentCount {
                expected, found, ..
            },
            Extra::None,
        ) => format!(
            "Generic type {type_name} expects {expected} type argument{}, but {found} were provided.",
            if *expected == 1 { "" } else { "s" }
        ),
        (Shape::OptionTypeSyntaxNotSupported { .. }, Extra::None) => {
            "`Option of T` is not Moth type syntax. Use the `T?` optional suffix.".to_string()
        }
        (Shape::ResultTypeSyntaxNotSupported { .. }, Extra::None) => {
            "`Result of T, E` is not Moth type syntax. Fallible functions declare a final `E!` error return slot.".to_string()
        }
        (Shape::TypeDoesNotAcceptArguments { .. }, Extra::None) => {
            format!("Type {type_name} does not accept generic arguments.")
        }
        (Shape::ExternalTypeArgumentsUnsupported { .. }, Extra::None) => format!(
            "External package type {type_name} cannot be generic. Expose a concrete external type instead."
        ),
        (Shape::MissingTypeArguments { .. }, Extra::None) => {
            format!("Generic type {type_name} requires type arguments.")
        }
        (Shape::CannotInferArguments { .. }, Extra::MissingParameters(missing_parameters)) => {
            let missing = missing_parameters
                .iter()
                .map(|parameter| context.string_table.resolve(*parameter))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Cannot infer type argument(s) for generic type {type_name}: {missing}. Provide an explicit type annotation or constructor arguments with concrete types."
            )
        }
        (
            Shape::CannotInferFunctionArguments { .. },
            Extra::MissingParameters(missing_parameters),
        ) => {
            let missing = missing_parameters
                .iter()
                .map(|parameter| context.string_table.resolve(*parameter))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Cannot infer type argument(s) for generic function {type_name}: {missing}. Add an immediate receiving-site type annotation or pass arguments that fix the type."
            )
        }
        (
            Shape::ConflictingInference {
                subject,
                existing_type,
                replacement_type,
            },
            Extra::ConflictingInference {
                parameter_name, ..
            },
        ) => {
            let parameter = context.string_table.resolve(parameter_name);
            let existing_type = diagnostic.types().display_for_render(
                *existing_type,
                context.string_table,
                context.path_table(),
            );
            let replacement_type = diagnostic.types().display_for_render(
                *replacement_type,
                context.string_table,
                context.path_table(),
            );
            let subject = match subject {
                crate::compiler_frontend::compiler_messages::GenericInferenceSubject::Function => {
                    "generic function"
                }
                crate::compiler_frontend::compiler_messages::GenericInferenceSubject::NominalType => {
                    "generic type"
                }
            };
            format!(
                "Generic parameter '{parameter}' in {subject} {type_name} was inferred as both {existing_type} and {replacement_type}."
            )
        }
        (
            Shape::MissingTraitEvidence {
                concrete_type, ..
            },
            Extra::MissingTraitEvidence {
                parameter_name,
                trait_name,
            },
        ) => {
            let parameter = context.string_table.resolve(parameter_name);
            let trait_name = context.string_table.resolve(trait_name);
            let concrete_type = diagnostic.types().display_for_render(
                *concrete_type,
                context.string_table,
                context.path_table(),
            );
            format!(
                "Generic function {type_name} requires '{parameter}' to satisfy trait '{trait_name}', but {concrete_type} does not conform to that trait."
            )
        }
        (
            Shape::MissingNominalTraitEvidence {
                concrete_type, ..
            },
            Extra::MissingTraitEvidence {
                parameter_name,
                trait_name,
            },
        ) => {
            let parameter = context.string_table.resolve(parameter_name);
            let trait_name = context.string_table.resolve(trait_name);
            let concrete_type = diagnostic.types().display_for_render(
                *concrete_type,
                context.string_table,
                context.path_table(),
            );
            format!(
                "Generic type {type_name} requires '{parameter}' to satisfy trait '{trait_name}', but {concrete_type} does not conform to that trait."
            )
        }
        (Shape::RecursiveFunctionInstantiation { .. }, Extra::None) => format!(
            "Generic function {type_name} recursively instantiates itself, which is deferred for Alpha."
        ),
        (Shape::ExplicitCallTypeArgumentsUnsupported { .. }, Extra::None) => {
            "Explicit generic call-site type arguments are not supported. Add an ordinary type annotation to the receiving declaration or argument instead.".to_string()
        }
        (Shape::GenericFunctionValueDeferred { .. }, Extra::None) => format!(
            "Generic functions cannot be used as values. Call {type_name} directly or write a concrete wrapper function."
        ),
        _ => crate::compiler_bug!(
            CompilerStage::DiagnosticReporting,
            "generic-instantiation code and decoded fact shapes agree"
        ),
    };
    let guidance = match &facts.shape {
        Shape::CannotInferFunctionArguments { .. } => vec![
            "Add a type annotation to the receiving declaration, for example `value I32 = ...`."
                .to_owned(),
        ],
        _ => Vec::new(),
    };
    RenderedPayload { message, guidance }
}

pub(crate) fn invalid_receiver_declaration_message(
    code: InvalidReceiverDeclarationCode,
    function_name: Option<StringId>,
    type_name: Option<String>,
    string_table: &dyn StringTableResolver,
) -> String {
    use InvalidReceiverDeclarationCode as Code;

    match code {
        Code::UnknownStructTarget => {
            "Receiver method targets an unknown receiver type.".to_string()
        }
        Code::NonlocalSourceType => {
            "Source-authored receiver methods must be declared in the same file as their user-defined receiver type. Use a free function for values owned by another file or package."
                .to_string()
        }
        Code::BuiltinScalarType => {
            "Source-authored receiver methods cannot target builtin scalar types. Use a free function for builtin values."
                .to_string()
        }
        Code::ExternalOpaqueType => {
            "Source-authored receiver methods cannot target external opaque types. Use a free function for values owned by another package."
                .to_string()
        }
        Code::FieldNameConflict => {
            "Struct declares both a field and a method with the same name.".to_string()
        }
        Code::DuplicateMethod => "Duplicate receiver method for this receiver and name.".to_string(),
        Code::DuplicateVisibleMethod => {
            "Visible receiver methods collide for the same receiver type and method name."
                .to_string()
        }
        Code::GenericReceiverType => {
            "Receiver methods on instantiated generic receiver types are not supported. Define the method on the generic type declaration using the receiver type's own parameters."
                .to_string()
        }
        Code::UnsupportedType => format!(
            "Function '{}' uses unsupported receiver type '{}'. Source-authored receiver methods must target a user-defined struct or choice.",
            string_table.resolve(function_name.expect("unsupported receiver type stores its function name")),
            type_name.expect("unsupported receiver type stores its type")
        ),
        Code::ReceiverMethodImportOrExportNotAllowed => {
            "Receiver methods are not dependency-bound, exported, or aliased independently. Expose the receiver type; its same-file methods travel with values of that type and are called through receiver-call syntax."
                .to_string()
        }
        Code::GenericOwnerAlignment => {
            "A generic receiver method requires a generic receiver type whose parameters it can reuse."
                .to_string()
        }
        Code::GenericParameterAlignment => {
            "Generic receiver method parameters must match the receiver type's parameters in count, name, and order."
                .to_string()
        }
    }
}

/// WHAT: renders a receiver declaration from its reason code and retained name/type facts.
/// WHY: the type display id resolves through the diagnostic's type snapshot, not the live
/// type environment.
pub(crate) fn render_invalid_receiver_declaration(
    diagnostic: &DiagnosticView<'_>,
    facts: &InvalidReceiverDeclarationView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let type_name = match facts.code {
        InvalidReceiverDeclarationCode::UnsupportedType => Some(
            diagnostic.types().display_for_render(
                facts
                    .type_id
                    .expect("unsupported receiver type stores its type"),
                context.string_table,
                context.path_table(),
            ),
        ),
        _ => None,
    };
    RenderedPayload {
        message: invalid_receiver_declaration_message(
            facts.code,
            facts.function_name,
            type_name,
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

/// WHAT: renders a duplicate declaration from its compact facts.
/// WHY: the record stores the declared name that `duplicate_declaration_message` consumes.
pub(crate) fn render_duplicate_declaration(
    _diagnostic: &DiagnosticView<'_>,
    facts: &DuplicateDeclarationView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: duplicate_declaration_message(facts.name, context.string_table),
        guidance: Vec::new(),
    }
}

/// WHAT: renders an unknown value name from its compact facts.
/// WHY: the family fixes the value namespace, so the record stores only the name.
pub(crate) fn render_unknown_value_name(
    _diagnostic: &DiagnosticView<'_>,
    facts: &UnknownValueNameView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: unknown_name_message(facts.name, NameNamespace::Value, context.string_table),
        guidance: Vec::new(),
    }
}

/// WHAT: renders an unknown type name from its compact facts and optional close candidates.
/// WHY: the family fixes the type namespace, so the record stores the name plus visible
/// candidates; builtin spellings are compared only for type-annotation sites.
pub(crate) fn render_unknown_type_name(
    diagnostic: &DiagnosticView<'_>,
    facts: &UnknownTypeNameView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let mut message = unknown_name_message(facts.name, NameNamespace::Type, context.string_table);
    let name_str = context.string_table.resolve(facts.name);
    let suggested = match facts.site {
        UnknownTypeNameSite::TypeAnnotation => closest_name_suggestion_with_extra(
            name_str,
            diagnostic.string_ids(),
            builtin_type_annotation_spellings(),
            context.string_table,
        ),
        UnknownTypeNameSite::NamespaceMember | UnknownTypeNameSite::ConformanceTarget => {
            closest_name_suggestion(name_str, diagnostic.string_ids(), context.string_table)
        }
    };
    if let Some(suggested) = suggested {
        message.push_str(&format!(" Did you mean '{suggested}'?"));
    }
    RenderedPayload {
        message,
        guidance: Vec::new(),
    }
}

pub(crate) fn render_unknown_name(
    _diagnostic: &DiagnosticView<'_>,
    facts: &UnknownNameView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: namespace_misuse_message(
            facts.name,
            facts.expected,
            facts.found,
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

pub(crate) fn render_value_used_as_type(
    _diagnostic: &DiagnosticView<'_>,
    facts: &ValueUsedAsTypeView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: namespace_misuse_message(
            facts.name,
            facts.expected,
            facts.found,
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

pub(crate) fn render_type_used_as_value(
    _diagnostic: &DiagnosticView<'_>,
    facts: &TypeUsedAsValueView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let name = context.string_table.resolve(facts.name);
    match facts.reason {
        TypeUsedAsValueReason::GenericStructApplication => RenderedPayload {
            message: format!(
                "`{name} of ...` is a type annotation, not a value expression. Construct the value with `{name}(...)` and provide a concrete receiving type when inference needs it."
            ),
            guidance: Vec::new(),
        },
        TypeUsedAsValueReason::GenericChoiceApplication => RenderedPayload {
            message: format!(
                "`{name} of ...` is a type annotation, not a value expression. Construct one of the `{name}` variants instead and provide a concrete receiving type when inference needs it."
            ),
            guidance: Vec::new(),
        },
        TypeUsedAsValueReason::BareTypeName => RenderedPayload {
            message: namespace_misuse_message(
                facts.name,
                facts.expected,
                facts.found,
                context.string_table,
            ),
            guidance: Vec::new(),
        },
    }
}
