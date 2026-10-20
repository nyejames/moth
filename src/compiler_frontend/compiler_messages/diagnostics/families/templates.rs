//! Family rows for template diagnostics.
//!
//! WHAT: identity rows for the seven template families. The group holds no pilot rows.
//! WHY: the table declares each family's stable code and category, so the registry covers this group.

use crate::compiler_frontend::compiler_messages::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};

diagnostic_family! {
    identity MALFORMED_CSS_TEMPLATE {
        internal: 1004,
        stable: "MOTH-SYNTAX-0004",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity MALFORMED_HTML_TEMPLATE {
        internal: 1005,
        stable: "MOTH-SYNTAX-0005",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_STYLE_DIRECTIVE {
        internal: 1010,
        stable: "MOTH-SYNTAX-0010",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_TEMPLATE_DIRECTIVE {
        internal: 1021,
        stable: "MOTH-SYNTAX-0021",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_TEMPLATE_STRUCTURE {
        internal: 1022,
        stable: "MOTH-SYNTAX-0022",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity UNESCAPED_IMPLICIT_TEMPLATE_CLOSE {
        internal: 1032,
        stable: "MOTH-SYNTAX-0032",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_TEMPLATE_SLOT {
        internal: 3052,
        stable: "MOTH-RULE-0052",
        category: DiagnosticCategory::Rule,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    MALFORMED_CSS_TEMPLATE,
    MALFORMED_HTML_TEMPLATE,
    INVALID_STYLE_DIRECTIVE,
    INVALID_TEMPLATE_DIRECTIVE,
    INVALID_TEMPLATE_STRUCTURE,
    UNESCAPED_IMPLICIT_TEMPLATE_CLOSE,
    INVALID_TEMPLATE_SLOT,
];
