//! Prepared const-instruction vocabulary and the declaration-time body compiler.
//!
//! WHAT: defines the bounded statement forms the interpreter executes and compiles authored body
//!       nodes into them. Nested lexical scopes append directly into their parent instruction
//!       vector; control-flow and loop bodies keep their own bounded slices.
//! WHY: one compiled representation is reused by every call. Keeping the vocabulary and compiler
//!      together leaves `mod.rs` as the module map and executable-preparation entry point.
//! MUST NOT: execute instructions, decide eligibility or own expression semantics.

use super::ConstBodyRejection;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, DeclarationValue, LoopBindings, MultiBindTarget, NodeKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueId;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, PlaceId,
};
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;

/// A statement form the bounded const executor understands.
#[derive(Clone, Debug)]
pub(super) enum ConstInstructionKind {
    Bind {
        path: PathId,
        expression: ExprId,
        value_mode: ValueMode,
    },
    BindValue {
        path: PathId,
        value: ConstValueId,
    },
    Assign {
        place: PlaceId,
        expression: ExprId,
    },
    Eval(ExprId),
    If {
        condition: ExprId,
        then_body: Rc<[ConstInstruction]>,
        else_body: Rc<[ConstInstruction]>,
    },
    /// `default` is `None` only for a choice or option match the AST proved exhaustive.
    Match {
        scrutinee: ExprId,
        arms: Rc<[ConstMatchArm]>,
        default: Option<Rc<[ConstInstruction]>>,
    },
    While {
        condition: ExprId,
        body: Rc<[ConstInstruction]>,
    },
    Range {
        bindings: Box<LoopBindings>,
        range: Box<RangeLoopSpec>,
        body: Rc<[ConstInstruction]>,
        span: Option<SourceSpan>,
    },
    Collection {
        bindings: Box<LoopBindings>,
        iterable: ExprId,
        body: Rc<[ConstInstruction]>,
        span: Option<SourceSpan>,
    },
    Return(Vec<ExprId>),
    /// `then` at a catch handler's own receiving site.
    Recover(Vec<ExprId>),
    /// A value-producing block at a closed receiving site. Its body selects one branch with the
    /// ordinary statement forms, and the `Then` that completes the branch hands its values to
    /// `receiver`.
    ///
    /// WHY: value blocks occur only as the root of a declaration, assignment, multi-bind or
    ///      return, so the receiver is known when the body is compiled and a branch exit such as
    ///      `return`, `break` or `continue` unwinds the same frame stack as any other statement.
    ValueBlock {
        body: Rc<[ConstInstruction]>,
        receiver: Rc<ValueReceiver>,
    },
    MultiBind {
        expression: ExprId,
        receiver: ValueReceiver,
    },
    /// `then` in a value block body: produce every slot, then complete the innermost value block.
    Then(Vec<ExprId>),
    Break,
    Continue,
    Assert {
        condition: ExprId,
        message: ExprId,
    },
}

/// One instruction with the authored span that owns its diagnostic location.
#[derive(Clone, Debug)]
pub(super) struct ConstInstruction {
    pub(super) kind: ConstInstructionKind,
    pub(super) span: Option<SourceSpan>,
}

/// The receiving site a value block, or a multi-bind producer, delivers its slots to.
#[derive(Clone, Debug)]
pub(super) enum ValueReceiver {
    Declaration(PathId),
    Assignment(PlaceId),
    MultiBind(Box<[MultiBindTarget]>),
    Return,
}

/// One prepared match arm: the AST's resolved pattern and guard, with its body compiled once.
///
/// WHY: the pattern is cloned once here because the AST keeps its own copy; the prepared arm is
///      then reused for every const call of this body.
#[derive(Debug)]
pub(super) struct ConstMatchArm {
    pub(super) pattern: MatchPattern,
    pub(super) guard: Option<ExprId>,
    pub(super) body: Rc<[ConstInstruction]>,
}

/// Which body a compiled instruction sequence belongs to.
///
/// WHAT: `return` leaves the const function from every body, while a bare `then` value is the
///       result of the innermost catch handler or value block that owns it.
/// WHY: a handler or value block produces values while still returning out of the function that
///      owns it, so one flag cannot describe both roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyContext {
    /// A const function body: `return` leaves the function and a bare `then` value is rejected.
    ConstFunction,
    /// A catch handler body: a bare `then` value is the handler result and an explicit `return`
    /// leaves the enclosing const function.
    CatchHandler,
    /// A value block branch: a bare `then` value completes the block.
    ValueBlock,
}

pub(super) fn compile_body(
    body: &[AstNode],
    expressions: &AstExpressionBuilder,
) -> Result<Rc<[ConstInstruction]>, ConstBodyRejection> {
    BodyCompiler { expressions }.nested_body(body, BodyContext::ConstFunction)
}

/// Compile one catch handler body, treating its `then` values as the handler result.
pub(super) fn compile_handler_body(
    body: &[AstNode],
    expressions: &AstExpressionBuilder,
) -> Result<Rc<[ConstInstruction]>, ConstBodyRejection> {
    BodyCompiler { expressions }.nested_body(body, BodyContext::CatchHandler)
}

/// One body compilation over the expression store that identifies value blocks at receiving
/// sites.
struct BodyCompiler<'a> {
    expressions: &'a AstExpressionBuilder,
}

impl BodyCompiler<'_> {
    /// Append one body's instructions under the body context that owns it.
    ///
    /// Lexical scopes are flattened into the caller's vector so a deeply nested block does not
    /// allocate an intermediate instruction slice only to clone it again.
    fn body_into(
        &self,
        body: &[AstNode],
        body_context: BodyContext,
        instructions: &mut Vec<ConstInstruction>,
    ) -> Result<(), ConstBodyRejection> {
        for node in body {
            let kind = match &node.kind {
                NodeKind::LexicalScope { body } => {
                    self.body_into(body, body_context, instructions)?;
                    continue;
                }
                NodeKind::VariableDeclaration(declaration) => match &declaration.value {
                    DeclarationValue::Initializer(expression) => {
                        match self.value_block_body(*expression)? {
                            Some(body) => {
                                value_block(body, ValueReceiver::Declaration(declaration.id))
                            }
                            None => ConstInstructionKind::Bind {
                                path: declaration.id,
                                expression: *expression,
                                value_mode: declaration.value_mode,
                            },
                        }
                    }
                    DeclarationValue::Default(value) => ConstInstructionKind::BindValue {
                        path: declaration.id,
                        value: *value,
                    },
                    DeclarationValue::Absent => continue,
                    DeclarationValue::StructDefinition(_) => {
                        return reject("struct definition", node.span);
                    }
                    DeclarationValue::PendingConstant => {
                        return reject("pending constant", node.span);
                    }
                    DeclarationValue::Shell(_) => {
                        return reject("declaration shell", node.span);
                    }
                },
                NodeKind::Assignment { target, value } => match self.value_block_body(*value)? {
                    Some(body) => value_block(body, ValueReceiver::Assignment(*target)),
                    None => ConstInstructionKind::Assign {
                        place: *target,
                        expression: *value,
                    },
                },
                NodeKind::MultiBind { targets, value } => {
                    let receiver = ValueReceiver::MultiBind(targets.clone().into());
                    match self.value_block_body(*value)? {
                        Some(body) => value_block(body, receiver),
                        None => ConstInstructionKind::MultiBind {
                            expression: *value,
                            receiver,
                        },
                    }
                }
                NodeKind::ExpressionStatement(expression) => {
                    ConstInstructionKind::Eval(*expression)
                }
                NodeKind::If(condition, then_body, else_body, _) => ConstInstructionKind::If {
                    condition: *condition,
                    then_body: self.nested_body(then_body, body_context)?,
                    else_body: match else_body {
                        Some(body) => self.nested_body(body, body_context)?,
                        None => Rc::from(Vec::new()),
                    },
                },
                NodeKind::WhileLoop(condition, body) => ConstInstructionKind::While {
                    condition: *condition,
                    body: self.nested_body(body, body_context)?,
                },
                NodeKind::RangeLoop {
                    bindings,
                    range,
                    body,
                } => ConstInstructionKind::Range {
                    bindings: bindings.clone(),
                    range: range.clone(),
                    body: self.nested_body(body, body_context)?,
                    span: node.span,
                },
                NodeKind::CollectionLoop {
                    bindings,
                    iterable,
                    step,
                    body,
                } => {
                    // A stepped collection loop needs the runtime index contract; the bounded core
                    // supports the ordinary forward walk first.
                    if step.is_some() {
                        return reject("stepped collection loop", node.span);
                    }
                    ConstInstructionKind::Collection {
                        bindings: bindings.clone(),
                        iterable: *iterable,
                        body: self.nested_body(body, body_context)?,
                        span: node.span,
                    }
                }
                NodeKind::Return(values) => {
                    let block = match values.as_slice() {
                        [value] => self.value_block_body(*value)?,
                        _ => None,
                    };
                    match block {
                        Some(body) => value_block(body, ValueReceiver::Return),
                        None => ConstInstructionKind::Return(values.clone()),
                    }
                }
                NodeKind::Break => ConstInstructionKind::Break,
                NodeKind::Continue => ConstInstructionKind::Continue,
                NodeKind::Assert { condition, message } => ConstInstructionKind::Assert {
                    condition: *condition,
                    message: *message,
                },
                NodeKind::ThenValue(values) => match body_context {
                    // A handler body is evaluated by the fallible expression that owns it, so its
                    // `then` values are that body's own result.
                    BodyContext::CatchHandler => {
                        ConstInstructionKind::Recover(values.expressions.clone())
                    }
                    BodyContext::ValueBlock => {
                        ConstInstructionKind::Then(values.expressions.clone())
                    }
                    BodyContext::ConstFunction => {
                        return reject("value-producing handler statement", node.span);
                    }
                },
                NodeKind::ReturnError(_) => return reject("error return", node.span),
                NodeKind::Match {
                    scrutinee,
                    arms,
                    default,
                    ..
                } => self.match_instruction(*scrutinee, arms, default.as_deref(), body_context)?,
                NodeKind::PushStartRuntimeFragment(_) => {
                    return reject("runtime fragment", node.span);
                }
                NodeKind::StructDefinition(_, _) => {
                    return reject("struct definition", node.span);
                }
                NodeKind::Function(_, _, _) => return reject("nested function", node.span),
                NodeKind::EntryStart(_, _) => return reject("entry start", node.span),
                NodeKind::NeverCall(_) => return reject("never-returning call", node.span),
            };
            instructions.push(ConstInstruction {
                kind,
                span: node.span,
            });
        }
        Ok(())
    }

    /// Compile a value block's selected-branch body, or return `None` when `expression` is an
    /// ordinary value. A catch block stays an expression: its handler runs only on failure.
    fn value_block_body(
        &self,
        expression: ExprId,
    ) -> Result<Option<Rc<[ConstInstruction]>>, ConstBodyRejection> {
        // An unreadable expression stays ordinary; eligibility reports it as infrastructure.
        let Ok(view) = self.expressions.view(expression) else {
            return Ok(None);
        };
        let ExpressionOperationView::ValueBlock(block) = view.operation() else {
            return Ok(None);
        };
        let branch_context = BodyContext::ValueBlock;
        let body = match block {
            ValueBlock::If(block) => Rc::from([ConstInstruction {
                kind: ConstInstructionKind::If {
                    condition: block.condition,
                    then_body: self.nested_body(&block.then_body, branch_context)?,
                    else_body: self.nested_body(&block.else_body, branch_context)?,
                },
                span: block.span,
            }]),
            ValueBlock::Match(block) => Rc::from([ConstInstruction {
                kind: self.match_instruction(
                    block.scrutinee,
                    &block.arms,
                    block.default.as_deref(),
                    branch_context,
                )?,
                span: block.span,
            }]),
            ValueBlock::LexicalScope(block) => self.nested_body(&block.body, branch_context)?,
            ValueBlock::Catch(_) => return Ok(None),
        };
        Ok(Some(body))
    }

    /// `default` is `None` only for a choice or option match the AST proved exhaustive.
    fn match_instruction(
        &self,
        scrutinee: ExprId,
        arms: &[MatchArm],
        default: Option<&[AstNode]>,
        body_context: BodyContext,
    ) -> Result<ConstInstructionKind, ConstBodyRejection> {
        let mut prepared_arms = Vec::with_capacity(arms.len());
        for MatchArm {
            pattern,
            guard,
            body,
        } in arms
        {
            prepared_arms.push(ConstMatchArm {
                pattern: pattern.clone(),
                guard: *guard,
                body: self.nested_body(body, body_context)?,
            });
        }
        Ok(ConstInstructionKind::Match {
            scrutinee,
            arms: Rc::from(prepared_arms),
            default: match default {
                Some(body) => Some(self.nested_body(body, body_context)?),
                None => None,
            },
        })
    }

    fn nested_body(
        &self,
        body: &[AstNode],
        body_context: BodyContext,
    ) -> Result<Rc<[ConstInstruction]>, ConstBodyRejection> {
        let mut instructions = Vec::with_capacity(body.len());
        self.body_into(body, body_context, &mut instructions)?;
        Ok(Rc::from(instructions.into_boxed_slice()))
    }
}

fn value_block(body: Rc<[ConstInstruction]>, receiver: ValueReceiver) -> ConstInstructionKind {
    ConstInstructionKind::ValueBlock {
        body,
        receiver: Rc::new(receiver),
    }
}

fn reject<T>(operation: &'static str, span: Option<SourceSpan>) -> Result<T, ConstBodyRejection> {
    Err(ConstBodyRejection { operation, span })
}
