//! Match exhaustiveness validation for AST match parsing.
//!
//! WHAT: owns the final coverage check after match headers have been parsed, plus the
//! duplicate-arm rule shared by parse-time and post-materialisation coverage.
//! WHY: full statement matches and related pattern predicates should share the
//! same choice/option/non-choice exhaustiveness rules while body construction
//! stays with the caller. A retained numeric pattern has no scalar until the body root selects
//! the subject type, so its duplicate facts are recorded when the pattern materialises.

use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredArmPattern, PendingOptionPattern,
};
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, PatternValue, PatternValueKind,
};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, NonExhaustiveMatchReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::datatypes::queries::TypeKind;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;
use moth_lexical::numeric::fixed_scalar::FixedScalarValue;
use rustc_hash::FxHashSet;

/// Exhaustiveness facts gathered while parsing accepted match arms.
///
/// WHAT: records only facts that influence final coverage, not body AST nodes.
/// WHY: callers can own their body representation while sharing one coverage
/// contract for statement match forms.
#[derive(Default)]
pub(crate) struct MatchExhaustivenessFacts {
    matched_choice_variants: FxHashSet<StringId>,
    has_guarded_arms: bool,
    option_patterns: OptionExhaustivenessFacts,
}

/// Option-specific coverage facts.
#[derive(Default)]
struct OptionExhaustivenessFacts {
    seen_unguarded_none: bool,
    seen_unguarded_present_capture: bool,
}

impl MatchExhaustivenessFacts {
    fn record_choice_variant(&mut self, variant_name: StringId) -> bool {
        self.matched_choice_variants.insert(variant_name)
    }

    fn record_guard(&mut self, guard_is_present: bool) {
        self.has_guarded_arms |= guard_is_present;
    }

    fn record_unguarded_pattern(&mut self, pattern: &MatchPattern) {
        match pattern {
            MatchPattern::OptionPresentCapture { .. } => {
                self.record_unguarded_present_capture();
            }
            MatchPattern::OptionNone { .. } => {
                self.option_patterns.seen_unguarded_none = true;
            }
            _ => {}
        }
    }

    fn record_unguarded_present_capture(&mut self) {
        self.option_patterns.seen_unguarded_present_capture = true;
    }

    /// Whether unguarded `none` and an unguarded present capture cover every optional value.
    fn option_arms_cover(&self) -> bool {
        self.option_patterns.seen_unguarded_none
            && self.option_patterns.seen_unguarded_present_capture
    }
}

/// Tracks per-arm coverage and duplicate facts while a caller parses match arms.
///
/// WHAT: stores the state needed to flag unreachable arms and later enforce
/// exhaustiveness, without owning statement body representation.
/// WHY: duplicate and catch-all handling should stay in the shared match-pattern
/// path instead of drifting between callers.
#[derive(Default)]
pub(crate) struct MatchArmCoverageTracker {
    facts: MatchExhaustivenessFacts,
    matched_unguarded_literal_patterns: FxHashSet<LiteralPatternKey>,
    seen_unguarded_none: bool,
    seen_unguarded_present_capture: bool,
}

pub(crate) struct MatchArmCoverageRecord {
    pub(crate) unreachable: bool,
}

/// Hashable key for comparing literal match patterns.
///
/// WHY: one materialised scalar keeps its exact-bit payload and scalar identity as the key, so
///      `U64` values above `i64::MAX`, signed zero and `Dec` coefficients cannot collide.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum LiteralPatternKey {
    FixedScalar(FixedScalarValue),
    Number(NumberValue),
    StringSlice(StringId),
    Bool(bool),
    Char(char),
}

impl MatchArmCoverageTracker {
    pub(crate) fn facts(&self) -> &MatchExhaustivenessFacts {
        &self.facts
    }

    /// Record one fully materialised arm.
    ///
    /// WHAT: takes an already-built pattern and guard presence so duplicate/unreachable facts use
    ///       their exact literal values.
    /// WHY: the parse path uses it for patterns whose values exist then, and the completion path
    ///      uses it again for retained numeric patterns once the subject type is selected.
    pub(crate) fn record_arm(
        &mut self,
        pattern: &MatchPattern,
        guard_is_present: bool,
        matched_choice_variant: Option<StringId>,
        const_values: &ConstValueStore,
    ) -> MatchArmCoverageRecord {
        self.record_materialised_pattern(
            pattern,
            guard_is_present,
            matched_choice_variant,
            const_values,
        )
    }

    /// Record one parsed arm whose pattern is built or retained and whose guard is only known by
    /// presence.
    ///
    /// WHY: a retained guard has no built expression to compare, and a retained numeric pattern has
    ///      no scalar until the subject type is selected, so parse-time coverage records only the
    ///      facts it can decide. The exact duplicate rule for retained patterns runs after
    ///      materialisation through [`MatchArmCoverageTracker::record_arm`].
    pub(crate) fn record_deferred_arm(
        &mut self,
        pattern: &DeferredArmPattern,
        guard_is_present: bool,
        matched_choice_variant: Option<StringId>,
        const_values: &ConstValueStore,
    ) -> MatchArmCoverageRecord {
        match pattern {
            DeferredArmPattern::Ready(pattern) => self.record_materialised_pattern(
                pattern,
                guard_is_present,
                matched_choice_variant,
                const_values,
            ),
            DeferredArmPattern::Retained(_) => {
                // Guard presence still feeds the choice-subject `else` requirement; the literal
                // duplicate policy waits for the materialised scalar.
                self.facts.record_guard(guard_is_present);
                MatchArmCoverageRecord { unreachable: false }
            }
            DeferredArmPattern::PendingChoice(_) => {
                // The declared owner names the variant before the subject instance is selected, so
                // the duplicate rule runs now, exactly as it does for a built choice pattern.
                let unreachable = matched_choice_variant
                    .is_some_and(|variant_name| !self.facts.record_choice_variant(variant_name));
                self.facts.record_guard(guard_is_present);
                MatchArmCoverageRecord { unreachable }
            }
            DeferredArmPattern::PendingOption(pending) => {
                self.record_pending_option_arm(pending, guard_is_present)
            }
        }
    }

    /// Record one pending optional arm, whose value is finished at the body root.
    ///
    /// WHAT: an unguarded present capture covers the present case; any present arm after it is
    ///       unreachable, exactly as a built option arm is.
    /// WHY: option coverage is fixed by the authored arm shape, so it is decided before the
    ///      subject's inner type is selected. Present values have no duplicate rule of their own,
    ///      matching the built option path.
    fn record_pending_option_arm(
        &mut self,
        pattern: &PendingOptionPattern,
        guard_is_present: bool,
    ) -> MatchArmCoverageRecord {
        let unreachable = self.seen_unguarded_present_capture;

        if !unreachable
            && !guard_is_present
            && matches!(pattern, PendingOptionPattern::PresentCapture(_))
        {
            self.seen_unguarded_present_capture = true;
            self.facts.record_unguarded_present_capture();
        }

        self.facts.record_guard(guard_is_present);

        MatchArmCoverageRecord { unreachable }
    }

    fn record_materialised_pattern(
        &mut self,
        pattern: &MatchPattern,
        guard_is_present: bool,
        matched_choice_variant: Option<StringId>,
        const_values: &ConstValueStore,
    ) -> MatchArmCoverageRecord {
        let option_present_arm_after_catch_all = self.seen_unguarded_present_capture
            && matches!(
                pattern,
                MatchPattern::OptionValue { .. }
                    | MatchPattern::Relational { .. }
                    | MatchPattern::OptionPresentCapture { .. }
            );

        let mut unreachable = option_present_arm_after_catch_all;

        if !unreachable {
            if let Some(variant_name) = matched_choice_variant
                && !self.facts.record_choice_variant(variant_name)
            {
                unreachable = true;
            }

            if let MatchPattern::Literal(value) = pattern
                && let Some(key) = extract_literal_key(value, const_values)
            {
                if self.matched_unguarded_literal_patterns.contains(&key) {
                    unreachable = true;
                } else if !guard_is_present {
                    self.matched_unguarded_literal_patterns.insert(key);
                }
            }

            match pattern {
                MatchPattern::OptionPresentCapture { .. } if !guard_is_present => {
                    self.seen_unguarded_present_capture = true;
                    self.facts.record_unguarded_pattern(pattern);
                }

                MatchPattern::OptionNone { .. } if !guard_is_present => {
                    if self.seen_unguarded_none {
                        unreachable = true;
                    }

                    self.seen_unguarded_none = true;
                    self.facts.record_unguarded_pattern(pattern);
                }

                _ => {}
            }
        }

        self.facts.record_guard(guard_is_present);

        MatchArmCoverageRecord { unreachable }
    }
}

/// The literal identity of one pattern value, read from its constant content.
///
/// WHAT: a constant scalar, bool, char or plain text keys its literal; a runtime value has no
///       compile-time content, so it never takes part in duplicate-literal detection.
/// WHY: literal arms compare exact constant content, so two occurrences of one value share a key.
fn extract_literal_key(
    value: &PatternValue,
    const_values: &ConstValueStore,
) -> Option<LiteralPatternKey> {
    let PatternValueKind::Constant(id) = value.kind else {
        return None;
    };
    match const_values.payload(id)? {
        ConstValuePayload::FixedScalar(scalar) => Some(LiteralPatternKey::FixedScalar(*scalar)),
        ConstValuePayload::Number(number) => Some(LiteralPatternKey::Number(number.clone())),
        ConstValuePayload::String(ConstStringValue::Text(text)) => {
            Some(LiteralPatternKey::StringSlice(*text))
        }
        ConstValuePayload::Bool(bool_value) => Some(LiteralPatternKey::Bool(*bool_value)),
        ConstValuePayload::Char(char_value) => Some(LiteralPatternKey::Char(*char_value)),
        _ => None,
    }
}

/// The match subject an exhaustiveness check runs against.
///
/// WHAT: the subject's exact type when it is already selected, the declared owner of a pending
///       generic choice, the inner shape of a pending generic optional, or the authored span of a
///       pending numeric origin.
/// WHY: a pending numeric subject can never be a choice or an option, so only the non-choice
///      `else` requirement applies; it still fires at parse time from the authored arms. A pending
///      generic choice fixes its variant set from its declared owner, so it uses the choice rule.
pub(crate) enum MatchExhaustivenessSubject {
    /// The subject's exact type is already selected; its authored span is the subject span.
    Known {
        type_id: TypeId,
        span: Option<SourceSpan>,
    },
    /// The subject is a pending numeric origin; only its authored span exists yet.
    PendingNumeric { span: Option<SourceSpan> },
    /// The subject is a pending generic choice whose declared owner fixes its variant set; the
    /// instance is selected at the body root.
    PendingChoice {
        declared_type_id: TypeId,
        span: Option<SourceSpan>,
    },
    /// The subject is a pending generic optional; its inner shape is retained and its option
    /// coverage is fixed by the authored arms, so `none` and an unguarded capture decide it.
    PendingOption { span: Option<SourceSpan> },
}

pub(crate) struct MatchExhaustivenessCheck<'a> {
    pub(crate) subject: MatchExhaustivenessSubject,
    pub(crate) has_default: bool,
    pub(crate) facts: &'a MatchExhaustivenessFacts,
    pub(crate) type_environment: &'a TypeEnvironment,
}

/// Verify that a match statement covers all possible values.
///
/// WHAT: for choice scrutinees, checks that every declared variant has an arm or an
/// `else` fallback exists; for non-choice types, requires an explicit `else =>` arm.
/// WHY: exhaustiveness at parse time prevents silent fallthrough bugs and gives users
/// actionable diagnostics listing the specific missing variants.
pub(crate) fn enforce_match_exhaustiveness(
    check: MatchExhaustivenessCheck<'_>,
) -> Result<(), CompilerDiagnostic> {
    let subject_span = match &check.subject {
        MatchExhaustivenessSubject::Known { span, .. } => *span,
        MatchExhaustivenessSubject::PendingChoice { span, .. }
        | MatchExhaustivenessSubject::PendingNumeric { span }
        | MatchExhaustivenessSubject::PendingOption { span } => *span,
    };

    // A pending generic choice fixes its variant set from its declared owner before the subject
    // instance is selected, so it shares the choice rule with a known choice subject.
    if let MatchExhaustivenessSubject::PendingChoice {
        declared_type_id, ..
    } = &check.subject
    {
        return enforce_choice_variants(*declared_type_id, subject_span, &check);
    }

    // A pending generic optional fixes its option coverage from the authored arms: `none` and an
    // unguarded present capture cover every case, and `else` covers the rest.
    if matches!(
        &check.subject,
        MatchExhaustivenessSubject::PendingOption { .. }
    ) {
        if check.has_default || check.facts.option_arms_cover() {
            return Ok(());
        }

        return Err(CompilerDiagnostic::non_exhaustive_match(
            NonExhaustiveMatchReason::MissingOptionPatterns,
            vec![],
            subject_span,
        ));
    }

    if let MatchExhaustivenessSubject::Known { type_id, .. } = &check.subject {
        let subject_type_id = *type_id;
        let is_choice = matches!(
            check.type_environment.type_kind(subject_type_id),
            Some(TypeKind::Choice | TypeKind::GenericInstance)
        );

        if is_choice {
            return enforce_choice_variants(subject_type_id, subject_span, &check);
        }

        if check.has_default {
            return Ok(());
        }

        // For optional scrutinees, unguarded `none` + unguarded `|name|` covers all cases.
        let is_option = check
            .type_environment
            .option_inner_type(subject_type_id)
            .is_some();
        if is_option && check.facts.option_arms_cover() {
            return Ok(());
        }

        let reason = if is_option {
            NonExhaustiveMatchReason::MissingOptionPatterns
        } else {
            NonExhaustiveMatchReason::MissingElseArm
        };

        return Err(CompilerDiagnostic::non_exhaustive_match(
            reason,
            vec![],
            subject_span,
        ));
    }

    // A pending numeric origin can never be a choice or an option, and its domain is unbounded:
    // only an explicit `else` arm makes the match exhaustive.
    if check.has_default {
        return Ok(());
    }

    Err(CompilerDiagnostic::non_exhaustive_match(
        NonExhaustiveMatchReason::MissingElseArm,
        vec![],
        subject_span,
    ))
}

/// Verify the choice rule for a subject whose variant set is fixed.
///
/// WHAT: accepts an explicit `else`; otherwise rejects guarded arms and reports every declared
/// variant that no arm names.
/// WHY: a known choice and a pending generic choice share one variant set, so they share one rule.
fn enforce_choice_variants(
    subject_type_id: TypeId,
    subject_span: Option<SourceSpan>,
    check: &MatchExhaustivenessCheck<'_>,
) -> Result<(), CompilerDiagnostic> {
    // `else` intentionally acts as an explicit "future variants" fallback in Alpha.
    if check.has_default {
        return Ok(());
    }

    if check.facts.has_guarded_arms {
        return Err(CompilerDiagnostic::non_exhaustive_match(
            NonExhaustiveMatchReason::GuardedArmsRequireElse,
            vec![],
            subject_span,
        ));
    }

    let missing_variants: Vec<StringId> = check
        .type_environment
        .variants_for(subject_type_id)
        .map(|variants| {
            variants
                .iter()
                .filter(|variant| !check.facts.matched_choice_variants.contains(&variant.name))
                .map(|variant| variant.name)
                .collect()
        })
        .unwrap_or_default();

    if missing_variants.is_empty() {
        return Ok(());
    }

    Err(CompilerDiagnostic::non_exhaustive_match(
        NonExhaustiveMatchReason::MissingVariants,
        missing_variants,
        subject_span,
    ))
}
