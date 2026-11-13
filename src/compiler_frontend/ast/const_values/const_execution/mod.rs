//! Bounded execution of prepared const functions at required call sites.
//!
//! WHAT: owns the executable row, the preparation entry points and this module map. Sibling
//!       modules compile bodies, rewrite source calls, select release candidates and run the
//!       bounded typed-AST frame machine over `ConstValueStore` values.
//! WHY: constant initializers and required calls fold during environment construction, before
//!      HIR exists. The typed AST body and the canonical expression store are the only available
//!      representation at that boundary, so execution stays over that typed vocabulary and never
//!      starts a second parser or a second template engine.
//! MUST NOT: publish partial results, evaluate runtime-only operations, or add a second expression
//!           parser.
//!
//! Architecture decision: required const calls fold during Stage 4 before HIR exists. Stage 5
//! lowers HIR only after the completed AST is available, so an early HIR executor would require
//! a parallel lowering pipeline or a retained incomplete environment. This module tree is
//! therefore the canonical typed-AST execution boundary: it consumes typed expressions and
//! `ConstValueStore` values, and delegates numeric, cast, Text and template semantics to their
//! existing owners.
//!
//! ## Module map
//!
//! - `eligibility`: the single operation eligibility policy. It validates prepared bodies, proves
//!   release candidates pure and scalar, and removes cyclic or incomplete candidate graphs.
//! - `rewrite`: required and release AST expression rewriting, call-target discovery and
//!   provenance transfer. `rewrite_required_const_calls` and `rewrite_release_calls` are the
//!   module-tree entry points for the expression evaluator and finalizer.
//! - `instructions`: the compiled instruction vocabulary and declaration-time body compiler.
//! - `labels`: the operation names shared by classification and execution refusals.
//! - `state`: the interpreter's borrowed service context, live execution state and handler
//!   snapshots.
//! - `execute`: the one executed-frame machine, including expression evaluation,
//!   range/collection bindings and the TIR template adapter.
//! - `places`: interpreted aliases, projected reads and writes, collection mutation and fallible
//!   handler execution.
//! - `values`: constant folding and projection/index helpers, error constructors and
//!   expression-with-value construction.
//! - `budget`: limits, meters and allocation charging. It answers how much work an attempt may
//!   still spend; live interpreter state belongs to `state`.
//!
//! Data flow: `executable_for_const_declaration` compiles and validates one declaration; the
//! rewrite layer replaces a call with a constant by evaluating the callee through `execute`;
//! `execute` reads and writes interpreted state through `places`; rewrite, execute and places all
//! use the shared value helpers, instruction vocabulary and budget owned by this module tree.
//! There is exactly one `ConstFunctionExecutable`, one eligibility policy and one executed-frame
//! machine.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::store::{
    ConstExecutionMeter, ConstStringValue, ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateOutputLimits, TemplateOutputMeter, TemplateWorkBudget,
};
use crate::compiler_frontend::compiler_errors::InfrastructureErrors;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;

mod budget;
mod eligibility;
mod execute;
mod instructions;
mod labels;
mod places;
mod rewrite;
mod state;
mod values;

pub(crate) use eligibility::const_body_rejection_diagnostic;
pub(crate) use rewrite::{rewrite_release_calls, rewrite_required_const_calls};
pub(crate) use state::ConstExecutionServices;

use eligibility::validate_const_body_expressions;
use instructions::{ConstInstruction, compile_body};

/// Why one authored const body cannot become a bounded typed executable.
///
/// WHAT: the first unsupported operation and its authored location, if the body supplied one.
/// WHY: declaration preparation, imported publication and the release classifier all
///      need the same refusal site. Callers choose whether that refusal is a source error or an
///      ineligible runtime candidate.
#[derive(Clone, Debug)]
pub(crate) struct ConstBodyRejection {
    pub(crate) operation: &'static str,
    pub(crate) span: Option<SourceSpan>,
}

/// Preparation failure kept separate from a source-level unsupported body.
#[derive(Debug)]
pub(crate) enum ConstBodyPreparationError {
    Unsupported(ConstBodyRejection),
    Infrastructure(Box<InfrastructureErrors>),
}

/// One const function compiled into bounded typed instructions.
///
/// WHY: a call can execute before HIR exists. The prepared body keeps the canonical typed
///      expression IDs and statement shape so every call reuses one compilation instead of
///      reparsing or re-lowering the declaration.
#[derive(Clone, Debug)]
pub(crate) struct ConstFunctionExecutable {
    parameters: Vec<(PathId, TypeId)>,
    return_slots: Vec<TypeId>,
    instructions: Rc<[ConstInstruction]>,
    /// Catch bodies compiled once during declaration preparation, keyed by their authored catch
    /// expression.
    ///
    /// A rewrite that rebuilds a catch under a fresh `ExprId` registers the same body in
    /// `ExecutionState::rewritten_handlers` as it produces the rebuild, so selection never depends
    /// on an authored span or on the rebuilt expression having been prepared.
    handler_bodies: Rc<FxHashMap<ExprId, Rc<[ConstInstruction]>>>,
}

/// One resolved field or index step inside an interpreted place.
#[derive(Clone, Copy, Debug)]
enum ResolvedPlaceProjection {
    Field(StringId),
    Index(usize),
}

/// One mutable alias target: a canonical binding root and the projections copied from its place.
#[derive(Clone, Debug)]
struct AliasTarget {
    root: PathId,
    projections: Vec<ResolvedPlaceProjection>,
}

/// One constant value with its completed type, span and authored value mode.
#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectedConstant {
    pub(super) value: ConstValueId,
    pub(super) type_id: TypeId,
    pub(super) span: Option<SourceSpan>,
    pub(super) value_mode: ValueMode,
}

/// The declaration-scope facts a prepared const body's references are checked against.
///
/// WHAT: the scope that owns the body's authored names, the path interner that reads them and the
///       constant store that holds explicit `#` values.
/// WHY: declaration preparation must reject a body whose reference can never produce a value. A
///      frame-local name or an explicit constant can be interpreted; a template content reference
///      cannot, and only the declaration scope can tell them apart before any call runs.
pub(crate) struct ConstBodyReferenceScope<'a> {
    pub(crate) scope: &'a ScopeContext,
    pub(crate) path_fork: &'a PathInternerFork,
    pub(crate) constants: &'a ConstValueStore,
}

/// Build the executable row for one const declaration when its body and result shape are supported.
pub(crate) fn executable_for_const_declaration(
    signature: &FunctionSignature,
    body: &[AstNode],
    expressions: &AstExpressionBuilder,
    references: ConstBodyReferenceScope<'_>,
) -> Result<ConstFunctionExecutable, ConstBodyPreparationError> {
    let mut executable =
        executable_for_body(signature, body).map_err(ConstBodyPreparationError::Unsupported)?;
    if executable.return_slots.len() != 1 {
        let operation = if executable.return_slots.is_empty() {
            labels::ZERO_SUCCESS_RESULTS
        } else {
            labels::MULTIPLE_SUCCESS_RESULTS
        };
        return Err(ConstBodyPreparationError::Unsupported(ConstBodyRejection {
            operation,
            span: None,
        }));
    }
    validate_const_body_expressions(&mut executable, expressions, references)?;
    Ok(executable)
}

/// Build the executable row for one prepared body when every statement is executable.
pub(crate) fn executable_for_body(
    signature: &FunctionSignature,
    body: &[AstNode],
) -> Result<ConstFunctionExecutable, ConstBodyRejection> {
    let return_slots = signature
        .return_contract
        .ordinary()
        .ok_or(ConstBodyRejection {
            operation: "ordinary success result",
            span: None,
        })?;
    Ok(ConstFunctionExecutable {
        parameters: signature
            .parameters
            .iter()
            .map(|parameter| (parameter.id, parameter.type_id))
            .collect(),
        return_slots: return_slots.success().to_vec(),
        instructions: compile_body(body)?,
        handler_bodies: Rc::new(FxHashMap::default()),
    })
}

#[cfg(test)]
mod tests;
