//! Family render contracts through the public terse entry.
//!
//! WHAT: pins how a severity override on a family draft reaches the rendered record.
//! WHY: the compact path reads severity from the stored record, so an override must win over the
//! family default while the rest of the line stays unchanged.

use crate::compiler_frontend::compiler_errors::CompilationOutcome;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    self, InvalidCallShapeView, InvalidReceiverCallView, InvalidValueReceiverView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    self, InvalidChoiceVariantView, InvalidFallibleHandlingView, UnreachableMatchArmView,
    unreachable_match_arm,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    self, InvalidAssignmentTargetView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    self, InvalidExpressionView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    self, UnsupportedOperatorTypesView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    DurableLabelMessage, LabelDraftMessage,
};
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    compact_bag_for_test, exact_span_in_source, finish_prepared_batch,
};
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostic_view;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, dev_server, terminal, terse,
};
use crate::compiler_frontend::compiler_messages::{
    BuiltinFailureOriginKind, BuiltinFailureWitness, CompileTimeEvaluationErrorReason,
    DeferredFeatureReason, DependencyClauseKind, DiagnosticBag,
    DiagnosticCompoundAssignmentOperator, DiagnosticOperator, DiagnosticSeverity,
    GenericApplicationErrorReason, InvalidAssignmentTargetReason, InvalidBuiltinCallReason,
    InvalidCallShapeReason, InvalidCastReason, InvalidChoiceVariantReason,
    InvalidCollectionTypeReason, InvalidConfigReason, InvalidDependencyClauseReason,
    InvalidExpressionReason, InvalidFallibleHandlingReason, InvalidFunctionSignatureReason,
    InvalidGenericParameterReason, InvalidImportPathReason, InvalidReceiverCallReason,
    InvalidSignatureMemberReason, InvalidStandaloneStatementReason, InvalidStatementPositionReason,
    InvalidStringEscapeReason, InvalidTemplateDirectiveReason, InvalidTemplateStructureReason,
    InvalidTraitKeywordUsageReason, InvalidTypeAnnotationReason, NameNamespace,
    NamespaceTypeValueMisuseKind, PathKind, ReceiverCallKind, TypeAnnotationContext,
    TypeMismatchContext, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::display::{TypeDisplayShape, TypeDisplayView};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, builtin_type_ids};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, FrozenIdentityHandle, SourceDatabase, SourceId,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;
use std::path::Path;
use std::sync::Arc;

#[test]
fn severity_override_on_a_family_changes_only_the_severity_letter() {
    let string_table = StringTable::new();
    let family_default = unreachable_match_arm(None);
    let escalated = unreachable_match_arm(None).with_severity(DiagnosticSeverity::Error);

    // Both lines must come from the compact path, or the comparison would not test a stored severity.
    let family_default_compact = compact_bag_for_test(DiagnosticBag::from(family_default), None);
    let escalated_compact = compact_bag_for_test(DiagnosticBag::from(escalated), None);
    let family_default_view = family_default_compact
        .first()
        .expect("the family default must take the compact path");
    let escalated_view = escalated_compact
        .first()
        .expect("the escalated diagnostic must take the compact path");
    assert!(
        family_default_view
            .family_view::<UnreachableMatchArmView>()
            .is_some(),
        "the family default must take the compact path"
    );
    assert!(
        escalated_view
            .family_view::<UnreachableMatchArmView>()
            .is_some(),
        "the escalated diagnostic must take the compact path"
    );
    assert_eq!(family_default_view.severity(), DiagnosticSeverity::Warning);
    assert_eq!(escalated_view.severity(), DiagnosticSeverity::Error);

    let default_line = format_terse_diagnostic_view(
        family_default_view,
        DiagnosticRenderContext::new(&string_table),
    );
    let escalated_line =
        format_terse_diagnostic_view(escalated_view, DiagnosticRenderContext::new(&string_table));

    assert!(
        default_line.starts_with("W|"),
        "the family default is a warning: {default_line}"
    );
    assert!(
        escalated_line.starts_with("E|"),
        "the override must win over the family default: {escalated_line}"
    );
    assert_eq!(
        &default_line[1..],
        &escalated_line[1..],
        "an override may change only the severity letter"
    );
}

fn assert_rendered_diagnostic_hides_internal_names(rendered: &str) {
    for internal_name in [
        "StringId(",
        "TypeId(",
        "DiagnosticPlace",
        "BorrowAccessKind",
        "InvalidConfigReason",
        "DeferredFeatureReason",
        "WhitespaceMustBeQuoted",
        "ImmutableBinding",
    ] {
        assert!(
            !rendered.contains(internal_name),
            "renderer leaked internal name '{internal_name}' in: {rendered}",
        );
    }
}

fn render_invalid_call_shape(
    string_table: &mut StringTable,
    reason: InvalidCallShapeReason,
    callee_name: &str,
) -> String {
    let callee = string_table.intern(callee_name);
    let compact = compact_bag_for_test(
        DiagnosticBag::from(calls_data::invalid_call_shape(reason, Some(callee), None)),
        None,
    );
    let view = compact.first().expect("invalid-call-shape draft compacts");
    assert!(view.family_view::<InvalidCallShapeView>().is_some());
    terminal::format_guidance_view(view, DiagnosticRenderContext::new(string_table))
        .into_iter()
        .next()
        .unwrap_or_default()
}

fn render_invalid_receiver_call(
    string_table: &mut StringTable,
    reason: InvalidReceiverCallReason,
    method_name: &str,
    receiver_kind: Option<ReceiverCallKind>,
    receiver_binding_name: Option<&str>,
) -> String {
    let method = string_table.intern(method_name);
    let binding = receiver_binding_name.map(|name| string_table.intern(name));
    let compact = compact_bag_for_test(
        DiagnosticBag::from(calls_data::invalid_receiver_call(
            reason,
            None,
            Some(method),
            receiver_kind,
            binding,
            None,
        )),
        None,
    );
    let view = compact
        .first()
        .expect("invalid-receiver-call draft compacts");
    assert!(view.family_view::<InvalidReceiverCallView>().is_some());
    terminal::format_guidance_view(view, DiagnosticRenderContext::new(string_table))
        .into_iter()
        .next()
        .unwrap_or_default()
}

#[test]
fn value_receiver_diagnostic_preserves_factual_arity_and_producer_span() {
    let mut spans = ExtendedSpanBuilder::new();
    let producer_span = exact_span_in_source(SourceId::from_index(1), 12, 6, &mut spans);
    let mut merged_table = StringTable::new();
    merged_table.intern("unrelated");
    let empty_table = StringTable::new();
    merged_table.merge_from(&empty_table);
    for produced_count in [0, 2] {
        let compact = compact_bag_for_test(
            DiagnosticBag::from(calls_data::invalid_value_receiver(
                ValueReceiverContext::Declaration,
                1,
                produced_count,
                Some(producer_span),
            )),
            None,
        );
        let view = compact.first().expect("value receiver draft compacts");
        let facts = view
            .family_view::<InvalidValueReceiverView>()
            .expect("value receiver facts");
        assert_eq!(facts.receiver, ValueReceiverContext::Declaration);
        assert_eq!(facts.target_count, 1);
        assert_eq!(facts.produced_count, produced_count);
        assert_eq!(view.descriptor().code, "MOTH-RULE-0087");
        assert_eq!(view.reason_key(), None);
        assert!(matches!(view.primary_site(), SiteRef::Local(span) if span == producer_span));
        let context = DiagnosticRenderContext::new(&merged_table);
        let rendered = terse::format_terse_diagnostic_view(view, context);
        assert!(rendered.contains(&format!("Invalid declaration receiver: expected 1 success slot(s), but this expression produces {produced_count}.")));
        let guidance = terminal::format_guidance_view(view, context).join("\n");
        if produced_count == 0 {
            assert!(
                guidance.contains("Use an expression that produces the required success values.")
            );
        } else {
            assert!(guidance.contains("Use a multi-bind with one target per success slot."));
        }
    }
}

#[test]
fn invalid_string_escape_renderer_preserves_the_authored_escape_spelling() {
    let string_table = StringTable::new();
    let compact = compact_bag_for_test(
        DiagnosticBag::from_diagnostics(vec![
            token_lexical::invalid_string_escape(
                InvalidStringEscapeReason::UnsupportedEscape { escaped: 'q' },
                None,
            ),
            token_lexical::invalid_string_escape(
                InvalidStringEscapeReason::UnsupportedEscape { escaped: '\t' },
                None,
            ),
        ]),
        None,
    );
    let context = DiagnosticRenderContext::new(&string_table);
    let messages = compact
        .views()
        .map(|view| terminal::format_guidance_view(view, context).join("\n"))
        .collect::<Vec<_>>();
    assert!(
        messages[0].contains("Unsupported string escape '\\q'."),
        "{}",
        messages[0]
    );
    assert!(
        messages[1].contains("Unsupported string escape '\\\\t'."),
        "{}",
        messages[1]
    );
}

#[test]
fn invalid_string_escape_renderer_distinguishes_physical_newlines_and_trailing_backslashes() {
    let string_table = StringTable::new();
    let compact = compact_bag_for_test(
        DiagnosticBag::from_diagnostics(vec![
            token_lexical::invalid_string_escape(InvalidStringEscapeReason::PhysicalNewline, None),
            token_lexical::invalid_string_escape(
                InvalidStringEscapeReason::TrailingBackslash,
                None,
            ),
        ]),
        None,
    );
    let context = DiagnosticRenderContext::new(&string_table);
    let messages = compact
        .views()
        .map(|view| terminal::format_guidance_view(view, context).join("\n"))
        .collect::<Vec<_>>();
    assert_eq!(
        messages[0],
        "A backslash cannot continue a quoted string across a physical newline. Remove the backslash or use the two-character '\\n' escape."
    );
    assert_eq!(
        messages[1],
        "The string ends with a backslash. Add a supported escaped character or remove the backslash."
    );
}

#[test]
fn assignment_guidance_preserves_target_operator_and_binding_problem() {
    let mut string_table = StringTable::new();
    let target = string_table.intern("point.x");
    let cases = [
        (None, "Updating `point.x` requires `~point.x = ...`."),
        (
            Some(DiagnosticCompoundAssignmentOperator::Add),
            "Updating `point.x` requires `~point.x += ...`.",
        ),
        (
            Some(DiagnosticCompoundAssignmentOperator::Subtract),
            "Updating `point.x` requires `~point.x -= ...`.",
        ),
        (
            Some(DiagnosticCompoundAssignmentOperator::Multiply),
            "Updating `point.x` requires `~point.x *= ...`.",
        ),
        (
            Some(DiagnosticCompoundAssignmentOperator::Divide),
            "Updating `point.x` requires `~point.x /= ...`.",
        ),
        (
            Some(DiagnosticCompoundAssignmentOperator::IntDivide),
            "Updating `point.x` requires `~point.x //= ...`.",
        ),
        (
            Some(DiagnosticCompoundAssignmentOperator::Modulus),
            "Updating `point.x` requires `~point.x %= ...`.",
        ),
        (
            Some(DiagnosticCompoundAssignmentOperator::Exponent),
            "Updating `point.x` requires `~point.x ^= ...`.",
        ),
    ];
    let mut reasons = cases
        .into_iter()
        .map(|(operator, message)| {
            (
                InvalidAssignmentTargetReason::MissingWritePrefix { operator },
                Some(target),
                message,
            )
        })
        .collect::<Vec<_>>();
    reasons.push((
        InvalidAssignmentTargetReason::ImmutableBinding,
        Some(target),
        "`point.x` is immutable. Declare it mutable to update it.",
    ));
    reasons.push((
        InvalidAssignmentTargetReason::MutableMarkerRequiresExistingPlace,
        None,
        "`~` must prefix an existing writable place.",
    ));
    let context = DiagnosticRenderContext::new(&string_table);
    for (reason, target, message) in reasons {
        let compact = compact_bag_for_test(
            DiagnosticBag::from(names_declarations::invalid_assignment_target(
                reason, target, None, None, None,
            )),
            None,
        );
        let view = compact
            .first()
            .expect("assignment diagnostic survives compaction");
        let facts = view
            .family_view::<InvalidAssignmentTargetView>()
            .expect("assignment facts");
        assert_eq!(facts.reason, reason);
        assert_eq!(facts.target_name, target);
        assert_eq!(view.descriptor().code, "MOTH-RULE-0044");
        assert_eq!(
            terminal::format_guidance_view(view, context),
            vec![message.to_owned()]
        );
        assert_eq!(
            terse::format_terse_diagnostic_view(view, context),
            format!("E|MOTH-RULE-0044|||{message}")
        );
    }
}

#[test]
fn syntax_and_choice_renderers_use_user_facing_messages_not_reason_debug_names() {
    let mut string_table = StringTable::new();
    let choice_name = string_table.intern("Status");
    let variant_name = string_table.intern("Ready");
    let cases = [
        (
            token_lexical::invalid_path(PathKind::WhitespaceMustBeQuoted, None),
            "Path components with whitespace must be quoted",
            "WhitespaceMustBeQuoted",
        ),
        (
            project_config::invalid_dependency_clause(
                DependencyClauseKind::NamespaceAlias,
                InvalidDependencyClauseReason::ExpectedAliasName,
                None,
            ),
            "Expected alias name after `as`",
            "ExpectedAliasName",
        ),
        (
            choices_control_flow::invalid_choice_variant(
                InvalidChoiceVariantReason::UnitVariantWithParentheses,
                Some(choice_name),
                Some(variant_name),
                Vec::new(),
                None,
            ),
            "Unit variant 'Status::Ready' cannot be called with empty parentheses",
            "UnitVariantWithParentheses",
        ),
    ];
    for (draft, expected_message, debug_name) in cases {
        let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
        let view = compact.first().expect("renderer draft compacts");
        let context = DiagnosticRenderContext::new(&string_table);
        let guidance = terminal::format_guidance_view(view, context);
        let terse_line = terse::format_terse_diagnostic_view(view, context);
        assert!(
            guidance.iter().any(|line| line.contains(expected_message)),
            "expected '{expected_message}', got {guidance:?}"
        );
        assert!(
            terse_line.contains(expected_message),
            "expected '{expected_message}', got {terse_line}"
        );
        assert!(
            !guidance.iter().any(|line| line.contains(debug_name)),
            "debug name leaked: {guidance:?}"
        );
        assert!(
            !terse_line.contains(debug_name),
            "debug name leaked: {terse_line}"
        );
    }
}

#[test]
fn choice_variant_unknown_variant_suggests_close_candidate() {
    let mut string_table = StringTable::new();
    let choice_name = string_table.intern("Status");
    let misspelled = string_table.intern("Reay");
    let ready = string_table.intern("Ready");
    let error = string_table.intern("Error");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(choices_control_flow::invalid_choice_variant(
            InvalidChoiceVariantReason::UnknownVariant,
            Some(choice_name),
            Some(misspelled),
            vec![ready, error],
            None,
        )),
        None,
    );
    let view = compact.first().expect("choice diagnostic compacts");
    assert!(view.family_view::<InvalidChoiceVariantView>().is_some());
    let guidance =
        terminal::format_guidance_view(view, DiagnosticRenderContext::new(&string_table));
    let message = guidance
        .iter()
        .find(|line| line.contains("Unknown variant"))
        .expect("unknown-variant message");
    assert!(message.contains("Status::Reay"));
    assert!(message.contains("Did you mean 'Ready'?"));
    assert!(message.contains("Available variants: [Ready, Error]"));
}

#[test]
fn choice_variant_unknown_variant_no_suggestion_for_unrelated_name() {
    let mut string_table = StringTable::new();
    let choice_name = string_table.intern("Status");
    let unrelated = string_table.intern("Xyzzy");
    let ready = string_table.intern("Ready");
    let error = string_table.intern("Error");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(choices_control_flow::invalid_choice_variant(
            InvalidChoiceVariantReason::UnknownVariant,
            Some(choice_name),
            Some(unrelated),
            vec![ready, error],
            None,
        )),
        None,
    );
    let view = compact.first().expect("choice diagnostic compacts");
    let guidance =
        terminal::format_guidance_view(view, DiagnosticRenderContext::new(&string_table));
    let message = guidance
        .iter()
        .find(|line| line.contains("Unknown variant"))
        .expect("unknown-variant message");
    assert!(message.contains("Status::Xyzzy"));
    assert!(!message.contains("Did you mean"));
    assert!(message.contains("Available variants: [Ready, Error]"));
}

#[test]
fn syntax_renderers_keep_typed_prose_without_error_conversion() {
    let mut string_table = StringTable::new();
    let path_table = PathInternerFork::empty().snapshot_table();
    let literal = string_table.intern("1.");
    let fixed_unsigned_literal = string_table.intern("300");
    let fixed_negative_literal = string_table.intern("-1");
    let fixed_signed_literal = string_table.intern("-129");
    let style_directive = string_table.intern("unknown");
    let supported_directives = string_table.intern("'$html', '$css'");
    let declaration_name = string_table.intern("Card");
    let template_directive = string_table.intern("insert");
    let namespace_name = string_table.intern("card");
    let cases = [
        (
            names_declarations::duplicate_declaration(declaration_name, None),
            "Cannot declare 'Card' because that name is already visible in this scope",
            "StringId",
        ),
        (
            token_lexical::invalid_number_literal(
                literal,
                NumberLiteralErrorReason::MultipleDecimalPoints,
                None,
            ),
            "Can't have more than one decimal point in numeric literal '1.'",
            "MultipleDecimalPoints",
        ),
        (
            token_lexical::invalid_number_literal(
                fixed_unsigned_literal,
                NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U8),
                None,
            ),
            "Integer literal '300' is outside the U8 range 0 to 255.",
            "OutsideFixedScalarRange",
        ),
        (
            token_lexical::invalid_number_literal(
                fixed_signed_literal,
                NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I8),
                None,
            ),
            "Integer literal '-129' is outside the I8 range -128 to 127.",
            "OutsideFixedScalarRange",
        ),
        (
            token_lexical::invalid_number_literal(
                fixed_negative_literal,
                NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::Byte),
                None,
            ),
            "Numeric literal '-1' is negative, but Byte cannot hold negative values.",
            "NegativeUnsignedLiteral",
        ),
        (
            templates::invalid_style_directive(style_directive, supported_directives, None),
            "Style directive '$unknown' is unsupported here",
            "InvalidStyleDirective",
        ),
        (
            names_declarations::invalid_type_annotation(
                TypeAnnotationContext::DeclarationTarget,
                InvalidTypeAnnotationReason::UnexpectedColon,
                None,
            ),
            "Unexpected ':' after declaration name",
            "UnexpectedColon",
        ),
        (
            names_declarations::invalid_signature_member(
                InvalidSignatureMemberReason::ChoicePayloadDefaultValue,
                None,
            ),
            "Choice payload fields cannot have default values.",
            "ChoicePayloadDefaultValue",
        ),
        (
            names_declarations::invalid_signature_member(
                InvalidSignatureMemberReason::MissingDefaultValue,
                None,
            ),
            "Expected a default value after '='.",
            "MissingDefaultValue",
        ),
        (
            generics_traits_api::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingColonAfterReturns,
                None,
            ),
            "Function return declarations must end with ':'",
            "MissingColonAfterReturns",
        ),
        (
            generics_traits_api::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingReturnType,
                None,
            ),
            "Function signature is missing a return type after '->'. Add a type followed by ':', or remove '->' for a no-value function.",
            "MissingReturnType",
        ),
        (
            generics_traits_api::invalid_function_signature(
                InvalidFunctionSignatureReason::MissingTraitRequirementReturnType,
                None,
            ),
            "Trait requirement is missing a return type after '->'. Add a type, or remove '->' for a no-value requirement.",
            "MissingTraitRequirementReturnType",
        ),
        (
            generics_traits_api::invalid_generic_application(
                GenericApplicationErrorReason::NestedApplication,
                None,
            ),
            "Nested generic type applications are not supported",
            "NestedApplication",
        ),
        (
            calls_data::invalid_collection_type(
                InvalidCollectionTypeReason::NegativeCapacity,
                None,
            ),
            "Fixed collection capacity must be greater than zero.",
            "NegativeCapacity",
        ),
        (
            calls_data::invalid_collection_type(InvalidCollectionTypeReason::ZeroCapacity, None),
            "Fixed collection capacity must be greater than zero.",
            "ZeroCapacity",
        ),
        (
            calls_data::invalid_collection_type(InvalidCollectionTypeReason::CapacityNotInt, None),
            "Collection capacity must be an integer.",
            "CapacityNotInt",
        ),
        (
            calls_data::invalid_collection_type(
                InvalidCollectionTypeReason::CapacityOverflow,
                None,
            ),
            "Collection capacity is too large.",
            "CapacityOverflow",
        ),
        (
            calls_data::invalid_collection_type(
                InvalidCollectionTypeReason::InitializerExceedsFixedCapacity {
                    capacity: 2,
                    length: 3,
                },
                None,
            ),
            "Collection literal has more items than the fixed collection capacity allows.",
            "InitializerExceedsFixedCapacity",
        ),
        (
            calls_data::invalid_collection_type(
                InvalidCollectionTypeReason::EmptyImmutableFixedCollection,
                None,
            ),
            "Immutable binding initialized with an empty fixed collection literal is not allowed.",
            "EmptyImmutableFixedCollection",
        ),
        (
            calls_data::invalid_collection_type(
                InvalidCollectionTypeReason::ShorthandEmptyLiteralAmbiguous,
                None,
            ),
            "Capacity-only shorthand requires a non-empty collection literal so the element type can be inferred.",
            "ShorthandEmptyLiteralAmbiguous",
        ),
        (
            calls_data::invalid_collection_type(
                InvalidCollectionTypeReason::ShorthandNonLiteralRhs,
                None,
            ),
            "Capacity-only shorthand requires a collection literal initializer.",
            "ShorthandNonLiteralRhs",
        ),
        (
            generics_traits_api::invalid_generic_parameter(
                InvalidGenericParameterReason::BoundsMustUseIs,
                None,
            ),
            "Generic parameter bounds use `is`.",
            "BoundsMustUseIs",
        ),
        (
            generics_traits_api::invalid_trait_keyword_usage(
                InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
                None,
            ),
            "Keyword 'must' is trait-only syntax",
            "MustOutsideTraitSyntax",
        ),
        (
            templates::invalid_template_directive(
                Some(template_directive),
                InvalidTemplateDirectiveReason::MissingArgument,
                None,
            ),
            "Template directive 'insert' is missing a required argument.",
            "MissingArgument",
        ),
        (
            templates::invalid_template_directive(
                Some(template_directive),
                InvalidTemplateDirectiveReason::invalid_argument_with_detail(
                    string_table.get_or_intern(
                        "Unsupported language \"rustt\". Supported aliases are \"rs\"/\"rust\"."
                            .to_owned(),
                    ),
                ),
                None,
            ),
            "Invalid argument for template directive 'insert'. Unsupported language \"rustt\". Supported aliases are \"rs\"/\"rust\".",
            "InvalidArgument",
        ),
        (
            names_declarations::namespace_misuse(
                namespace_name,
                NameNamespace::Type,
                NameNamespace::Value,
                None,
            ),
            "'card' is a value and cannot be used as a type.",
            "NamespaceMisuse",
        ),
    ];
    for (draft, expected_message, debug_name) in cases {
        let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
        let view = compact.first().expect("syntax draft compacts");
        let context = DiagnosticRenderContext::new(&string_table).with_path_table(&path_table);
        let guidance = terminal::format_guidance_view(view, context);
        let terse_line = terse::format_terse_diagnostic_view(view, context);
        assert!(
            guidance.iter().any(|line| line.contains(expected_message)),
            "Expected '{expected_message}', got {guidance:?}"
        );
        assert!(
            terse_line.contains(expected_message),
            "Expected '{expected_message}', got {terse_line}"
        );
        assert!(
            !guidance.iter().any(|line| line.contains(debug_name)),
            "Guidance exposed '{debug_name}': {guidance:?}"
        );
        assert!(
            !terse_line.contains(debug_name),
            "Terse output exposed '{debug_name}': {terse_line}"
        );
    }

    let type_environment = TypeEnvironment::new();
    let compact = compact_bag_for_test(
        DiagnosticBag::from(types_borrows_targets::unsupported_operator_types(
            DiagnosticOperator::Add,
            type_environment.builtins().string,
            Some(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
            None,
        )),
        Some(&type_environment),
    );
    let view = compact.first().expect("operator diagnostic compacts");
    assert!(view.family_view::<UnsupportedOperatorTypesView>().is_some());
    let context = DiagnosticRenderContext::new(&string_table).with_path_table(&path_table);
    let guidance = terminal::format_guidance_view(view, context);
    let terse_line = terse::format_terse_diagnostic_view(view, context);
    assert!(
        guidance
            .iter()
            .any(|line| line.contains("Operator `+` cannot concatenate")),
        "{guidance:?}"
    );
    assert!(
        terse_line.contains("Operator `+` cannot concatenate"),
        "{terse_line}"
    );
    assert!(
        !guidance.iter().any(|line| line.contains("Add")),
        "{guidance:?}"
    );
    assert!(!terse_line.contains("Add"), "{terse_line}");
}

#[test]
fn invalid_match_guard_uses_its_typed_reason() {
    use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
        InvalidMatchGuardView, invalid_match_guard,
    };
    use crate::compiler_frontend::compiler_messages::InvalidMatchGuardReason;

    let string_table = StringTable::new();
    let diagnostic = invalid_match_guard(InvalidMatchGuardReason::SideEffectingPrelude, None);
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let view = compact.first().expect("match guard diagnostic compacts");
    assert!(view.family_view::<InvalidMatchGuardView>().is_some());
    assert_eq!(
        view.reason_key(),
        Some("invalid_match_guard.side_effecting_prelude")
    );
    let context = DiagnosticRenderContext::new(&string_table);
    assert_eq!(
        terminal::format_guidance_view(view, context),
        vec!["Match guards must be pure Bool expressions and cannot lower to side-effect statements.".to_owned()]
    );
    assert!(
        terse::format_terse_diagnostic_view(view, context)
            .contains("Match guards must be pure Bool expressions")
    );
}
#[test]
fn invalid_expression_renderers_keep_structured_reason_prose() {
    let string_table = StringTable::new();
    let cases = [
        (
            InvalidExpressionReason::ExpectedOperatorBeforeExpression,
            "Expected an operator before this expression.",
            "ExpectedOperatorBeforeExpression",
        ),
        (
            InvalidExpressionReason::UnresolvedStackShape,
            "This expression does not resolve to exactly one value.",
            "UnresolvedStackShape",
        ),
        (
            InvalidExpressionReason::MothFileHasNoValue,
            "A `.moth` file has no file value. Bind its declarations through a dependency clause.",
            "MothFileHasNoValue",
        ),
        (
            InvalidExpressionReason::ExtensionlessFileValue,
            "A file value needs an explicit extension. Write the path with a file extension, or use a dependency clause to bind declarations.",
            "ExtensionlessFileValue",
        ),
        (
            InvalidExpressionReason::EmptyRuntimeAnonymousRecord,
            "Empty anonymous records are outside the initial runtime surface.",
            "EmptyRuntimeAnonymousRecord",
        ),
        (
            InvalidExpressionReason::AnonymousRecordFieldNotNamed,
            "Each const-record parameter needs a value. Write `name = value`.",
            "AnonymousRecordFieldNotNamed",
        ),
    ];
    for (reason, expected_message, debug_name) in cases {
        let compact = compact_bag_for_test(
            DiagnosticBag::from(token_lexical::invalid_expression(reason, None)),
            None,
        );
        let view = compact.first().expect("expression diagnostic compacts");
        assert!(view.family_view::<InvalidExpressionView>().is_some());
        let context = DiagnosticRenderContext::new(&string_table);
        let guidance = terminal::format_guidance_view(view, context);
        let terse_line = terse::format_terse_diagnostic_view(view, context);
        assert!(
            guidance.iter().any(|line| line.contains(expected_message)),
            "Expected '{expected_message}', got {guidance:?}"
        );
        assert!(
            terse_line.contains(expected_message),
            "Expected '{expected_message}', got {terse_line}"
        );
        assert!(
            !guidance.iter().any(|line| line.contains(debug_name)),
            "Guidance exposed '{debug_name}': {guidance:?}"
        );
        assert!(
            !terse_line.contains(debug_name),
            "Terse output exposed '{debug_name}': {terse_line}"
        );
    }
}

#[test]
fn failure_handling_diagnostics_preserve_producer_types_and_spans() {
    let type_environment = TypeEnvironment::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(1);
    let catch_span = exact_span_in_source(source, 0, 30, &mut span_builder);
    let first_span = exact_span_in_source(source, 0, 8, &mut span_builder);
    let second_span = exact_span_in_source(source, 11, 9, &mut span_builder);
    let cases = [
        (
            choices_control_flow::invalid_fallible_handling(
                InvalidFallibleHandlingReason::FunctionHasMultipleSuccessReturns { count: 3 },
                Some(catch_span),
            ),
            "invalid_fallible_handling.function_has_multiple_success_returns",
            vec![],
        ),
        (
            choices_control_flow::invalid_fallible_handling(
                InvalidFallibleHandlingReason::EntryHasIncompatibleError {
                    produced_error_type: builtin_type_ids::STRING,
                    entry_error_type: builtin_type_ids::BOOL,
                },
                Some(catch_span),
            ),
            "invalid_fallible_handling.entry_has_incompatible_error",
            vec![],
        ),
        (
            choices_control_flow::invalid_fallible_handling(
                InvalidFallibleHandlingReason::IncompatibleCatchErrorTypes {
                    first_error_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    second_error_type_id: builtin_type_ids::STRING,
                    first_producer_span: Some(first_span),
                    second_producer_span: Some(second_span),
                },
                Some(catch_span),
            )
            .with_label(
                Some(first_span),
                Some(LabelDraftMessage::TypedFailureProducer {
                    error_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                }),
            )
            .with_label(
                Some(second_span),
                Some(LabelDraftMessage::TypedFailureProducer {
                    error_type_id: builtin_type_ids::STRING,
                }),
            ),
            "invalid_fallible_handling.incompatible_catch_error_types",
            vec![first_span, second_span],
        ),
        (
            choices_control_flow::invalid_fallible_handling(
                InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                    error_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    typed_producer_span: Some(first_span),
                    implicit_producer_span: Some(second_span),
                },
                Some(catch_span),
            )
            .with_label(
                Some(first_span),
                Some(LabelDraftMessage::TypedFailureProducer {
                    error_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                }),
            )
            .with_label(
                Some(second_span),
                Some(LabelDraftMessage::ImplicitFailureProducer),
            ),
            "invalid_fallible_handling.custom_error_mixed_with_implicit_failure",
            vec![first_span, second_span],
        ),
        (
            choices_control_flow::invalid_fallible_handling(
                InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                    error_type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    witness: Box::new(BuiltinFailureWitness {
                        call_spans: vec![Some(first_span), None],
                        origin_span: Some(second_span),
                        origin: BuiltinFailureOriginKind::Operation,
                        elided_call_hops: 2,
                    }),
                },
                Some(catch_span),
            )
            .with_label(
                Some(first_span),
                Some(LabelDraftMessage::BuiltinFailureCall),
            )
            .with_label(
                Some(second_span),
                Some(LabelDraftMessage::BuiltinFailureOrigin),
            ),
            "invalid_fallible_handling.unhandled_builtin_failure_in_custom_error_function",
            vec![first_span, second_span],
        ),
        (
            choices_control_flow::invalid_fallible_handling(
                InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction {
                    witness: Box::new(BuiltinFailureWitness {
                        call_spans: vec![Some(first_span)],
                        origin_span: Some(second_span),
                        origin: BuiltinFailureOriginKind::Operation,
                        elided_call_hops: 0,
                    }),
                },
                Some(catch_span),
            )
            .with_label(
                Some(first_span),
                Some(LabelDraftMessage::BuiltinFailureCall),
            )
            .with_label(
                Some(second_span),
                Some(LabelDraftMessage::BuiltinFailureOrigin),
            ),
            "invalid_fallible_handling.unhandled_builtin_failure_in_exported_function",
            vec![first_span, second_span],
        ),
    ];
    for (draft, reason_key, expected_spans) in cases {
        let compact = compact_bag_for_test(DiagnosticBag::from(draft), Some(&type_environment));
        let view = compact
            .first()
            .expect("failure-handling diagnostic compacts");
        let _facts = view
            .family_view::<InvalidFallibleHandlingView>()
            .expect("failure-handling facts");
        assert!(matches!(view.primary_site(), SiteRef::Local(span) if span == catch_span));
        assert_eq!(view.reason_key(), Some(reason_key));
        let actual_spans = view
            .durable_labels()
            .map(|label| match label.site {
                SiteRef::Local(span) => span,
                other => panic!("expected local producer span, got {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(actual_spans, expected_spans);
    }
}

#[test]
fn propagation_boundary_renderers_preserve_authored_count_and_semantic_error_types() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let error_path = path_fork
        .try_intern_portable_path("ParseFailure", &mut string_table)
        .expect("type path fits");
    let (_, produced_error_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: error_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let entry_error_path = path_fork
        .try_intern_portable_path("Error", &mut string_table)
        .expect("entry type path fits");
    let (_, entry_error_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: entry_error_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let path_table = path_fork.snapshot_table();
    for count in [2, 5] {
        let compact = compact_bag_for_test(
            DiagnosticBag::from(choices_control_flow::invalid_fallible_handling(
                InvalidFallibleHandlingReason::FunctionHasMultipleSuccessReturns { count },
                None,
            )),
            Some(&type_environment),
        );
        let view = compact
            .first()
            .expect("multiple-return diagnostic compacts");
        let facts = view
            .family_view::<InvalidFallibleHandlingView>()
            .expect("fallible facts");
        assert_eq!(facts.count, Some(count));
        let context = DiagnosticRenderContext::new(&string_table).with_path_table(&path_table);
        let guidance = terminal::format_guidance_view(view, context).join("\n");
        let terse_line = terse::format_terse_diagnostic_view(view, context);
        let count_fact = format!("{count} authored success return slots");
        for message in [&guidance, &terse_line] {
            assert!(message.contains(&count_fact), "{message}");
            assert!(message.contains("Inspect the option locally"), "{message}");
            assert!(
                message.contains("use a single optional success return slot"),
                "{message}"
            );
            assert!(!message.contains("FunctionHasMultipleSuccessReturns"));
        }
    }
    let compact = compact_bag_for_test(
        DiagnosticBag::from(choices_control_flow::invalid_fallible_handling(
            InvalidFallibleHandlingReason::EntryHasIncompatibleError {
                produced_error_type,
                entry_error_type,
            },
            None,
        )),
        Some(&type_environment),
    );
    let view = compact
        .first()
        .expect("incompatible error diagnostic compacts");
    let facts = view
        .family_view::<InvalidFallibleHandlingView>()
        .expect("fallible facts");
    assert_eq!(
        view.types()
            .shape(facts.first_error_type.expect("produced error type")),
        TypeDisplayShape::Struct {
            path: error_path,
            const_record: false
        }
    );
    assert_eq!(
        view.types()
            .shape(facts.second_error_type.expect("entry error type")),
        TypeDisplayShape::Struct {
            path: entry_error_path,
            const_record: false
        }
    );
    let context = DiagnosticRenderContext::new(&string_table).with_path_table(&path_table);
    let guidance = terminal::format_guidance_view(view, context).join("\n");
    let terse_line = terse::format_terse_diagnostic_view(view, context);
    for message in [&guidance, &terse_line] {
        assert!(message.contains("produces `ParseFailure!`"), "{message}");
        assert!(message.contains("`start` `Error!` boundary"), "{message}");
        assert!(
            message.contains("Handle `ParseFailure` locally with `catch` or recover explicitly"),
            "{message}"
        );
        assert!(!message.contains("EntryHasIncompatibleError"));
        assert!(!message.contains("TypeId"));
        assert!(!message.contains("declare"));
        assert!(!message.contains("add"));
    }
}

#[test]
fn builtin_failure_witness_labels_render_calls_before_the_original_operation() {
    let mut string_table = StringTable::new();
    let source_path = Path::new("/project/failure.moth");
    let mut sources = SourceDatabase::build(
        std::iter::once(source_path),
        source_path,
        None,
        &mut string_table,
    )
    .expect("test source identity should build");
    let source = sources
        .get_by_canonical_path(source_path)
        .expect("registered source")
        .id;
    sources
        .retain_text(
            source,
            "export\nfirst\nsecond\nthird\nmultiply\n".to_owned(),
        )
        .expect("source text");
    let mut builder = ExtendedSpanBuilder::new();
    let boundary = exact_span_in_source(source, 0, 6, &mut builder);
    let call_spans = [
        exact_span_in_source(source, 7, 5, &mut builder),
        exact_span_in_source(source, 13, 6, &mut builder),
        exact_span_in_source(source, 20, 5, &mut builder),
    ];
    let origin_span = exact_span_in_source(source, 26, 8, &mut builder);
    let make_draft = || {
        choices_control_flow::invalid_fallible_handling(
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction {
                witness: Box::new(BuiltinFailureWitness {
                    call_spans: call_spans.iter().copied().map(Some).collect(),
                    origin_span: Some(origin_span),
                    origin: BuiltinFailureOriginKind::Operation,
                    elided_call_hops: 2,
                }),
            },
            Some(boundary),
        )
        .with_label(
            Some(call_spans[0]),
            Some(LabelDraftMessage::BuiltinFailureCall),
        )
        .with_label(
            Some(call_spans[1]),
            Some(LabelDraftMessage::BuiltinFailureCall),
        )
        .with_label(
            Some(call_spans[2]),
            Some(LabelDraftMessage::BuiltinFailureCall),
        )
        .with_label(
            Some(origin_span),
            Some(LabelDraftMessage::BuiltinFailureOrigin),
        )
    };
    let draft = make_draft();
    let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let view = compact.first().expect("witness diagnostic compacts");
    assert!(view.family_view::<InvalidFallibleHandlingView>().is_some());
    assert!(matches!(view.primary_site(), SiteRef::Local(span) if span == boundary));
    let labels = view.durable_labels().collect::<Vec<_>>();
    assert_eq!(labels.len(), 4);
    assert!(
        labels[..3]
            .iter()
            .all(|label| matches!(label.message, DurableLabelMessage::BuiltinFailureCall))
    );
    assert!(matches!(
        labels[3].message,
        DurableLabelMessage::BuiltinFailureOrigin
    ));
    assert!(matches!(labels[3].site, SiteRef::Local(span) if span == origin_span));
    let frozen_identity = FrozenIdentityContext::from_parts(string_table, sources);
    let context = DiagnosticRenderContext::new(frozen_identity.strings())
        .with_frozen_identity(&frozen_identity);
    assert_eq!(
        terminal::format_label_messages_view(view, context),
        vec![
            "info: 2:1 - implicit built-in failure propagates through this call",
            "info: 3:1 - implicit built-in failure propagates through this call",
            "info: 4:1 - implicit built-in failure propagates through this call",
            "info: 5:1 - original failing numeric or range operation here",
        ],
    );

    let owner = FrozenIdentityHandle::new();
    let mut identity_draft = make_draft();
    identity_draft.attach_frozen_identity_handle_if_missing(owner.clone());
    assert_eq!(
        identity_draft.primary_frozen_identity_handle(),
        Some(&owner)
    );
    assert!(identity_draft.has_label_at_with_owner(Some(call_spans[0]), Some(&owner)));
    assert!(identity_draft.has_label_at_with_owner(Some(origin_span), Some(&owner)));
}

#[test]
fn phase_1_2_renderers_keep_source_language_terminology() {
    let mut string_table = StringTable::new();
    let config_key = string_table.intern("homepage");
    let drafts = vec![
        choices_control_flow::invalid_standalone_statement(
            InvalidStandaloneStatementReason::StandaloneTemplate,
            None,
        ),
        choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::GenericParameterOutsideDeclarationHeader,
            None,
        ),
        choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedOf,
            None,
        ),
        templates::invalid_template_structure(
            InvalidTemplateStructureReason::FallibleValueInTemplateHead,
            None,
        ),
        templates::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred,
            None,
        ),
        templates::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateIfConditionNotConst,
            None,
        ),
        choices_control_flow::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchOnOptional,
            None,
        ),
        choices_control_flow::invalid_fallible_handling(
            InvalidFallibleHandlingReason::BangOnNonFallible,
            None,
        ),
        project_config::invalid_config_reason(
            Some(config_key),
            InvalidConfigReason::ValueCouldNotFold,
            None,
        ),
        calls_data::invalid_cast(
            InvalidCastReason::UserDefinedEvidenceNotConstFoldable,
            None,
            None,
            None,
        ),
        choices_control_flow::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext,
            None,
            None,
        ),
        names_declarations::deferred_feature_reason(DeferredFeatureReason::AsyncBlock, None),
    ];
    let compact = compact_bag_for_test(DiagnosticBag::from_diagnostics(drafts), None);
    let rendered = compact
        .views()
        .flat_map(|view| {
            terminal::format_guidance_view(view, DiagnosticRenderContext::new(&string_table))
        })
        .collect::<Vec<_>>()
        .join("\n");
    for expected in [
        "A standalone template is not a valid statement here",
        "top-level generic declaration header",
        "`Box of String`",
        "compatible fallible function",
        "Inspect the optional value with an `if` present-value capture",
        "`!` propagates an `Error!` return, but this expression is not fallible",
        "Config declarations cannot depend on runtime evaluation",
        "User-defined cast evidence must be fully evaluable at compile time",
        "`value String? = none`",
        "future language support",
        "This template must be fully evaluated at compile time",
        "optional value's presence cannot be determined at compile time",
    ] {
        assert!(
            rendered.contains(expected),
            "expected source-language text '{expected}' in: {rendered}"
        );
    }
}

#[test]
fn source_dependency_renderers_use_current_language_terminology() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let dependency_path = path_fork
        .try_intern_portable_path("missing.moth", &mut string_table)
        .expect("dependency path fits");
    let namespace_name = string_table.intern("docs");
    let alias_name = string_table.intern("readFile");
    let symbol_name = string_table.intern("read_file");
    let member_name = string_table.intern("member");
    let mut drafts = vec![
        imports::missing_import_target(dependency_path, None),
        imports::invalid_import_path(
            dependency_path,
            InvalidImportPathReason::ParentDirectorySegment,
            None,
        ),
        project_config::dependency_namespace_used_as_value(namespace_name, None),
        project_config::nested_dependency_traversal(namespace_name, None),
        imports::dependency_alias_case_mismatch(alias_name, symbol_name, None),
    ];
    for (expected, found) in [
        (
            NamespaceTypeValueMisuseKind::Type,
            NamespaceTypeValueMisuseKind::Value,
        ),
        (
            NamespaceTypeValueMisuseKind::Value,
            NamespaceTypeValueMisuseKind::Type,
        ),
        (
            NamespaceTypeValueMisuseKind::Value,
            NamespaceTypeValueMisuseKind::Namespace,
        ),
        (
            NamespaceTypeValueMisuseKind::Type,
            NamespaceTypeValueMisuseKind::Namespace,
        ),
        (
            NamespaceTypeValueMisuseKind::Namespace,
            NamespaceTypeValueMisuseKind::Value,
        ),
        (
            NamespaceTypeValueMisuseKind::Namespace,
            NamespaceTypeValueMisuseKind::Type,
        ),
    ] {
        drafts.push(project_config::namespace_type_value_misuse(
            member_name,
            expected,
            found,
            None,
        ));
    }
    let compact = compact_bag_for_test(DiagnosticBag::from_diagnostics(drafts), None);
    let context = DiagnosticRenderContext::new(&string_table).with_path_fork(&path_fork);
    let rendered_lines = compact
        .views()
        .map(|view| terse::format_terse_diagnostic_view(view, context))
        .collect::<Vec<_>>();
    assert_eq!(
        rendered_lines.len(),
        11,
        "complete terse renderer emits one record per draft"
    );
    let rendered = rendered_lines.join("\n");
    for code in [
        "MOTH-IMPORT-0005",
        "MOTH-IMPORT-0016",
        "MOTH-RULE-0065",
        "MOTH-RULE-0066",
        "MOTH-IMPORT-0003",
        "MOTH-RULE-0067",
    ] {
        assert!(
            rendered.contains(code),
            "expected dependency code '{code}' in: {rendered}"
        );
    }
    for expected in [
        "Cannot resolve dependency",
        "Dependency paths containing '..'",
        "dependency namespace binding",
        "Dependency namespace records do not expose nested filesystem paths",
        "Dependency alias 'readFile' case mismatch with symbol 'read_file'",
        "member of the dependency namespace",
    ] {
        assert!(
            rendered.contains(expected),
            "expected current dependency terminology '{expected}' in: {rendered}"
        );
    }
    for stale in [
        "Cannot resolve import",
        "Import paths",
        "Import alias",
        "import namespace",
        "import record",
        "Import record",
        "Nested import-record traversal",
    ] {
        assert!(
            !rendered.contains(stale),
            "stale dependency term '{stale}': {rendered}"
        );
    }
}

#[test]
fn render_boundary_smoke_coverage_hides_internal_debug_names_by_family() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("source path fits");
    let import_path = path_fork
        .try_intern_portable_path("missing.moth", &mut string_table)
        .expect("import path fits");
    let value_name = string_table.intern("value");
    let config_key = string_table.intern("homepage");
    let feature_name = string_table.intern("traits");
    let type_environment = TypeEnvironment::new();
    let drafts = vec![
        token_lexical::invalid_path(PathKind::WhitespaceMustBeQuoted, None),
        types_borrows_targets::type_mismatch(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            type_environment.builtins().string,
            TypeMismatchContext::Declaration,
            None,
        ),
        names_declarations::invalid_assignment_target(
            InvalidAssignmentTargetReason::ImmutableBinding,
            Some(value_name),
            None,
            None,
            None,
        ),
        imports::missing_import_target(import_path, None),
        project_config::invalid_config_reason(
            Some(config_key),
            InvalidConfigReason::UnsupportedScalarValue,
            None,
        ),
        names_declarations::deferred_feature_reason(
            DeferredFeatureReason::NamedFeature {
                feature: feature_name,
            },
            None,
        ),
    ];
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(drafts),
        string_table,
        Some(&type_environment),
        Some(Arc::new(path_fork.snapshot_table())),
    );
    let failure = finish_prepared_batch(prepared, StringTable::new());
    let html = dev_server::render_compilation_result_html(&failure, Path::new("/tmp"));
    assert_rendered_diagnostic_hides_internal_names(&html);
    let terse_lines = terse::format_terse_compilation_result(&failure);
    assert_eq!(terse_lines.len(), 6);
    let Ok(CompilationOutcome::Diagnosed(reports)) = &failure else {
        panic!("family diagnostics should produce a diagnosed result");
    };
    for (report, view) in reports.iter() {
        let context = DiagnosticRenderContext::for_report(report);
        let terminal_guidance = terminal::format_guidance_view(view, context).join("\n");
        let terse_line = terse::format_terse_diagnostic_view(view, context);
        assert_rendered_diagnostic_hides_internal_names(&terminal_guidance);
        assert_rendered_diagnostic_hides_internal_names(&terse_line);
        assert!(terse_line.contains(view.descriptor().code));
        assert!(html.contains(view.descriptor().code));
    }
    let _ = source_path;
}

#[test]
fn mutable_access_required_renders_explicit_marker_guidance() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessRequired {
            parameter_name: Some(parameter),
            parameter_index: 0,
        },
        "consume",
    );
    assert!(
        message
            .contains("Call to 'consume' requires explicit mutable access for parameter 'value'."),
        "{message}"
    );
    assert!(
        message.contains("Prefix the existing mutable place with `~`"),
        "{message}"
    );
}

#[test]
fn immutable_place_mutable_access_renders_binding_name_for_missing_marker() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("values");
    let binding = string_table.intern("values");
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: Some(binding),
        },
        "consume",
    );
    assert!(
        message
            .contains("requires mutable access for parameter 'values', but `values` is immutable."),
        "{message}"
    );
    assert!(
        message
            .contains("Declare the binding as mutable, then pass the existing argument with `~`."),
        "{message}"
    );
}

#[test]
fn immutable_place_mutable_access_renders_authored_marker_with_binding_name() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let binding = string_table.intern("x");
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessOnImmutablePlace {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: Some(binding),
        },
        "mutate",
    );
    assert!(
        message.contains("requires mutable access for parameter 'value', but `x` is immutable."),
        "{message}"
    );
    assert!(
        message.contains("then pass the existing argument with `~`."),
        "{message}"
    );
}

#[test]
fn projected_immutable_argument_guidance_does_not_replace_projection_with_root() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let binding = string_table.intern("box");
    for reason in [
        InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: Some(binding),
        },
        InvalidCallShapeReason::MutableAccessOnImmutablePlace {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: Some(binding),
        },
    ] {
        let message = render_invalid_call_shape(&mut string_table, reason, "mutate");
        assert!(message.contains("`box` is immutable"), "{message}");
        assert!(
            message.contains("pass the existing argument with `~`"),
            "{message}"
        );
        assert!(!message.contains("`~box`"), "{message}");
    }
}

#[test]
fn immutable_place_mutable_access_uses_generic_fallback_without_binding_name() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let missing_marker = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: None,
        },
        "mutate",
    );
    let authored_marker = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessOnImmutablePlace {
            parameter_name: Some(parameter),
            parameter_index: 0,
            binding_name: None,
        },
        "mutate",
    );
    let expected = "but this argument comes from an immutable binding or field.";
    assert!(missing_marker.contains(expected), "{missing_marker}");
    assert!(authored_marker.contains(expected), "{authored_marker}");
    for message in [missing_marker.as_str(), authored_marker.as_str()] {
        for internal in ["place", "non-place", "rvalue", "variable"] {
            assert!(
                !message.contains(internal),
                "fallback must not say '{internal}': {message}"
            );
        }
    }
}

#[test]
fn mutable_access_on_non_place_renders_fresh_value_guidance() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessOnNonPlace {
            parameter_name: Some(parameter),
            parameter_index: 0,
        },
        "mutate",
    );
    assert!(
        message.contains(
            "Call to 'mutate' cannot use `~` on a fresh or computed value for parameter 'value'."
        ),
        "{message}"
    );
    assert!(
        message.contains("Remove `~` and pass the value directly."),
        "{message}"
    );
    for internal in ["place", "non-place", "rvalue", "variable"] {
        assert!(
            !message.contains(internal),
            "fresh/computed branch must not say '{internal}': {message}"
        );
    }
}

#[test]
fn unnamed_parameter_renders_one_based_position_without_internal_slot() {
    let mut string_table = StringTable::new();
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessRequired {
            parameter_name: None,
            parameter_index: 0,
        },
        "consume",
    );
    assert!(message.contains("parameter 1"), "{message}");
    assert!(
        !message.contains("parameter 0"),
        "must not render zero-based slot: {message}"
    );
    assert!(
        !message.contains("1-based"),
        "must not render conversion note: {message}"
    );
    assert!(!message.contains("#1"), "must not render #N: {message}");
}

#[test]
fn mutable_access_not_allowed_tells_author_to_remove_authored_marker() {
    let mut string_table = StringTable::new();
    let parameter = string_table.intern("value");
    let message = render_invalid_call_shape(
        &mut string_table,
        InvalidCallShapeReason::MutableAccessNotAllowed {
            parameter_name: Some(parameter),
            parameter_index: 0,
        },
        "consume",
    );
    assert!(
        message.contains("Call to 'consume' does not accept mutable access for parameter 'value'."),
        "{message}"
    );
    assert!(
        message.contains("Remove the authored `~` from this argument."),
        "{message}"
    );
    assert!(
        !message.contains("(~)"),
        "must not render parenthetical marker: {message}"
    );
    assert!(
        !message.contains("Remove the ~ "),
        "must not render bare tilde: {message}"
    );
}

#[test]
fn source_method_missing_marker_renders_named_receiver_example() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::MutableReceiverMissingMarker,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        Some("p"),
    );
    assert!(
        message.contains("Mutable receiver method `move` requires explicit mutable access."),
        "{message}"
    );
    assert!(message.contains("for example `~p.move(...)`"), "{message}");
}

#[test]
fn source_method_missing_marker_omits_example_without_binding_name() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::MutableReceiverMissingMarker,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        None,
    );
    assert!(
        message.contains("Prefix the receiver with `~`."),
        "{message}"
    );
    assert!(
        !message.contains("~this receiver"),
        "internal placeholder leaked: {message}"
    );
    assert!(!message.contains("for example"), "{message}");
}

#[test]
fn source_method_immutable_receiver_names_binding_to_declare_mutable() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        Some("p"),
    );
    assert!(
        message.contains("`p` is immutable. Declare `p` as mutable, then call it with `~`."),
        "{message}"
    );
    assert!(!message.contains("temporary"), "{message}");
}

#[test]
fn source_method_non_place_receiver_requires_mutable_place() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::NonPlaceReceiverMutableMethod,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        None,
    );
    assert!(
        message.contains("requires a mutable place receiver."),
        "{message}"
    );
    assert!(!message.contains("immutable"), "{message}");
    assert!(!message.contains("Declare"), "{message}");
}

#[test]
fn collection_missing_marker_names_kind_and_explicit_access() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::MutableReceiverMissingMarker,
        "push",
        Some(ReceiverCallKind::CollectionBuiltin),
        Some("values"),
    );
    assert!(
        message.contains("`push` requires a mutable collection receiver"),
        "{message}"
    );
    assert!(
        message.contains("Call it with explicit `~` access."),
        "{message}"
    );
}

#[test]
fn collection_immutable_receiver_names_binding_to_declare_mutable() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        "push",
        Some(ReceiverCallKind::CollectionBuiltin),
        Some("values"),
    );
    assert!(
        message.contains("`push` requires a mutable collection receiver"),
        "{message}"
    );
    assert!(
        message.contains("Declare `values` as mutable, then call it with explicit `~` access."),
        "{message}"
    );
}

#[test]
fn collection_non_place_receiver_requires_mutable_binding() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::NonPlaceReceiverMutableMethod,
        "push",
        Some(ReceiverCallKind::CollectionBuiltin),
        None,
    );
    assert!(
        message.contains("`push` requires a mutable collection receiver"),
        "{message}"
    );
    assert!(
        message.contains("Bind this value in a mutable binding first, then call it with `~`."),
        "{message}"
    );
    assert!(!message.contains("immutable"), "{message}");
}

#[test]
fn map_immutable_receiver_names_kind_and_binding() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::ImmutableReceiverMutableMethod,
        "set",
        Some(ReceiverCallKind::MapBuiltin),
        Some("scores"),
    );
    assert!(
        message.contains("`set` requires a mutable map receiver"),
        "{message}"
    );
    assert!(
        message.contains("Declare `scores` as mutable, then call it with explicit `~` access."),
        "{message}"
    );
}

#[test]
fn authored_marker_on_immutable_receiver_keeps_marker_wording() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::MutableMarkerOnImmutableReceiver,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        Some("p"),
    );
    assert!(
        message.contains("`~` accepts only an existing mutable place."),
        "{message}"
    );
    assert!(
        message.contains("Declare `p` as mutable before calling it with `~`."),
        "{message}"
    );
    assert!(message.contains("`p` is immutable"), "{message}");
}

#[test]
fn authored_marker_on_non_place_receiver_explains_temporary() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::MutableMarkerOnNonPlaceReceiver,
        "move",
        Some(ReceiverCallKind::SourceMethod),
        None,
    );
    assert!(
        message.contains("`~` accepts only an existing mutable place."),
        "{message}"
    );
    assert!(
        message.contains("cannot be called on a temporary value."),
        "{message}"
    );
    assert!(!message.contains("immutable"), "{message}");
}

#[test]
fn unneeded_mutable_marker_tells_author_to_remove_it() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::UnneededMutableAccessMarker,
        "length",
        Some(ReceiverCallKind::CollectionBuiltin),
        None,
    );
    assert!(
        message.contains("`length` does not accept an explicit mutable access marker `~`."),
        "{message}"
    );
    assert!(
        message.contains("Remove the `~` from this call."),
        "{message}"
    );
}

#[test]
fn const_record_runtime_call_renders_current_source_term() {
    let mut string_table = StringTable::new();
    let message = render_invalid_receiver_call(
        &mut string_table,
        InvalidReceiverCallReason::ConstRecordNoRuntimeCalls,
        "length",
        Some(ReceiverCallKind::SourceMethod),
        None,
    );
    assert!(
        message.contains("Const records are data-only"),
        "current `const record` term required: {message}"
    );
    assert!(
        !message.contains("const struct record"),
        "stale terminology: {message}"
    );
}

#[test]
fn builtin_cast_shape_diagnostic_preserves_stable_code_and_rendering() {
    let mut string_table = StringTable::new();
    let scalar_name = string_table.intern("I32");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(calls_data::invalid_builtin_call(
            InvalidBuiltinCallReason::CastMissingArgument,
            Some(scalar_name),
            None,
        )),
        None,
    );
    let view = compact.first().expect("builtin call draft compacts");
    assert!(
        view.family_view::<calls_data::InvalidBuiltinCallView>()
            .is_some()
    );
    assert_eq!(view.descriptor().code, "MOTH-RULE-0046");
    let terse_line =
        terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::new(&string_table));
    assert!(
        terse_line.contains("'I32' cast requires exactly one argument"),
        "{terse_line}"
    );
}

#[test]
fn extra_positional_argument_renders_counts_and_removal_guidance() {
    let mut string_table = StringTable::new();
    let callee = string_table.intern("add");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(calls_data::invalid_call_shape(
            InvalidCallShapeReason::ExtraPositionalArgument {
                expected_count: 2,
                provided_count: 3,
            },
            Some(callee),
            None,
        )),
        None,
    );
    let view = compact.first().expect("extra-positional draft compacts");
    assert!(view.family_view::<InvalidCallShapeView>().is_some());
    let context = DiagnosticRenderContext::new(&string_table);
    let guidance = terminal::format_guidance_view(view, context);
    let message = guidance
        .iter()
        .find(|line| line.contains("positional arguments"))
        .expect("extra-positional message");
    assert!(
        message.contains("Call to 'add' has 3 positional arguments, but accepts 2."),
        "{message}"
    );
    assert!(
        guidance
            .iter()
            .any(|line| line.contains("Remove the extra argument starting here.")),
        "{guidance:?}"
    );
}

#[test]
fn scalar_type_called_as_function_renders_cast_guidance() {
    let mut string_table = StringTable::new();
    let scalar_name = string_table.intern("I32");
    let compact = compact_bag_for_test(
        DiagnosticBag::from(calls_data::invalid_builtin_call(
            InvalidBuiltinCallReason::ScalarTypeCalledAsFunction,
            Some(scalar_name),
            None,
        )),
        None,
    );
    let view = compact.first().expect("scalar type-call draft compacts");
    assert!(
        view.family_view::<calls_data::InvalidBuiltinCallView>()
            .is_some()
    );
    assert_eq!(view.descriptor().code, "MOTH-RULE-0046");
    let terse_line =
        terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::new(&string_table));
    assert!(
        terse_line.contains("`I32` is a type, not a conversion function"),
        "{terse_line}"
    );
    assert!(
        terse_line.contains("Use `cast` at an explicit typed receiving boundary"),
        "{terse_line}"
    );
}
