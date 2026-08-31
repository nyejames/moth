//! Core AST node declarations.
//!
//! WHAT: defines statement node shapes and temporary statement-side place nodes.
//! WHY: parser output, HIR lowering, and frontend finalization need one authoritative AST surface
//! while expression internals stay owned by `ast/expressions`.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` in this module means an internal compiler invariant or setup failure only.
//! Source-authored syntax, type, and rule failures are rejected earlier with `CompilerDiagnostic`.

use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, PlaceExpression,
};
use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::MatchArm;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;

use crate::compiler_frontend::value_mode::ValueMode;
use crate::return_compiler_error;

pub(crate) use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

#[derive(Debug, Clone)]
pub struct Declaration {
    pub id: InternedPath,
    pub value: Expression,
}

impl Declaration {
    pub(crate) fn is_unresolved_constant_placeholder(&self) -> bool {
        matches!(self.value.kind, ExpressionKind::NoValue)
            && matches!(self.value.diagnostic_type, DataType::Inferred)
    }
}

#[derive(Debug, Clone)]
pub enum MultiBindTargetKind {
    Declaration,
    Assignment,
}

#[derive(Debug, Clone)]
pub struct MultiBindTarget {
    pub id: InternedPath,
    pub type_id: TypeId,
    pub value_mode: ValueMode,
    pub kind: MultiBindTargetKind,
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct AstNode {
    pub kind: NodeKind,
    pub location: SourceLocation,
    pub scope: InternedPath,
}

/// Authored statement-branch facts retained until static selection.
///
/// WHAT: carries each body's immediate lexical scope and provisional generic-request range.
/// WHY: finalisation must preserve the selected branch identity and discard inactive generated
/// work without reconstructing either fact from body contents.
#[derive(Debug, Clone)]
pub struct IfBranchMetadata {
    pub(crate) request_ranges: IfGenericRequestRanges,
    pub(crate) then_scope: InternedPath,
    pub(crate) else_scope: Option<InternedPath>,
}

impl IfBranchMetadata {
    pub(crate) fn new(
        request_ranges: IfGenericRequestRanges,
        then_scope: InternedPath,
        else_scope: Option<InternedPath>,
    ) -> Self {
        Self {
            request_ranges,
            then_scope,
            else_scope,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeEndKind {
    Exclusive,
    Inclusive,
}

#[derive(Debug, Clone)]
pub struct LoopBindings {
    pub item: Option<Declaration>,
    pub index: Option<Declaration>,
}

#[derive(Debug, Clone)]
pub struct RangeLoopSpec {
    pub start: Expression,
    pub end: Expression,
    pub end_kind: RangeEndKind,
    pub step: Option<Expression>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchExhaustiveness {
    /// The match has an explicit `else =>` arm.
    HasDefault,
    /// The match has no explicit default, but AST proved it covers every choice variant.
    ExhaustiveChoice,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum NodeKind {
    // Control Flow
    Return(Vec<Expression>), // Return value,
    ReturnError(Expression), // return! value
    If(
        Expression,
        Vec<AstNode>,
        Option<Vec<AstNode>>,
        IfBranchMetadata,
    ), // Condition, If true, Else, provisional generic work

    Match {
        scrutinee: Expression,
        arms: Vec<MatchArm>,
        default: Option<Vec<AstNode>>,
        exhaustiveness: MatchExhaustiveness,
    },

    /// Compiler-generated lexical scope retained after static control-flow specialization.
    /// Executable source parsing never emits this node directly.
    LexicalScope {
        body: Vec<AstNode>,
    },

    RangeLoop {
        bindings: LoopBindings,
        range: RangeLoopSpec,
        body: Vec<AstNode>,
    },
    CollectionLoop {
        bindings: LoopBindings,
        iterable: Expression,
        body: Vec<AstNode>,
    },
    WhileLoop(Expression, Vec<AstNode>), // Condition, Body,
    Break,
    Continue,

    /// Runtime assertion statement intrinsic.
    ///
    /// WHAT: `assert(condition)` and `assert(condition, "message")` are language-owned
    ///       statement surfaces for runtime invariant checking.
    /// WHY: keeping assert statement-only prevents shadowing and result handling while the
    ///      shared call parser owns its named/default/access/type argument contract.
    Assert {
        condition: Expression,
        message: Expression,
    },

    /// Value-production terminator for active value-producing blocks.
    ///
    /// WHAT: statement-shaped marker carrying one or more expressions that are
    /// returned from the nearest active value-producing block.
    /// WHY: `then` must be a statement so it can see locals declared earlier in
    /// the same body. The owning block parser (catch, future value `if`, match)
    /// consumes this node before HIR lowering.
    ThenValue(crate::compiler_frontend::ast::statements::value_production::ProducedValues),

    // Basics
    VariableDeclaration(Declaration), // Variable name, Value, Visibility,

    /// Accumulate a runtime string expression into the entry start() fragment list.
    ///
    /// WHAT: marks a top-level runtime template for entry-start fragment emission.
    /// WHY: explicit intent avoids synthetic variable protocols and post-hoc fragment
    /// extraction from the start body.
    /// The HIR builder lowers this into a `PushRuntimeFragment` statement inside entry start().
    PushStartRuntimeFragment(Expression),

    // example: new_struct_instance = MyStructDefinition(arg1, arg2)
    //          new_struct_instance(arg) -- Calls the main function of the struct
    StructDefinition(
        InternedPath,     // Full unique name path
        Vec<Declaration>, // Fields
    ),

    Function(InternedPath, FunctionSignature, Vec<AstNode>),

    // Mutation of existing mutable variables
    Assignment {
        target: PlaceExpression, // Variable or field projection
        value: Expression,
    },

    MultiBind {
        targets: Vec<MultiBindTarget>,
        value: Expression,
    },

    /// Statement-level expression.
    ///
    /// WHAT: wraps an expression that appears as a standalone statement or as
    ///       a temporary statement-side expression value during parsing.
    /// WHY: expression internals own calls/operators/literals; this statement
    ///      node carries the expression value when a body position needs it.
    ExpressionStatement(Expression),
}

impl AstNode {
    /// Returns the expression TypeId for expression-shaped nodes without rebuilding
    /// an owned `Expression`.
    ///
    /// WHAT: postfix/member parsing needs the receiver type at every chain step.
    /// WHY: using this lightweight query avoids cloning call arguments, field
    /// access trees, and diagnostic `DataType` payloads just to inspect type
    /// identity.
    pub fn expression_type_id(&self) -> Result<TypeId, CompilerError> {
        match &self.kind {
            NodeKind::VariableDeclaration(declaration) => Ok(declaration.value.type_id),
            NodeKind::ExpressionStatement(expression) => Ok(expression.type_id),

            // Non-expression nodes — compiler invariant violation.
            // This path should never be reached from well-formed AST; it indicates a bug in
            // an earlier stage that allowed a non-expression node into expression context.
            _ => {
                return_compiler_error!(
                    "AST invariant: tried to get the type of a non-expression AST node: {:?}",
                    &self.kind
                );
            }
        }
    }

    /// Returns whether this expression-shaped node represents a const-record value.
    ///
    /// WHAT: receiver-call validation needs to distinguish an actual const-record
    /// value from an exported struct's nominal `TypeId`.
    /// WHY: exported structs share the same canonical type identity for runtime
    /// and const-record values; the const-record call restriction is value-level,
    /// so it must inspect the explicit `ConstRecordState` on the expression rather
    /// than branching on diagnostic-only `DataType` spelling.
    pub fn expression_is_const_record_value(&self) -> Result<bool, CompilerError> {
        match &self.kind {
            NodeKind::VariableDeclaration(declaration) => {
                Ok(declaration.value.is_const_record_value())
            }
            NodeKind::ExpressionStatement(expression) => Ok(expression.is_const_record_value()),

            // Non-expression nodes — compiler invariant violation.
            // This path should never be reached from well-formed AST; it indicates a bug in
            // an earlier stage that allowed a non-expression node into expression context.
            _ => {
                return_compiler_error!(
                    "AST invariant: tried to inspect the const-record value state of a non-expression AST node: {:?}",
                    &self.kind
                );
            }
        }
    }
}
