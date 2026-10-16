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

use crate::compiler_frontend::ast::const_values::ConstValueId;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, PlaceId,
};
use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::build_config_contract::BuildConfigQualifierSyntax;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

use crate::compiler_frontend::value_mode::ValueMode;
use crate::return_compiler_error;

#[derive(Debug, Clone)]
pub struct Declaration {
    pub id: PathId,
    /// Canonical static type of the binding, independent of any initializer result.
    pub type_id: TypeId,
    /// Parsed annotation retained only while a later type-resolution phase still needs source syntax.
    pub(crate) type_syntax: Option<ParsedTypeRef>,
    /// Access mode of the binding rather than the initializer value.
    pub value_mode: ValueMode,
    pub value: DeclarationValue,
    /// Span of the declaration name/binding token when authored.
    ///
    /// WHAT: exact source span of the binding-name token for authored declarations.
    /// WHY: HIR lowering must attribute locals to their binding site without guessing
    /// from initializer value spans; synthetic/materialised declarations use `None`.
    pub binding_span: Option<SourceSpan>,
    /// Declaration-owned `#Config of T` syntax metadata, consumed by config resolution when
    /// applicable and otherwise retained for source-contract header consumers.
    pub(crate) config_qualifier: Option<BuildConfigQualifierSyntax>,
}

/// Declaration state is distinct from a value-producing expression.
#[derive(Debug, Clone)]
pub enum DeclarationValue {
    /// An authored expression. In struct-field shells it is an unfolded default that the strict
    /// member pass folds to `Default`; a folded field never keeps this variant.
    Initializer(ExprId),
    /// Folded content owned by the callable or field-default contract, not an executable root.
    Default(ConstValueId),
    /// Nominal definition data, never a first-class semantic expression.
    StructDefinition(Vec<Declaration>),
    /// A caller or control-flow owner supplies the binding value.
    Absent,
    /// An authored constant-header value awaiting folding. Struct field defaults never use this
    /// variant; they stay `Initializer` until the strict member pass folds them.
    PendingConstant,
    /// An identity-only declaration whose semantic contract lives in its owning table.
    Shell(DeclarationShell),
}

#[derive(Debug, Clone)]
pub enum DeclarationShell {
    Function { receiver: Option<ReceiverKey> },
    Struct,
    Choice,
    TypeAlias,
}

impl DeclarationValue {
    pub fn default(&self) -> Option<ConstValueId> {
        match self {
            Self::Default(value) => Some(*value),
            _ => None,
        }
    }

    pub fn initializer(&self) -> Option<ExprId> {
        match self {
            Self::Initializer(expression) => Some(*expression),
            Self::Default(_)
            | Self::StructDefinition(_)
            | Self::Absent
            | Self::PendingConstant
            | Self::Shell(_) => None,
        }
    }

    pub fn initializer_mut(&mut self) -> Option<&mut ExprId> {
        match self {
            Self::Initializer(expression) => Some(expression),
            Self::Default(_)
            | Self::StructDefinition(_)
            | Self::Absent
            | Self::PendingConstant
            | Self::Shell(_) => None,
        }
    }

    pub fn is_const_record_value(
        &self,
        expressions: &AstExpressionBuilder,
    ) -> Result<bool, CompilerError> {
        match self.initializer() {
            Some(id) => Ok(expressions.view(id)?.is_const_record_value()),
            None => Ok(false),
        }
    }
}

impl Declaration {
    pub fn is_receiver_function(&self) -> bool {
        match &self.value {
            DeclarationValue::Shell(DeclarationShell::Function { receiver }) => receiver.is_some(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MultiBindTargetKind {
    Declaration,
    Assignment,
}

#[derive(Debug, Clone)]
pub struct MultiBindTarget {
    pub id: PathId,
    pub type_id: TypeId,
    pub value_mode: ValueMode,
    pub kind: MultiBindTargetKind,
    /// Exact source span of the authored binding, or `None` for generated targets.
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone)]
pub struct AstNode {
    pub kind: NodeKind,
    /// Exact source span of the authored node, or `None` for generated nodes.
    pub span: Option<SourceSpan>,
    pub scope: PathId,
}

/// Authored statement-branch facts retained until static selection.
///
/// WHAT: carries each body's immediate lexical scope and provisional generic-request range.
/// WHY: finalisation must preserve the selected branch identity and discard inactive generated
/// work without reconstructing either fact from body contents.
#[derive(Debug, Clone)]
pub struct IfBranchMetadata {
    pub(crate) request_ranges: IfGenericRequestRanges,
    pub(crate) then_scope: PathId,
    pub(crate) else_scope: Option<PathId>,
}

impl IfBranchMetadata {
    pub(crate) fn new(
        request_ranges: IfGenericRequestRanges,
        then_scope: PathId,
        else_scope: Option<PathId>,
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
    pub start: ExprId,
    pub end: ExprId,
    pub end_kind: RangeEndKind,
    pub step: Option<ExprId>,
    /// The iteration domain the range policy selected from these operands.
    ///
    /// WHAT: the counter type, operand conversion target, unit-step domain and generated
    ///       stepping-op domain of this header, already selected and checked by
    ///       `range_loop_spec_from_completed_operands`.
    /// WHY: domain selection is a pure function of the operands' canonical types, and those types
    ///      only exist at AST construction. Carrying the selected domain with the header keeps
    ///      HIR lowering, the counter declaration and template copies reading one selected fact
    ///      instead of each re-deriving it from operand types.
    pub domain: NumericScalar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchExhaustiveness {
    /// The match has an explicit `else =>` arm.
    HasDefault,
    /// The match has no explicit default, but AST proved it covers every choice variant.
    ExhaustiveChoice,
}

/// Statement node payloads.
///
/// Executable expressions are compact IDs into the phase store. Bulky loop header records
/// remain boxed because recursive body parsing keeps several statement nodes live per level.
#[derive(Debug, Clone)]
pub enum NodeKind {
    // Control Flow
    Return(Vec<ExprId>), // Return values
    ReturnError(ExprId), // return! value
    If(ExprId, Vec<AstNode>, Option<Vec<AstNode>>, IfBranchMetadata), // Condition, If true, Else, provisional generic work

    Match {
        scrutinee: ExprId,
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
        bindings: Box<LoopBindings>,
        range: Box<RangeLoopSpec>,
        body: Vec<AstNode>,
    },
    CollectionLoop {
        bindings: Box<LoopBindings>,
        iterable: ExprId,
        body: Vec<AstNode>,
    },
    WhileLoop(ExprId, Vec<AstNode>), // Condition, Body
    Break,
    Continue,

    /// Runtime assertion statement intrinsic.
    ///
    /// WHAT: `assert(condition)` and `assert(condition, "message")` are language-owned
    ///       statement surfaces for runtime invariant checking.
    /// WHY: keeping assert statement-only prevents shadowing and result handling while the
    ///      shared call parser owns its named/default/access/type argument contract.
    Assert {
        condition: ExprId,
        message: ExprId,
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
    PushStartRuntimeFragment(ExprId),

    // example: new_struct_instance = MyStructDefinition(arg1, arg2)
    //          new_struct_instance(arg) -- Calls the main function of the struct
    StructDefinition(
        PathId,           // Full unique name path
        Vec<Declaration>, // Fields
    ),

    Function(PathId, FunctionSignature, Vec<AstNode>),
    // Mutation of existing mutable variables
    Assignment {
        target: PlaceId, // Variable or field projection
        value: ExprId,
    },

    MultiBind {
        targets: Vec<MultiBindTarget>,
        value: ExprId,
    },

    /// Statement-level expression.
    ///
    /// WHAT: wraps an expression that appears as a standalone statement or as
    ///       a temporary statement-side expression value during parsing.
    /// WHY: expression internals own calls/operators/literals; this statement
    ///      node carries the expression value when a body position needs it.
    ExpressionStatement(ExprId),
}

/// One pending row of the immutable expression walk.
///
/// WHAT: a statement, declaration, match pattern or expression row that has not been visited yet.
/// WHY: nested bodies are walked on an explicit heap stack, so nesting depth costs heap space
/// rather than call-stack frames. Rows are pushed in reverse, so popping them visits rows
/// in authored pre-order.
enum ExpressionVisit<'a> {
    Node(&'a AstNode),
    Declaration(&'a Declaration),
    Pattern(&'a MatchPattern),
    Expression(ExprId),
}

/// Mutable counterpart of [`ExpressionVisit`], with identical row order.
enum ExpressionVisitMut<'a> {
    Node(&'a mut AstNode),
    Declaration(&'a mut Declaration),
    Pattern(&'a mut MatchPattern),
    Expression(&'a mut ExprId),
}

fn push_body<'a>(stack: &mut Vec<ExpressionVisit<'a>>, body: &'a [AstNode]) {
    for node in body.iter().rev() {
        stack.push(ExpressionVisit::Node(node));
    }
}

fn push_declarations<'a>(stack: &mut Vec<ExpressionVisit<'a>>, declarations: &'a [Declaration]) {
    for declaration in declarations.iter().rev() {
        stack.push(ExpressionVisit::Declaration(declaration));
    }
}

fn visit_declaration_expressions<'a>(
    declaration: &'a Declaration,
    stack: &mut Vec<ExpressionVisit<'a>>,
    visitor: &mut dyn FnMut(ExprId),
) {
    match &declaration.value {
        DeclarationValue::Initializer(expression) => visitor(*expression),
        DeclarationValue::StructDefinition(fields) => push_declarations(stack, fields),
        DeclarationValue::Default(_)
        | DeclarationValue::Absent
        | DeclarationValue::PendingConstant
        | DeclarationValue::Shell(_) => {}
    }
}

/// Pushes loop bindings so the item declaration is visited before the index declaration.
fn push_bindings<'a>(stack: &mut Vec<ExpressionVisit<'a>>, bindings: &'a LoopBindings) {
    if let Some(index) = &bindings.index {
        stack.push(ExpressionVisit::Declaration(index));
    }
    if let Some(item) = &bindings.item {
        stack.push(ExpressionVisit::Declaration(item));
    }
}

/// Visits immediate roots directly and schedules nested work in reverse authored order.
///
/// Ordinary statements need no heap work rows. Deferred expression rows only preserve order
/// around match patterns and loop declarations that must be visited first.
fn visit_node_expressions<'a>(
    node: &'a AstNode,
    stack: &mut Vec<ExpressionVisit<'a>>,
    visitor: &mut dyn FnMut(ExprId),
) {
    match &node.kind {
        NodeKind::Return(values) => {
            for &value in values {
                visitor(value);
            }
        }
        NodeKind::ReturnError(value)
        | NodeKind::PushStartRuntimeFragment(value)
        | NodeKind::ExpressionStatement(value)
        | NodeKind::Assignment { value, .. }
        | NodeKind::MultiBind { value, .. } => visitor(*value),
        NodeKind::If(condition, then_body, else_body, _) => {
            visitor(*condition);
            if let Some(body) = else_body {
                push_body(stack, body);
            }
            push_body(stack, then_body);
        }
        NodeKind::Match {
            scrutinee,
            arms,
            default,
            ..
        } => {
            visitor(*scrutinee);
            if let Some(body) = default {
                push_body(stack, body);
            }
            for arm in arms.iter().rev() {
                push_body(stack, &arm.body);
                if let Some(guard) = arm.guard {
                    stack.push(ExpressionVisit::Expression(guard));
                }
                stack.push(ExpressionVisit::Pattern(&arm.pattern));
            }
        }
        NodeKind::LexicalScope { body } | NodeKind::Function(_, _, body) => {
            push_body(stack, body);
        }
        NodeKind::RangeLoop {
            bindings,
            range,
            body,
        } => {
            push_body(stack, body);
            if let Some(step) = range.step {
                stack.push(ExpressionVisit::Expression(step));
            }
            stack.push(ExpressionVisit::Expression(range.end));
            stack.push(ExpressionVisit::Expression(range.start));
            push_bindings(stack, bindings);
        }
        NodeKind::CollectionLoop {
            bindings,
            iterable,
            body,
        } => {
            push_body(stack, body);
            stack.push(ExpressionVisit::Expression(*iterable));
            push_bindings(stack, bindings);
        }
        NodeKind::WhileLoop(condition, body) => {
            visitor(*condition);
            push_body(stack, body);
        }
        NodeKind::Assert { condition, message } => {
            visitor(*condition);
            visitor(*message);
        }
        NodeKind::ThenValue(values) => {
            for &value in &values.expressions {
                visitor(value);
            }
        }
        NodeKind::VariableDeclaration(declaration) => {
            visit_declaration_expressions(declaration, stack, visitor);
        }
        NodeKind::StructDefinition(_, fields) => push_declarations(stack, fields),
        NodeKind::Break | NodeKind::Continue => {}
    }
}

fn push_body_mut<'a>(stack: &mut Vec<ExpressionVisitMut<'a>>, body: &'a mut [AstNode]) {
    for node in body.iter_mut().rev() {
        stack.push(ExpressionVisitMut::Node(node));
    }
}

fn push_declarations_mut<'a>(
    stack: &mut Vec<ExpressionVisitMut<'a>>,
    declarations: &'a mut [Declaration],
) {
    for declaration in declarations.iter_mut().rev() {
        stack.push(ExpressionVisitMut::Declaration(declaration));
    }
}

fn visit_declaration_expressions_mut<'a>(
    declaration: &'a mut Declaration,
    stack: &mut Vec<ExpressionVisitMut<'a>>,
    visitor: &mut dyn FnMut(&mut ExprId),
) {
    match &mut declaration.value {
        DeclarationValue::Initializer(expression) => visitor(expression),
        DeclarationValue::StructDefinition(fields) => push_declarations_mut(stack, fields),
        DeclarationValue::Default(_)
        | DeclarationValue::Absent
        | DeclarationValue::PendingConstant
        | DeclarationValue::Shell(_) => {}
    }
}

/// Pushes loop bindings so the item declaration is visited before the index declaration.
fn push_bindings_mut<'a>(stack: &mut Vec<ExpressionVisitMut<'a>>, bindings: &'a mut LoopBindings) {
    if let Some(index) = &mut bindings.index {
        stack.push(ExpressionVisitMut::Declaration(index));
    }
    if let Some(item) = &mut bindings.item {
        stack.push(ExpressionVisitMut::Declaration(item));
    }
}

/// Mutable counterpart of [`visit_node_expressions`], with identical visitation order.
fn visit_node_expressions_mut<'a>(
    node: &'a mut AstNode,
    stack: &mut Vec<ExpressionVisitMut<'a>>,
    visitor: &mut dyn FnMut(&mut ExprId),
) {
    match &mut node.kind {
        NodeKind::Return(values) => {
            for value in values {
                visitor(value);
            }
        }
        NodeKind::ReturnError(value)
        | NodeKind::PushStartRuntimeFragment(value)
        | NodeKind::ExpressionStatement(value)
        | NodeKind::Assignment { value, .. }
        | NodeKind::MultiBind { value, .. } => visitor(value),
        NodeKind::If(condition, then_body, else_body, _) => {
            visitor(condition);
            if let Some(body) = else_body {
                push_body_mut(stack, body);
            }
            push_body_mut(stack, then_body);
        }
        NodeKind::Match {
            scrutinee,
            arms,
            default,
            ..
        } => {
            visitor(scrutinee);
            if let Some(body) = default {
                push_body_mut(stack, body);
            }
            for arm in arms.iter_mut().rev() {
                push_body_mut(stack, &mut arm.body);
                if let Some(guard) = &mut arm.guard {
                    stack.push(ExpressionVisitMut::Expression(guard));
                }
                stack.push(ExpressionVisitMut::Pattern(&mut arm.pattern));
            }
        }
        NodeKind::LexicalScope { body } | NodeKind::Function(_, _, body) => {
            push_body_mut(stack, body);
        }
        NodeKind::RangeLoop {
            bindings,
            range,
            body,
        } => {
            push_body_mut(stack, body);
            if let Some(step) = &mut range.step {
                stack.push(ExpressionVisitMut::Expression(step));
            }
            stack.push(ExpressionVisitMut::Expression(&mut range.end));
            stack.push(ExpressionVisitMut::Expression(&mut range.start));
            push_bindings_mut(stack, bindings);
        }
        NodeKind::CollectionLoop {
            bindings,
            iterable,
            body,
        } => {
            push_body_mut(stack, body);
            stack.push(ExpressionVisitMut::Expression(iterable));
            push_bindings_mut(stack, bindings);
        }
        NodeKind::WhileLoop(condition, body) => {
            visitor(condition);
            push_body_mut(stack, body);
        }
        NodeKind::Assert { condition, message } => {
            visitor(condition);
            visitor(message);
        }
        NodeKind::ThenValue(values) => {
            for value in &mut values.expressions {
                visitor(value);
            }
        }
        NodeKind::VariableDeclaration(declaration) => {
            visit_declaration_expressions_mut(declaration, stack, visitor);
        }
        NodeKind::StructDefinition(_, fields) => push_declarations_mut(stack, fields),
        NodeKind::Break | NodeKind::Continue => {}
    }
}

fn push_place_bodies<'a>(stack: &mut Vec<&'a AstNode>, body: &'a [AstNode]) {
    for node in body.iter().rev() {
        stack.push(node);
    }
}

fn push_place_bodies_mut<'a>(stack: &mut Vec<&'a mut AstNode>, body: &'a mut [AstNode]) {
    for node in body.iter_mut().rev() {
        stack.push(node);
    }
}

impl AstNode {
    pub(crate) fn visit_place_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut PlaceId)) {
        let mut stack = Vec::new();
        let mut next = Some(self);
        while let Some(node) = next {
            match &mut node.kind {
                NodeKind::Assignment { target, .. } => visitor(target),
                NodeKind::If(_, then_body, else_body, _) => {
                    if let Some(body) = else_body {
                        push_place_bodies_mut(&mut stack, body);
                    }
                    push_place_bodies_mut(&mut stack, then_body);
                }
                NodeKind::Match { arms, default, .. } => {
                    if let Some(body) = default {
                        push_place_bodies_mut(&mut stack, body);
                    }
                    for arm in arms.iter_mut().rev() {
                        push_place_bodies_mut(&mut stack, &mut arm.body);
                    }
                }
                NodeKind::LexicalScope { body }
                | NodeKind::Function(_, _, body)
                | NodeKind::RangeLoop { body, .. }
                | NodeKind::CollectionLoop { body, .. }
                | NodeKind::WhileLoop(_, body) => push_place_bodies_mut(&mut stack, body),
                NodeKind::Return(_)
                | NodeKind::ReturnError(_)
                | NodeKind::Break
                | NodeKind::Continue
                | NodeKind::Assert { .. }
                | NodeKind::ThenValue(_)
                | NodeKind::VariableDeclaration(_)
                | NodeKind::PushStartRuntimeFragment(_)
                | NodeKind::StructDefinition(_, _)
                | NodeKind::MultiBind { .. }
                | NodeKind::ExpressionStatement(_) => {}
            }
            next = stack.pop();
        }
    }

    pub(crate) fn visit_expression_ids_mut(&mut self, visitor: &mut dyn FnMut(&mut ExprId)) {
        let mut stack = Vec::new();
        visit_node_expressions_mut(self, &mut stack, visitor);
        while let Some(item) = stack.pop() {
            match item {
                ExpressionVisitMut::Node(node) => {
                    visit_node_expressions_mut(node, &mut stack, visitor)
                }
                ExpressionVisitMut::Declaration(declaration) => {
                    visit_declaration_expressions_mut(declaration, &mut stack, visitor)
                }
                ExpressionVisitMut::Pattern(pattern) => pattern.visit_expression_ids_mut(visitor),
                ExpressionVisitMut::Expression(expression) => visitor(expression),
            }
        }
    }

    pub(crate) fn visit_place_ids(&self, visitor: &mut dyn FnMut(PlaceId)) {
        let mut stack = Vec::new();
        let mut next = Some(self);
        while let Some(node) = next {
            match &node.kind {
                NodeKind::Assignment { target, .. } => visitor(*target),
                NodeKind::If(_, then_body, else_body, _) => {
                    if let Some(body) = else_body {
                        push_place_bodies(&mut stack, body);
                    }
                    push_place_bodies(&mut stack, then_body);
                }
                NodeKind::Match { arms, default, .. } => {
                    if let Some(body) = default {
                        push_place_bodies(&mut stack, body);
                    }
                    for arm in arms.iter().rev() {
                        push_place_bodies(&mut stack, &arm.body);
                    }
                }
                NodeKind::LexicalScope { body }
                | NodeKind::Function(_, _, body)
                | NodeKind::RangeLoop { body, .. }
                | NodeKind::CollectionLoop { body, .. }
                | NodeKind::WhileLoop(_, body) => push_place_bodies(&mut stack, body),
                NodeKind::Return(_)
                | NodeKind::ReturnError(_)
                | NodeKind::Break
                | NodeKind::Continue
                | NodeKind::Assert { .. }
                | NodeKind::ThenValue(_)
                | NodeKind::VariableDeclaration(_)
                | NodeKind::PushStartRuntimeFragment(_)
                | NodeKind::StructDefinition(_, _)
                | NodeKind::MultiBind { .. }
                | NodeKind::ExpressionStatement(_) => {}
            }
            next = stack.pop();
        }
    }

    /// Visit semantic expression roots held by this statement and its nested bodies.
    /// Place projections remain owned and visited by the expression store's place owner.
    pub(crate) fn visit_expression_ids(&self, visitor: &mut dyn FnMut(ExprId)) {
        let mut stack = Vec::new();
        visit_node_expressions(self, &mut stack, visitor);
        while let Some(item) = stack.pop() {
            match item {
                ExpressionVisit::Node(node) => visit_node_expressions(node, &mut stack, visitor),
                ExpressionVisit::Declaration(declaration) => {
                    visit_declaration_expressions(declaration, &mut stack, visitor)
                }
                ExpressionVisit::Pattern(pattern) => pattern.visit_expression_ids(visitor),
                ExpressionVisit::Expression(expression) => visitor(expression),
            }
        }
    }
}

impl AstNode {
    /// Returns whether this expression-shaped node represents a const-record value.
    ///
    /// WHAT: receiver-call validation needs to distinguish an actual const-record
    /// value from an exported struct's nominal `TypeId`.
    /// WHY: exported structs share the same canonical type identity for runtime
    /// and const-record values; the const-record call restriction is value-level,
    /// so it must inspect the explicit `ConstRecordState` on the expression rather
    /// than branching on the declaration's canonical type identity.
    pub fn expression_is_const_record_value(
        &self,
        expressions: &AstExpressionBuilder,
    ) -> Result<bool, CompilerError> {
        match &self.kind {
            NodeKind::VariableDeclaration(declaration) => {
                declaration.value.is_const_record_value(expressions)
            }
            NodeKind::ExpressionStatement(expression) => {
                Ok(expressions.view(*expression)?.is_const_record_value())
            }

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

#[cfg(test)]
#[path = "tests/ast_nodes_tests.rs"]
mod ast_nodes_tests;
