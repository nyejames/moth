//! Match exhaustiveness validation for AST match parsing.
//!
//! WHAT: owns the final coverage check after match headers have been parsed.
//! WHY: full statement matches and related pattern predicates should share the
//! same choice/option/non-choice exhaustiveness rules while body construction
//! stays with the caller.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, NonExhaustiveMatchReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::queries::TypeKind;
use crate::compiler_frontend::symbols::string_interning::StringId;
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
                self.option_patterns.seen_unguarded_present_capture = true;
            }
            MatchPattern::OptionNone { .. } => {
                self.option_patterns.seen_unguarded_none = true;
            }
            _ => {}
        }
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
    seen_unconditional_capture: bool,
    seen_unguarded_none: bool,
    seen_unguarded_present_capture: bool,
}

pub(crate) struct MatchArmCoverageRecord {
    pub(crate) unreachable: bool,
}

/// Hashable key for comparing literal match patterns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum LiteralPatternKey {
    Int(i32),
    Float(u64),
    StringSlice(StringId),
    Bool(bool),
    Char(char),
}

impl MatchArmCoverageTracker {
    pub(crate) fn facts(&self) -> &MatchExhaustivenessFacts {
        &self.facts
    }

    pub(crate) fn default_after_unconditional_capture_is_unreachable(&self) -> bool {
        self.seen_unconditional_capture
    }

    pub(crate) fn record_arm(
        &mut self,
        pattern: &MatchPattern,
        guard: Option<&Expression>,
        matched_choice_variant: Option<StringId>,
    ) -> MatchArmCoverageRecord {
        let guard_is_present = guard.is_some();
        let option_present_arm_after_catch_all = self.seen_unguarded_present_capture
            && matches!(
                pattern,
                MatchPattern::OptionValue { .. }
                    | MatchPattern::Relational { .. }
                    | MatchPattern::OptionPresentCapture { .. }
            );

        let mut unreachable = self.seen_unconditional_capture || option_present_arm_after_catch_all;

        if !unreachable {
            if let Some(variant_name) = matched_choice_variant
                && !self.facts.record_choice_variant(variant_name)
            {
                unreachable = true;
            }

            if let MatchPattern::Literal(expression) = pattern
                && let Some(key) = extract_literal_key(expression)
            {
                if self.matched_unguarded_literal_patterns.contains(&key) {
                    unreachable = true;
                } else if guard.is_none() {
                    self.matched_unguarded_literal_patterns.insert(key);
                }
            }

            match pattern {
                MatchPattern::Capture { .. } if guard.is_none() => {
                    self.seen_unconditional_capture = true;
                }

                MatchPattern::OptionPresentCapture { .. } if guard.is_none() => {
                    self.seen_unguarded_present_capture = true;
                    self.facts.record_unguarded_pattern(pattern);
                }

                MatchPattern::OptionNone { .. } if guard.is_none() => {
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

fn extract_literal_key(expression: &Expression) -> Option<LiteralPatternKey> {
    match &expression.kind {
        ExpressionKind::Int(value) => Some(LiteralPatternKey::Int(*value)),
        ExpressionKind::Float(value) => Some(LiteralPatternKey::Float(value.to_bits())),
        ExpressionKind::StringSlice(id) => Some(LiteralPatternKey::StringSlice(*id)),
        ExpressionKind::Bool(value) => Some(LiteralPatternKey::Bool(*value)),
        ExpressionKind::Char(value) => Some(LiteralPatternKey::Char(*value)),
        _ => None,
    }
}

pub(crate) struct MatchExhaustivenessCheck<'a> {
    pub(crate) scrutinee: &'a Expression,
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
) -> Result<(), Box<CompilerDiagnostic>> {
    let is_choice = matches!(
        check.type_environment.type_kind(check.scrutinee.type_id),
        Some(TypeKind::Choice | TypeKind::GenericInstance)
    );

    if is_choice {
        // `else` intentionally acts as an explicit "future variants" fallback in Alpha.
        if check.has_default {
            return Ok(());
        }

        if check.facts.has_guarded_arms {
            return Err(Box::new(CompilerDiagnostic::non_exhaustive_match(
                NonExhaustiveMatchReason::GuardedArmsRequireElse,
                vec![],
                check.scrutinee.location.clone(),
            )));
        }

        let missing_variants: Vec<StringId> = check
            .type_environment
            .variants_for(check.scrutinee.type_id)
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

        return Err(Box::new(CompilerDiagnostic::non_exhaustive_match(
            NonExhaustiveMatchReason::MissingVariants,
            missing_variants,
            check.scrutinee.location.clone(),
        )));
    }

    if check.has_default {
        return Ok(());
    }

    // For optional scrutinees, unguarded `none` + unguarded `|name|` covers all cases.
    let is_option = check
        .type_environment
        .option_inner_type(check.scrutinee.type_id)
        .is_some();
    if is_option
        && check.facts.option_patterns.seen_unguarded_none
        && check.facts.option_patterns.seen_unguarded_present_capture
    {
        return Ok(());
    }

    let reason = if is_option {
        NonExhaustiveMatchReason::MissingOptionPatterns
    } else {
        NonExhaustiveMatchReason::MissingElseArm
    };

    Err(Box::new(CompilerDiagnostic::non_exhaustive_match(
        reason,
        vec![],
        check.scrutinee.location.clone(),
    )))
}
