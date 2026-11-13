//! Live interpreter state and the borrowed service context for const execution.
//!
//! WHAT: owns the per-attempt interpreter state (frame aliases, prepared catch bodies, active and
//!       inherited handler recovery, nesting depths and the pending handler return) and bundles
//!       the compiler services every operation reads.
//! WHY: one context replaces long positional parameter lists, while resource limits stay in
//!       `ExecutionBudget` and frame bindings stay explicit per function frame. Separating state
//!       from the budget keeps "how much work may this attempt spend" distinct from "what is the
//!       interpreter currently doing".
//! MUST NOT: define instruction semantics, budget limits, value projection or eligibility.

use super::budget::ExecutionBudget;
use super::instructions::ConstInstruction;
use super::{AliasTarget, ConstFunctionExecutable, ConstValueId, ExprId, FxHashMap, PathId, Rc};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Interpreted names bound in one function frame or handler body.
///
/// Bindings belong to a frame rather than to the whole interpreter: a call, a handler body and a
/// nested template site each supply their own name-to-value map.
pub(super) type FrameBindings = FxHashMap<PathId, ConstValueId>;

/// Borrowed compiler services required by one const-execution attempt.
///
/// The rewrite entry points build this bundle instead of repeating its field list, and
/// `rewrite_required_const_calls` accepts it so its signature stays stable as the interpreter
/// grows. The caller creates the attempt budget and interpreter state.
pub(crate) struct ConstExecutionServices<'a> {
    pub(crate) builder: &'a mut AstExpressionBuilder,
    pub(crate) constants: &'a mut ConstValueStore,
    pub(crate) executables: &'a FxHashMap<PathId, ConstFunctionExecutable>,
    pub(crate) scope: &'a ScopeContext,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) types: &'a TypeEnvironment,
    pub(crate) path_fork: &'a PathInternerFork,
}

impl<'services> ConstExecutionServices<'services> {
    /// Borrow these services for one attempt with the caller's budget and state.
    ///
    /// The services are reborrowed, so one bundle can start several attempts (release rewriting
    /// starts one per expression) without rebuilding the field list.
    pub(super) fn interpreter<'attempt>(
        &'attempt mut self,
        budget: &'attempt mut ExecutionBudget,
        state: &'attempt mut ExecutionState,
    ) -> ConstInterpreter<'attempt>
    where
        'services: 'attempt,
    {
        ConstInterpreter {
            builder: &mut *self.builder,
            constants: &mut *self.constants,
            executables: self.executables,
            scope: self.scope,
            string_table: &mut *self.string_table,
            types: self.types,
            path_fork: self.path_fork,
            budget,
            state,
        }
    }
}

/// The executor's shared services plus one attempt's resource budget and interpreter state.
///
/// WHAT: the one value threaded through expression evaluation, the frame machine, place handling,
///       rewriting and template folding.
/// WHY: every operation needs the same eight compiler services. Passing them positionally made
///      call sites unreadable and hid which state was frame-local.
pub(super) struct ConstInterpreter<'a> {
    pub(super) builder: &'a mut AstExpressionBuilder,
    pub(super) constants: &'a mut ConstValueStore,
    pub(super) executables: &'a FxHashMap<PathId, ConstFunctionExecutable>,
    pub(super) scope: &'a ScopeContext,
    pub(super) string_table: &'a mut StringTable,
    pub(super) types: &'a TypeEnvironment,
    pub(super) path_fork: &'a PathInternerFork,
    pub(super) budget: &'a mut ExecutionBudget,
    pub(super) state: &'a mut ExecutionState,
}

/// Interpreter control, recovery and frame state for one const-execution attempt.
#[derive(Default)]
pub(super) struct ExecutionState {
    /// Mutable aliases in the active function frame; values are canonical roots plus projections.
    pub(super) aliases: FxHashMap<PathId, AliasTarget>,
    /// Prepared catch bodies for the active function frame.
    pub(super) handler_bodies: Option<Rc<FxHashMap<ExprId, Rc<[ConstInstruction]>>>>,
    /// Prepared catch bodies carried to catch expressions a rewrite rebuilt under a fresh `ExprId`.
    ///
    /// WHY: a rewrite replaces folded children, so a catch whose handled value or arguments folded
    /// is rebuilt under a new id. Completion can evaluate that rebuilt catch (a successful
    /// collection read under an arithmetic catch is enough), and the handler compiled for the
    /// authored expression must follow it. The entry is registered with the rebuild, so the
    /// identity travels with the expression instead of being guessed from an authored span.
    pub(super) rewritten_handlers: FxHashMap<ExprId, Rc<[ConstInstruction]>>,
    /// Catch handler currently available to collection failures.
    ///
    /// This is not owned by one collection operation: a larger handled expression installs its
    /// handler while its handled value is evaluated, so nested collection failures can inherit it.
    pub(super) active_handler: ActiveHandler,
    /// Nested selected-handler execution depth.
    pub(super) handler_depth: usize,
    /// Current expression evaluation nesting depth, used to consume inherited recovery once.
    pub(super) expression_depth: usize,
    /// True when an inherited catch handler completed, including a handler with no value.
    pub(super) inherited_handler_recovered: bool,
    /// Recovery value produced by an inherited catch handler; the whole handled expression
    /// yields this value instead of the partially evaluated child expression.
    pub(super) inherited_handler_value: Option<ConstValueId>,
    /// A `return` executed by a selected catch handler, awaiting its owning function boundary.
    ///
    /// WHY this is a slot rather than a returned value: a handler body runs from inside expression
    /// evaluation and the expression rewriter, which yield a value or a rewritten expression to
    /// their caller. Its `return` must abandon the whole enclosing statement, not merely produce
    /// the handler's recovery value, and the template callback trait
    /// (`RequiredCallSiteRewriter`) has no channel for an early-exit variant. The pending values
    /// therefore stay visible until `execute_frames` in `FrameMode::FunctionBody` consumes them
    /// (or `execute_body` does after handler instructions run); nested handler frames only peek
    /// so the return can cross them, and expression evaluation keeps returning its first value so
    /// no enclosing expression continues past the handled statement.
    pub(super) pending_handler_return: Option<Vec<ConstValueId>>,
    /// Set once an escaping diagnostic has received its call-site labels.
    pub(super) call_chain_attached: bool,
}

/// The catch handler currently available to fallible collection operations.
///
/// This value is swapped wholesale while an inner operation runs, so a saved copy restores the
/// body, error binding and boundary together.
#[derive(Clone, Default)]
pub(super) struct ActiveHandler {
    /// Body of the handler selected by the nearest enclosing catch.
    pub(super) body: Option<Rc<[ConstInstruction]>>,
    /// Error binding inherited with `body`.
    pub(super) error_binding: Option<PathId>,
    /// True when this handler owns a nested catch boundary rather than the whole expression being
    /// evaluated.
    pub(super) boundary: bool,
}

impl ActiveHandler {
    /// Replace the handler body and error binding, leaving the boundary flag untouched.
    pub(super) fn replace(
        state: &mut ExecutionState,
        body: Option<Rc<[ConstInstruction]>>,
        error_binding: Option<PathId>,
    ) -> Self {
        let boundary = state.active_handler.boundary;
        std::mem::replace(
            &mut state.active_handler,
            ActiveHandler {
                body,
                error_binding,
                boundary,
            },
        )
    }
}
