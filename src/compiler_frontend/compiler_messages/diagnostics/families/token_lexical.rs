//! Family rows for token and lexical diagnostics, and for compiler-capacity diagnostics.
//!
//! WHAT: owns the compact token, lexical and capacity family rows, their data codecs, mirror constructors,
//! legacy adapters and view renderers.
//! WHY: source-token and parser failures share one compact record layout, while capacity faults can come from any stage.

use super::calls_data::InvalidMultiBindCode;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    CharFact, CountFact, FactCapacityError, FactCodec, IdRemap, OptStringFact, OptTokenFact,
    StringFact, TokenFact, UnitCode, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_common_syntax_mistake, render_compiler_capacity_exceeded,
    render_expected_symbol_statement, render_expected_token, render_invalid_character,
    render_invalid_expression, render_invalid_number_literal, render_invalid_path,
    render_invalid_string_escape, render_missing_closing_delimiter, render_missing_collection_item,
    render_missing_operator_operand, render_source_span_capacity, render_title_only,
    render_unexpected_end_of_file, render_unexpected_token, render_unexpected_trailing_comma,
};
use crate::compiler_frontend::compiler_messages::{
    AstCapacityResource, CapacityResource, ChoiceVariantSeparatorGap, CommonSyntaxMistakeReason,
    CompilerDiagnostic, DiagnosticCapacityResource, DiagnosticCategory,
    DiagnosticCompoundAssignmentOperator, DiagnosticKind, DiagnosticOperator, DiagnosticPayload,
    DiagnosticSeverity, DiagnosticToken, HirCapacityResource, InvalidExpressionReason,
    InvalidMultiBindReason, InvalidStringEscapeReason, MissingWhitespace, OperatorOperandPosition,
    PathKind, SourceSpanCapacityResource, SymbolicSpacingConstruct, SymbolicSpacingError,
    SyntaxDiagnosticKind,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;

/// Capacity resources of each group sit in their own range of one word, so a decode can tell the groups apart.
const GROUP_STRIDE: u32 = 256;

unit_code! {
    impl HirCapacityResource {
        ExpressionRows = 1,
        ValueEdges = 2,
        StructFields = 3,
        VariantFields = 4,
        MapEntries = 5,
        StringPieces = 6,
        PlaceProjections = 7,
    }
}

unit_code! {
    impl AstCapacityResource {
        ExpressionRows = 1,
        ExpressionSideEntries = 2,
        ConstValues = 3,
        ChoiceTags = 4,
        GroupDistances = 5,
    }
}

unit_code! {
    impl DiagnosticCapacityResource {
        Records = 1,
        Extras = 2,
        Labels = 3,
        LabelData = 4,
        StringIds = 5,
        Substitutions = 6,
        ForeignSites = 7,
        IdentityContexts = 8,
        FactValue = 9,
    }
}

/// Stores a `CapacityResource` as one word: the group's range plus the resource's code.
pub(crate) struct CapacityResourceFact;

impl FactCodec for CapacityResourceFact {
    type Draft = CapacityResource;
    type Durable = CapacityResource;
    const WORDS: usize = 1;

    fn encode(draft: &CapacityResource, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = match *draft {
            CapacityResource::Hir(resource) => resource.code(),
            CapacityResource::Ast(resource) => GROUP_STRIDE + resource.code(),
            CapacityResource::Store(resource) => 2 * GROUP_STRIDE + resource.code(),
        };
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<CapacityResource> {
        let (group, code) = (words[0] / GROUP_STRIDE, words[0] % GROUP_STRIDE);
        match group {
            0 => HirCapacityResource::from_code(code).map(CapacityResource::Hir),
            1 => AstCapacityResource::from_code(code).map(CapacityResource::Ast),
            2 => DiagnosticCapacityResource::from_code(code).map(CapacityResource::Store),
            _ => None,
        }
    }

    fn remap_ids(_words: &mut [u32], _remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        Ok(())
    }
}

/// WHAT: the payload of an unexpected-token diagnostic: the token it found, or a multi-bind misuse.
/// WHY: the legacy kind accepts two payload shapes, so one compact row must hold both.
pub(crate) enum UnexpectedTokenShape {
    Token(DiagnosticToken),
    MultiBind {
        code: InvalidMultiBindCode,
        target_name: Option<StringId>,
        expected: usize,
        found: usize,
    },
}

/// WHAT: stores an unexpected-token shape in four words: a shape tag, then the shape's payload words.
/// WHY: a token reuses the token codec; a multi-bind misuse stores its code, target, and arity counts.
pub(crate) struct UnexpectedTokenFact;

impl FactCodec for UnexpectedTokenFact {
    type Draft = UnexpectedTokenShape;
    type Durable = UnexpectedTokenShape;
    const WORDS: usize = 4;

    fn encode(draft: &UnexpectedTokenShape, words: &mut [u32]) -> Result<(), FactCapacityError> {
        match draft {
            UnexpectedTokenShape::Token(found) => {
                words[0] = 0;
                TokenFact::encode(found, &mut words[1..3])?;
                words[3] = 0;
            }
            UnexpectedTokenShape::MultiBind {
                code,
                target_name,
                expected,
                found,
            } => {
                words[0] = code.code();
                OptStringFact::encode(target_name, &mut words[1..2])?;
                CountFact::encode(expected, &mut words[2..3])?;
                CountFact::encode(found, &mut words[3..4])?;
            }
        }
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<UnexpectedTokenShape> {
        match *words.first()? {
            0 => Some(UnexpectedTokenShape::Token(TokenFact::decode(
                words.get(1..3)?,
            )?)),
            code => Some(UnexpectedTokenShape::MultiBind {
                code: InvalidMultiBindCode::from_code(code)?,
                target_name: OptStringFact::decode(words.get(1..2)?)?,
                expected: CountFact::decode(words.get(2..3)?)?,
                found: CountFact::decode(words.get(3..4)?)?,
            }),
        }
    }

    fn remap_ids(words: &mut [u32], remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        // Tag 0 is a token shape. Other tags are multi-bind shapes with a target name in word 1.
        if words[0] == 0 {
            TokenFact::remap_ids(&mut words[1..3], remap)
        } else {
            OptStringFact::remap_ids(&mut words[1..2], remap)
        }
    }
}

unit_code! {
    /// WHAT: the reason code of an invalid numeric literal, one code per legacy reason.
    /// WHY: the code owns the stable reason key; the typed data a reason names sits in its own fact words.
    pub(crate) enum NumberLiteralCode {
        SeparatorNotBetweenDigits = 1,
        MultipleDecimalPoints = 2,
        DecimalPointNotAfterDigit = 3,
        EndsWithSeparator = 4,
        MissingFractionalDigits = 5,
        UppercaseExponentMarker = 6,
        MissingExponentDigits = 7,
        InvalidExponentSignPlacement = 8,
        InvalidSeparatorPlacement = 9,
        ParseOverflow = 10,
        OutsideFixedScalarRange = 11,
        NegativeUnsignedLiteral = 12,
        NonFiniteFixedFloat = 13,
        InexactNumberScale = 14,
    }
}

impl NumberLiteralCode {
    /// WHAT: splits a legacy reason into its code and the typed data that code names.
    /// WHY: a scalar or scale gets its own fact word, present only for the codes that name it.
    pub(crate) fn split(
        reason: &NumberLiteralErrorReason,
    ) -> (Self, Option<FixedScalar>, Option<NumberScale>) {
        match *reason {
            NumberLiteralErrorReason::SeparatorNotBetweenDigits => {
                (Self::SeparatorNotBetweenDigits, None, None)
            }
            NumberLiteralErrorReason::MultipleDecimalPoints => {
                (Self::MultipleDecimalPoints, None, None)
            }
            NumberLiteralErrorReason::DecimalPointNotAfterDigit => {
                (Self::DecimalPointNotAfterDigit, None, None)
            }
            NumberLiteralErrorReason::EndsWithSeparator => (Self::EndsWithSeparator, None, None),
            NumberLiteralErrorReason::MissingFractionalDigits => {
                (Self::MissingFractionalDigits, None, None)
            }
            NumberLiteralErrorReason::UppercaseExponentMarker => {
                (Self::UppercaseExponentMarker, None, None)
            }
            NumberLiteralErrorReason::MissingExponentDigits => {
                (Self::MissingExponentDigits, None, None)
            }
            NumberLiteralErrorReason::InvalidExponentSignPlacement => {
                (Self::InvalidExponentSignPlacement, None, None)
            }
            NumberLiteralErrorReason::InvalidSeparatorPlacement => {
                (Self::InvalidSeparatorPlacement, None, None)
            }
            NumberLiteralErrorReason::ParseOverflow => (Self::ParseOverflow, None, None),
            NumberLiteralErrorReason::OutsideFixedScalarRange(scalar) => {
                (Self::OutsideFixedScalarRange, Some(scalar), None)
            }
            NumberLiteralErrorReason::NegativeUnsignedLiteral(scalar) => {
                (Self::NegativeUnsignedLiteral, Some(scalar), None)
            }
            NumberLiteralErrorReason::NonFiniteFixedFloat(scalar) => {
                (Self::NonFiniteFixedFloat, Some(scalar), None)
            }
            NumberLiteralErrorReason::InexactNumberScale(scale) => {
                (Self::InexactNumberScale, None, Some(scale))
            }
        }
    }

    /// WHAT: rebuilds the legacy reason from a code and the typed data it names.
    /// WHY: the renderer speaks the legacy reason; `None` means the stored data does not match the code.
    pub(crate) fn to_reason(
        self,
        scalar: Option<FixedScalar>,
        scale: Option<NumberScale>,
    ) -> Option<NumberLiteralErrorReason> {
        Some(match self {
            Self::SeparatorNotBetweenDigits => NumberLiteralErrorReason::SeparatorNotBetweenDigits,
            Self::MultipleDecimalPoints => NumberLiteralErrorReason::MultipleDecimalPoints,
            Self::DecimalPointNotAfterDigit => NumberLiteralErrorReason::DecimalPointNotAfterDigit,
            Self::EndsWithSeparator => NumberLiteralErrorReason::EndsWithSeparator,
            Self::MissingFractionalDigits => NumberLiteralErrorReason::MissingFractionalDigits,
            Self::UppercaseExponentMarker => NumberLiteralErrorReason::UppercaseExponentMarker,
            Self::MissingExponentDigits => NumberLiteralErrorReason::MissingExponentDigits,
            Self::InvalidExponentSignPlacement => {
                NumberLiteralErrorReason::InvalidExponentSignPlacement
            }
            Self::InvalidSeparatorPlacement => NumberLiteralErrorReason::InvalidSeparatorPlacement,
            Self::ParseOverflow => NumberLiteralErrorReason::ParseOverflow,
            Self::OutsideFixedScalarRange => {
                NumberLiteralErrorReason::OutsideFixedScalarRange(scalar?)
            }
            Self::NegativeUnsignedLiteral => {
                NumberLiteralErrorReason::NegativeUnsignedLiteral(scalar?)
            }
            Self::NonFiniteFixedFloat => NumberLiteralErrorReason::NonFiniteFixedFloat(scalar?),
            Self::InexactNumberScale => NumberLiteralErrorReason::InexactNumberScale(scale?),
        })
    }
}

/// WHAT: stores an optional fixed scalar in one word; `u32::MAX` encodes absence.
/// WHY: only the scalar-carrying number-literal codes name a scalar.
pub(crate) struct OptFixedScalarFact;

impl FactCodec for OptFixedScalarFact {
    type Draft = Option<FixedScalar>;
    type Durable = Option<FixedScalar>;
    const WORDS: usize = 1;

    fn encode(draft: &Option<FixedScalar>, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = match draft {
            Some(scalar) => *scalar as u32,
            None => u32::MAX,
        };
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Option<FixedScalar>> {
        match words[0] {
            u32::MAX => Some(None),
            word => fixed_scalar_from_word(word).map(Some),
        }
    }

    fn remap_ids(_words: &mut [u32], _remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        Ok(())
    }
}

/// WHAT: stores an optional number scale in one word; `u32::MAX` encodes absence.
/// WHY: only the inexact-scale number-literal code names a scale.
pub(crate) struct OptNumberScaleFact;

impl FactCodec for OptNumberScaleFact {
    type Draft = Option<NumberScale>;
    type Durable = Option<NumberScale>;
    const WORDS: usize = 1;

    fn encode(draft: &Option<NumberScale>, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words[0] = match draft {
            Some(scale) => u32::from(scale.get()),
            None => u32::MAX,
        };
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Option<NumberScale>> {
        match words[0] {
            u32::MAX => Some(None),
            word => NumberScale::new(u16::try_from(word).ok()?).map(Some),
        }
    }

    fn remap_ids(_words: &mut [u32], _remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        Ok(())
    }
}

/// WHAT: maps a stored word back to the fixed scalar whose discriminant it holds.
/// WHY: scalars are stored by discriminant, so decoding searches the full scalar list.
fn fixed_scalar_from_word(word: u32) -> Option<FixedScalar> {
    FixedScalar::ALL
        .into_iter()
        .find(|scalar| *scalar as u32 == word)
}
unit_code! {
    impl PathKind {
        Empty = 1,
        TrailingSeparator = 2,
        InvalidRoot = 3,
        InvalidComponent = 4,
        OnlyRootSlashSupported = 5,
        EmptyComponent = 6,
        WhitespaceMustBeQuoted = 7,
        MissingSeparator = 8,
        MissingClosingQuote = 9,
        InvalidEscape = 10,
        LeadingAtInPathComponent = 11,
    }
}

unit_code! {
    impl InvalidExpressionReason {
        ExpectedOperatorBeforeExpression = 1,
        UnresolvedStackShape = 2,
        MothFileHasNoValue = 3,
        ExtensionlessFileValue = 4,
        EmptyRuntimeAnonymousRecord = 5,
        AnonymousRecordFieldNotNamed = 6,
    }
}

unit_code! {
    impl OperatorOperandPosition {
        Unary = 1,
        BinaryLeft = 2,
        BinaryRight = 3,
    }
}

unit_code! {
    impl SourceSpanCapacityResource {
        ExtendedSpan = 1,
        LogicalPath = 2,
        SourceIdentity = 3,
        PathSyntax = 4,
        NumericLiteral = 5,
        Token = 6,
    }
}

unit_code! {
    pub(crate) enum InvalidStringEscapeCode {
        UnsupportedEscape = 1,
        PhysicalNewline = 2,
        TrailingBackslash = 3,
    }
}

impl InvalidStringEscapeCode {
    /// Data-less reasons keep a neutral character fact that their code never reads.
    pub(crate) fn split(reason: &InvalidStringEscapeReason) -> (Self, char) {
        match *reason {
            InvalidStringEscapeReason::UnsupportedEscape { escaped } => {
                (Self::UnsupportedEscape, escaped)
            }
            InvalidStringEscapeReason::PhysicalNewline => (Self::PhysicalNewline, '\0'),
            InvalidStringEscapeReason::TrailingBackslash => (Self::TrailingBackslash, '\0'),
        }
    }

    pub(crate) fn of(reason: &InvalidStringEscapeReason) -> Self {
        Self::split(reason).0
    }

    pub(crate) fn to_reason(self, escaped: char) -> InvalidStringEscapeReason {
        match self {
            Self::UnsupportedEscape => InvalidStringEscapeReason::UnsupportedEscape { escaped },
            Self::PhysicalNewline => InvalidStringEscapeReason::PhysicalNewline,
            Self::TrailingBackslash => InvalidStringEscapeReason::TrailingBackslash,
        }
    }
}

unit_code! {
    pub(crate) enum CommonSyntaxMistakeCode {
        EqualityOperator = 1,
        InequalityOperator = 2,
        LogicalAndOperator = 3,
        LogicalOrOperator = 4,
        BooleanBangNegation = 5,
        ExpressionAssignment = 6,
        RustBorrowPrefix = 7,
        InvalidAsOperator = 8,
        StatementLineComment = 9,
        FunctionKeyword = 10,
        LetOrVarKeyword = 11,
        ConstKeyword = 12,
        MatchKeyword = 13,
        StructKeyword = 14,
        SignatureParenthesisDelimiter = 15,
        SignatureAsKeyword = 16,
        InvalidCompileTimeBindingSpacing = 17,
        InvalidConfigQualifierSpacing = 18,
        InvalidMutableBindingSpacing = 19,
        InvalidSymbolicSpacing = 20,
        InvalidChoiceVariantSpacing = 21,
        InvalidUnaryNegationSpacing = 22,
        UnsupportedUnaryPlus = 23,
    }
}

impl CommonSyntaxMistakeCode {
    pub(crate) fn split(
        reason: &CommonSyntaxMistakeReason,
    ) -> (
        Self,
        Option<StringId>,
        Option<SymbolicSpacingError>,
        Option<ChoiceVariantSeparatorGap>,
    ) {
        match reason {
            CommonSyntaxMistakeReason::EqualityOperator => {
                (Self::EqualityOperator, None, None, None)
            }
            CommonSyntaxMistakeReason::InequalityOperator => {
                (Self::InequalityOperator, None, None, None)
            }
            CommonSyntaxMistakeReason::LogicalAndOperator => {
                (Self::LogicalAndOperator, None, None, None)
            }
            CommonSyntaxMistakeReason::LogicalOrOperator => {
                (Self::LogicalOrOperator, None, None, None)
            }
            CommonSyntaxMistakeReason::BooleanBangNegation => {
                (Self::BooleanBangNegation, None, None, None)
            }
            CommonSyntaxMistakeReason::ExpressionAssignment => {
                (Self::ExpressionAssignment, None, None, None)
            }
            CommonSyntaxMistakeReason::RustBorrowPrefix => {
                (Self::RustBorrowPrefix, None, None, None)
            }
            CommonSyntaxMistakeReason::InvalidAsOperator => {
                (Self::InvalidAsOperator, None, None, None)
            }
            CommonSyntaxMistakeReason::StatementLineComment => {
                (Self::StatementLineComment, None, None, None)
            }
            CommonSyntaxMistakeReason::FunctionKeyword { keyword } => {
                (Self::FunctionKeyword, Some(*keyword), None, None)
            }
            CommonSyntaxMistakeReason::LetOrVarKeyword => (Self::LetOrVarKeyword, None, None, None),
            CommonSyntaxMistakeReason::ConstKeyword => (Self::ConstKeyword, None, None, None),
            CommonSyntaxMistakeReason::MatchKeyword => (Self::MatchKeyword, None, None, None),
            CommonSyntaxMistakeReason::StructKeyword { keyword } => {
                (Self::StructKeyword, Some(*keyword), None, None)
            }
            CommonSyntaxMistakeReason::SignatureParenthesisDelimiter => {
                (Self::SignatureParenthesisDelimiter, None, None, None)
            }
            CommonSyntaxMistakeReason::SignatureAsKeyword => {
                (Self::SignatureAsKeyword, None, None, None)
            }
            CommonSyntaxMistakeReason::InvalidCompileTimeBindingSpacing => {
                (Self::InvalidCompileTimeBindingSpacing, None, None, None)
            }
            CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing => {
                (Self::InvalidConfigQualifierSpacing, None, None, None)
            }
            CommonSyntaxMistakeReason::InvalidMutableBindingSpacing => {
                (Self::InvalidMutableBindingSpacing, None, None, None)
            }
            CommonSyntaxMistakeReason::InvalidSymbolicSpacing { error } => {
                (Self::InvalidSymbolicSpacing, None, Some(*error), None)
            }
            CommonSyntaxMistakeReason::InvalidChoiceVariantSpacing { gap } => {
                (Self::InvalidChoiceVariantSpacing, None, None, Some(*gap))
            }
            CommonSyntaxMistakeReason::InvalidUnaryNegationSpacing => {
                (Self::InvalidUnaryNegationSpacing, None, None, None)
            }
            CommonSyntaxMistakeReason::UnsupportedUnaryPlus => {
                (Self::UnsupportedUnaryPlus, None, None, None)
            }
        }
    }

    pub(crate) fn of(reason: &CommonSyntaxMistakeReason) -> Self {
        Self::split(reason).0
    }

    pub(crate) fn to_reason(
        self,
        keyword: Option<StringId>,
        spacing_error: Option<SymbolicSpacingError>,
        choice_gap: Option<ChoiceVariantSeparatorGap>,
    ) -> Option<CommonSyntaxMistakeReason> {
        Some(match self {
            Self::EqualityOperator => CommonSyntaxMistakeReason::EqualityOperator,
            Self::InequalityOperator => CommonSyntaxMistakeReason::InequalityOperator,
            Self::LogicalAndOperator => CommonSyntaxMistakeReason::LogicalAndOperator,
            Self::LogicalOrOperator => CommonSyntaxMistakeReason::LogicalOrOperator,
            Self::BooleanBangNegation => CommonSyntaxMistakeReason::BooleanBangNegation,
            Self::ExpressionAssignment => CommonSyntaxMistakeReason::ExpressionAssignment,
            Self::RustBorrowPrefix => CommonSyntaxMistakeReason::RustBorrowPrefix,
            Self::InvalidAsOperator => CommonSyntaxMistakeReason::InvalidAsOperator,
            Self::StatementLineComment => CommonSyntaxMistakeReason::StatementLineComment,
            Self::FunctionKeyword => {
                CommonSyntaxMistakeReason::FunctionKeyword { keyword: keyword? }
            }
            Self::LetOrVarKeyword => CommonSyntaxMistakeReason::LetOrVarKeyword,
            Self::ConstKeyword => CommonSyntaxMistakeReason::ConstKeyword,
            Self::MatchKeyword => CommonSyntaxMistakeReason::MatchKeyword,
            Self::StructKeyword => CommonSyntaxMistakeReason::StructKeyword { keyword: keyword? },
            Self::SignatureParenthesisDelimiter => {
                CommonSyntaxMistakeReason::SignatureParenthesisDelimiter
            }
            Self::SignatureAsKeyword => CommonSyntaxMistakeReason::SignatureAsKeyword,
            Self::InvalidCompileTimeBindingSpacing => {
                CommonSyntaxMistakeReason::InvalidCompileTimeBindingSpacing
            }
            Self::InvalidConfigQualifierSpacing => {
                CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing
            }
            Self::InvalidMutableBindingSpacing => {
                CommonSyntaxMistakeReason::InvalidMutableBindingSpacing
            }
            Self::InvalidSymbolicSpacing => CommonSyntaxMistakeReason::InvalidSymbolicSpacing {
                error: spacing_error?,
            },
            Self::InvalidChoiceVariantSpacing => {
                CommonSyntaxMistakeReason::InvalidChoiceVariantSpacing { gap: choice_gap? }
            }
            Self::InvalidUnaryNegationSpacing => {
                CommonSyntaxMistakeReason::InvalidUnaryNegationSpacing
            }
            Self::UnsupportedUnaryPlus => CommonSyntaxMistakeReason::UnsupportedUnaryPlus,
        })
    }
}

unit_code! {
    enum SymbolicSpacingConstructCode {
        BinaryAdd = 1,
        BinarySubtract = 2,
        BinaryMultiply = 3,
        BinaryDivide = 4,
        BinaryIntDivide = 5,
        BinaryModulus = 6,
        BinaryExponent = 7,
        BinaryAnd = 8,
        BinaryOr = 9,
        BinaryGreaterThan = 10,
        BinaryGreaterThanOrEqual = 11,
        BinaryLessThan = 12,
        BinaryLessThanOrEqual = 13,
        BinaryEquality = 14,
        BinaryNotEqual = 15,
        BinaryNot = 16,
        BinaryRange = 17,
        Assignment = 18,
        CompoundAdd = 19,
        CompoundSubtract = 20,
        CompoundMultiply = 21,
        CompoundDivide = 22,
        CompoundIntDivide = 23,
        CompoundModulus = 24,
        CompoundExponent = 25,
        MutableDeclaration = 26,
    }
}

impl SymbolicSpacingConstructCode {
    fn of(construct: SymbolicSpacingConstruct) -> Self {
        match construct {
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Add,
            } => Self::BinaryAdd,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Subtract,
            } => Self::BinarySubtract,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Multiply,
            } => Self::BinaryMultiply,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Divide,
            } => Self::BinaryDivide,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::IntDivide,
            } => Self::BinaryIntDivide,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Modulus,
            } => Self::BinaryModulus,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Exponent,
            } => Self::BinaryExponent,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::And,
            } => Self::BinaryAnd,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Or,
            } => Self::BinaryOr,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::GreaterThan,
            } => Self::BinaryGreaterThan,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::GreaterThanOrEqual,
            } => Self::BinaryGreaterThanOrEqual,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::LessThan,
            } => Self::BinaryLessThan,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::LessThanOrEqual,
            } => Self::BinaryLessThanOrEqual,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Equality,
            } => Self::BinaryEquality,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::NotEqual,
            } => Self::BinaryNotEqual,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Not,
            } => Self::BinaryNot,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Range,
            } => Self::BinaryRange,
            SymbolicSpacingConstruct::Assignment => Self::Assignment,
            SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Add,
            } => Self::CompoundAdd,
            SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Subtract,
            } => Self::CompoundSubtract,
            SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Multiply,
            } => Self::CompoundMultiply,
            SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Divide,
            } => Self::CompoundDivide,
            SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::IntDivide,
            } => Self::CompoundIntDivide,
            SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Modulus,
            } => Self::CompoundModulus,
            SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Exponent,
            } => Self::CompoundExponent,
            SymbolicSpacingConstruct::MutableDeclaration => Self::MutableDeclaration,
        }
    }

    fn construct(self) -> SymbolicSpacingConstruct {
        match self {
            Self::BinaryAdd => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Add,
            },
            Self::BinarySubtract => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Subtract,
            },
            Self::BinaryMultiply => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Multiply,
            },
            Self::BinaryDivide => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Divide,
            },
            Self::BinaryIntDivide => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::IntDivide,
            },
            Self::BinaryModulus => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Modulus,
            },
            Self::BinaryExponent => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Exponent,
            },
            Self::BinaryAnd => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::And,
            },
            Self::BinaryOr => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Or,
            },
            Self::BinaryGreaterThan => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::GreaterThan,
            },
            Self::BinaryGreaterThanOrEqual => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::GreaterThanOrEqual,
            },
            Self::BinaryLessThan => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::LessThan,
            },
            Self::BinaryLessThanOrEqual => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::LessThanOrEqual,
            },
            Self::BinaryEquality => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Equality,
            },
            Self::BinaryNotEqual => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::NotEqual,
            },
            Self::BinaryNot => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Not,
            },
            Self::BinaryRange => SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Range,
            },
            Self::Assignment => SymbolicSpacingConstruct::Assignment,
            Self::CompoundAdd => SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Add,
            },
            Self::CompoundSubtract => SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Subtract,
            },
            Self::CompoundMultiply => SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Multiply,
            },
            Self::CompoundDivide => SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Divide,
            },
            Self::CompoundIntDivide => SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::IntDivide,
            },
            Self::CompoundModulus => SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Modulus,
            },
            Self::CompoundExponent => SymbolicSpacingConstruct::CompoundAssignment {
                operator: DiagnosticCompoundAssignmentOperator::Exponent,
            },
            Self::MutableDeclaration => SymbolicSpacingConstruct::MutableDeclaration,
        }
    }
}

/// Encodes the construct and whitespace side of a symbolic-spacing reason in one fact word.
pub(crate) struct OptSymbolicSpacingErrorFact;

impl FactCodec for OptSymbolicSpacingErrorFact {
    type Draft = Option<SymbolicSpacingError>;
    type Durable = Option<SymbolicSpacingError>;
    const WORDS: usize = 1;

    fn encode(
        draft: &Option<SymbolicSpacingError>,
        words: &mut [u32],
    ) -> Result<(), FactCapacityError> {
        let Some(error) = draft else {
            words[0] = u32::MAX;
            return Ok(());
        };
        let construct = SymbolicSpacingConstructCode::of(error.construct);
        let missing = match error.missing {
            MissingWhitespace::Before => 0,
            MissingWhitespace::After => 1,
            MissingWhitespace::Both => 2,
        };
        words[0] = (construct.code() << 2) | missing;
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Option<SymbolicSpacingError>> {
        if words[0] == u32::MAX {
            return Some(None);
        }
        let construct = SymbolicSpacingConstructCode::from_code(words[0] >> 2)?;
        let missing = match words[0] & 0b11 {
            0 => MissingWhitespace::Before,
            1 => MissingWhitespace::After,
            2 => MissingWhitespace::Both,
            _ => return None,
        };
        Some(Some(SymbolicSpacingError {
            construct: construct.construct(),
            missing,
        }))
    }

    fn remap_ids(_words: &mut [u32], _remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        Ok(())
    }
}

/// Stores the exact gap around a qualified choice separator.
pub(crate) struct OptChoiceVariantSeparatorGapFact;

impl FactCodec for OptChoiceVariantSeparatorGapFact {
    type Draft = Option<ChoiceVariantSeparatorGap>;
    type Durable = Option<ChoiceVariantSeparatorGap>;
    const WORDS: usize = 1;

    fn encode(
        draft: &Option<ChoiceVariantSeparatorGap>,
        words: &mut [u32],
    ) -> Result<(), FactCapacityError> {
        words[0] = match draft {
            None => u32::MAX,
            Some(ChoiceVariantSeparatorGap::Before) => 1,
            Some(ChoiceVariantSeparatorGap::After) => 2,
            Some(ChoiceVariantSeparatorGap::Both) => 3,
        };
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Option<ChoiceVariantSeparatorGap>> {
        match words[0] {
            u32::MAX => Some(None),
            1 => Some(Some(ChoiceVariantSeparatorGap::Before)),
            2 => Some(Some(ChoiceVariantSeparatorGap::After)),
            3 => Some(Some(ChoiceVariantSeparatorGap::Both)),
            _ => None,
        }
    }

    fn remap_ids(_words: &mut [u32], _remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        Ok(())
    }
}

diagnostic_family! {
    EXPECTED_TOKEN {
        internal: 1001,
        stable: "MOTH-SYNTAX-0001",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Expected token",
        facts: [expected: TokenFact, found: OptTokenFact],
        labels: [],
        extra: None,
        ctor: generated expected_token_from_projections,
        view: ExpectedTokenView,
        render: render_expected_token,
    }
}

diagnostic_family! {
    COMPILER_CAPACITY_EXCEEDED {
        internal: 1037,
        stable: "MOTH-SYNTAX-0037",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Compiler capacity exceeded",
        facts: [resource: CapacityResourceFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: CompilerCapacityExceededView,
        render: render_compiler_capacity_exceeded,
    }
}

/// The compiler-capacity draft for `resource` at `span`. Callers use it when a fixed-size limit is reached.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn compiler_capacity_exceeded(
    resource: impl Into<CapacityResource>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&COMPILER_CAPACITY_EXCEEDED, span, |cursor| {
        cursor.push::<CapacityResourceFact>(&resource.into())
    })
}

/// Maps this group's legacy diagnostics onto their compact constructors. Declines other payloads.
pub(super) fn from_legacy(diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    let span = diagnostic.primary_span;
    match (diagnostic.kind, &diagnostic.payload) {
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::ExpectedToken),
            DiagnosticPayload::ExpectedToken { expected, found },
        ) => Some(expected_token_from_projections(*expected, *found, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::CompilerCapacityExceeded),
            DiagnosticPayload::CompilerCapacityExceeded { resource },
        ) => Some(compiler_capacity_exceeded(*resource, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken),
            DiagnosticPayload::UnexpectedToken { found },
        ) => Some(unexpected_token_from_tag(*found, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken),
            DiagnosticPayload::InvalidMultiBind {
                reason,
                target_name,
            },
        ) => Some(invalid_multi_bind_syntax(*reason, *target_name, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedTrailingComma),
            DiagnosticPayload::UnexpectedTrailingComma,
        ) => Some(unexpected_trailing_comma(span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnterminatedStringLiteral),
            DiagnosticPayload::None,
        ) => Some(unterminated_string_literal(span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCharacter),
            DiagnosticPayload::InvalidCharacter { character },
        ) => Some(invalid_character(*character, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidNumberLiteral),
            DiagnosticPayload::InvalidNumberLiteral {
                literal_text,
                reason,
            },
        ) => Some(invalid_number_literal(*literal_text, *reason, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCharLiteral),
            DiagnosticPayload::None,
        ) => Some(invalid_char_literal(span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidIdentifier),
            DiagnosticPayload::None,
        ) => Some(invalid_identifier(span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MissingClosingDelimiter),
            DiagnosticPayload::MissingClosingDelimiter { expected_delimiter },
        ) => Some(missing_closing_delimiter(*expected_delimiter, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedTokenInDeclaration),
            DiagnosticPayload::None,
        ) => Some(unexpected_token_in_declaration(span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedEndOfFile),
            DiagnosticPayload::UnexpectedEndOfFile { expected_delimiter },
        ) => Some(unexpected_end_of_file(*expected_delimiter, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidPath),
            DiagnosticPayload::InvalidPath { path_kind },
        ) => Some(invalid_path(*path_kind, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidExpression),
            DiagnosticPayload::InvalidExpression { reason },
        ) => Some(invalid_expression(*reason, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MissingOperatorOperand),
            DiagnosticPayload::MissingOperatorOperand { operator, position },
        ) => Some(missing_operator_operand(*operator, *position, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::ExpectedSymbolStatement),
            DiagnosticPayload::ExpectedSymbolStatement,
        ) => Some(expected_symbol_statement(span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MissingCollectionItem),
            DiagnosticPayload::MissingCollectionItem,
        ) => Some(missing_collection_item(span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::CommonSyntaxMistake),
            DiagnosticPayload::CommonSyntaxMistake { reason },
        ) => Some(match reason {
            CommonSyntaxMistakeReason::InvalidChoiceVariantSpacing { gap } => {
                invalid_choice_variant_spacing(*gap, span)
            }
            _ => common_syntax_mistake(reason.clone(), span),
        }),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStringEscape),
            DiagnosticPayload::InvalidStringEscape { reason },
        ) => Some(invalid_string_escape(*reason, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::SourceSpanCapacity),
            DiagnosticPayload::SourceSpanCapacity {
                start,
                length,
                resource,
            },
        ) => Some(source_span_capacity(*start, *length, *resource, span)),
        _ => None,
    }
}

diagnostic_family! {
    UNEXPECTED_END_OF_FILE {
        internal: 1017,
        stable: "MOTH-SYNTAX-0017",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Unexpected end of file",
        facts: [expected_delimiter: OptStringFact],
        labels: [],
        extra: None,
        ctor: generated unexpected_end_of_file,
        view: UnexpectedEndOfFileView,
        render: render_unexpected_end_of_file,
    }
}

diagnostic_family! {
    INVALID_PATH {
        internal: 1018,
        stable: "MOTH-SYNTAX-0018",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid path",
        facts: [path_kind: UnitFact<PathKind>],
        labels: [],
        extra: None,
        ctor: generated invalid_path,
        view: InvalidPathView,
        render: render_invalid_path,
    }
}

diagnostic_family! {
    INVALID_EXPRESSION {
        internal: 1023,
        stable: "MOTH-SYNTAX-0023",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid expression",
        facts: [reason: UnitFact<InvalidExpressionReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_expression,
        view: InvalidExpressionView,
        render: render_invalid_expression,
    }
}

diagnostic_family! {
    MISSING_OPERATOR_OPERAND {
        internal: 1024,
        stable: "MOTH-SYNTAX-0024",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Missing operator operand",
        facts: [operator: StringFact, position: UnitFact<OperatorOperandPosition>],
        labels: [],
        extra: None,
        ctor: generated missing_operator_operand,
        view: MissingOperatorOperandView,
        render: render_missing_operator_operand,
    }
}

diagnostic_family! {
    EXPECTED_SYMBOL_STATEMENT {
        internal: 1026,
        stable: "MOTH-SYNTAX-0026",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Expected symbol statement",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated expected_symbol_statement,
        view: ExpectedSymbolStatementView,
        render: render_expected_symbol_statement,
    }
}

diagnostic_family! {
    MISSING_COLLECTION_ITEM {
        internal: 1027,
        stable: "MOTH-SYNTAX-0027",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Missing collection item",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated missing_collection_item,
        view: MissingCollectionItemView,
        render: render_missing_collection_item,
    }
}

diagnostic_family! {
    COMMON_SYNTAX_MISTAKE {
        internal: 1031,
        stable: "MOTH-SYNTAX-0031",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Common syntax mistake",
        facts: [
            reason: UnitFact<CommonSyntaxMistakeCode>,
            keyword: OptStringFact,
            spacing_error: OptSymbolicSpacingErrorFact,
            choice_gap: OptChoiceVariantSeparatorGapFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        view: CommonSyntaxMistakeView,
        render: render_common_syntax_mistake,
    }
}

diagnostic_family! {
    INVALID_STRING_ESCAPE {
        internal: 1034,
        stable: "MOTH-SYNTAX-0034",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid string escape",
        facts: [reason: UnitFact<InvalidStringEscapeCode>, escaped: CharFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidStringEscapeView,
        render: render_invalid_string_escape,
    }
}

diagnostic_family! {
    SOURCE_SPAN_CAPACITY {
        internal: 1036,
        stable: "MOTH-SYNTAX-0036",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Source span capacity exceeded",
        facts: [
            start: CountFact,
            length: CountFact,
            resource: UnitFact<SourceSpanCapacityResource>,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        view: SourceSpanCapacityView,
        render: render_source_span_capacity,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn common_syntax_mistake(
    reason: CommonSyntaxMistakeReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (reason, keyword, spacing_error, choice_gap) = CommonSyntaxMistakeCode::split(&reason);
    DiagnosticDraft::with_facts(&COMMON_SYNTAX_MISTAKE, span, |cursor| {
        cursor.push::<UnitFact<CommonSyntaxMistakeCode>>(&reason)?;
        cursor.push::<OptStringFact>(&keyword)?;
        cursor.push::<OptSymbolicSpacingErrorFact>(&spacing_error)?;
        cursor.push::<OptChoiceVariantSeparatorGapFact>(&choice_gap)
    })
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_choice_variant_spacing(
    gap: ChoiceVariantSeparatorGap,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    common_syntax_mistake(
        CommonSyntaxMistakeReason::InvalidChoiceVariantSpacing { gap },
        span,
    )
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_string_escape(
    reason: InvalidStringEscapeReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (reason, escaped) = InvalidStringEscapeCode::split(&reason);
    DiagnosticDraft::with_facts(&INVALID_STRING_ESCAPE, span, |cursor| {
        cursor.push::<UnitFact<InvalidStringEscapeCode>>(&reason)?;
        cursor.push::<CharFact>(&escaped)
    })
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn source_span_capacity(
    start: u32,
    length: u32,
    resource: SourceSpanCapacityResource,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&SOURCE_SPAN_CAPACITY, span, |cursor| {
        let start = usize::try_from(start).map_err(|_| FactCapacityError)?;
        let length = usize::try_from(length).map_err(|_| FactCapacityError)?;
        cursor.push::<CountFact>(&start)?;
        cursor.push::<CountFact>(&length)?;
        cursor.push::<UnitFact<SourceSpanCapacityResource>>(&resource)
    })
}

diagnostic_family! {
    UNEXPECTED_TOKEN {
        internal: 1002,
        stable: "MOTH-SYNTAX-0002",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Unexpected token",
        facts: [shape: UnexpectedTokenFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: UnexpectedTokenView,
        render: render_unexpected_token,
    }
}

/// The unexpected-token draft for a token the parser did not expect, at `span`.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn unexpected_token_from_tag(
    found: DiagnosticToken,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&UNEXPECTED_TOKEN, span, |cursor| {
        cursor.push::<UnexpectedTokenFact>(&UnexpectedTokenShape::Token(found))
    })
}

/// The unexpected-token draft for a multi-bind mismatch at `span`.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_multi_bind_syntax(
    reason: InvalidMultiBindReason,
    target_name: Option<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (expected, found) = match &reason {
        InvalidMultiBindReason::ArityMismatch { expected, found } => (*expected, *found),
        _ => (0, 0),
    };
    let code = InvalidMultiBindCode::of(&reason);
    DiagnosticDraft::with_facts(&UNEXPECTED_TOKEN, span, |cursor| {
        cursor.push::<UnexpectedTokenFact>(&UnexpectedTokenShape::MultiBind {
            code,
            target_name,
            expected,
            found,
        })
    })
}

diagnostic_family! {
    UNEXPECTED_TRAILING_COMMA {
        internal: 1003,
        stable: "MOTH-SYNTAX-0003",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Unexpected trailing comma",
        facts: [],
        labels: [],
        extra: None,
        ctor: manual,
        view: UnexpectedTrailingCommaView,
        render: render_unexpected_trailing_comma,
    }
}

/// The unexpected-trailing-comma draft at `span`. The row carries no facts.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn unexpected_trailing_comma(
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&UNEXPECTED_TRAILING_COMMA, span, |_cursor| Ok(()))
}

diagnostic_family! {
    UNTERMINATED_STRING_LITERAL {
        internal: 1006,
        stable: "MOTH-SYNTAX-0006",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Unterminated string literal",
        facts: [],
        labels: [],
        extra: None,
        ctor: manual,
        view: UnterminatedStringLiteralView,
        render: render_title_only,
    }
}

/// The unterminated-string-literal draft at `span`. The row carries no facts.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn unterminated_string_literal(
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&UNTERMINATED_STRING_LITERAL, span, |_cursor| Ok(()))
}

diagnostic_family! {
    INVALID_CHARACTER {
        internal: 1007,
        stable: "MOTH-SYNTAX-0007",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid character",
        facts: [character: CharFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidCharacterView,
        render: render_invalid_character,
    }
}

/// The invalid-character draft for `character` at `span`.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_character(
    character: char,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_CHARACTER, span, |cursor| {
        cursor.push::<CharFact>(&character)
    })
}

diagnostic_family! {
    INVALID_NUMBER_LITERAL {
        internal: 1008,
        stable: "MOTH-SYNTAX-0008",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid number literal",
        facts: [
            literal_text: StringFact,
            code: UnitFact<NumberLiteralCode>,
            scalar: OptFixedScalarFact,
            scale: OptNumberScaleFact
        ],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidNumberLiteralView,
        render: render_invalid_number_literal,
    }
}

/// The invalid-number-literal draft for `literal_text` and `reason` at `span`.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_number_literal(
    literal_text: StringId,
    reason: NumberLiteralErrorReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, scalar, scale) = NumberLiteralCode::split(&reason);
    DiagnosticDraft::with_facts(&INVALID_NUMBER_LITERAL, span, |cursor| {
        cursor.push::<StringFact>(&literal_text)?;
        cursor.push::<UnitFact<NumberLiteralCode>>(&code)?;
        cursor.push::<OptFixedScalarFact>(&scalar)?;
        cursor.push::<OptNumberScaleFact>(&scale)
    })
}

diagnostic_family! {
    INVALID_CHAR_LITERAL {
        internal: 1009,
        stable: "MOTH-SYNTAX-0009",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid character literal",
        facts: [],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidCharLiteralView,
        render: render_title_only,
    }
}

/// The invalid-character-literal draft at `span`. The row carries no facts.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_char_literal(
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_CHAR_LITERAL, span, |_cursor| Ok(()))
}

diagnostic_family! {
    INVALID_IDENTIFIER {
        internal: 1011,
        stable: "MOTH-SYNTAX-0011",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid identifier",
        facts: [],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidIdentifierView,
        render: render_title_only,
    }
}

/// The invalid-identifier draft at `span`. The row carries no facts.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_identifier(
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_IDENTIFIER, span, |_cursor| Ok(()))
}

diagnostic_family! {
    MISSING_CLOSING_DELIMITER {
        internal: 1012,
        stable: "MOTH-SYNTAX-0012",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Missing closing delimiter",
        facts: [expected_delimiter: StringFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: MissingClosingDelimiterView,
        render: render_missing_closing_delimiter,
    }
}

/// The missing-closing-delimiter draft for `expected_delimiter` at `span`.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn missing_closing_delimiter(
    expected_delimiter: StringId,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&MISSING_CLOSING_DELIMITER, span, |cursor| {
        cursor.push::<StringFact>(&expected_delimiter)
    })
}

diagnostic_family! {
    UNEXPECTED_TOKEN_IN_DECLARATION {
        internal: 1013,
        stable: "MOTH-SYNTAX-0013",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Unexpected token in declaration",
        facts: [],
        labels: [],
        extra: None,
        ctor: manual,
        view: UnexpectedTokenInDeclarationView,
        render: render_title_only,
    }
}

/// The unexpected-token-in-declaration draft at `span`. The row carries no facts.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn unexpected_token_in_declaration(
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&UNEXPECTED_TOKEN_IN_DECLARATION, span, |_cursor| Ok(()))
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    EXPECTED_TOKEN,
    COMPILER_CAPACITY_EXCEEDED,
    UNEXPECTED_TOKEN,
    UNEXPECTED_TRAILING_COMMA,
    UNTERMINATED_STRING_LITERAL,
    INVALID_CHARACTER,
    INVALID_NUMBER_LITERAL,
    INVALID_CHAR_LITERAL,
    INVALID_IDENTIFIER,
    MISSING_CLOSING_DELIMITER,
    UNEXPECTED_TOKEN_IN_DECLARATION,
    UNEXPECTED_END_OF_FILE,
    INVALID_PATH,
    INVALID_EXPRESSION,
    MISSING_OPERATOR_OPERAND,
    EXPECTED_SYMBOL_STATEMENT,
    MISSING_COLLECTION_ITEM,
    COMMON_SYNTAX_MISTAKE,
    INVALID_STRING_ESCAPE,
    SOURCE_SPAN_CAPACITY,
];
