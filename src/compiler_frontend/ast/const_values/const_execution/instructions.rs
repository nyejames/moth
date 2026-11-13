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
    AstNode, DeclarationValue, LoopBindings, NodeKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueId;
use crate::compiler_frontend::ast::expressions::expression_store::{ExprId, PlaceId};
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
    /// An explicit `return` inside a catch handler must leave the enclosing const function,
    /// not merely produce the handler recovery value.
    ReturnFromFunction(Vec<ExprId>),
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

/// Which body a compiled instruction sequence belongs to.
///
/// WHAT: `return` leaves the const function from either body, but only a catch handler turns a
///       bare `then` value into its own result.
/// WHY: a handler produces a recovery value while still returning out of the function that owns
///      it, so one flag cannot describe both roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyContext {
    /// A const function body: `return` leaves the function and a bare `then` value is rejected.
    ConstFunction,
    /// A catch handler body: a bare `then` value is the handler result and an explicit `return`
    /// leaves the enclosing const function.
    CatchHandler,
}

pub(super) fn compile_body(body: &[AstNode]) -> Result<Rc<[ConstInstruction]>, ConstBodyRejection> {
    let mut rejection = None;
    let mut instructions = Vec::with_capacity(body.len());
    compile_body_into(
        body,
        BodyContext::ConstFunction,
        &mut rejection,
        &mut instructions,
    )
    .ok_or_else(|| {
        rejection.unwrap_or(ConstBodyRejection {
            operation: "body statement",
            span: None,
        })
    })?;
    Ok(Rc::from(instructions.into_boxed_slice()))
}

/// Compile one catch handler body, treating its `then` values as the handler result.
pub(super) fn compile_handler_body(
    body: &[AstNode],
) -> Result<Rc<[ConstInstruction]>, ConstBodyRejection> {
    let mut rejection = None;
    let mut instructions = Vec::with_capacity(body.len());
    compile_body_into(
        body,
        BodyContext::CatchHandler,
        &mut rejection,
        &mut instructions,
    )
    .ok_or_else(|| {
        rejection.unwrap_or(ConstBodyRejection {
            operation: "catch handler statement",
            span: None,
        })
    })?;
    Ok(Rc::from(instructions.into_boxed_slice()))
}

/// Append one body's instructions under the body context that owns it.
///
/// Lexical scopes are flattened into the caller's vector so a deeply nested block does not
/// allocate an intermediate instruction slice only to clone it again.
fn compile_body_into(
    body: &[AstNode],
    body_context: BodyContext,
    rejection: &mut Option<ConstBodyRejection>,
    instructions: &mut Vec<ConstInstruction>,
) -> Option<()> {
    for node in body {
        match &node.kind {
            NodeKind::LexicalScope { body } => {
                compile_body_into(body, body_context, rejection, instructions)?;
            }
            NodeKind::VariableDeclaration(declaration) => match &declaration.value {
                DeclarationValue::Initializer(expression) => {
                    push_instruction(
                        instructions,
                        node.span,
                        ConstInstructionKind::Bind {
                            path: declaration.id,
                            expression: *expression,
                            value_mode: declaration.value_mode,
                        },
                    );
                }
                DeclarationValue::Default(value) => {
                    push_instruction(
                        instructions,
                        node.span,
                        ConstInstructionKind::BindValue {
                            path: declaration.id,
                            value: *value,
                        },
                    );
                }
                DeclarationValue::Absent => {}
                DeclarationValue::StructDefinition(_) => {
                    return reject_const_body(rejection, "struct definition", node.span);
                }
                DeclarationValue::PendingConstant => {
                    return reject_const_body(rejection, "pending constant", node.span);
                }
                DeclarationValue::Shell(_) => {
                    return reject_const_body(rejection, "declaration shell", node.span);
                }
            },
            NodeKind::Assignment { target, value } => {
                push_instruction(
                    instructions,
                    node.span,
                    ConstInstructionKind::Assign {
                        place: *target,
                        expression: *value,
                    },
                );
            }
            NodeKind::ExpressionStatement(expression) => {
                push_instruction(
                    instructions,
                    node.span,
                    ConstInstructionKind::Eval(*expression),
                );
            }
            NodeKind::If(condition, then_body, else_body, _) => {
                push_instruction(
                    instructions,
                    node.span,
                    ConstInstructionKind::If {
                        condition: *condition,
                        then_body: compile_nested_body(then_body, body_context, rejection)?,
                        else_body: match else_body {
                            Some(body) => compile_nested_body(body, body_context, rejection)?,
                            None => Rc::from(Vec::new()),
                        },
                    },
                );
            }
            NodeKind::WhileLoop(condition, body) => {
                push_instruction(
                    instructions,
                    node.span,
                    ConstInstructionKind::While {
                        condition: *condition,
                        body: compile_nested_body(body, body_context, rejection)?,
                    },
                );
            }
            NodeKind::RangeLoop {
                bindings,
                range,
                body,
            } => {
                push_instruction(
                    instructions,
                    node.span,
                    ConstInstructionKind::Range {
                        bindings: bindings.clone(),
                        range: range.clone(),
                        body: compile_nested_body(body, body_context, rejection)?,
                        span: node.span,
                    },
                );
            }
            NodeKind::CollectionLoop {
                bindings,
                iterable,
                step,
                body,
            } => {
                // A stepped collection loop needs the runtime index contract; the bounded core
                // supports the ordinary forward walk first.
                if step.is_some() {
                    return reject_const_body(rejection, "stepped collection loop", node.span);
                }
                push_instruction(
                    instructions,
                    node.span,
                    ConstInstructionKind::Collection {
                        bindings: bindings.clone(),
                        iterable: *iterable,
                        body: compile_nested_body(body, body_context, rejection)?,
                        span: node.span,
                    },
                );
            }
            NodeKind::Return(values) => {
                let kind = match body_context {
                    BodyContext::ConstFunction => ConstInstructionKind::Return(values.clone()),
                    BodyContext::CatchHandler => {
                        ConstInstructionKind::ReturnFromFunction(values.clone())
                    }
                };
                push_instruction(instructions, node.span, kind);
            }
            NodeKind::Break => {
                push_instruction(instructions, node.span, ConstInstructionKind::Break)
            }
            NodeKind::Continue => {
                push_instruction(instructions, node.span, ConstInstructionKind::Continue)
            }
            NodeKind::Assert { condition, message } => {
                push_instruction(
                    instructions,
                    node.span,
                    ConstInstructionKind::Assert {
                        condition: *condition,
                        message: *message,
                    },
                );
            }
            // A value-producing handler body is evaluated by the fallible expression that owns
            // it, not as an unconditional statement of the enclosing function body.
            NodeKind::ThenValue(values) => match body_context {
                BodyContext::CatchHandler => push_instruction(
                    instructions,
                    node.span,
                    ConstInstructionKind::Return(values.expressions.clone()),
                ),
                BodyContext::ConstFunction => {
                    return reject_const_body(
                        rejection,
                        "value-producing handler statement",
                        node.span,
                    );
                }
            },
            NodeKind::ReturnError(_) => {
                return reject_const_body(rejection, "error return", node.span);
            }
            NodeKind::Match { .. } => {
                return reject_const_body(rejection, "match statement", node.span);
            }
            NodeKind::PushStartRuntimeFragment(_) => {
                return reject_const_body(rejection, "runtime fragment", node.span);
            }
            NodeKind::StructDefinition(_, _) => {
                return reject_const_body(rejection, "struct definition", node.span);
            }
            NodeKind::Function(_, _, _) => {
                return reject_const_body(rejection, "nested function", node.span);
            }
            NodeKind::EntryStart(_, _) => {
                return reject_const_body(rejection, "entry start", node.span);
            }
            NodeKind::MultiBind { .. } => {
                return reject_const_body(rejection, "multi-bind statement", node.span);
            }
            NodeKind::NeverCall(_) => {
                return reject_const_body(rejection, "never-returning call", node.span);
            }
        }
    }
    Some(())
}

fn compile_nested_body(
    body: &[AstNode],
    body_context: BodyContext,
    rejection: &mut Option<ConstBodyRejection>,
) -> Option<Rc<[ConstInstruction]>> {
    let mut instructions = Vec::with_capacity(body.len());
    compile_body_into(body, body_context, rejection, &mut instructions)?;
    Some(Rc::from(instructions.into_boxed_slice()))
}

fn push_instruction(
    instructions: &mut Vec<ConstInstruction>,
    span: Option<SourceSpan>,
    kind: ConstInstructionKind,
) {
    instructions.push(ConstInstruction { kind, span });
}

fn reject_const_body(
    rejection: &mut Option<ConstBodyRejection>,
    operation: &'static str,
    span: Option<SourceSpan>,
) -> Option<()> {
    if rejection.is_none() {
        *rejection = Some(ConstBodyRejection { operation, span });
    }
    None
}
