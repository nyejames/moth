use super::*;

#[test]
fn supported_directives_for_diagnostic_uses_stable_registry_order() {
    let registry = StyleDirectiveRegistry::built_ins();

    assert_eq!(
        registry.supported_directives_for_diagnostic(),
        "'$before', '$after', '$fresh', '$slot', '$in', '$note', '$todo', '$doc', '$raw', '$md', '$html', '$css', '$escape_html', '$code'"
    );
}

#[test]
fn frontend_built_ins_have_expected_classification() {
    let built_ins = StyleDirectiveRegistry::built_ins();

    for required in [
        "before", "after", "slot", "in", "fresh", "doc", "todo", "note", "raw",
    ] {
        let directive = built_ins
            .find(required)
            .unwrap_or_else(|| panic!("missing core built-in directive '{required}'"));
        assert!(matches!(directive.kind, StyleDirectiveKind::Core(_)));
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

    for (comment_directive, conflict_reason) in [
        (
            "note",
            TemplateHeadConflictReason::DiscardedCommentDirectiveMustBeOnlyMeaningfulItem,
        ),
        (
            "todo",
            TemplateHeadConflictReason::DiscardedCommentDirectiveMustBeOnlyMeaningfulItem,
        ),
        (
            "doc",
            TemplateHeadConflictReason::DocumentationDirectiveMustBeOnlyMeaningfulItem,
        ),
    ] {
        let directive = built_ins
            .find(comment_directive)
            .unwrap_or_else(|| panic!("missing '${comment_directive}' directive"));
        assert_eq!(
            directive.head_compatibility.presence_tags,
            TemplateHeadTag::MEANINGFUL_ITEM | TemplateHeadTag::COMMENT_DIRECTIVE
        );
        assert_eq!(
            directive.head_compatibility.required_absent_tags,
            TemplateHeadTag::MEANINGFUL_ITEM
        );
        assert_eq!(
            directive.head_compatibility.blocks_future_tags,
            TemplateHeadTag::MEANINGFUL_ITEM
        );
        assert_eq!(
            directive.head_compatibility.conflict_reason,
            Some(conflict_reason)
        );
    }

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
