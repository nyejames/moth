//! Family rows for token and lexical diagnostics, and for the compiler-capacity diagnostic.
//!
//! WHAT: the expected-token and compiler-capacity pilots, in full, and the codec that stores a capacity resource.
//! The other 18 rows are identity rows for syntax families in this group.
//! WHY: capacity faults can come from any stage, so their pilot row lives beside the token rows.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    FactCapacityError, FactCodec, OptTokenFact, TokenFact, UnitCode, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_compiler_capacity_exceeded, render_expected_token,
};
use crate::compiler_frontend::compiler_messages::{
    AstCapacityResource, CapacityResource, CompilerDiagnostic, DiagnosticCapacityResource,
    DiagnosticCategory, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity, HirCapacityResource,
    SyntaxDiagnosticKind,
};
use crate::compiler_frontend::source::SourceSpan;

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
}

diagnostic_family! {
    pilot EXPECTED_TOKEN {
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
    pilot COMPILER_CAPACITY_EXCEEDED {
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

/// Maps the legacy expected-token and capacity diagnostics onto their compact constructors. Declines other payloads.
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
        _ => None,
    }
}

diagnostic_family! {
    identity UNEXPECTED_TOKEN {
        internal: 1002,
        stable: "MOTH-SYNTAX-0002",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity UNEXPECTED_TRAILING_COMMA {
        internal: 1003,
        stable: "MOTH-SYNTAX-0003",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity UNTERMINATED_STRING_LITERAL {
        internal: 1006,
        stable: "MOTH-SYNTAX-0006",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_CHARACTER {
        internal: 1007,
        stable: "MOTH-SYNTAX-0007",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_NUMBER_LITERAL {
        internal: 1008,
        stable: "MOTH-SYNTAX-0008",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_CHAR_LITERAL {
        internal: 1009,
        stable: "MOTH-SYNTAX-0009",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_IDENTIFIER {
        internal: 1011,
        stable: "MOTH-SYNTAX-0011",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity MISSING_CLOSING_DELIMITER {
        internal: 1012,
        stable: "MOTH-SYNTAX-0012",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity UNEXPECTED_TOKEN_IN_DECLARATION {
        internal: 1013,
        stable: "MOTH-SYNTAX-0013",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity UNEXPECTED_END_OF_FILE {
        internal: 1017,
        stable: "MOTH-SYNTAX-0017",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_PATH {
        internal: 1018,
        stable: "MOTH-SYNTAX-0018",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_EXPRESSION {
        internal: 1023,
        stable: "MOTH-SYNTAX-0023",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity MISSING_OPERATOR_OPERAND {
        internal: 1024,
        stable: "MOTH-SYNTAX-0024",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity EXPECTED_SYMBOL_STATEMENT {
        internal: 1026,
        stable: "MOTH-SYNTAX-0026",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity MISSING_COLLECTION_ITEM {
        internal: 1027,
        stable: "MOTH-SYNTAX-0027",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity COMMON_SYNTAX_MISTAKE {
        internal: 1031,
        stable: "MOTH-SYNTAX-0031",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_STRING_ESCAPE {
        internal: 1034,
        stable: "MOTH-SYNTAX-0034",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity SOURCE_SPAN_CAPACITY {
        internal: 1036,
        stable: "MOTH-SYNTAX-0036",
        category: DiagnosticCategory::Syntax,
    }
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
