//! Diagnostic render boundary.
//!
//! WHAT: owns user-visible text generation for typed diagnostics.
//! WHY: frontend stages emit facts; this module is the only normal place where those facts become
//! prose, terminal output, terse records, or dev-server HTML.

pub(crate) mod dev_server;
pub(crate) mod terminal;
pub(crate) mod terse;

#[cfg(test)]
mod tests;

mod borrow;
mod calls;
mod context;
mod control_flow;
mod declarations;
mod import_config;
mod paths;
mod payload;
mod suggestions;
mod syntax;
mod templates;

pub(crate) use borrow::*;
pub(crate) use calls::*;
pub(crate) use context::*;
pub(crate) use control_flow::*;
pub(crate) use declarations::*;
pub(crate) use import_config::*;
pub(crate) use paths::*;
pub(crate) use payload::*;
pub(crate) use suggestions::*;
pub(crate) use syntax::*;
pub(crate) use templates::*;

use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, DeferredFeatureReason, DiagnosticOperator, DiagnosticPlace,
    GenericApplicationErrorReason, IncompatibleChoiceComparisonReason, InvalidChoiceVariantReason,
    InvalidCollectionTypeReason, InvalidCompileTimePathReason, InvalidConfigReason,
    InvalidDependencyClauseReason, InvalidExpressionReason, InvalidFallibleOperandReason,
    InvalidGenericParameterReason, InvalidImportPathReason, InvalidMapLiteralReason,
    InvalidMapTypeReason, InvalidMutableAccessReason, InvalidOutputFolderReason,
    InvalidPageMetadataReason, InvalidTemplateDirectiveReason, NameNamespace,
    NamespaceTypeValueMisuseKind, PathKind, RangeOperandKind, UnsupportedOperatorCategory,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::display::display_type;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source_packages::root_file::{
    dependency_component_is_config_file, dependency_component_is_support_root_file,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenKind;

pub(crate) fn invalid_generic_application_message(
    reason: GenericApplicationErrorReason,
) -> &'static str {
    match reason {
        GenericApplicationErrorReason::OnNonNamedType => {
            "Generic application can only be used with a named type."
        }
        GenericApplicationErrorReason::EmptyArgumentList => {
            "Generic application requires at least one type argument."
        }
        GenericApplicationErrorReason::MissingArgumentAfterComma => {
            "Generic application is missing a type argument after ','."
        }
        GenericApplicationErrorReason::NestedApplication => {
            "Nested generic type applications are not supported in a single annotation."
        }
    }
}

pub(crate) fn invalid_collection_type_message(reason: InvalidCollectionTypeReason) -> &'static str {
    match reason {
        InvalidCollectionTypeReason::NegativeCapacity => {
            "Fixed collection capacity must be greater than zero."
        }
        InvalidCollectionTypeReason::ShorthandCapacityNotAllowed => {
            "Capacity-only shorthand is not allowed in type signatures, aliases, fields, or return types."
        }
        InvalidCollectionTypeReason::ZeroCapacity => {
            "Fixed collection capacity must be greater than zero."
        }
        InvalidCollectionTypeReason::CapacityNotInt => "Collection capacity must be an integer.",
        InvalidCollectionTypeReason::CapacityNotConstant => {
            "Collection capacity must be a positive integer literal or the bare name of a visible compile-time `Int` constant."
        }
        InvalidCollectionTypeReason::CapacityOverflow => "Collection capacity is too large.",
        InvalidCollectionTypeReason::InitializerExceedsFixedCapacity { .. } => {
            "Collection literal has more items than the fixed collection capacity allows."
        }
        InvalidCollectionTypeReason::EmptyImmutableFixedCollection => {
            "Immutable binding initialized with an empty fixed collection literal is not allowed."
        }
        InvalidCollectionTypeReason::ShorthandEmptyLiteralAmbiguous => {
            "Capacity-only shorthand requires a non-empty collection literal so the element type can be inferred."
        }
        InvalidCollectionTypeReason::ShorthandNonLiteralRhs => {
            "Capacity-only shorthand requires a collection literal initializer."
        }
    }
}

pub(crate) fn invalid_map_type_message(
    reason: InvalidMapTypeReason,
    context: DiagnosticRenderContext,
) -> String {
    match reason {
        InvalidMapTypeReason::UnsupportedKeyType { key_type } => {
            let type_name = diagnostic_type_name(key_type, context);
            format!(
                "Map key type '{type_name}' is not supported. Builtin hashmap keys are limited to String, Int, Bool, and Char. Use a package or user-defined map type for custom key behavior."
            )
        }
        InvalidMapTypeReason::ExcessiveInlineNesting { depth } => {
            format!(
                "Map types nested {depth} levels deep are not allowed inline. Use a type alias instead."
            )
        }
        InvalidMapTypeReason::EmptyMapKeyType => {
            "Map type is missing the key type before the '=' separator.".to_owned()
        }
        InvalidMapTypeReason::EmptyMapValueType => {
            "Map type is missing the value type after the '=' separator.".to_owned()
        }
        InvalidMapTypeReason::MultipleMapSeparators => {
            "Map type can only contain one top-level '=' separator.".to_owned()
        }
        InvalidMapTypeReason::FixedCapacityNotAllowed => {
            "Fixed or capacity map syntax is outside Moth's builtin hashmap design.".to_owned()
        }
    }
}

pub(crate) fn invalid_map_literal_message(reason: InvalidMapLiteralReason) -> String {
    match reason {
        InvalidMapLiteralReason::MixedCollectionMapEntries => {
            "Map literal entries must all use `key = value` syntax. Mixed collection and map entries are not allowed.".to_owned()
        }
        InvalidMapLiteralReason::DuplicateKnownKey => {
            "Duplicate key in map literal. A foldable key with the same value appears more than once.".to_owned()
        }
        InvalidMapLiteralReason::MissingKeyExpression => {
            "Map literal entry is missing a key expression before '='.".to_owned()
        }
        InvalidMapLiteralReason::MissingValueExpression => {
            "Map literal entry is missing a value expression after '='.".to_owned()
        }
    }
}

pub(crate) fn invalid_generic_parameter_message(
    reason: &InvalidGenericParameterReason,
    string_table: &StringTable,
) -> String {
    match reason {
        InvalidGenericParameterReason::EmptyParameterList
        | InvalidGenericParameterReason::BoundsMustUseIs
        | InvalidGenericParameterReason::ListMustStayWithHeader => {
            invalid_generic_parameter_static_message(reason).to_owned()
        }
        InvalidGenericParameterReason::InvalidToken { found } => {
            format!(
                "Invalid generic parameter token {}.",
                token_kind_name(found, string_table)
            )
        }
    }
}

fn invalid_generic_parameter_static_message(
    reason: &InvalidGenericParameterReason,
) -> &'static str {
    match reason {
        InvalidGenericParameterReason::EmptyParameterList => {
            "Expected at least one generic parameter after `type`."
        }
        InvalidGenericParameterReason::BoundsMustUseIs => {
            "Generic parameter bounds use `is`. Write `type T is TRAIT`."
        }
        InvalidGenericParameterReason::ListMustStayWithHeader => {
            "Generic parameter lists must stay with the declaration header."
        }
        InvalidGenericParameterReason::InvalidToken { .. } => "Invalid generic parameter token.",
    }
}

pub(crate) fn invalid_template_directive_message(
    directive_name: Option<StringId>,
    reason: InvalidTemplateDirectiveReason,
    string_table: &StringTable,
) -> String {
    let name = directive_name
        .map(|id| string_table.resolve(id))
        .unwrap_or("unknown");

    match reason {
        InvalidTemplateDirectiveReason::UnknownDirective => {
            format!("Unknown template directive '{name}'.")
        }
        InvalidTemplateDirectiveReason::MissingArgument => {
            format!("Template directive '{name}' is missing a required argument.")
        }
        InvalidTemplateDirectiveReason::InvalidArgument { detail } => {
            let message = format!("Invalid argument for template directive '{name}'.");
            match detail {
                Some(detail_id) => format!("{message} {}", string_table.resolve(detail_id)),
                None => message,
            }
        }
        InvalidTemplateDirectiveReason::DirectiveNotAllowedHere => {
            format!("Template directive '{name}' is not allowed here.")
        }
        InvalidTemplateDirectiveReason::UnexpectedArguments => {
            format!("`${name}` does not take arguments. Write `${name}:` instead.")
        }
        InvalidTemplateDirectiveReason::EmptyArguments => {
            format!("`${name}()` has empty parentheses. Remove the parentheses or provide an argument.")
        }
        InvalidTemplateDirectiveReason::InvalidSlotTarget => {
            "`$slot` accepts no argument, a string name, or a positive whole-number position.\n                 Use `$slot`, `$slot(\"title\")`, or `$slot(1)`.".to_owned()
        }
        InvalidTemplateDirectiveReason::InvalidInsertTarget => {
            "`$insert(...)` requires a string slot name.\n                 Use `$insert(\"title\")`.".to_owned()
        }
        InvalidTemplateDirectiveReason::InvalidChildrenArgument => {
            "`$children(...)` requires one wrapper template or string argument.\n                 Example: `$children([:<li>[$slot]</li>])`.".to_owned()
        }
    }
}

pub(crate) fn namespace_misuse_message(
    name: StringId,
    expected: NameNamespace,
    found: NameNamespace,
    string_table: &StringTable,
) -> String {
    let name = string_table.resolve(name);

    match (expected, found) {
        (NameNamespace::Type, NameNamespace::Value) => {
            format!("'{name}' is a value and cannot be used as a type.")
        }
        (NameNamespace::Value, NameNamespace::Type) => {
            format!("'{name}' is a type and cannot be used as a value.")
        }
        _ => {
            let expected = namespace_name(expected);
            let found = namespace_name(found);
            format!(
                "'{name}' is in the {found} namespace, but the {expected} namespace was expected."
            )
        }
    }
}

pub(crate) fn unsupported_operator_types_message(
    operator: DiagnosticOperator,
    lhs: TypeId,
    rhs: Option<TypeId>,
    context: DiagnosticRenderContext<'_>,
) -> String {
    // Generic parameter operands explain that operators are compiler-owned and bounds do not
    // grant operator support, then recommend a concrete type or bound-provided receiver method.
    if let Some(message) = generic_parameter_operator_message(operator, lhs, rhs, context) {
        return message;
    }

    let operator_spelling = operator.source_spelling();

    // Unary `not` always requires a Bool operand.
    if operator == DiagnosticOperator::Not {
        let expected = diagnostic_type_name(builtin_type_ids::BOOL, context);
        let found = diagnostic_type_name(lhs, context);
        return format!(
            "Operator `{operator_spelling}` requires a `{expected}` operand, found `{found}`."
        );
    }

    // Mixed String `+` is not concatenation; templates own textual interpolation.
    if operator == DiagnosticOperator::Add
        && let Some(rhs) = rhs
        && exactly_one_operand_is_plain_string(lhs, rhs)
    {
        let left = diagnostic_type_name(lhs, context);
        let right = diagnostic_type_name(rhs, context);
        return format!(
            "Operator `{operator_spelling}` cannot concatenate `{left}` and `{right}`. Use a template for mixed textual interpolation."
        );
    }

    // Factual exact-operator fallback for every other unsupported operand combination.
    if let Some(rhs) = rhs {
        let left = diagnostic_type_name(lhs, context);
        let right = diagnostic_type_name(rhs, context);
        format!(
            "Operator `{operator_spelling}` does not support operand types `{left}` and `{right}`."
        )
    } else {
        let operand = diagnostic_type_name(lhs, context);
        format!("Operator `{operator_spelling}` does not support operand type `{operand}`.")
    }
}

fn generic_parameter_operator_message(
    operator: DiagnosticOperator,
    lhs: TypeId,
    rhs: Option<TypeId>,
    context: DiagnosticRenderContext<'_>,
) -> Option<String> {
    let mut parameter_names = Vec::new();

    if let Some(name) = generic_parameter_name(lhs, context) {
        parameter_names.push(name);
    }

    if let Some(rhs) = rhs
        && rhs != lhs
        && let Some(name) = generic_parameter_name(rhs, context)
    {
        parameter_names.push(name);
    }

    if parameter_names.is_empty() {
        return None;
    }

    let operator_spelling = operator.source_spelling();
    let subject = if parameter_names.len() == 1 {
        format!("generic parameter `{}`", parameter_names[0])
    } else {
        format!(
            "generic parameters `{}` and `{}`",
            parameter_names[0], parameter_names[1]
        )
    };

    Some(format!(
        "Operator `{operator_spelling}` is not available for {subject}. Moth operators are compiler-owned and generic bounds do not provide operator support. Use a concrete type or a receiver method provided by an explicit bound."
    ))
}

fn exactly_one_operand_is_plain_string(lhs: TypeId, rhs: TypeId) -> bool {
    (lhs == builtin_type_ids::STRING) != (rhs == builtin_type_ids::STRING)
}

fn generic_parameter_name(type_id: TypeId, context: DiagnosticRenderContext<'_>) -> Option<String> {
    let type_environment = context.type_environment?;
    match type_environment.get(type_id) {
        Some(TypeDefinition::GenericParameter(_)) => Some(diagnostic_type_name(type_id, context)),
        _ => None,
    }
}

pub(crate) fn invalid_fallible_operand_message(
    reason: InvalidFallibleOperandReason,
    category: UnsupportedOperatorCategory,
    operand_type: TypeId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let category_name = unsupported_operator_category_name(category);
    let type_name = diagnostic_type_name(operand_type, context);

    match reason {
        InvalidFallibleOperandReason::FallibleValueNotHandled => {
            format!(
                "{category_name} operator cannot use a fallible value that has not been handled. Use postfix `!` in a compatible fallible function or recover with `catch` before applying the operator (found '{type_name}')."
            )
        }
    }
}

fn namespace_name(namespace: NameNamespace) -> &'static str {
    match namespace {
        NameNamespace::Value => "value",
        NameNamespace::Type => "type",
        NameNamespace::Module => "module",
        NameNamespace::Field => "field",
        NameNamespace::Variant => "variant",
        NameNamespace::Function => "function",
        NameNamespace::Method => "method",
        NameNamespace::TemplateSlot => "template slot",
        NameNamespace::ConfigKey => "config key",
    }
}

fn unsupported_operator_category_name(category: UnsupportedOperatorCategory) -> &'static str {
    match category {
        UnsupportedOperatorCategory::Arithmetic => "arithmetic",
        UnsupportedOperatorCategory::Comparison => "comparison",
        UnsupportedOperatorCategory::Range => "range",
        UnsupportedOperatorCategory::Logical => "logical",
        UnsupportedOperatorCategory::Unary => "unary",
        UnsupportedOperatorCategory::Other => "this",
    }
}

pub(crate) fn unsupported_external_function_message(
    function_name: StringId,
    package_path: Option<StringId>,
    backend_name: StringId,
    string_table: &StringTable,
) -> String {
    let function_name = string_table.resolve(function_name);
    let backend_name = string_table.resolve(backend_name);

    if let Some(package_path) = package_path {
        let package_path = string_table.resolve(package_path);
        format!(
            "External function '{function_name}' from package '{package_path}' is not supported by the {backend_name} backend."
        )
    } else {
        format!(
            "External function '{function_name}' is not supported by the {backend_name} backend."
        )
    }
}

pub(crate) fn invalid_choice_variant_message(
    reason: InvalidChoiceVariantReason,
    choice_name: Option<StringId>,
    variant_name: Option<StringId>,
    available_variants: &[StringId],
    string_table: &StringTable,
) -> String {
    let choice_name = choice_name
        .map(|name| string_table.resolve(name).to_owned())
        .unwrap_or_default();
    let variant_name = variant_name
        .map(|name| string_table.resolve(name).to_owned())
        .unwrap_or_default();

    // Keep name suggestions consistent across named arguments, fields and choice variants.
    let variant_suggestion =
        closest_name_suggestion(&variant_name, available_variants, string_table);

    let available_variants_hint = if available_variants.is_empty() {
        String::new()
    } else {
        let names = available_variants
            .iter()
            .map(|name| string_table.resolve(*name).to_owned())
            .collect::<Vec<_>>()
            .join(", ");
        format!(" Available variants: [{names}].")
    };

    match reason {
        InvalidChoiceVariantReason::EmptyRecordBody => {
            "Choice variant record body cannot be empty.".to_owned()
        }
        InvalidChoiceVariantReason::RecursiveDeclaration => {
            "Recursive choice declarations are not supported.".to_owned()
        }
        InvalidChoiceVariantReason::ConstructorStyleNotSupported => {
            "Constructor-style choice declarations are not supported.".to_owned()
        }
        InvalidChoiceVariantReason::PayloadShorthandNotSupported => {
            "Choice payload shorthand is not supported. Use a record body `| field Type, ... |`."
                .to_owned()
        }
        InvalidChoiceVariantReason::UnexpectedSeparator => {
            "Unexpected separator in choice variant declaration.".to_owned()
        }
        InvalidChoiceVariantReason::MissingVariants => {
            "Choice declarations must define at least one variant.".to_owned()
        }
        InvalidChoiceVariantReason::UnknownVariant => match variant_suggestion.as_deref() {
            Some(suggested) => format!(
                "Unknown variant '{choice_name}::{variant_name}'. Did you mean '{suggested}'?{available_variants_hint}"
            ),
            None => {
                format!("Unknown variant '{choice_name}::{variant_name}'.{available_variants_hint}")
            }
        },
        InvalidChoiceVariantReason::UnitVariantWithParentheses => {
            format!(
                "Unit variant '{choice_name}::{variant_name}' cannot be called with empty parentheses."
            )
        }
        InvalidChoiceVariantReason::UnitVariantAsConstructor => {
            format!(
                "Unit variant '{choice_name}::{variant_name}' cannot be called as a constructor."
            )
        }
        InvalidChoiceVariantReason::PayloadVariantMissingArguments => {
            format!(
                "Payload variant '{choice_name}::{variant_name}' requires constructor arguments."
            )
        }
    }
}

pub(crate) fn incompatible_choice_comparison_message(
    reason: &IncompatibleChoiceComparisonReason,
    lhs: TypeId,
    rhs: TypeId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let lhs_name = diagnostic_type_name(lhs, context);
    let rhs_name = diagnostic_type_name(rhs, context);

    match reason {
        IncompatibleChoiceComparisonReason::DifferentChoiceTypes => {
            format!("Cannot compare choices of different types: '{lhs_name}' and '{rhs_name}'.")
        }
        IncompatibleChoiceComparisonReason::ChoiceWithNonChoice => {
            format!(
                "Cannot compare choice '{lhs_name}' with '{rhs_name}'. Choices can only be compared with values of the same choice type."
            )
        }
        IncompatibleChoiceComparisonReason::PayloadEqualityNotSupported {
            field_name,
            field_type,
        } => {
            let field_name = context.string_table.resolve(*field_name);
            let field_type_name = diagnostic_type_name(*field_type, context);
            format!(
                "Choice payload equality is not supported because field '{field_name}' has type '{field_type_name}', which does not support equality."
            )
        }
    }
}

pub(crate) fn deferred_feature_message(
    reason: &DeferredFeatureReason,
    string_table: &StringTable,
) -> String {
    if let DeferredFeatureReason::NamedFeature { feature } = reason {
        return format!("Deferred feature: {}.", string_table.resolve(*feature));
    }

    deferred_feature_static_message(reason).to_owned()
}

fn deferred_feature_static_message(reason: &DeferredFeatureReason) -> &'static str {
    match reason {
        DeferredFeatureReason::NamedFeature { .. } => "Deferred feature.",
        DeferredFeatureReason::CaptureTaggedPattern => {
            "Capture/tagged patterns using '|...|' are deferred for Alpha."
        }
        DeferredFeatureReason::NegatedMatchPattern => {
            "Negated match patterns (for example 'not ... =>') are deferred."
        }
        DeferredFeatureReason::NamedPayloadPatternAssignment => {
            "Named payload pattern assignment is deferred. Use positional capture with the field name."
        }
        DeferredFeatureReason::NestedPayloadPattern => {
            "Nested payload patterns are deferred. Use flat capture bindings with declared field names only."
        }
        DeferredFeatureReason::ChoiceVariantDefaultValue => {
            "Choice variant default values are deferred for Alpha. Declare explicit unit or payload variants and pass values when constructing payload variants."
        }
        DeferredFeatureReason::GenericReceiverMethod => {
            "Receiver methods on instantiated generic receiver types are not supported. Define the method on the generic type declaration using the receiver type's own parameters."
        }
        DeferredFeatureReason::DeclaredRegion => {
            "Declared regions using `name:` and `into name` are accepted but not implemented yet."
        }
        DeferredFeatureReason::CheckedBlock => {
            "`checked:` blocks are reserved for future advanced validation, but are not implemented yet."
        }
        DeferredFeatureReason::AsyncBlock => {
            "`async:` blocks are reserved for future language support, but are not implemented yet."
        }
        DeferredFeatureReason::RuntimeAnonymousRecord => {
            "Runtime anonymous records are deferred. Use a compile-time receiving context (`#=`) for anonymous const records."
        }
    }
}

pub(crate) fn invalid_range_operand_message(
    operand: RangeOperandKind,
    found_type: TypeId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let operand_name = match operand {
        RangeOperandKind::Start => "start",
        RangeOperandKind::End => "end",
        RangeOperandKind::Step => "step",
    };
    let type_name = diagnostic_type_name(found_type, context);

    format!("Range {operand_name} must be numeric (Int or Float). Found '{type_name}'")
}

pub(crate) fn unsupported_builder_package_message(
    package_path: StringId,
    string_table: &StringTable,
) -> String {
    let package_path_str = string_table.resolve(package_path);
    format!(
        "Core package '{package_path_str}' is not supported by this builder. \
         Use a builder that exposes this core package or remove the dependency clause."
    )
}

pub(crate) fn invalid_page_metadata_message(
    key: StringId,
    reason: InvalidPageMetadataReason,
    string_table: &StringTable,
) -> String {
    let key_str = string_table.resolve(key);
    match reason {
        InvalidPageMetadataReason::NotAString => {
            format!("Reserved HTML page metadata constant '{key_str}' must fold to a string.")
        }
        InvalidPageMetadataReason::DuplicateDeclaration => {
            format!(
                "Reserved HTML page metadata constant '{key_str}' is declared more than once for this entry page."
            )
        }
    }
}

pub(crate) fn invalid_expression_message(reason: InvalidExpressionReason) -> String {
    match reason {
        InvalidExpressionReason::ExpectedOperatorBeforeExpression => {
            "Expected an operator before this expression.".to_owned()
        }
        InvalidExpressionReason::UnresolvedStackShape => {
            "This expression does not resolve to exactly one value.".to_owned()
        }
        InvalidExpressionReason::MothFileHasNoValue => {
            "A `.moth` file has no file value. Bind its declarations through a dependency clause.".to_owned()
        }
        InvalidExpressionReason::ExtensionlessFileValue => {
            "A file value needs an explicit extension. Write the path with a file extension, or use a dependency clause to bind declarations.".to_owned()
        }
        InvalidExpressionReason::AnonymousRecordFieldNotNamed => {
            "Each const-record parameter needs a value. Write `name = value`.".to_owned()
        }
        InvalidExpressionReason::NestedAnonymousConstRecord => {
            "Do not nest a `|...|` parameter list inside another. Declare the inner struct or record first, then use that name as a parameter value.".to_owned()
        }
    }
}

/// Determine which special file name is referenced by a dependency path.
///
/// WHAT: inspects path components to find a support-root or canonical config reference.
/// WHY: the direct-special-file diagnostic covers support roots and config files, and
/// renderers should name the specific file when possible so the author knows which file
/// to avoid binding directly. Normal `@*.moth` root references are caught earlier by
/// the path parser's `LeadingAtInPathComponent` rejection.
pub(crate) fn special_file_name_from_path(
    path: &InternedPath,
    string_table: &StringTable,
) -> String {
    // Support-root markers (`+`) are unambiguous even when an earlier folder shares a name.
    for component in path.as_components() {
        let segment = string_table.resolve(*component);
        if dependency_component_is_support_root_file(segment) {
            let suffix = if segment.contains('.') { "" } else { ".moth" };
            return format!("{segment}{suffix}");
        }
    }

    for component in path.as_components() {
        let segment = string_table.resolve(*component);
        if dependency_component_is_config_file(segment) {
            return "config.moth".to_owned();
        }
    }
    "special file".to_owned()
}

fn named_value_or_default(
    name: Option<StringId>,
    string_table: &StringTable,
    fallback: &'static str,
) -> String {
    name.map(|name| format!("'{}'", string_table.resolve(name)))
        .unwrap_or_else(|| fallback.to_string())
}

/// Build a human-facing suggestion for a direct support-root-file dependency.
///
/// WHAT: inspects the dependency path and produces a hint that names the package directory the
/// author should bind instead of the support root file.
/// WHY: the `+` prefix on support root filenames is cosmetic and does not appear in dependency
///      paths. Authors who write `@helper/+page symbol` should be told to use
///      `@helper symbol` instead.
pub(crate) fn support_root_import_suggestion(
    path: &InternedPath,
    string_table: &StringTable,
) -> String {
    let components = path.as_components();
    let mut suggestion_path: Vec<String> = Vec::new();
    let mut found_root_component = false;

    for component in components {
        let segment = string_table.resolve(*component);
        if dependency_component_is_support_root_file(segment) {
            found_root_component = true;
            continue;
        }
        suggestion_path.push(segment.to_owned());
    }

    if !found_root_component {
        return String::new();
    }

    let directory = suggestion_path.join("/");
    if directory.is_empty() {
        " Bind the support package directory instead of the root file.".to_owned()
    } else {
        format!(" Bind the support package `@{directory}` instead of the root file.")
    }
}

pub(crate) fn display_line_number(raw_line: i32) -> i32 {
    raw_line.saturating_add(1).max(1)
}

pub(crate) fn display_column_number(raw_column: i32) -> i32 {
    raw_column.saturating_add(1).max(1)
}
