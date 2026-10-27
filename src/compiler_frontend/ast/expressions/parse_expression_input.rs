//! Input bundle for the AST expression parser.
//!
//! Parsing explicitly reborrows the phase expression and constant owners alongside lexical,
//! canonical type and source services. Recursive entries share that domain; scopes never retain
//! a builder handle. Receiving hints and cast targets remain separate from trailing-token policy.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::value_mode::ValueMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RouteArgumentContext {
    pub(crate) output_type: TypeId,
    pub(crate) input_type: Option<TypeId>,
}

impl RouteArgumentContext {
    pub(crate) fn matches_output_type(
        self,
        actual_output_type: TypeId,
        type_environment: &TypeEnvironment,
    ) -> bool {
        self.output_type == actual_output_type
            || type_environment.contains_generic_parameter(self.output_type)
            || type_environment.contains_generic_parameter(actual_output_type)
            || matches!(
                (
                    type_environment.get(actual_output_type),
                    type_environment.get(self.output_type),
                ),
                (
                    Some(TypeDefinition::Choice(actual_choice)),
                    Some(TypeDefinition::GenericInstance(expected_instance)),
                ) if actual_choice.generic_parameters.is_some()
                    && type_environment.nominal_path_by_id(expected_instance.base)
                        == Some(&actual_choice.path)
            )
    }

    pub(crate) fn matches_input_type(
        self,
        actual_input_type: Option<TypeId>,
        type_environment: &TypeEnvironment,
    ) -> bool {
        match (self.input_type, actual_input_type) {
            (None, None) => true,
            (Some(expected), Some(actual)) => {
                expected == actual
                    || type_environment.contains_generic_parameter(expected)
                    || type_environment.contains_generic_parameter(actual)
            }
            (None, Some(_)) | (Some(_), None) => false,
        }
    }

    pub(crate) fn matches_contract(
        self,
        actual_output_type: TypeId,
        actual_input_type: Option<TypeId>,
        type_environment: &TypeEnvironment,
    ) -> bool {
        self.matches_output_type(actual_output_type, type_environment)
            && self.matches_input_type(actual_input_type, type_environment)
    }
}

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
/// Each entry borrows construction owners for its parsing interval. Nested parses reborrow these
/// resources and complete any expression escaping into TIR before installing its retained ID.
pub(crate) struct ExpressionParseResources<'a, 'env, 'tokens> {
    pub(crate) token_stream: &'a mut AstCursor<'tokens>,
    pub(crate) scope_context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'env>,
    pub(crate) expression_builder: &'a mut AstExpressionBuilder,
    pub(crate) const_values: &'a mut ConstValueStore,
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
    pub(crate) expression_builder: &'a mut AstExpressionBuilder,
    pub(crate) const_values: &'a mut ConstValueStore,
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
    pub(crate) route_argument_context: Option<RouteArgumentContext>,
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
            expression_builder: resources.expression_builder,
            const_values: resources.const_values,
            expected_type: resources.expected_type,
            cast_target_context: resources.cast_target_context,
            value_mode: resources.value_mode,
            trailing_policy,
            stop_at_named_entry: false,
            numeric_deferral_owner: false,
            retain_pending_numeric_fragment: false,
            route_argument_context: None,
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

    pub(crate) fn with_route_argument_context(
        mut self,
        route_argument_context: RouteArgumentContext,
    ) -> Self {
        self.route_argument_context = Some(route_argument_context);
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
}
