//! Family rows for template diagnostics.
//!
//! WHAT: compact rows, constructors, typed views and adapters for template diagnostics.
//! WHY: each template diagnostic has one family whose schema owns its facts, labels, and stable code.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    OptCharFact, OptCountFact, OptStringFact, OptTypeFact, StringFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_invalid_style_directive, render_invalid_template_directive,
    render_invalid_template_slot, render_invalid_template_structure, render_malformed_css_template,
    render_malformed_html_template, render_unescaped_implicit_template_close,
};
use crate::compiler_frontend::compiler_messages::{
    CssTemplateWarning, DiagnosticCategory, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity,
    HtmlTemplateWarning, InvalidTemplateDirectiveReason, InvalidTemplateSlotReason,
    InvalidTemplateStructureReason, MalformedTemplateReason, RuleDiagnosticKind,
    SyntaxDiagnosticKind,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

unit_code! {
    impl SourceFileKind {
        Moth = 1,
        MothTemplate = 2,
        PlainMarkdown = 3,
    }
}

unit_code! {
    impl InvalidTemplateSlotReason {
        InsertOutsideParentSlot = 1,
        ExtraLooseContentWithoutDefaultSlot = 2,
        LooseContentWithoutDefaultSlot = 3,
        InsertCannotTargetDefaultSlot = 4,
        InsertTargetsUnknownNamedSlot = 5,
        InsertTargetsUnknownPositionalSlot = 6,
        SlotDefinitionOutsideTemplateBody = 7,
    }
}

unit_code! {
    pub(crate) enum InvalidTemplateDirectiveCode {
        UnknownDirective = 1,
        MissingArgument = 2,
        InvalidArgument = 3,
        DirectiveNotAllowedHere = 4,
        UnexpectedArguments = 5,
        EmptyArguments = 6,
        InvalidSlotTarget = 7,
        InvalidInsertTarget = 8,
        InvalidChildrenArgument = 9,
    }
}

impl InvalidTemplateDirectiveCode {
    pub(crate) fn of(reason: &InvalidTemplateDirectiveReason) -> Self {
        Self::split(*reason).0
    }

    pub(crate) fn split(reason: InvalidTemplateDirectiveReason) -> (Self, Option<StringId>) {
        match reason {
            InvalidTemplateDirectiveReason::UnknownDirective => (Self::UnknownDirective, None),
            InvalidTemplateDirectiveReason::MissingArgument => (Self::MissingArgument, None),
            InvalidTemplateDirectiveReason::InvalidArgument { detail } => {
                (Self::InvalidArgument, detail)
            }
            InvalidTemplateDirectiveReason::DirectiveNotAllowedHere => {
                (Self::DirectiveNotAllowedHere, None)
            }
            InvalidTemplateDirectiveReason::UnexpectedArguments => {
                (Self::UnexpectedArguments, None)
            }
            InvalidTemplateDirectiveReason::EmptyArguments => (Self::EmptyArguments, None),
            InvalidTemplateDirectiveReason::InvalidSlotTarget => (Self::InvalidSlotTarget, None),
            InvalidTemplateDirectiveReason::InvalidInsertTarget => {
                (Self::InvalidInsertTarget, None)
            }
            InvalidTemplateDirectiveReason::InvalidChildrenArgument => {
                (Self::InvalidChildrenArgument, None)
            }
        }
    }

    pub(crate) fn to_reason(self, detail: Option<StringId>) -> InvalidTemplateDirectiveReason {
        match self {
            Self::UnknownDirective => InvalidTemplateDirectiveReason::UnknownDirective,
            Self::MissingArgument => InvalidTemplateDirectiveReason::MissingArgument,
            Self::InvalidArgument => InvalidTemplateDirectiveReason::InvalidArgument { detail },
            Self::DirectiveNotAllowedHere => {
                InvalidTemplateDirectiveReason::DirectiveNotAllowedHere
            }
            Self::UnexpectedArguments => InvalidTemplateDirectiveReason::UnexpectedArguments,
            Self::EmptyArguments => InvalidTemplateDirectiveReason::EmptyArguments,
            Self::InvalidSlotTarget => InvalidTemplateDirectiveReason::InvalidSlotTarget,
            Self::InvalidInsertTarget => InvalidTemplateDirectiveReason::InvalidInsertTarget,
            Self::InvalidChildrenArgument => {
                InvalidTemplateDirectiveReason::InvalidChildrenArgument
            }
        }
    }
}

unit_code! {
    pub(crate) enum CssTemplateWarningCode {
        MismatchedClosingBrace = 1,
        UnexpectedClosingBrace = 2,
        MismatchedClosingParenthesis = 3,
        UnexpectedClosingParenthesis = 4,
        UnclosedDelimiter = 5,
        InlineSelectorBlock = 6,
        MissingSelectorOrAtRulePrelude = 7,
        NestedBlock = 8,
        TopLevelContent = 9,
        MalformedDeclaration = 10,
        InvalidPropertyName = 11,
        MissingDeclarationValue = 12,
    }
}

impl CssTemplateWarningCode {
    pub(crate) fn of(reason: &CssTemplateWarning) -> Self {
        Self::split(*reason).0
    }

    pub(crate) fn split(reason: CssTemplateWarning) -> (Self, Option<char>, Option<StringId>) {
        match reason {
            CssTemplateWarning::MismatchedClosingBrace { expected } => {
                (Self::MismatchedClosingBrace, Some(expected), None)
            }
            CssTemplateWarning::UnexpectedClosingBrace => {
                (Self::UnexpectedClosingBrace, None, None)
            }
            CssTemplateWarning::MismatchedClosingParenthesis { expected } => {
                (Self::MismatchedClosingParenthesis, Some(expected), None)
            }
            CssTemplateWarning::UnexpectedClosingParenthesis => {
                (Self::UnexpectedClosingParenthesis, None, None)
            }
            CssTemplateWarning::UnclosedDelimiter { opening } => {
                (Self::UnclosedDelimiter, Some(opening), None)
            }
            CssTemplateWarning::InlineSelectorBlock => (Self::InlineSelectorBlock, None, None),
            CssTemplateWarning::MissingSelectorOrAtRulePrelude => {
                (Self::MissingSelectorOrAtRulePrelude, None, None)
            }
            CssTemplateWarning::NestedBlock => (Self::NestedBlock, None, None),
            CssTemplateWarning::TopLevelContent => (Self::TopLevelContent, None, None),
            CssTemplateWarning::MalformedDeclaration => (Self::MalformedDeclaration, None, None),
            CssTemplateWarning::InvalidPropertyName { name } => {
                (Self::InvalidPropertyName, None, Some(name))
            }
            CssTemplateWarning::MissingDeclarationValue => {
                (Self::MissingDeclarationValue, None, None)
            }
        }
    }

    pub(crate) fn to_reason(
        self,
        expected: Option<char>,
        property_name: Option<StringId>,
    ) -> Option<CssTemplateWarning> {
        Some(match self {
            Self::MismatchedClosingBrace => CssTemplateWarning::MismatchedClosingBrace {
                expected: expected?,
            },
            Self::UnexpectedClosingBrace => CssTemplateWarning::UnexpectedClosingBrace,
            Self::MismatchedClosingParenthesis => {
                CssTemplateWarning::MismatchedClosingParenthesis {
                    expected: expected?,
                }
            }
            Self::UnexpectedClosingParenthesis => CssTemplateWarning::UnexpectedClosingParenthesis,
            Self::UnclosedDelimiter => CssTemplateWarning::UnclosedDelimiter { opening: expected? },
            Self::InlineSelectorBlock => CssTemplateWarning::InlineSelectorBlock,
            Self::MissingSelectorOrAtRulePrelude => {
                CssTemplateWarning::MissingSelectorOrAtRulePrelude
            }
            Self::NestedBlock => CssTemplateWarning::NestedBlock,
            Self::TopLevelContent => CssTemplateWarning::TopLevelContent,
            Self::MalformedDeclaration => CssTemplateWarning::MalformedDeclaration,
            Self::InvalidPropertyName => CssTemplateWarning::InvalidPropertyName {
                name: property_name?,
            },
            Self::MissingDeclarationValue => CssTemplateWarning::MissingDeclarationValue,
        })
    }
}

unit_code! {
    pub(crate) enum HtmlTemplateWarningCode {
        ScriptTag = 1,
        JavascriptUrl = 2,
        InlineEventHandler = 3,
    }
}

impl HtmlTemplateWarningCode {
    pub(crate) fn of(reason: &HtmlTemplateWarning) -> Self {
        match reason {
            HtmlTemplateWarning::ScriptTag => Self::ScriptTag,
            HtmlTemplateWarning::JavascriptUrl => Self::JavascriptUrl,
            HtmlTemplateWarning::InlineEventHandler => Self::InlineEventHandler,
        }
    }

    pub(crate) fn to_reason(self) -> HtmlTemplateWarning {
        match self {
            Self::ScriptTag => HtmlTemplateWarning::ScriptTag,
            Self::JavascriptUrl => HtmlTemplateWarning::JavascriptUrl,
            Self::InlineEventHandler => HtmlTemplateWarning::InlineEventHandler,
        }
    }
}

unit_code! {
    pub(crate) enum InvalidTemplateStructureCode {
        MissingClosingBracket = 1,
        SlotInHead = 2,
        MissingHandlerBody = 3,
        InvalidChildDirective = 4,
        NestedTemplateNotAllowed = 5,
        HelperInConstTemplate = 6,
        NonFoldableConstTemplate = 7,
        NonFoldableDocComment = 8,
        FallibleValueInTemplateHead = 9,
        UnsupportedTypeInTemplateHead = 10,
        RuntimeTemplateInConst = 11,
        RuntimeValueInConstTemplateHead = 12,
        EmptyPathInTemplateHead = 13,
        IncompatibleHeadItem = 14,
        HelperOutsideWrapperSlot = 15,
        RuntimeControlFlowUnresolvedInsert = 16,
        MissingCommaBeforeControlFlowSuffix = 17,
        ControlFlowSuffixNotFinal = 18,
        MissingTemplateIfCondition = 19,
        MissingTemplateLoopHeader = 20,
        UnexpectedTokenAfterControlFlowSuffix = 21,
        TemplateMatchStyleControlFlowUnsupported = 22,
        TemplateIfConditionNotConst = 23,
        TemplateIfBranchNotConst = 24,
        TemplateOptionCaptureConstDeferred = 25,
        TemplateLoopRangeBoundsNotConst = 26,
        TemplateLoopSourceNotConst = 27,
        TemplateLoopConditionNotConst = 28,
        TemplateConditionalLoopConstTrue = 29,
        TemplateLoopBodyNotConst = 30,
        TemplateConstLoopExpansionLimitExceeded = 31,
    }
}

impl InvalidTemplateStructureCode {
    pub(crate) fn of(reason: &InvalidTemplateStructureReason) -> Self {
        Self::split(*reason).0
    }

    pub(crate) fn split(
        reason: InvalidTemplateStructureReason,
    ) -> (Self, Option<TypeId>, Option<usize>) {
        match reason {
            InvalidTemplateStructureReason::MissingClosingBracket => {
                (Self::MissingClosingBracket, None, None)
            }
            InvalidTemplateStructureReason::SlotInHead => (Self::SlotInHead, None, None),
            InvalidTemplateStructureReason::MissingHandlerBody => {
                (Self::MissingHandlerBody, None, None)
            }
            InvalidTemplateStructureReason::InvalidChildDirective => {
                (Self::InvalidChildDirective, None, None)
            }
            InvalidTemplateStructureReason::NestedTemplateNotAllowed => {
                (Self::NestedTemplateNotAllowed, None, None)
            }
            InvalidTemplateStructureReason::HelperInConstTemplate => {
                (Self::HelperInConstTemplate, None, None)
            }
            InvalidTemplateStructureReason::NonFoldableConstTemplate => {
                (Self::NonFoldableConstTemplate, None, None)
            }
            InvalidTemplateStructureReason::NonFoldableDocComment => {
                (Self::NonFoldableDocComment, None, None)
            }
            InvalidTemplateStructureReason::FallibleValueInTemplateHead => {
                (Self::FallibleValueInTemplateHead, None, None)
            }
            InvalidTemplateStructureReason::UnsupportedTypeInTemplateHead { type_id } => {
                (Self::UnsupportedTypeInTemplateHead, Some(type_id), None)
            }
            InvalidTemplateStructureReason::RuntimeTemplateInConst => {
                (Self::RuntimeTemplateInConst, None, None)
            }
            InvalidTemplateStructureReason::RuntimeValueInConstTemplateHead => {
                (Self::RuntimeValueInConstTemplateHead, None, None)
            }
            InvalidTemplateStructureReason::EmptyPathInTemplateHead => {
                (Self::EmptyPathInTemplateHead, None, None)
            }
            InvalidTemplateStructureReason::IncompatibleHeadItem => {
                (Self::IncompatibleHeadItem, None, None)
            }
            InvalidTemplateStructureReason::HelperOutsideWrapperSlot => {
                (Self::HelperOutsideWrapperSlot, None, None)
            }
            InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedInsert => {
                (Self::RuntimeControlFlowUnresolvedInsert, None, None)
            }
            InvalidTemplateStructureReason::MissingCommaBeforeControlFlowSuffix => {
                (Self::MissingCommaBeforeControlFlowSuffix, None, None)
            }
            InvalidTemplateStructureReason::ControlFlowSuffixNotFinal => {
                (Self::ControlFlowSuffixNotFinal, None, None)
            }
            InvalidTemplateStructureReason::MissingTemplateIfCondition => {
                (Self::MissingTemplateIfCondition, None, None)
            }
            InvalidTemplateStructureReason::MissingTemplateLoopHeader => {
                (Self::MissingTemplateLoopHeader, None, None)
            }
            InvalidTemplateStructureReason::UnexpectedTokenAfterControlFlowSuffix => {
                (Self::UnexpectedTokenAfterControlFlowSuffix, None, None)
            }
            InvalidTemplateStructureReason::TemplateMatchStyleControlFlowUnsupported => {
                (Self::TemplateMatchStyleControlFlowUnsupported, None, None)
            }
            InvalidTemplateStructureReason::TemplateIfConditionNotConst => {
                (Self::TemplateIfConditionNotConst, None, None)
            }
            InvalidTemplateStructureReason::TemplateIfBranchNotConst => {
                (Self::TemplateIfBranchNotConst, None, None)
            }
            InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred => {
                (Self::TemplateOptionCaptureConstDeferred, None, None)
            }
            InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst => {
                (Self::TemplateLoopRangeBoundsNotConst, None, None)
            }
            InvalidTemplateStructureReason::TemplateLoopSourceNotConst => {
                (Self::TemplateLoopSourceNotConst, None, None)
            }
            InvalidTemplateStructureReason::TemplateLoopConditionNotConst => {
                (Self::TemplateLoopConditionNotConst, None, None)
            }
            InvalidTemplateStructureReason::TemplateConditionalLoopConstTrue => {
                (Self::TemplateConditionalLoopConstTrue, None, None)
            }
            InvalidTemplateStructureReason::TemplateLoopBodyNotConst => {
                (Self::TemplateLoopBodyNotConst, None, None)
            }
            InvalidTemplateStructureReason::TemplateConstLoopExpansionLimitExceeded { limit } => (
                Self::TemplateConstLoopExpansionLimitExceeded,
                None,
                Some(limit),
            ),
        }
    }

    pub(crate) fn to_reason(
        self,
        type_id: Option<TypeId>,
        limit: Option<usize>,
    ) -> Option<InvalidTemplateStructureReason> {
        Some(match self {
            Self::MissingClosingBracket => InvalidTemplateStructureReason::MissingClosingBracket,
            Self::SlotInHead => InvalidTemplateStructureReason::SlotInHead,
            Self::MissingHandlerBody => InvalidTemplateStructureReason::MissingHandlerBody,
            Self::InvalidChildDirective => InvalidTemplateStructureReason::InvalidChildDirective,
            Self::NestedTemplateNotAllowed => {
                InvalidTemplateStructureReason::NestedTemplateNotAllowed
            }
            Self::HelperInConstTemplate => InvalidTemplateStructureReason::HelperInConstTemplate,
            Self::NonFoldableConstTemplate => {
                InvalidTemplateStructureReason::NonFoldableConstTemplate
            }
            Self::NonFoldableDocComment => InvalidTemplateStructureReason::NonFoldableDocComment,
            Self::FallibleValueInTemplateHead => {
                InvalidTemplateStructureReason::FallibleValueInTemplateHead
            }
            Self::UnsupportedTypeInTemplateHead => {
                InvalidTemplateStructureReason::UnsupportedTypeInTemplateHead { type_id: type_id? }
            }
            Self::RuntimeTemplateInConst => InvalidTemplateStructureReason::RuntimeTemplateInConst,
            Self::RuntimeValueInConstTemplateHead => {
                InvalidTemplateStructureReason::RuntimeValueInConstTemplateHead
            }
            Self::EmptyPathInTemplateHead => {
                InvalidTemplateStructureReason::EmptyPathInTemplateHead
            }
            Self::IncompatibleHeadItem => InvalidTemplateStructureReason::IncompatibleHeadItem,
            Self::HelperOutsideWrapperSlot => {
                InvalidTemplateStructureReason::HelperOutsideWrapperSlot
            }
            Self::RuntimeControlFlowUnresolvedInsert => {
                InvalidTemplateStructureReason::RuntimeControlFlowUnresolvedInsert
            }
            Self::MissingCommaBeforeControlFlowSuffix => {
                InvalidTemplateStructureReason::MissingCommaBeforeControlFlowSuffix
            }
            Self::ControlFlowSuffixNotFinal => {
                InvalidTemplateStructureReason::ControlFlowSuffixNotFinal
            }
            Self::MissingTemplateIfCondition => {
                InvalidTemplateStructureReason::MissingTemplateIfCondition
            }
            Self::MissingTemplateLoopHeader => {
                InvalidTemplateStructureReason::MissingTemplateLoopHeader
            }
            Self::UnexpectedTokenAfterControlFlowSuffix => {
                InvalidTemplateStructureReason::UnexpectedTokenAfterControlFlowSuffix
            }
            Self::TemplateMatchStyleControlFlowUnsupported => {
                InvalidTemplateStructureReason::TemplateMatchStyleControlFlowUnsupported
            }
            Self::TemplateIfConditionNotConst => {
                InvalidTemplateStructureReason::TemplateIfConditionNotConst
            }
            Self::TemplateIfBranchNotConst => {
                InvalidTemplateStructureReason::TemplateIfBranchNotConst
            }
            Self::TemplateOptionCaptureConstDeferred => {
                InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred
            }
            Self::TemplateLoopRangeBoundsNotConst => {
                InvalidTemplateStructureReason::TemplateLoopRangeBoundsNotConst
            }
            Self::TemplateLoopSourceNotConst => {
                InvalidTemplateStructureReason::TemplateLoopSourceNotConst
            }
            Self::TemplateLoopConditionNotConst => {
                InvalidTemplateStructureReason::TemplateLoopConditionNotConst
            }
            Self::TemplateConditionalLoopConstTrue => {
                InvalidTemplateStructureReason::TemplateConditionalLoopConstTrue
            }
            Self::TemplateLoopBodyNotConst => {
                InvalidTemplateStructureReason::TemplateLoopBodyNotConst
            }
            Self::TemplateConstLoopExpansionLimitExceeded => {
                InvalidTemplateStructureReason::TemplateConstLoopExpansionLimitExceeded {
                    limit: limit?,
                }
            }
        })
    }
}
diagnostic_family! {
    MALFORMED_CSS_TEMPLATE {
        internal: 1004,
        stable: "MOTH-SYNTAX-0004",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Warning,
        title: "Malformed CSS template",
        facts: [
            reason: UnitFact<CssTemplateWarningCode>,
            expected: OptCharFact,
            property_name: OptStringFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: MalformedCssTemplateView,
        render: render_malformed_css_template,
    }
}

diagnostic_family! {
    MALFORMED_HTML_TEMPLATE {
        internal: 1005,
        stable: "MOTH-SYNTAX-0005",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Warning,
        title: "Malformed HTML template",
        facts: [reason: UnitFact<HtmlTemplateWarningCode>],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: MalformedHtmlTemplateView,
        render: render_malformed_html_template,
    }
}

diagnostic_family! {
    INVALID_STYLE_DIRECTIVE {
        internal: 1010,
        stable: "MOTH-SYNTAX-0010",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid style directive",
        facts: [
            directive_name: StringFact,
            supported_directives: StringFact,
        ],
        labels: [],
        extra: None,
        ctor: generated invalid_style_directive,
        view: InvalidStyleDirectiveView,
        render: render_invalid_style_directive,
    }
}

diagnostic_family! {
    INVALID_TEMPLATE_DIRECTIVE {
        internal: 1021,
        stable: "MOTH-SYNTAX-0021",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid template directive",
        facts: [
            directive_name: OptStringFact,
            reason: UnitFact<InvalidTemplateDirectiveCode>,
            detail: OptStringFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidTemplateDirectiveView,
        render: render_invalid_template_directive,
    }
}

diagnostic_family! {
    INVALID_TEMPLATE_STRUCTURE {
        internal: 1022,
        stable: "MOTH-SYNTAX-0022",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid template structure",
        facts: [
            reason: UnitFact<InvalidTemplateStructureCode>,
            type_id: OptTypeFact,
            limit: OptCountFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidTemplateStructureView,
        render: render_invalid_template_structure,
    }
}

diagnostic_family! {
    UNESCAPED_IMPLICIT_TEMPLATE_CLOSE {
        internal: 1032,
        stable: "MOTH-SYNTAX-0032",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Unescaped implicit template close",
        facts: [source_kind: UnitFact<SourceFileKind>],
        labels: [],
        extra: None,
        ctor: generated unescaped_implicit_template_close,
        view: UnescapedImplicitTemplateCloseView,
        render: render_unescaped_implicit_template_close,
    }
}

diagnostic_family! {
    INVALID_TEMPLATE_SLOT {
        internal: 3052,
        stable: "MOTH-RULE-0052",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid template slot",
        facts: [
            reason: UnitFact<InvalidTemplateSlotReason>,
            slot_name: OptStringFact,
        ],
        labels: [],
        extra: None,
        ctor: generated invalid_template_slot,
        reason_key: reason,
        view: InvalidTemplateSlotView,
        render: render_invalid_template_slot,
    }
}

/// Builds compact CSS and HTML template warnings from the legacy constructor's paired kind and reason.
pub(crate) fn malformed_template(
    kind: SyntaxDiagnosticKind,
    reason: MalformedTemplateReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    match (kind, reason) {
        (SyntaxDiagnosticKind::MalformedCssTemplate, MalformedTemplateReason::Css(reason)) => {
            malformed_css_template(reason, span)
        }
        (SyntaxDiagnosticKind::MalformedHtmlTemplate, MalformedTemplateReason::Html(reason)) => {
            malformed_html_template(reason, span)
        }
        _ => panic!("malformed-template kind and warning reason must match"),
    }
}

fn malformed_css_template(reason: CssTemplateWarning, span: Option<SourceSpan>) -> DiagnosticDraft {
    let (reason, expected, property_name) = CssTemplateWarningCode::split(reason);
    DiagnosticDraft::with_facts(&MALFORMED_CSS_TEMPLATE, span, |cursor| {
        cursor.push::<UnitFact<CssTemplateWarningCode>>(&reason)?;
        cursor.push::<OptCharFact>(&expected)?;
        cursor.push::<OptStringFact>(&property_name)?;
        Ok(())
    })
}

fn malformed_html_template(
    reason: HtmlTemplateWarning,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let reason = HtmlTemplateWarningCode::of(&reason);
    DiagnosticDraft::with_facts(&MALFORMED_HTML_TEMPLATE, span, |cursor| {
        cursor.push::<UnitFact<HtmlTemplateWarningCode>>(&reason)?;
        Ok(())
    })
}

/// Builds the compact template-directive diagnostic from its legacy reason and optional detail.
pub(crate) fn invalid_template_directive(
    directive_name: Option<StringId>,
    reason: InvalidTemplateDirectiveReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (reason, detail) = InvalidTemplateDirectiveCode::split(reason);
    DiagnosticDraft::with_facts(&INVALID_TEMPLATE_DIRECTIVE, span, |cursor| {
        cursor.push::<OptStringFact>(&directive_name)?;
        cursor.push::<UnitFact<InvalidTemplateDirectiveCode>>(&reason)?;
        cursor.push::<OptStringFact>(&detail)?;
        Ok(())
    })
}

/// Builds the compact template-structure diagnostic from its typed legacy reason.
pub(crate) fn invalid_template_structure(
    reason: InvalidTemplateStructureReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (reason, type_id, limit) = InvalidTemplateStructureCode::split(reason);
    DiagnosticDraft::with_facts(&INVALID_TEMPLATE_STRUCTURE, span, |cursor| {
        cursor.push::<UnitFact<InvalidTemplateStructureCode>>(&reason)?;
        cursor.push::<OptTypeFact>(&type_id)?;
        cursor.push::<OptCountFact>(&limit)?;
        Ok(())
    })
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    MALFORMED_CSS_TEMPLATE,
    MALFORMED_HTML_TEMPLATE,
    INVALID_STYLE_DIRECTIVE,
    INVALID_TEMPLATE_DIRECTIVE,
    INVALID_TEMPLATE_STRUCTURE,
    UNESCAPED_IMPLICIT_TEMPLATE_CLOSE,
    INVALID_TEMPLATE_SLOT,
];
/// Maps the legacy template diagnostics onto their compact constructors.
pub(super) fn from_legacy(
    kind: DiagnosticKind,
    payload: DiagnosticPayload,
    span: Option<SourceSpan>,
) -> Result<DiagnosticDraft, DiagnosticPayload> {
    match (kind, payload) {
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedCssTemplate),
            DiagnosticPayload::MalformedTemplate {
                reason: MalformedTemplateReason::Css(reason),
            },
        ) => Ok(malformed_template(
            SyntaxDiagnosticKind::MalformedCssTemplate,
            MalformedTemplateReason::Css(reason),
            span,
        )),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedHtmlTemplate),
            DiagnosticPayload::MalformedTemplate {
                reason: MalformedTemplateReason::Html(reason),
            },
        ) => Ok(malformed_template(
            SyntaxDiagnosticKind::MalformedHtmlTemplate,
            MalformedTemplateReason::Html(reason),
            span,
        )),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStyleDirective),
            DiagnosticPayload::InvalidStyleDirective {
                directive_name,
                supported_directives,
            },
        ) => Ok(invalid_style_directive(
            directive_name,
            supported_directives,
            span,
        )),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTemplateDirective),
            DiagnosticPayload::InvalidTemplateDirective {
                directive_name,
                reason,
            },
        ) => Ok(invalid_template_directive(directive_name, reason, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTemplateStructure),
            DiagnosticPayload::InvalidTemplateStructure { reason },
        ) => Ok(invalid_template_structure(reason, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnescapedImplicitTemplateClose),
            DiagnosticPayload::UnescapedImplicitTemplateClose { source_kind },
        ) => Ok(unescaped_implicit_template_close(source_kind, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTemplateSlot),
            DiagnosticPayload::InvalidTemplateSlot { reason, slot_name },
        ) => Ok(invalid_template_slot(reason, slot_name, span)),
        (_, payload) => Err(payload),
    }
}
