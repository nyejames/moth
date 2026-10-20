//! Family rows for project configuration diagnostics.
//!
//! WHAT: identity rows for the eleven project-configuration families. The group holds no pilot rows.
//! WHY: the table declares each family's stable code and category, so the registry covers this group.

use crate::compiler_frontend::compiler_messages::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};

diagnostic_family! {
    identity INVALID_DEPENDENCY_CLAUSE {
        internal: 1019,
        stable: "MOTH-SYNTAX-0019",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity LEGACY_DEPENDENCY_CLAUSE {
        internal: 1035,
        stable: "MOTH-SYNTAX-0035",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity CIRCULAR_DEPENDENCY {
        internal: 3033,
        stable: "MOTH-RULE-0033",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity UNSUPPORTED_BUILDER_PACKAGE {
        internal: 3060,
        stable: "MOTH-RULE-0060",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_PAGE_METADATA {
        internal: 3061,
        stable: "MOTH-RULE-0061",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_COMPILE_TIME_PATH {
        internal: 3063,
        stable: "MOTH-RULE-0063",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity DEPENDENCY_NAMESPACE_USED_AS_VALUE {
        internal: 3065,
        stable: "MOTH-RULE-0065",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity NESTED_DEPENDENCY_TRAVERSAL {
        internal: 3066,
        stable: "MOTH-RULE-0066",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity NAMESPACE_TYPE_VALUE_MISUSE {
        internal: 3067,
        stable: "MOTH-RULE-0067",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity PROJECT_CONTEXT_ESCAPE {
        internal: 3086,
        stable: "MOTH-RULE-0086",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_CONFIG {
        internal: 6001,
        stable: "MOTH-CONFIG-0001",
        category: DiagnosticCategory::Config,
    }
}

pub(crate) const FAMILIES: &[FamilyRow] = &[
    INVALID_DEPENDENCY_CLAUSE,
    LEGACY_DEPENDENCY_CLAUSE,
    CIRCULAR_DEPENDENCY,
    UNSUPPORTED_BUILDER_PACKAGE,
    INVALID_PAGE_METADATA,
    INVALID_COMPILE_TIME_PATH,
    DEPENDENCY_NAMESPACE_USED_AS_VALUE,
    NESTED_DEPENDENCY_TRAVERSAL,
    NAMESPACE_TYPE_VALUE_MISUSE,
    PROJECT_CONTEXT_ESCAPE,
    INVALID_CONFIG,
];
