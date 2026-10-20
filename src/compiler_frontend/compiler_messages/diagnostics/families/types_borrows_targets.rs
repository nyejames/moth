//! Family rows for type, borrow and target diagnostics.
//!
//! WHAT: the type-mismatch and uninitialised-local pilots, in full, with the code for each mismatch context.
//! The other 13 rows are identity rows for type, borrow and target families; four of them are reserved.
//! WHY: type and borrow checks report over the same type and place facts, so their rows share one group.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    PlaceFact, TypeFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_type_mismatch, render_use_of_uninitialized_local,
};
use crate::compiler_frontend::compiler_messages::{
    BorrowDiagnosticKind, CompilerDiagnostic, DiagnosticCategory, DiagnosticKind,
    DiagnosticPayload, DiagnosticPlace, DiagnosticSeverity, TypeDiagnosticKind,
    TypeMismatchContext,
};
use crate::compiler_frontend::source::SourceSpan;

unit_code! {
    impl TypeMismatchContext {
        Assignment = 1,
        Declaration = 2,
        ReturnValue = 3,
        FunctionArgument = 4,
        AssertionArgument = 5,
        ConstructorArgument = 6,
        ReceiverArgument = 7,
        Operator = 8,
        Condition = 9,
        CollectionElement = 10,
        StructFieldDefault = 11,
        TemplateInterpolation = 12,
        MatchScrutinee = 13,
        MatchPattern = 14,
        ErrorReturn = 15,
        Pattern = 16,
        General = 17,
    }
}

diagnostic_family! {
    pilot TYPE_MISMATCH {
        internal: 2001,
        stable: "MOTH-TYPE-0001",
        category: DiagnosticCategory::Type,
        severity: DiagnosticSeverity::Error,
        title: "Type mismatch",
        facts: [expected: TypeFact, found: TypeFact, context: UnitFact<TypeMismatchContext>],
        labels: [],
        extra: None,
        ctor: generated type_mismatch,
        view: TypeMismatchView,
        render: render_type_mismatch,
    }
}

diagnostic_family! {
    pilot USE_OF_UNINITIALIZED_LOCAL {
        internal: 5009,
        stable: "MOTH-BORROW-0009",
        category: DiagnosticCategory::Borrow,
        severity: DiagnosticSeverity::Error,
        title: "Use of uninitialized local",
        facts: [place: PlaceFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: UseOfUninitializedLocalView,
        render: render_use_of_uninitialized_local,
    }
}

/// The uninitialised-local draft. The place is stored in the place fact. A `RenderedText` place is stored as a local,
/// since both render as the same backticked name (see `PlaceFact`).
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn use_of_uninitialized_local(
    place: DiagnosticPlace,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&USE_OF_UNINITIALIZED_LOCAL, span, |cursor| {
        cursor.push::<PlaceFact>(&place)
    })
}

/// Maps the legacy type-mismatch and uninitialised-local diagnostics onto their compact constructors. Declines
/// other payloads.
pub(super) fn from_legacy(diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    let span = diagnostic.primary_span;
    match (diagnostic.kind, &diagnostic.payload) {
        (
            DiagnosticKind::Type(TypeDiagnosticKind::TypeMismatch),
            DiagnosticPayload::TypeMismatch {
                expected,
                found,
                context,
            },
        ) => Some(type_mismatch(*expected, *found, *context, span)),
        (
            DiagnosticKind::Borrow(BorrowDiagnosticKind::UseOfUninitializedLocal),
            DiagnosticPayload::UseOfUninitializedLocal { place },
        ) => Some(use_of_uninitialized_local(place.clone(), span)),
        _ => None,
    }
}

diagnostic_family! {
    identity EMPTY_COLLECTION_TYPE_AMBIGUITY {
        internal: 2002,
        stable: "MOTH-TYPE-0002",
        category: DiagnosticCategory::Type,
    }
}

diagnostic_family! {
    identity UNSUPPORTED_OPERATOR_TYPES {
        internal: 2003,
        stable: "MOTH-TYPE-0003",
        category: DiagnosticCategory::Type,
    }
}

diagnostic_family! {
    identity INCOMPATIBLE_CHOICE_COMPARISON {
        internal: 2005,
        stable: "MOTH-TYPE-0005",
        category: DiagnosticCategory::Type,
    }
}

diagnostic_family! {
    identity INVALID_ROUTE_OUTPUT {
        internal: 2006,
        stable: "MOTH-TYPE-0006",
        category: DiagnosticCategory::Type,
    }
}

diagnostic_family! {
    identity UNSUPPORTED_EXTERNAL_FUNCTION {
        internal: 3058,
        stable: "MOTH-RULE-0058",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity UNSUPPORTED_BACKEND_FEATURE {
        internal: 3064,
        stable: "MOTH-RULE-0064",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity BORROW_CONFLICT {
        internal: 5001,
        stable: "MOTH-BORROW-0001",
        category: DiagnosticCategory::Borrow,
        reserved,
    }
}

diagnostic_family! {
    identity MULTIPLE_MUTABLE_BORROWS {
        internal: 5002,
        stable: "MOTH-BORROW-0002",
        category: DiagnosticCategory::Borrow,
    }
}

diagnostic_family! {
    identity SHARED_MUTABLE_CONFLICT {
        internal: 5003,
        stable: "MOTH-BORROW-0003",
        category: DiagnosticCategory::Borrow,
    }
}

diagnostic_family! {
    identity USE_AFTER_POSSIBLE_MOVE {
        internal: 5004,
        stable: "MOTH-BORROW-0004",
        category: DiagnosticCategory::Borrow,
        reserved,
    }
}

diagnostic_family! {
    identity MOVE_WHILE_BORROWED {
        internal: 5005,
        stable: "MOTH-BORROW-0005",
        category: DiagnosticCategory::Borrow,
        reserved,
    }
}

diagnostic_family! {
    identity WHOLE_OBJECT_BORROW_CONFLICT {
        internal: 5006,
        stable: "MOTH-BORROW-0006",
        category: DiagnosticCategory::Borrow,
        reserved,
    }
}

diagnostic_family! {
    identity INVALID_MUTABLE_ACCESS {
        internal: 5007,
        stable: "MOTH-BORROW-0007",
        category: DiagnosticCategory::Borrow,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    TYPE_MISMATCH,
    USE_OF_UNINITIALIZED_LOCAL,
    EMPTY_COLLECTION_TYPE_AMBIGUITY,
    UNSUPPORTED_OPERATOR_TYPES,
    INCOMPATIBLE_CHOICE_COMPARISON,
    INVALID_ROUTE_OUTPUT,
    UNSUPPORTED_EXTERNAL_FUNCTION,
    UNSUPPORTED_BACKEND_FEATURE,
    BORROW_CONFLICT,
    MULTIPLE_MUTABLE_BORROWS,
    SHARED_MUTABLE_CONFLICT,
    USE_AFTER_POSSIBLE_MOVE,
    MOVE_WHILE_BORROWED,
    WHOLE_OBJECT_BORROW_CONFLICT,
    INVALID_MUTABLE_ACCESS,
];
