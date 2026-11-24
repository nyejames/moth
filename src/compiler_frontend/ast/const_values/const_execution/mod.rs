//! Bounded execution of prepared const functions at required call sites.
//!
//! WHAT: owns the executable row, the preparation entry points and this module map. Sibling
//!       modules compile bodies, rewrite required source calls and run the bounded typed-AST
//!       frame machine over live storage and immutable `ConstValueStore` payloads.
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
//! - `eligibility`: the single operation eligibility policy that validates prepared bodies.
//! - `rewrite`: required AST expression rewriting and provenance transfer.
//!   `rewrite_required_const_calls` is the module-tree entry point for the expression evaluator.
//! - `instructions`: the compiled instruction vocabulary and declaration-time body compiler.
//! - `labels`: the operation names shared by classification and execution refusals.
//! - `state`: the interpreter's borrowed service context, live execution state and handler
//!   snapshots.
//! - `storage`: bounded dynamic binding cells, aggregate edges and access permissions.
//! - `execute`: the one executed-frame machine, including expression evaluation,
//!   range/collection bindings and the TIR template adapter.
//! - `matching`: match arm selection over the AST's resolved patterns, guards and captures.
//! - `places`: typed place resolution, collection operations and fallible handler execution.
//! - `values`: constant folding and projection/index helpers, error constructors and
//!   expression-with-value construction.
//! - `budget`: limits, meters and allocation charging. It answers how much work an attempt may
//!   still spend; live interpreter state belongs to `state`.
//!
//! Data flow: `executable_for_const_declaration` compiles and validates one declaration;
//! the frame machine returns ordered live values. The rewrite layer freezes a result only
//! when an AST consumer needs a replacement. Place operations share the attempt-local arena.
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
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;
use std::rc::Rc;

mod budget;
mod eligibility;
mod execute;
mod instructions;
mod labels;
mod matching;
mod places;
mod rewrite;
mod state;
mod storage;
mod values;

pub(crate) use eligibility::const_body_rejection_diagnostic;
pub(crate) use execute::{TirCollectionCapture, TirLiveItem};
pub(crate) use rewrite::rewrite_required_const_calls;
pub(crate) use state::ConstExecutionServices;

use eligibility::validate_const_body_expressions;
use instructions::{ConstInstruction, compile_body};

/// Why one authored const body cannot become a bounded typed executable.
///
/// WHAT: the first unsupported operation and its authored location, if the body supplied one.
/// WHY: declaration preparation and imported publication both need the same refusal site.
#[derive(Clone, Debug)]
pub(crate) struct ConstBodyRejection {
    pub(crate) operation: &'static str,
    pub(crate) span: Option<SourceSpan>,
}

/// Preparation failure kept separate from a source-level unsupported body.
#[derive(Debug)]
pub(crate) enum ConstBodyPreparationError {
    Unsupported(ConstBodyRejection),
    Diagnostic(DiagnosticDraft),
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
    handler_bodies: Rc<FxHashMap<ExprId, Rc<[ConstInstruction]>>>,
    /// Declaring-domain owner for a body imported from a provider boundary.
    ///
    /// Reparsed provider spans belong to the donor source database, not the consumer's. Execution
    /// diagnostics carry this handle so report rendering resolves them against the authored file.
    donor_identity: Option<FrozenIdentityHandle>,
}

impl ConstFunctionExecutable {
    /// Mark this executable's body as owned by a foreign declaring domain.
    pub(crate) fn set_donor_identity(&mut self, donor_identity: FrozenIdentityHandle) {
        self.donor_identity = Some(donor_identity);
    }
}

/// One resolved field or index step inside an interpreted place.
#[derive(Clone, Copy, Debug)]
enum ResolvedPlaceProjection {
    Field(StringId),
    Index(usize),
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
    let mut executable = executable_for_body(signature, body, expressions)
        .map_err(ConstBodyPreparationError::Unsupported)?;
    // A call must produce a value to fold into its site; several ordered slots commit together.
    if executable.return_slots.is_empty() {
        return Err(ConstBodyPreparationError::Unsupported(ConstBodyRejection {
            operation: labels::ZERO_SUCCESS_RESULTS,
            span: None,
        }));
    }
    validate_const_body_expressions(&mut executable, expressions, references)?;
    Ok(executable)
}

/// Build the executable row for one prepared body when every statement is executable.
fn executable_for_body(
    signature: &FunctionSignature,
    body: &[AstNode],
    expressions: &AstExpressionBuilder,
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
        instructions: compile_body(body, expressions)?,
        handler_bodies: Rc::new(FxHashMap::default()),
        donor_identity: None,
    })
}

#[cfg(test)]
mod tests;
