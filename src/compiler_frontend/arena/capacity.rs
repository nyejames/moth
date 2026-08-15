//! Frontend typed-arena capacity estimates.
//!
//! WHAT: turns cheap token/header stats and existing fragment counts into conservative,
//!       capped capacity guesses for future typed `Vec` arenas.
//! WHY: capacity is policy-only. These estimates must never affect diagnostics, ordering,
//!      lowering, type identity, or emitted artifacts.

use crate::compiler_frontend::arena::{HeaderStats, TokenStats};

/// Hard per-field capacity ceiling.
///
/// WHAT: an intentionally large upper bound to prevent a single pathological input from
///       producing nonsensical `Vec::with_capacity` sizes while still allowing generous growth.
const HARD_CAPACITY_CAP: usize = 1_000_000;

/// Modest over-allocation factor applied to estimates that are expected to grow during lowering.
///
/// WHAT: multiplies base estimates by 3/2 to reduce reallocations for nested scopes,
///       temporaries, and template sub-pieces.
const OVER_ALLOCATION_NUMERATOR: usize = 3;
const OVER_ALLOCATION_DENOMINATOR: usize = 2;

/// Conservative capacity estimates for frontend arenas.
///
/// WHAT: a policy-only bundle of initial `Vec` capacities. Fields are public so later phases can
///       read the subset they need, but only the fields required by the current phase are wired
///       into counters today.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrontendArenaCapacityEstimate {
    pub scope_frames: usize,
    pub declarations: usize,
    pub expressions: usize,
    pub expression_items: usize,
    pub statements: usize,
    pub templates: usize,
    pub template_atoms: usize,
    pub render_pieces: usize,
    pub hir_blocks: usize,
    pub hir_statements: usize,
    pub hir_expressions: usize,
    pub borrow_facts: usize,
    /// Number of estimate fields that hit `HARD_CAPACITY_CAP`.
    ///
    /// WHAT: a diagnostic signal that an input is large enough to saturate a heuristic.
    /// WHY: helps distinguish "estimate is huge because input is huge" from formula bugs.
    pub capped_field_count: usize,
}

impl FrontendArenaCapacityEstimate {
    /// Build a capacity estimate from module-level frontend facts.
    ///
    /// WHAT: applies conservative, capped formulas to cheap stats already gathered during
    ///       tokenization and header aggregation.
    /// WHY: keeps all heuristic policy in one place with a short rationale comment per field.
    pub(crate) fn new(
        source_file_count: usize,
        source_byte_count: usize,
        token_stats: TokenStats,
        header_stats: HeaderStats,
        const_fragment_count: usize,
        runtime_fragment_count: usize,
    ) -> Self {
        let mut estimate = Self::default();
        let mut capped_count = 0usize;

        // Byte volume is a modest fallback for source kinds or generated declarations where the
        // token/header counts are intentionally small compared with the input size.
        let source_kib = source_byte_count / 1024;

        // Scope frames: root contexts come from files and function-like declarations, while
        // executable body frames come mostly from explicit control-flow/template syntax. Header
        // structure adds a small pressure signal for generated signature/variant work, and token
        // volume remains the fallback for body-local declarations that headers cannot see.
        let scope_base = source_file_count
            .saturating_add(header_stats.functions)
            .saturating_add(header_stats.start_functions)
            .saturating_add(header_stats.const_templates);
        let scope_control_frames = token_stats
            .if_tokens
            .saturating_mul(2)
            .saturating_add(token_stats.loop_tokens.saturating_mul(2))
            .saturating_add(token_stats.catch_tokens.saturating_mul(2))
            .saturating_add(token_stats.then_tokens / 2)
            .saturating_add(token_stats.template_markers / 3);
        let scope_header_pressure = header_stats
            .signature_members
            .saturating_add(header_stats.choice_variants)
            .saturating_add(header_stats.generic_parameters)
            / 8;
        let scope_from_tokens = token_stats.total_tokens / 18;
        let scope_from_bytes = source_kib / 16;
        estimate.scope_frames = capped(
            over_allocate(
                scope_base
                    .saturating_add(scope_control_frames)
                    .saturating_add(scope_header_pressure)
                    .saturating_add(scope_from_tokens)
                    .saturating_add(scope_from_bytes),
            ),
            HARD_CAPACITY_CAP,
            &mut capped_count,
        );

        // Declarations: all top-level declaration-like headers plus dependency clauses.
        // This seeds an arena for named top-level bindings.
        estimate.declarations = capped(
            header_stats
                .functions
                .saturating_add(header_stats.constants)
                .saturating_add(header_stats.structs)
                .saturating_add(header_stats.choices)
                .saturating_add(header_stats.type_aliases)
                .saturating_add(header_stats.traits)
                .saturating_add(header_stats.conformances)
                .saturating_add(header_stats.trait_incompatibilities)
                .saturating_add(header_stats.const_templates)
                .saturating_add(header_stats.start_functions)
                .saturating_add(header_stats.dependency_clauses),
            HARD_CAPACITY_CAP,
            &mut capped_count,
        );

        // Expressions: rough guess that about one in four tokens becomes an expression node.
        // Source byte volume adds a small fallback for large non-token-heavy content assets.
        // Expression items (sub-expressions inside a node) scale with that.
        let expression_base =
            std::cmp::max(1, token_stats.total_tokens / 4).saturating_add(source_kib);
        estimate.expressions = capped(
            over_allocate(expression_base),
            HARD_CAPACITY_CAP,
            &mut capped_count,
        );
        estimate.expression_items = capped(
            over_allocate(estimate.expressions.saturating_mul(2)),
            HARD_CAPACITY_CAP,
            &mut capped_count,
        );

        // Statements: roughly half as many statement nodes as expression nodes in typical code.
        estimate.statements = capped(
            over_allocate(std::cmp::max(1, token_stats.total_tokens / 8)),
            HARD_CAPACITY_CAP,
            &mut capped_count,
        );

        // Templates: const templates, runtime entry fragments, plus an allowance for template
        // markers (heads/bodies/closes). This seeds template and render-plan arenas.
        let template_base = header_stats
            .const_templates
            .saturating_add(runtime_fragment_count)
            .saturating_add(const_fragment_count);
        let template_from_markers = token_stats.template_markers / 4;
        estimate.templates = capped(
            over_allocate(template_base.saturating_add(template_from_markers)),
            HARD_CAPACITY_CAP,
            &mut capped_count,
        );
        estimate.template_atoms = capped(
            over_allocate(estimate.templates.saturating_mul(4)),
            HARD_CAPACITY_CAP,
            &mut capped_count,
        );
        estimate.render_pieces = capped(
            over_allocate(estimate.templates.saturating_mul(2)),
            HARD_CAPACITY_CAP,
            &mut capped_count,
        );

        // HIR nodes: scale with declarations and statements/expressions. Blocks are much rarer
        // than statements because control flow introduces them.
        let hir_blocks_base = header_stats
            .functions
            .saturating_add(header_stats.start_functions)
            .saturating_add(source_file_count);
        let hir_blocks_from_tokens = token_stats.total_tokens / 64;
        estimate.hir_blocks = capped(
            over_allocate(hir_blocks_base.saturating_add(hir_blocks_from_tokens)),
            HARD_CAPACITY_CAP,
            &mut capped_count,
        );
        estimate.hir_statements = capped(estimate.statements, HARD_CAPACITY_CAP, &mut capped_count);
        estimate.hir_expressions =
            capped(estimate.expressions, HARD_CAPACITY_CAP, &mut capped_count);

        // Borrow facts: each statement and expression can produce multiple borrow facts, and
        // each function adds function-level metadata.
        let borrow_facts_base = header_stats
            .functions
            .saturating_add(header_stats.start_functions)
            .saturating_mul(4);
        estimate.borrow_facts = capped(
            over_allocate(
                estimate
                    .hir_statements
                    .saturating_add(estimate.hir_expressions)
                    .saturating_add(borrow_facts_base),
            ),
            HARD_CAPACITY_CAP,
            &mut capped_count,
        );

        estimate.capped_field_count = capped_count;
        estimate
    }
}

/// Multiply a base estimate by the configured over-allocation factor.
fn over_allocate(base: usize) -> usize {
    base.saturating_mul(OVER_ALLOCATION_NUMERATOR)
        .div_ceil(OVER_ALLOCATION_DENOMINATOR)
}

/// Apply a hard cap and record when the cap is reached.
fn capped(value: usize, cap: usize, capped_count: &mut usize) -> usize {
    if value >= cap {
        *capped_count += 1;
        cap
    } else {
        value
    }
}
