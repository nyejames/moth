//! Source operator vocabulary shared by transient parsing and resolved operation facts.

use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;

#[derive(Clone, Copy, Debug, PartialEq)]
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

    /// The numeric operator this source operator applies, or `None` for logical, comparison and
    /// range operators.
    ///
    /// WHY: AST typing, constant folding and HIR lowering all select the operation domain through
    ///      `numeric_operators`, so the source-to-numeric mapping lives in one place.
    pub(crate) fn numeric_operator(&self) -> Option<NumericOperator> {
        match self {
            Operator::Add => Some(NumericOperator::Add),
            Operator::Subtract => Some(NumericOperator::Subtract),
            Operator::Multiply => Some(NumericOperator::Multiply),
            Operator::Divide => Some(NumericOperator::Divide),
            Operator::IntDivide => Some(NumericOperator::IntegerDivide),
            Operator::Modulus => Some(NumericOperator::Remainder),
            Operator::Exponent => Some(NumericOperator::Power),
            Operator::Negate => Some(NumericOperator::Negate),

            Operator::And
            | Operator::Or
            | Operator::GreaterThan
            | Operator::GreaterThanOrEqual
            | Operator::LessThan
            | Operator::LessThanOrEqual
            | Operator::Equality
            | Operator::NotEqual
            | Operator::Not
            | Operator::Range => None,
        }
    }

    /// Source spelling for this operator, used in diagnostics and debug output.
    pub fn to_str(self) -> &'static str {
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
