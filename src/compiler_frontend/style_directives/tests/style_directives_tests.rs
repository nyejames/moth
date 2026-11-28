use super::*;

#[test]
fn supported_directives_for_diagnostic_uses_stable_registry_order() {
    let registry = StyleDirectiveRegistry::built_ins();

    assert_eq!(
        registry.supported_directives_for_diagnostic(DirectiveOccurrence::TemplateHead),
        "'$before', '$after', '$fresh', '$slot', '$supplied', '$in', '$note', '$raw', '$md', '$html', '$css', '$escape_html', '$code'"
    );
}

#[test]
fn diagnostic_suggestions_are_limited_to_the_occurrence() {
    let registry = StyleDirectiveRegistry::built_ins();

    assert_eq!(
        registry.supported_directives_for_diagnostic(DirectiveOccurrence::Source),
        "'$export', '$config', '$doc', '$todo', '$fast_math', '$safe_math', '$infallible'",
        "a source position must not suggest template-only directives"
    );
    assert!(
        !registry
            .supported_directives_for_diagnostic(DirectiveOccurrence::TemplateHead)
            .contains("$config"),
        "a template head must not suggest the source modifier"
    );
    assert!(
        !registry
            .supported_directives_for_diagnostic(DirectiveOccurrence::TemplateHead)
            .contains("$export"),
        "a template head must not suggest the source modifier"
    );
}

#[test]
fn export_and_config_are_source_modifiers() {
    let built_ins = StyleDirectiveRegistry::built_ins();

    let export = built_ins
        .find("export")
        .expect("missing '$export' directive");
    assert!(matches!(
        export.kind,
        StyleDirectiveKind::SourceModifier {
            kind: SourceDirectiveKind::Export,
            operand: SourceModifierOperand::OptionalPublicRoute,
        }
    ));

    let config = built_ins
        .find("config")
        .expect("missing '$config' directive");
    assert!(matches!(
        config.kind,
        StyleDirectiveKind::SourceModifier {
            kind: SourceDirectiveKind::Config,
            operand: SourceModifierOperand::None,
        }
    ));

    for template_directive in [
        "before",
        "after",
        "fresh",
        "slot",
        "in",
        "note",
        "raw",
        "md",
        "html",
        "css",
        "escape_html",
        "code",
    ] {
        let directive = built_ins
            .find(template_directive)
            .unwrap_or_else(|| panic!("missing '${template_directive}' directive"));
        assert!(source_modifier_contract(directive).is_none());
    }
}

#[test]
fn lexical_constraint_directives_share_one_target_contract() {
    let built_ins = StyleDirectiveRegistry::built_ins();

    for (name, kind) in [
        ("fast_math", SourceDirectiveKind::FastMath),
        ("safe_math", SourceDirectiveKind::SafeMath),
        ("infallible", SourceDirectiveKind::Infallible),
    ] {
        let directive = built_ins
            .find(name)
            .unwrap_or_else(|| panic!("missing '${name}' directive"));
        assert!(
            matches!(
                directive.kind,
                StyleDirectiveKind::SourceModifier {
                    kind: attached,
                    operand: SourceModifierOperand::None,
                } if attached == kind
            ),
            "'${name}' must use the shared parameterless constraint target"
        );
    }
}

#[test]
fn constraint_sets_union_and_report_their_active_modes() {
    let all = SourceConstraintSet::default()
        .with_directive(SourceDirectiveKind::FastMath)
        .with_directive(SourceDirectiveKind::SafeMath)
        .with_directive(SourceDirectiveKind::Infallible);
    assert!(!all.is_empty());
    assert!(all.fast_math_active());
    assert!(all.safe_math_active());
    assert!(all.infallible_active());

    let docs_only = SourceConstraintSet::default()
        .with_directive(SourceDirectiveKind::Doc)
        .with_directive(SourceDirectiveKind::Todo);
    assert!(
        docs_only.is_empty(),
        "metadata directives are not constraints"
    );

    let inherited = SourceConstraintSet::default()
        .with_directive(SourceDirectiveKind::FastMath)
        .union(SourceConstraintSet::default().with_directive(SourceDirectiveKind::SafeMath));
    assert!(inherited.fast_math_active());
    assert!(inherited.safe_math_active());
    assert!(!inherited.infallible_active());
}

#[test]
fn frontend_built_ins_have_expected_classification() {
    let built_ins = StyleDirectiveRegistry::built_ins();

    for required in ["before", "after", "slot", "in", "fresh", "note", "raw"] {
        let directive = built_ins
            .find(required)
            .unwrap_or_else(|| panic!("missing core built-in directive '{required}'"));
        assert!(matches!(directive.kind, StyleDirectiveKind::Core(_)));
    }

    for source_directive in ["doc", "todo"] {
        let directive = built_ins
            .find(source_directive)
            .unwrap_or_else(|| panic!("missing source directive '${source_directive}'"));
        assert!(matches!(
            directive.kind,
            StyleDirectiveKind::SourceModifier {
                operand: SourceModifierOperand::RequiredSingleString,
                ..
            }
        ));
    }

    for handler_directive in ["md", "html", "css", "escape_html", "code"] {
        let directive = built_ins
            .find(handler_directive)
            .unwrap_or_else(|| panic!("missing frontend-owned '${handler_directive}' directive"));
        assert!(matches!(directive.kind, StyleDirectiveKind::Handler(_)));
    }
}

#[test]
fn frontend_built_in_head_compatibility_profiles_match_contract() {
    let built_ins = StyleDirectiveRegistry::built_ins();

    let slot = built_ins.find("slot").expect("missing '$slot' directive");
    assert_eq!(
        slot.head_compatibility.presence_tags,
        TemplateHeadTag::MEANINGFUL_ITEM | TemplateHeadTag::SLOT_DIRECTIVE
    );
    assert_eq!(
        slot.head_compatibility.required_absent_tags,
        TemplateHeadTag::MEANINGFUL_ITEM
    );
    assert_eq!(
        slot.head_compatibility.blocks_future_tags,
        TemplateHeadTag::MEANINGFUL_ITEM
    );
    assert_eq!(
        slot.head_compatibility.conflict_reason,
        Some(TemplateHeadConflictReason::SlotDirectiveMustBeExclusive)
    );

    let in_directive = built_ins.find("in").expect("missing '$in' directive");
    assert_eq!(
        in_directive.head_compatibility,
        TemplateHeadCompatibility::blocks_same(
            TemplateHeadTag::IN_DIRECTIVE,
            TemplateHeadConflictReason::DuplicateExclusiveDirective,
        )
    );

    let note = built_ins.find("note").expect("missing '$note' directive");
    assert_eq!(
        note.head_compatibility.presence_tags,
        TemplateHeadTag::MEANINGFUL_ITEM | TemplateHeadTag::COMMENT_DIRECTIVE
    );
    assert_eq!(
        note.head_compatibility.required_absent_tags,
        TemplateHeadTag::MEANINGFUL_ITEM
    );
    assert_eq!(
        note.head_compatibility.blocks_future_tags,
        TemplateHeadTag::MEANINGFUL_ITEM
    );
    assert_eq!(
        note.head_compatibility.conflict_reason,
        Some(TemplateHeadConflictReason::DiscardedCommentDirectiveMustBeOnlyMeaningfulItem)
    );

    for formatter_directive in ["md", "raw", "html", "css", "escape_html", "code"] {
        let directive = built_ins
            .find(formatter_directive)
            .unwrap_or_else(|| panic!("missing '${formatter_directive}' directive"));
        assert!(
            directive
                .head_compatibility
                .presence_tags
                .intersects(TemplateHeadTag::FORMATTER_DIRECTIVE)
        );
        assert_eq!(
            directive.head_compatibility.blocks_future_tags,
            TemplateHeadTag::FORMATTER_DIRECTIVE
        );
        assert_eq!(
            directive.head_compatibility.conflict_reason,
            Some(TemplateHeadConflictReason::MultipleFormatters)
        );
    }
}
