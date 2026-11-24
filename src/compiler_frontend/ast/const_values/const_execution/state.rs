//! Live interpreter state and the borrowed service context for const execution.
//!
//! WHAT: owns per-attempt interpreter state (frame aliases, prepared catch bodies, active handler,
//!       pending frame outcome and nesting depths) and bundles
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
    /// Catch handler currently available to collection failures.
    ///
    /// This is not owned by one collection operation: a larger handled expression installs its
    /// handler while its handled value is evaluated, so nested collection failures can inherit it.
    pub(super) active_handler: ActiveHandler,
    /// Nested selected-handler execution depth.
    pub(super) handler_depth: usize,
    /// Current expression evaluation nesting depth across every running body, bounding native
    /// recursion.
    pub(super) expression_depth: usize,
    /// An outcome from a handler reached through expression evaluation rather than a frame step.
    /// `Recovered` belongs to the protected expression; other outcomes belong to the frame
    /// machine. An empty recovery is still an outcome.
    pub(super) outcome: Option<FrameOutcome>,
    /// Set once an escaping diagnostic has received its call-site labels.
    pub(super) call_chain_attached: bool,
    /// Compact identity of the executable body currently running, if any.
    ///
    /// WHY: a body's donor handle lives in the executables table. Keeping its path here avoids
    ///      cloning an Arc-backed handle on every successful call; diagnostics resolve the handle
    ///      when they need to own it.
    pub(super) current_body: Option<PathId>,
}

pub(super) enum FrameOutcome {
    Recovered(Vec<ConstValueId>),
    Return(Vec<ConstValueId>),
    Break,
    Continue,
}
/// The catch handler currently available to fallible collection operations.
///
/// This value is swapped wholesale while an inner operation runs, so a saved copy restores the
/// body and its error binding together.
#[derive(Clone, Default)]
pub(super) struct ActiveHandler {
    /// Body of the handler selected by the nearest enclosing catch.
    pub(super) body: Option<Rc<[ConstInstruction]>>,
    /// Error binding inherited with `body`.
    pub(super) error_binding: Option<PathId>,
}

impl ActiveHandler {
    /// Install a handler for the duration of a protected expression.
    pub(super) fn replace(
        state: &mut ExecutionState,
        body: Option<Rc<[ConstInstruction]>>,
        error_binding: Option<PathId>,
    ) -> Self {
        std::mem::replace(
            &mut state.active_handler,
            ActiveHandler {
                body,
                error_binding,
            },
        )
    }
}
