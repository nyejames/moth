//! Input bundle for the AST expression parser.
//!
//! WHAT: groups the token stream, scope context, type interner, expected-type
//!      hint, explicit cast target, value mode, trailing-token policy, and
//!      string table into one context struct.
//! WHY: the expression parser was threading eight or more individual arguments
//!      through every entry point. A single named input struct removes the
//!      long-argument noise, lets callers set only the fields that differ, and
//!      makes the parser's dependencies explicit.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::value_mode::ValueMode;

/// Policy that controls how the expression parser handles trailing tokens
/// such as closing delimiters and recovery boundaries.
pub(crate) struct ExpressionTrailingPolicy {
    pub(crate) consume_closing_parenthesis: bool,
    pub(crate) skip_trailing_newlines: bool,
    /// `catch` is boundary-only. Nested expression parsers use this flag to keep
    /// function arguments, collection items, and parenthesized subexpressions
    /// from silently becoming recovery boundaries.
    pub(crate) allow_boundary_catch: bool,
    /// Generic expected-result inference is also boundary-sensitive, but it is
    /// not identical to `catch`: parenthesized grouping should preserve evidence
    /// from a receiving declaration/return, while function arguments must not
    /// inherit an outer expected result type.
    pub(crate) allow_expected_result_evidence: bool,
}

/// References shared by every expression parse mode.
///
/// WHAT: keeps the parser's required handles together before a mode-specific
///      trailing policy is selected.
/// WHY: callers should choose the parse mode, not thread a long positional
///      argument list through every expression boundary.
pub(crate) struct ExpressionParseResources<'a, 'env, 'tokens> {
    pub(crate) token_stream: &'a mut AstCursor<'tokens>,
    pub(crate) scope_context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'env>,
    pub(crate) expected_type: &'a mut ExpectedType,
    pub(crate) cast_target_context: &'a mut CastTargetContext,
    pub(crate) value_mode: &'a ValueMode,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// Unified input for the central expression parser.
///
/// WHAT: carries every piece of state the expression parser needs so the
///      implementation can operate on one named value instead of a long
///      argument list.
/// WHY: `ExpectedType` and `CastTargetContext` intentionally stay separate:
///      - `ExpectedType` is for context-sensitive literals (`none`, empty `{}`)
///        so the parser can resolve types that would otherwise be ambiguous.
///      - `CastTargetContext` is for explicit `cast` target boundaries
///        supplied by typed receivers; it does not affect ordinary literal
///        resolution and is intentionally independent of expected-type hints.
pub(crate) struct ExpressionParseInput<'a, 'env, 'tokens> {
    pub(crate) token_stream: &'a mut AstCursor<'tokens>,
    pub(crate) scope_context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'env>,
    pub(crate) expected_type: &'a mut ExpectedType,
    pub(crate) cast_target_context: &'a mut CastTargetContext,
    pub(crate) value_mode: &'a ValueMode,
    pub(crate) trailing_policy: ExpressionTrailingPolicy,
    /// Stop a named-only value before an adjacent `name = value` entry so the shared list owner
    /// can report its missing comma.
    pub(crate) stop_at_named_entry: bool,
    /// Whether this entry owns a numeric deferral for its complete fragment.
    ///
    /// WHAT: true when the caller can retain the position's whole ordered syntax (statement
    ///       positions, declaration initializers, assignment RHSs, return slots, conditions,
    ///       loop headers and arguments of a retaining source-call list).
    /// WHY: a nested entry that swallowed enclosing structure must raise a non-owning deferral,
    ///      so no consumer can mistake a sub-fragment for a whole statement.
    pub(crate) numeric_deferral_owner: bool,
    /// Whether this position retains eligible numeric fragments containing *any*
    /// construction-only item.
    ///
    /// WHAT: an eligible origin boundary (an unannotated/annotated runtime declaration, or the
    ///       right-hand side owned by a pending mutation target) must keep raw literal arithmetic
    ///       unresolved until the body root has recorded every permitted use.
    /// WHY: a literal-only fragment has no unresolvable reference, so without this opt-in the
    ///      evaluator would materialise it at the language default before a later exact receiver
    ///      is known — the early default the bounded inference contract forbids.
    pub(crate) retain_pending_numeric_fragment: bool,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

impl<'a, 'env, 'tokens> ExpressionParseInput<'a, 'env, 'tokens> {
    /// Build an input with a fully custom trailing policy.
    ///
    /// WHAT: the low-level constructor used by the named helpers below.
    /// WHY: keeps field initialization in one place while the caller provides a
    ///      named resource bundle instead of another long argument list.
    pub(crate) fn new(
        resources: ExpressionParseResources<'a, 'env, 'tokens>,
        trailing_policy: ExpressionTrailingPolicy,
    ) -> Self {
        Self {
            token_stream: resources.token_stream,
            scope_context: resources.scope_context,
            type_interner: resources.type_interner,
            expected_type: resources.expected_type,
            cast_target_context: resources.cast_target_context,
            value_mode: resources.value_mode,
            trailing_policy,
            stop_at_named_entry: false,
            numeric_deferral_owner: false,
            retain_pending_numeric_fragment: false,
            string_table: resources.string_table,
            path_fork: resources.path_fork,
        }
    }

    /// Mark this entry as the owner of a numeric deferral for its complete fragment.
    ///
    /// WHAT: only the position that owned the whole expression syntax sets this, so nested
    ///       parses keep raising non-owning deferrals.
    pub(crate) fn with_numeric_deferral_owner(mut self) -> Self {
        self.numeric_deferral_owner = true;
        self
    }

    /// Mark this position as an eligible numeric origin boundary.
    ///
    /// WHAT: retains a fragment that contains any construction-only numeric item, including a
    ///       literal-only recipe with no unresolvable reference.
    /// WHY: retypeable literal arithmetic must reach the body root unresolved when a later exact
    ///      receiver may select its single static type.
    pub(crate) fn with_retained_pending_numeric_fragment(mut self) -> Self {
        self.retain_pending_numeric_fragment = true;
        self
    }

    /// Ordinary expression input: boundary catch and expected-result evidence
    /// are allowed only when the caller is not consuming a closing parenthesis.
    ///
    /// WHAT: normal receiver-boundary expression input for declarations,
    ///      assignments, returns, and other typed sites.
    pub(crate) fn ordinary(
        resources: ExpressionParseResources<'a, 'env, 'tokens>,
        consume_closing_parenthesis: bool,
    ) -> Self {
        Self::new(
            resources,
            ExpressionTrailingPolicy {
                consume_closing_parenthesis,
                skip_trailing_newlines: true,
                allow_boundary_catch: !consume_closing_parenthesis,
                allow_expected_result_evidence: !consume_closing_parenthesis,
            },
        )
    }

    /// Nested expression input: `catch` and expected-result evidence are both
    /// rejected, matching expression positions inside function arguments,
    /// collection items, and loop headers.
    ///
    /// WHAT: replaces the previous `create_expression_without_boundary_catch`
    ///      entry point.
    pub(crate) fn without_boundary_catch(
        resources: ExpressionParseResources<'a, 'env, 'tokens>,
        consume_closing_parenthesis: bool,
    ) -> Self {
        Self::new(
            resources,
            ExpressionTrailingPolicy {
                consume_closing_parenthesis,
                skip_trailing_newlines: true,
                allow_boundary_catch: false,
                allow_expected_result_evidence: false,
            },
        )
    }

    /// Call-argument value input.
    ///
    /// WHAT: boundary catch and expected-result evidence stay with the enclosing call;
    ///       `owns_numeric_deferral` marks the argument as the owner of its own complete fragment
    ///       exactly when its receiving list can retain a numeric deferral per argument.
    /// WHY: a retaining source-call argument list resumes each retained argument at the body root;
    ///      a nested parse inside the argument keeps raising a non-owning deferral, so a
    ///      sub-fragment is never mistaken for the whole argument value.
    pub(crate) fn call_argument(
        resources: ExpressionParseResources<'a, 'env, 'tokens>,
        owns_numeric_deferral: bool,
    ) -> Self {
        let input = Self::without_boundary_catch(resources, false);
        if owns_numeric_deferral {
            input.with_numeric_deferral_owner()
        } else {
            input
        }
    }

    /// Bounded expression input for stop-token parsing with boundary recovery
    /// allowed.
    ///
    /// WHAT: sets up the policy half of a `create_expression_until` call; the
    ///      stop tokens are supplied separately to the bounded parser.
    pub(crate) fn until(resources: ExpressionParseResources<'a, 'env, 'tokens>) -> Self {
        Self::new(
            resources,
            ExpressionTrailingPolicy {
                consume_closing_parenthesis: false,
                skip_trailing_newlines: true,
                allow_boundary_catch: true,
                allow_expected_result_evidence: true,
            },
        )
    }

    /// Grouped subexpression input: cast target context is deliberately erased
    /// so `(cast value)` is invalid as an operator operand, while expected-type
    /// evidence from the surrounding boundary is preserved.
    ///
    /// WHAT: replaces the inline policy construction previously duplicated in
    ///      the open-parenthesis dispatch arm.
    ///
    /// The caller must supply a `CastTargetContext` reference; passing
    /// `&mut CastTargetContext::None` is the intended usage. Newlines after the
    /// closing delimiter stay on the outer expression: the inner fragment stops
    /// at `)` and leaves the terminator for its own newline dispatch, so a
    /// trailing group ends the enclosing expression like a trailing literal.
    pub(crate) fn grouped_without_cast_target(
        resources: ExpressionParseResources<'a, 'env, 'tokens>,
    ) -> Self {
        Self::new(
            resources,
            ExpressionTrailingPolicy {
                consume_closing_parenthesis: true,
                skip_trailing_newlines: false,
                allow_boundary_catch: false,
                allow_expected_result_evidence: true,
            },
        )
    }
}
