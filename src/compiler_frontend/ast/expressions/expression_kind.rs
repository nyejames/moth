//! AST expression variants and operator contracts.
//!
//! WHAT: defines the runtime/compile-time shape of expression values once the
//! frontend has resolved their types.
//! WHY: separating the value shape from constructor helpers keeps expression
//! lowering review focused on the data contract first.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_rpn::{ExpressionRpn, PlaceExpression};
#[cfg(test)]
use crate::compiler_frontend::ast::expressions::expression_types::FallibleCarrierVariant;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, FallibleExpressionHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastTarget;
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// One key/value pair inside a `{...}` map literal.
///
/// WHAT: AST shape for map entries after key and value expressions have been
///       parsed and coerced.
/// WHY: map literals need a dedicated variant so lowering stages can distinguish
///      them from homogeneous collections.
#[derive(Clone, Debug)]
pub struct MapLiteralEntry {
    pub(crate) key: Expression,
    pub(crate) value: Expression,
}

/// Resolved AST representation of an explicit `cast` expression.
///
/// WHAT: records the source expression, target builtin type, selected evidence,
///      and handling form once the boundary target is known.
/// WHY: keeping the full resolution in one AST node lets later stages (folding,
///      HIR lowering) consume a single resolved fact instead of re-running trait
///      or evidence lookups.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ResolvedCastExpression {
    pub(crate) source: Box<Expression>,
    pub(crate) source_type_id: TypeId,
    pub(crate) target_type_id: TypeId,
    pub(crate) target: BuiltinCastTarget,
    pub(crate) requires_optional_wrap_after_cast: bool,
    pub(crate) evidence: ResolvedCastEvidence,
    pub(crate) handling: CastHandling,
    pub(crate) location: SourceLocation,
}

#[derive(Clone, Debug)]
pub enum ExpressionKind {
    /// Internal sentinel for "no source value was provided" (for example, a
    /// parameter default that is intentionally absent).
    NoValue,

    /// User-authored `none` literal in an explicit option context.
    OptionNone,

    /// A runtime expression fragment that could not be constant-folded.
    ///
    /// WHAT: carries an expression-owned RPN stack for expressions whose value is only
    ///       known at runtime.
    /// WHY: operands are `Expression` values, not general `AstNode` fragments, so runtime
    ///      RPN cannot smuggle statement bodies into value contexts.
    Runtime(ExpressionRpn),

    Int(i32),
    Float(f64),
    StringSlice(StringId),
    Bool(bool),
    Char(char),

    /// A folded `String` whose resource and site-root anchors remain structural.
    ///
    /// Value-position file paths use this form instead of manufacturing URL text. Plain authored
    /// strings retain the compact `StringSlice` representation.
    StructuralString {
        pieces: Vec<ConstStringPiece>,
    },
    /// Reference to a variable by name.
    Reference(InternedPath),

    /// Explicitly materialize a fresh value from an aliasing place.
    Copy(PlaceExpression),

    /// Functions are first-class values.
    ///
    /// WHAT: carries callable signature metadata only. Function bodies are statement-level
    /// `AstNode::Function` payloads and must not be stored inside expressions.
    Function(FunctionSignature),

    /// Infallible user function call.
    ///
    /// `result_type_ids` are canonical semantic identities from the active
    /// `TypeEnvironment`; display spelling is recovered separately when needed.
    FunctionCall {
        name: InternedPath,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
    },

    /// Field/member access on a value expression.
    FieldAccess {
        base: Box<Expression>,
        field: StringId,
    },

    /// Receiver method call.
    MethodCall {
        receiver: Box<Expression>,
        method_path: InternedPath,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        location: SourceLocation,
    },

    /// Compiler-owned collection builtin call (`get`, `set`, `push`, `remove`, `length`).
    CollectionBuiltinCall {
        receiver: Box<Expression>,
        op: CollectionBuiltinOp,
        receiver_requires_mutable: bool,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        location: SourceLocation,
    },

    /// Compiler-owned map builtin call (`get`, `contains`, `set`, `remove`, `clear`, `length`).
    MapBuiltinCall {
        receiver: Box<Expression>,
        op: MapBuiltinOp,
        receiver_requires_mutable: bool,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        location: SourceLocation,
    },

    /// User function call with explicit `!` or `catch` handling.
    ///
    /// The success slots stay TypeId-first so HIR lowering never needs
    /// diagnostic return spelling to build call result values.
    HandledFallibleFunctionCall {
        name: InternedPath,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        handling: FallibleExpressionHandling,
        /// Authored postfix `!` location, kept separate from the call expression location.
        ///
        /// WHAT: preserves the source site of propagation while `Expression::location`
        ///       remains the call location used by ordinary call lowering and side tables.
        /// WHY: one source location cannot faithfully identify both the call and its
        ///      control-flow effect.
        propagation_location: Option<SourceLocation>,
    },

    /// External fallible function call with explicit handling.
    ///
    /// The error slot is kept as a TypeId alongside success `result_type_ids`;
    /// backend package spelling is not part of executable AST identity.
    HandledFallibleHostFunctionCall {
        id: ExternalFunctionId,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
        error_type_id: TypeId,
        handling: FallibleExpressionHandling,
        /// Authored postfix `!` location, kept separate from the host-call location.
        propagation_location: Option<SourceLocation>,
    },

    /// Explicit `cast` / `cast!` expression resolved at an explicit typed boundary.
    ///
    /// WHAT: carries the resolved source, target, evidence, and handling form.
    /// WHY: the cast surface models builtin and user-defined evidence, fallibility
    ///      forms, and optional target wrapping in one resolved AST node before HIR
    ///      consumes it.
    Cast(ResolvedCastExpression),

    /// Construct a `Success` or `Failure` carrier value.
    #[cfg(test)]
    FallibleCarrierConstruct {
        variant: FallibleCarrierVariant,
        value: Box<Expression>,
    },

    /// An expression with explicit fallible handling (`!` or `catch`).
    HandledFallibleExpression {
        value: Box<Expression>,
        handling: FallibleExpressionHandling,
        /// Authored postfix `!` location, kept separate from the wrapped value location.
        propagation_location: Option<SourceLocation>,
    },

    /// Postfix option propagation (`expr?`).
    ///
    /// WHAT: unwraps `T?` to `T` on the present path and returns `none` from
    ///       the current function on the absent path.
    /// WHY: option propagation is control flow like fallible propagation, but
    ///      options are ordinary values and do not use the internal result carrier.
    OptionPropagation {
        value: Box<Expression>,
    },

    /// Infallible external function call.
    ///
    /// HIR carries the stable external function ID and canonical result TypeIds;
    /// backends map the external ID to target-specific runtime names later.
    HostFunctionCall {
        id: ExternalFunctionId,
        args: Vec<CallArgument>,
        result_type_ids: Vec<TypeId>,
    },

    /// Equivalent to a string when folded at compile time.
    Template(Box<Template>),

    /// Final AST-owned handoff for an ordinary runtime template.
    ///
    /// WHAT: carries the neutral owned runtime-template payload that HIR will consume after the
    /// Phase 11 cutover. The payload contains no TIR store, view, registry, overlay, or reference
    /// identity.
    /// WHY: final AST expressions should own runtime template data directly instead of asking HIR
    /// to inspect `Template` internals. Current consumers remain on `Template` until the cutover
    /// slice wires this variant through lowering.
    RuntimeTemplateHandoff(Box<OwnedRuntimeTemplateHandoff>),

    /// Final AST-owned handoff for a runtime slot application.
    ///
    /// WHAT: carries the neutral owned slot-application payload that preserves routed wrapper,
    /// contribution-source, and slot-site render data without exposing TIR IDs.
    /// WHY: slot applications need a distinct expression shape so the later HIR cutover can
    /// preserve the current dispatch rule without reading `Template::runtime_slot_handoff`.
    RuntimeSlotApplicationHandoff(Box<OwnedRuntimeSlotApplicationHandoff>),

    /// Homogeneous collection literal.
    Collection(Vec<Expression>),

    /// Ordered map literal with key = value entries.
    ///
    /// WHAT: carries typed key/value pairs after frontend parsing and coercion.
    /// WHY: map literals are a distinct language construct from collections;
    ///      they require separate HIR lowering and backend runtime support.
    MapLiteral(Vec<MapLiteralEntry>),

    /// Struct type definition literal.
    StructDefinition(Vec<Declaration>),

    /// Struct instance construction literal.
    StructInstance(Vec<Declaration>),

    /// Anonymous compile-time record literal (`| field = value, ... |`).
    ///
    /// WHAT: carries ordered, named field declarations parsed in a compile-time receiving
    ///       context. The record is a member group, not a runtime value or structural type.
    /// WHY: anonymous records need a distinct variant so lowering never mistakes them for
    ///      `StructInstance` values; field types live on the field values and the record
    ///      itself resolves to the compile-time marker type.
    AnonymousConstRecord {
        fields: Vec<Declaration>,
    },

    /// Inclusive range operator (`..`). Kept as a dedicated variant to simplify
    /// constant folding; this may become a general operator in the future.
    Range(Box<Expression>, Box<Expression>),

    /// An implicit contextual coercion applied by the compiler at an explicit
    /// type boundary. The inner value retains its original expression kind;
    /// `to_type` records the canonical promoted target so lowering stages can
    /// emit the correct conversion.
    ///
    /// WHY a separate variant: silent type pretending (e.g. storing an `Int`
    /// expression but calling it `Float`, or passing `String` where `String?`
    /// is expected) makes later lowering fragile. An explicit `Coerced` node
    /// makes the coercion visible and auditable.
    Coerced {
        value: Box<Expression>,
        to_type: TypeId,
    },

    /// Explicit choice variant construction: `Choice::Variant` or `Choice::Variant(...)`.
    ///
    /// WHY: choice values must not masquerade as raw integer literals in AST.
    /// The tag index is deterministic within the resolved nominal choice.
    /// For unit variants, `fields` is empty. For payload variants, `fields`
    /// carries the resolved constructor arguments in declaration order.
    ChoiceConstruct {
        nominal_path: InternedPath,
        tag: usize,
        fields: Vec<Declaration>,
    },

    /// A value-producing control-flow block used only at closed receiving sites.
    ///
    /// WHAT: wraps `ValueBlock` variants (`if`, future `match`, `catch`) into an
    ///       expression shape so receiving sites can type-check and lower them.
    /// WHY: value blocks are not general expressions; this variant keeps them
    ///      distinguishable from ordinary statements while allowing them to
    ///      appear as declaration initializers and assignment right-hand sides.
    ValueBlock {
        block: Box<ValueBlock>,
    },
}

impl ExpressionKind {
    /// Whether this expression can be folded to a constant at compile time.
    pub fn is_foldable(&self) -> bool {
        if matches!(
            self,
            ExpressionKind::Int(_)
                | ExpressionKind::Float(_)
                | ExpressionKind::Bool(_)
                | ExpressionKind::StringSlice(_)
                | ExpressionKind::Char(_)
                | ExpressionKind::StructuralString { .. }
                | ExpressionKind::ChoiceConstruct { .. }
        ) {
            return true;
        }

        false
    }
}

#[cfg(test)]
#[path = "tests/runtime_handoff_expression_payload_tests.rs"]
mod runtime_handoff_expression_payload_tests;

#[derive(Clone, Debug, PartialEq)]
pub enum Operator {
    // Arithmetic
    Add,
    Subtract,
    Multiply,
    Divide,
    IntDivide,
    Modulus,
    Exponent,

    // Comparison and logical
    And,
    Or,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Equality,
    NotEqual,

    // Unary logical negation
    Not,
    // Unary numeric negation
    Negate,

    // Range construction
    Range,
}

impl Operator {
    /// Precedence used by expression shunting-yard ordering.
    pub fn precedence(&self) -> u32 {
        match self {
            // Special unary/range operators bind most tightly.
            Operator::Range | Operator::Not | Operator::Negate => 6,

            // Exponentiation is right-associative, but still lower than unary operators.
            Operator::Exponent => 5,

            Operator::Multiply | Operator::Divide | Operator::IntDivide | Operator::Modulus => 4,

            Operator::Add | Operator::Subtract => 3,

            Operator::LessThan
            | Operator::LessThanOrEqual
            | Operator::GreaterThan
            | Operator::GreaterThanOrEqual
            | Operator::Equality
            | Operator::NotEqual => 2,

            Operator::And => 1,

            Operator::Or => 0,
        }
    }

    /// Whether this operator associates left-to-right during shunting-yard ordering.
    pub fn is_left_associative(&self) -> bool {
        !matches!(self, Operator::Exponent)
    }

    /// Number of operand expressions required by this operator.
    pub fn required_values(&self) -> usize {
        match self {
            Operator::Add
            | Operator::Subtract
            | Operator::Multiply
            | Operator::Divide
            | Operator::IntDivide
            | Operator::Modulus
            | Operator::Exponent
            | Operator::And
            | Operator::Or
            | Operator::GreaterThan
            | Operator::GreaterThanOrEqual
            | Operator::LessThan
            | Operator::LessThanOrEqual
            | Operator::Range
            | Operator::Equality
            | Operator::NotEqual => 2,

            Operator::Not | Operator::Negate => 1,
        }
    }

    /// Source spelling for this operator, used in diagnostics and debug output.
    pub fn to_str(&self) -> &str {
        match self {
            Operator::Add => "+",
            Operator::Subtract => "-",
            Operator::Multiply => "*",
            Operator::Divide => "/",
            Operator::IntDivide => "//",
            Operator::Modulus => "%",
            Operator::Exponent => "^",
            Operator::And => "and",
            Operator::Or => "or",
            Operator::GreaterThan => ">",
            Operator::GreaterThanOrEqual => ">=",
            Operator::LessThan => "<",
            Operator::LessThanOrEqual => "<=",
            Operator::Equality => "is",
            Operator::NotEqual => "is not",
            Operator::Not => "not",
            Operator::Negate => "-",
            Operator::Range => "to",
        }
    }
}
