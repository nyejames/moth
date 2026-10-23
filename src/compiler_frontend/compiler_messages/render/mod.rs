//! Diagnostic render boundary.
//!
//! WHAT: owns user-visible text generation for typed diagnostics.
//! WHY: frontend stages emit facts; this module is the only normal place where those facts become
//! prose, terminal output, terse records, or dev-server HTML.

pub(crate) mod dev_server;
pub(crate) mod infrastructure;
pub(crate) mod terminal;
pub(crate) mod terse;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use tests::infrastructure_tests::assert_compiler_bug_views;

mod borrow;
mod calls;
pub(super) mod context;
mod control_flow;
mod declarations;
mod import_config;
mod paths;
mod payload;
pub(super) mod subject;
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

use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    InvalidCollectionTypeView, InvalidMapLiteralView, InvalidMapTypeCode, InvalidMapTypeView,
    InvalidRangeOperandView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    InvalidChoiceVariantView, invalid_choice_variant_available_variants,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    InvalidGenericApplicationView, InvalidGenericParameterView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::DeferredFeatureView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
    InvalidPageMetadataView, UnsupportedBuilderPackageView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::{
    InvalidTemplateDirectiveCode, InvalidTemplateDirectiveView, MalformedCssTemplateView,
    MalformedHtmlTemplateView,
};

use crate::compiler_frontend::compiler_messages::diagnostic_types::{
    TypeDisplayId, type_display_snapshot::TypeDisplaySnapshot,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    IncompatibleChoiceComparisonCode, IncompatibleChoiceComparisonView, InvalidRouteOutputView,
    UnsupportedBackendFeatureView, UnsupportedExternalFunctionView, UnsupportedOperatorTypesView,
    incompatible_choice_comparison_field_name,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::{
    AstCapacityResource, BorrowAccessKind, CapacityResource, CssTemplateWarning,
    DeferredFeatureReason, DiagnosticOperator, DiagnosticPlace, GenericApplicationErrorReason,
    HirCapacityResource, HtmlTemplateWarning, InvalidChoiceVariantReason,
    InvalidCollectionTypeReason, InvalidCompileTimePathReason, InvalidConfigReason,
    InvalidDependencyClauseReason, InvalidExpressionReason, InvalidExternalModuleReason,
    InvalidGenericParameterReason, InvalidImportPathReason, InvalidMapLiteralReason,
    InvalidMutableAccessReason, InvalidOutputFolderReason, InvalidPageMetadataReason,
    InvalidRangeOperandReason, InvalidRouteOutputReason, InvalidTemplateDirectiveReason,
    MalformedTemplateReason, NameNamespace, NamespaceTypeValueMisuseKind, PathKind,
    RangeOperandKind, SourceSpanCapacityResource, UnsupportedBackendFeatureReason,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticToken, TokenDescriptorPayload};
use crate::compiler_frontend::datatypes::display::{
    BOOL_TYPE_SPELLING, TypeDisplayShape, TypeDisplayView,
};
use crate::compiler_frontend::datatypes::numeric_operators::common_fixed_integer;
use crate::compiler_frontend::source_packages::root_file::{
    dependency_component_is_config_file, dependency_component_is_support_root_file,
};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTableResolver};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass};

pub(crate) fn malformed_template_message(
    reason: MalformedTemplateReason,
    string_table: &dyn StringTableResolver,
) -> String {
    match reason {
        MalformedTemplateReason::Css(reason) => css_template_warning_message(reason, string_table),
        MalformedTemplateReason::Html(reason) => html_template_warning_message(reason),
    }
}

/// WHAT: renders a compact CSS-template warning through the legacy warning helper.
/// WHY: the warning code and its optional detail facts rebuild the legacy reason exactly.
pub(crate) fn render_malformed_css_template(
    _diagnostic: &crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView<
        '_,
    >,
    facts: &MalformedCssTemplateView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let reason = facts
        .reason
        .to_reason(facts.expected, facts.property_name)
        .expect("a CSS template warning code stores its required detail");
    RenderedPayload {
        message: malformed_template_message(
            MalformedTemplateReason::Css(reason),
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

/// WHAT: renders a compact HTML-template warning through the legacy warning helper.
/// WHY: each HTML warning is fully represented by its one-word reason code.
pub(crate) fn render_malformed_html_template(
    _diagnostic: &crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView<
        '_,
    >,
    facts: &MalformedHtmlTemplateView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: malformed_template_message(
            MalformedTemplateReason::Html(facts.reason.to_reason()),
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

fn css_template_warning_message(
    reason: CssTemplateWarning,
    string_table: &dyn StringTableResolver,
) -> String {
    match reason {
        CssTemplateWarning::MismatchedClosingBrace { expected } => format!(
            "Malformed template: Mismatched closing brace. Expected '{expected}' to close before '}}'."
        ),
        CssTemplateWarning::UnexpectedClosingBrace => {
            "Malformed template: Unexpected closing brace '}' with no matching '{'.".to_owned()
        }
        CssTemplateWarning::MismatchedClosingParenthesis { expected } => format!(
            "Malformed template: Mismatched closing parenthesis. Expected '{expected}' to close before ')'."
        ),
        CssTemplateWarning::UnexpectedClosingParenthesis => {
            "Malformed template: Unexpected closing parenthesis ')' with no matching '('.".to_owned()
        }
        CssTemplateWarning::UnclosedDelimiter { opening } => {
            format!("Malformed template: Unclosed '{opening}' in CSS template body.")
        }
        CssTemplateWarning::InlineSelectorBlock => {
            "Malformed template: Inline '$css(\"inline\")' templates only allow declarations and cannot contain selector blocks.".to_owned()
        }
        CssTemplateWarning::MissingSelectorOrAtRulePrelude => {
            "Malformed template: CSS block is missing a selector or at-rule prelude before '{'."
                .to_owned()
        }
        CssTemplateWarning::NestedBlock => {
            "Malformed template: Nested CSS blocks are only lightly validated in '$css'. Use nested at-rules for predictable results.".to_owned()
        }
        CssTemplateWarning::TopLevelContent => {
            "Malformed template: Top-level CSS content should be selector blocks or at-rules."
                .to_owned()
        }
        CssTemplateWarning::MalformedDeclaration => {
            "Malformed template: Malformed CSS declaration. Expected 'property: value'."
                .to_owned()
        }
        CssTemplateWarning::InvalidPropertyName { name } => format!(
            "Malformed template: Malformed CSS declaration. Invalid property name '{}'.",
            string_table.resolve(name)
        ),
        CssTemplateWarning::MissingDeclarationValue => {
            "Malformed template: Malformed CSS declaration. Missing value after ':'."
                .to_owned()
        }
    }
}

fn html_template_warning_message(reason: HtmlTemplateWarning) -> String {
    match reason {
        HtmlTemplateWarning::ScriptTag => {
            "Malformed template: Potentially unsafe '<script' tag found in '$html' template."
                .to_owned()
        }
        HtmlTemplateWarning::JavascriptUrl => {
            "Malformed template: Potentially unsafe 'javascript:' URL found in '$html' template."
                .to_owned()
        }
        HtmlTemplateWarning::InlineEventHandler => {
            "Malformed template: Potentially unsafe inline 'on*=' event handler found in '$html' template.".to_owned()
        }
    }
}

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

/// WHAT: renders an invalid generic application from its unit reason.
/// WHY: the compact row stores the same reason used by the legacy message helper.
pub(crate) fn render_invalid_generic_application(
    _diagnostic: &DiagnosticView<'_>,
    facts: &InvalidGenericApplicationView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: invalid_generic_application_message(facts.reason).to_owned(),
        guidance: Vec::new(),
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
            "Collection capacity must be a positive integer literal or the bare name of a visible compile-time integer constant."
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

/// WHAT: renders collection-type diagnostics from a typed reason code and its details.
/// WHY: the compact row preserves all fields needed to rebuild the legacy message reason.
pub(crate) fn render_invalid_collection_type(
    _diagnostic: &DiagnosticView<'_>,
    view: &InvalidCollectionTypeView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let reason = view.reason.to_reason(view.capacity, view.length);
    RenderedPayload {
        message: invalid_collection_type_message(reason).to_owned(),
        guidance: Vec::new(),
    }
}

pub(crate) fn invalid_map_type_message(
    reason: InvalidMapTypeCode,
    key_type_name: Option<String>,
    depth: Option<usize>,
) -> String {
    match reason {
        InvalidMapTypeCode::UnsupportedKeyType => {
            let type_name = key_type_name.expect("unsupported-key code stores its key type");
            format!(
                "Map key type '{type_name}' is not supported. Builtin hashmap keys are limited to String, Bool, Char, I8, I16, I32, I64, U8, U16, U32, U64, and Byte. Use a package or user-defined map type for custom key behavior."
            )
        }
        InvalidMapTypeCode::ExcessiveInlineNesting => {
            let depth = depth.expect("nested-map code stores its depth");
            format!(
                "Map types nested {depth} levels deep are not allowed inline. Use a type alias instead."
            )
        }
        InvalidMapTypeCode::EmptyMapKeyType => {
            "Map type is missing the key type before the '=' separator.".to_owned()
        }
        InvalidMapTypeCode::EmptyMapValueType => {
            "Map type is missing the value type after the '=' separator.".to_owned()
        }
        InvalidMapTypeCode::MultipleMapSeparators => {
            "Map type can only contain one top-level '=' separator.".to_owned()
        }
        InvalidMapTypeCode::FixedCapacityNotAllowed => {
            "Fixed or capacity map syntax is outside Moth's builtin hashmap design.".to_owned()
        }
    }
}

/// WHAT: renders map-type diagnostics from compact type displays and their reason code.
/// WHY: key type names come from the diagnostic's captured display records.
pub(crate) fn render_invalid_map_type(
    diagnostic: &DiagnosticView<'_>,
    view: &InvalidMapTypeView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let path_table = context.path_table();
    let key_type_name = view.key_type.map(|type_id| {
        diagnostic
            .types()
            .display_for_render(type_id, context.string_table, path_table)
    });
    RenderedPayload {
        message: invalid_map_type_message(view.reason, key_type_name, view.depth),
        guidance: Vec::new(),
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
/// WHAT: renders a map-literal diagnostic from its unit reason.
/// WHY: the compact row stores the same reason consumed by the legacy message helper.
pub(crate) fn render_invalid_map_literal(
    _diagnostic: &DiagnosticView<'_>,
    view: &InvalidMapLiteralView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: invalid_map_literal_message(view.reason),
        guidance: Vec::new(),
    }
}

pub(crate) fn invalid_generic_parameter_message(
    reason: &InvalidGenericParameterReason,
    string_table: &dyn StringTableResolver,
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

/// WHAT: renders an invalid generic parameter from its reason code and optional token.
/// WHY: the code determines whether the token fact is required.
pub(crate) fn render_invalid_generic_parameter(
    _diagnostic: &DiagnosticView<'_>,
    facts: &InvalidGenericParameterView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let reason = facts
        .code
        .to_reason(facts.found)
        .expect("an invalid generic parameter code stores its required token");
    RenderedPayload {
        message: invalid_generic_parameter_message(&reason, context.string_table),
        guidance: Vec::new(),
    }
}

pub(crate) fn invalid_template_directive_message(
    directive_name: Option<StringId>,
    reason: InvalidTemplateDirectiveReason,
    string_table: &dyn StringTableResolver,
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

/// WHAT: renders a compact template-directive diagnostic through its legacy prose helper.
/// WHY: the reason code and optional detail reconstruct the data-carrying legacy reason.
pub(crate) fn render_invalid_template_directive(
    _diagnostic: &crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView<
        '_,
    >,
    facts: &InvalidTemplateDirectiveView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let reason = facts.reason.to_reason(facts.detail);
    let guidance = if facts.reason == InvalidTemplateDirectiveCode::UnknownDirective {
        vec![
            "Known template directives include `$md`, `$raw`, `$children`, `$slot`, `$insert`, `$fresh`, `$html`, `$css`, `$escape_html`, `$code`, `$note`, `$todo`, and `$doc`.".to_owned(),
        ]
    } else {
        Vec::new()
    };
    RenderedPayload {
        message: invalid_template_directive_message(
            facts.directive_name,
            reason,
            context.string_table,
        ),
        guidance,
    }
}

pub(crate) fn namespace_misuse_message(
    name: StringId,
    expected: NameNamespace,
    found: NameNamespace,
    string_table: &dyn StringTableResolver,
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
    lhs: TypeDisplayId,
    rhs: Option<TypeDisplayId>,
    types: &TypeDisplaySnapshot,
    context: DiagnosticRenderContext<'_>,
) -> String {
    if let Some(message) = generic_parameter_operator_message(operator, lhs, rhs, types, context) {
        return message;
    }

    let operator_spelling = operator.source_spelling();
    if operator == DiagnosticOperator::Not {
        let expected = BOOL_TYPE_SPELLING;
        let found = diagnostic_type_display_name(types, lhs, context);
        return format!(
            "Operator `{operator_spelling}` requires a `{expected}` operand, found `{found}`."
        );
    }

    if operator == DiagnosticOperator::Add
        && let Some(rhs) = rhs
        && exactly_one_operand_is_plain_string(lhs, rhs, types)
    {
        let left = diagnostic_type_display_name(types, lhs, context);
        let right = diagnostic_type_display_name(types, rhs, context);
        return format!(
            "Operator `{operator_spelling}` cannot concatenate `{left}` and `{right}`. Use a template for mixed textual interpolation."
        );
    }

    let is_unary_minus = operator == DiagnosticOperator::Subtract && rhs.is_none();
    if is_arithmetic_operator(operator)
        && (diagnostic_fixed_scalar(types, lhs) == Some(FixedScalar::Byte)
            || rhs
                .is_some_and(|rhs| diagnostic_fixed_scalar(types, rhs) == Some(FixedScalar::Byte)))
    {
        return "`Byte` has no arithmetic operators.".to_owned();
    }

    if is_unary_minus
        && let Some(scalar) = diagnostic_fixed_scalar(types, lhs)
        && scalar.class() == FixedScalarClass::UnsignedInteger
    {
        let operand = diagnostic_type_display_name(types, lhs, context);
        return format!(
            "Unary `-` does not apply to unsigned `{operand}`. Convert the value to a signed type with `cast` first."
        );
    }

    if is_arithmetic_operator(operator)
        && let Some(rhs) = rhs
        && let (Some(left_scalar), Some(right_scalar)) = (
            diagnostic_fixed_scalar(types, lhs),
            diagnostic_fixed_scalar(types, rhs),
        )
        && is_fixed_integer(left_scalar)
        && is_fixed_integer(right_scalar)
        && common_fixed_integer(left_scalar, right_scalar).is_none()
    {
        let left = diagnostic_type_display_name(types, lhs, context);
        let right = diagnostic_type_display_name(types, rhs, context);
        return format!(
            "`{left}` and `{right}` have no common integer type for `{operator_spelling}`. Convert one operand with `cast` first."
        );
    }

    if (is_arithmetic_operator(operator) || is_comparison_operator(operator))
        && let Some(rhs) = rhs
        && implicitly_mixed_numeric_pair(types, lhs, rhs)
    {
        let left = diagnostic_type_display_name(types, lhs, context);
        let right = diagnostic_type_display_name(types, rhs, context);
        return format!(
            "`{left}` and `{right}` do not mix implicitly. Convert one operand with `cast` first."
        );
    }

    if let Some(rhs) = rhs {
        let left = diagnostic_type_display_name(types, lhs, context);
        let right = diagnostic_type_display_name(types, rhs, context);
        format!(
            "Operator `{operator_spelling}` does not support operand types `{left}` and `{right}`."
        )
    } else {
        let operand = diagnostic_type_display_name(types, lhs, context);
        format!("Operator `{operator_spelling}` does not support operand type `{operand}`.")
    }
}

/// Renders operator failure from its typed operator and type-display facts.
pub(crate) fn render_unsupported_operator_types(
    diagnostic: &DiagnosticView<'_>,
    facts: &UnsupportedOperatorTypesView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: unsupported_operator_types_message(
            facts.operator,
            facts.lhs,
            facts.rhs,
            diagnostic.types(),
            context,
        ),
        guidance: Vec::new(),
    }
}

fn is_arithmetic_operator(operator: DiagnosticOperator) -> bool {
    matches!(
        operator,
        DiagnosticOperator::Add
            | DiagnosticOperator::Subtract
            | DiagnosticOperator::Multiply
            | DiagnosticOperator::Divide
            | DiagnosticOperator::IntDivide
            | DiagnosticOperator::Modulus
            | DiagnosticOperator::Exponent
    )
}

fn is_comparison_operator(operator: DiagnosticOperator) -> bool {
    matches!(
        operator,
        DiagnosticOperator::GreaterThan
            | DiagnosticOperator::GreaterThanOrEqual
            | DiagnosticOperator::LessThan
            | DiagnosticOperator::LessThanOrEqual
            | DiagnosticOperator::Equality
            | DiagnosticOperator::NotEqual
    )
}

fn diagnostic_fixed_scalar(
    types: &TypeDisplaySnapshot,
    type_id: TypeDisplayId,
) -> Option<FixedScalar> {
    match types.shape(type_id) {
        TypeDisplayShape::FixedScalar(scalar) => Some(scalar),
        _ => None,
    }
}

fn is_fixed_integer(scalar: FixedScalar) -> bool {
    matches!(
        scalar.class(),
        FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger
    )
}

fn implicitly_mixed_numeric_pair(
    types: &TypeDisplaySnapshot,
    lhs: TypeDisplayId,
    rhs: TypeDisplayId,
) -> bool {
    match (
        diagnostic_fixed_scalar(types, lhs),
        diagnostic_fixed_scalar(types, rhs),
    ) {
        (Some(left), Some(right)) => {
            (is_fixed_integer(left) && right.class() == FixedScalarClass::BinaryFloat)
                || (left.class() == FixedScalarClass::BinaryFloat && is_fixed_integer(right))
        }
        (Some(_), None) | (None, Some(_)) | (None, None) => false,
    }
}

fn generic_parameter_operator_message(
    operator: DiagnosticOperator,
    lhs: TypeDisplayId,
    rhs: Option<TypeDisplayId>,
    types: &TypeDisplaySnapshot,
    context: DiagnosticRenderContext<'_>,
) -> Option<String> {
    let lhs_name = generic_parameter_name(lhs, types, context);
    let rhs_name = rhs
        .filter(|rhs| *rhs != lhs)
        .and_then(|rhs| generic_parameter_name(rhs, types, context));
    let subject = match (lhs_name.as_deref(), rhs_name.as_deref()) {
        (None, None) => return None,
        (Some(name), None) | (None, Some(name)) => format!("generic parameter `{name}`"),
        (Some(lhs), Some(rhs)) => format!("generic parameters `{lhs}` and `{rhs}`"),
    };
    Some(format!(
        "Operator `{}` is not available for {subject}. Moth operators are compiler-owned and generic bounds do not provide operator support. Use a concrete type or a receiver method provided by an explicit bound.",
        operator.source_spelling()
    ))
}

fn exactly_one_operand_is_plain_string(
    lhs: TypeDisplayId,
    rhs: TypeDisplayId,
    types: &TypeDisplaySnapshot,
) -> bool {
    is_plain_string(types, lhs) != is_plain_string(types, rhs)
}

fn is_plain_string(types: &TypeDisplaySnapshot, type_id: TypeDisplayId) -> bool {
    matches!(types.shape(type_id), TypeDisplayShape::String)
}

/// The generic parameter an operand names, if its captured type snapshot records one.
fn generic_parameter_name(
    type_id: TypeDisplayId,
    types: &TypeDisplaySnapshot,
    context: DiagnosticRenderContext<'_>,
) -> Option<String> {
    types
        .is_generic_parameter(type_id)
        .then(|| diagnostic_type_display_name(types, type_id, context))
}

fn diagnostic_type_display_name(
    types: &TypeDisplaySnapshot,
    type_id: TypeDisplayId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    types.display_for_render(type_id, context.string_table, context.path_table())
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

pub(crate) fn unsupported_external_function_message(
    function_name: StringId,
    package_path: Option<StringId>,
    backend_name: StringId,
    string_table: &dyn StringTableResolver,
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

/// Renders an unsupported external function from its name, optional package and backend facts.
pub(crate) fn render_unsupported_external_function(
    _diagnostic: &DiagnosticView<'_>,
    facts: &UnsupportedExternalFunctionView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: unsupported_external_function_message(
            facts.function_name,
            facts.package_path,
            facts.backend_name,
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

fn unsupported_backend_feature_message(
    backend_name: StringId,
    reason: UnsupportedBackendFeatureReason,
    string_table: &dyn StringTableResolver,
) -> String {
    format!(
        "Backend '{}' does not support {} yet.",
        string_table.resolve(backend_name),
        reason.feature_name()
    )
}

/// Renders a backend limitation from its backend and reason facts.
pub(crate) fn render_unsupported_backend_feature(
    _diagnostic: &DiagnosticView<'_>,
    facts: &UnsupportedBackendFeatureView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: unsupported_backend_feature_message(
            facts.backend_name,
            facts.reason,
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

pub(crate) fn invalid_choice_variant_message(
    reason: InvalidChoiceVariantReason,
    choice_name: Option<StringId>,
    variant_name: Option<StringId>,
    available_variants: &[StringId],
    string_table: &dyn StringTableResolver,
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

/// WHAT: renders an invalid choice variant from its compact facts and available-name list.
/// WHY: the row preserves the reason, names, and suggestion candidates used by the legacy helper.
pub(crate) fn render_invalid_choice_variant(
    diagnostic: &DiagnosticView<'_>,
    facts: &InvalidChoiceVariantView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: invalid_choice_variant_message(
            facts.reason,
            facts.choice_name,
            facts.variant_name,
            invalid_choice_variant_available_variants(diagnostic),
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

pub(crate) fn incompatible_choice_comparison_message(
    reason: IncompatibleChoiceComparisonCode,
    lhs_name: &str,
    rhs_name: &str,
    field_name: Option<StringId>,
    field_type_name: Option<&str>,
    string_table: &dyn StringTableResolver,
) -> String {
    match reason {
        IncompatibleChoiceComparisonCode::DifferentChoiceTypes => {
            format!("Cannot compare choices of different types: '{lhs_name}' and '{rhs_name}'.")
        }
        IncompatibleChoiceComparisonCode::ChoiceWithNonChoice => {
            format!(
                "Cannot compare choice '{lhs_name}' with '{rhs_name}'. Choices can only be compared with values of the same choice type."
            )
        }
        IncompatibleChoiceComparisonCode::PayloadEqualityNotSupported => {
            let field_name = string_table
                .resolve(field_name.expect("payload-equality code stores its field name"));
            let field_type_name =
                field_type_name.expect("payload-equality code stores its field type");
            format!(
                "Choice payload equality is not supported because field '{field_name}' has type '{field_type_name}', which does not support equality."
            )
        }
    }
}

/// Renders an incompatible choice comparison from the code and type-display facts.
pub(crate) fn render_incompatible_choice_comparison(
    diagnostic: &DiagnosticView<'_>,
    facts: &IncompatibleChoiceComparisonView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let field_name =
        incompatible_choice_comparison_field_name(facts.reason, diagnostic.string_ids());
    let lhs_name = diagnostic_type_display_name(diagnostic.types(), facts.lhs, context);
    let rhs_name = diagnostic_type_display_name(diagnostic.types(), facts.rhs, context);
    let field_type_name = facts
        .field_type
        .map(|field_type| diagnostic_type_display_name(diagnostic.types(), field_type, context));
    RenderedPayload {
        message: incompatible_choice_comparison_message(
            facts.reason,
            &lhs_name,
            &rhs_name,
            field_name,
            field_type_name.as_deref(),
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

pub(crate) fn invalid_route_output_message(
    reason: InvalidRouteOutputReason,
    output_name: &str,
) -> String {
    match reason {
        InvalidRouteOutputReason::NotChoiceOrGeneric => format!(
            "`Route` output type '{output_name}' must be a choice type or a generic parameter."
        ),
    }
}

/// Renders an invalid route output from its reason and type-display facts.
pub(crate) fn render_invalid_route_output(
    diagnostic: &DiagnosticView<'_>,
    facts: &InvalidRouteOutputView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let output_name = diagnostic_type_display_name(diagnostic.types(), facts.output, context);
    RenderedPayload {
        message: invalid_route_output_message(facts.reason, &output_name),
        guidance: Vec::new(),
    }
}

pub(crate) fn deferred_feature_message(
    reason: &DeferredFeatureReason,
    string_table: &dyn StringTableResolver,
) -> String {
    if let DeferredFeatureReason::NamedFeature { feature } = reason {
        return format!("Deferred feature: {}.", string_table.resolve(*feature));
    }

    deferred_feature_static_message(reason).to_owned()
}

pub(crate) fn render_deferred_feature(
    _diagnostic: &DiagnosticView<'_>,
    facts: &DeferredFeatureView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let reason = facts
        .code
        .to_reason(facts.feature)
        .expect("a deferred-feature code stores its required data");
    RenderedPayload {
        message: deferred_feature_message(&reason, context.string_table),
        guidance: Vec::new(),
    }
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
    found_type_name: &str,
    reason: InvalidRangeOperandReason,
) -> String {
    let operand_name = match operand {
        RangeOperandKind::Start => "start",
        RangeOperandKind::End => "end",
        RangeOperandKind::Step => "step",
    };
    match reason {
        InvalidRangeOperandReason::NotNumeric => {
            format!(
                "Range {operand_name} must be an integer or binary-float type. Found '{found_type_name}'"
            )
        }
        InvalidRangeOperandReason::IncompatibleDomain => format!(
            "Range {operand_name} type '{found_type_name}' has no shared numeric domain with the other range operands. Integer and binary-float operands do not mix, and integer operands need one type that holds every operand's range. Convert the operand explicitly with `cast` at a typed declaration."
        ),
    }
}

/// WHAT: renders a range operand diagnostic from its compact facts.
/// WHY: the type snapshot supplies the same source-facing type name after compaction.
pub(crate) fn render_invalid_range_operand(
    diagnostic: &DiagnosticView<'_>,
    view: &InvalidRangeOperandView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let found_type_name = diagnostic.types().display_for_render(
        view.found_type,
        context.string_table,
        context.path_table(),
    );
    RenderedPayload {
        message: invalid_range_operand_message(view.operand, &found_type_name, view.reason),
        guidance: Vec::new(),
    }
}

pub(crate) fn unsupported_builder_package_message(
    package_path: StringId,
    string_table: &dyn StringTableResolver,
) -> String {
    let package_path_str = string_table.resolve(package_path);
    format!(
        "Core package '{package_path_str}' is not supported by this builder. \
         Use a builder that exposes this core package or remove the dependency clause."
    )
}
/// WHAT: renders an unsupported builder package from its compact path fact.
/// WHY: the package path is the message's only variable data.
pub(crate) fn render_unsupported_builder_package(
    _diagnostic: &DiagnosticView<'_>,
    facts: &UnsupportedBuilderPackageView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: unsupported_builder_package_message(facts.package_path, context.string_table),
        guidance: Vec::new(),
    }
}

pub(crate) fn invalid_page_metadata_message(
    key: StringId,
    reason: InvalidPageMetadataReason,
    string_table: &dyn StringTableResolver,
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
/// WHAT: renders invalid page metadata from its key and reason facts.
/// WHY: the legacy prose helper owns both substitutions.
pub(crate) fn render_invalid_page_metadata(
    _diagnostic: &DiagnosticView<'_>,
    facts: &InvalidPageMetadataView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: invalid_page_metadata_message(facts.key, facts.reason, context.string_table),
        guidance: Vec::new(),
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
        InvalidExpressionReason::EmptyRuntimeAnonymousRecord => {
            "Empty anonymous records are outside the initial runtime surface.".to_owned()
        }
        InvalidExpressionReason::AnonymousRecordFieldNotNamed => {
            "Each const-record parameter needs a value. Write `name = value`.".to_owned()
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
    path: PathId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    // Support-root markers (`+`) are unambiguous even when an earlier folder shares a name.
    let path_text = context.render_path(path);
    for segment in path_text.split('/') {
        if dependency_component_is_support_root_file(segment) {
            let suffix = if segment.contains('.') { "" } else { ".moth" };
            return format!("{segment}{suffix}");
        }
    }

    for segment in path_text.split('/') {
        if dependency_component_is_config_file(segment) {
            return "config.moth".to_owned();
        }
    }
    "special file".to_owned()
}

fn named_value_or_default(
    name: Option<StringId>,
    string_table: &dyn StringTableResolver,
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
    path: PathId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let path_text = context.render_path(path);
    let mut suggestion_path: Vec<&str> = Vec::new();
    let mut found_root_component = false;

    for segment in path_text.split('/') {
        if dependency_component_is_support_root_file(segment) {
            found_root_component = true;
            continue;
        }
        suggestion_path.push(segment);
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
