//! Family rows for generics, traits and API diagnostics.
//!
//! WHAT: identity rows for the 21 generics, trait and API families. The group holds no pilot rows.
//! WHY: the table declares each family's stable code and category, so the registry covers this group.

use crate::compiler_frontend::compiler_messages::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};

diagnostic_family! {
    identity INVALID_GENERIC_APPLICATION {
        internal: 1015,
        stable: "MOTH-SYNTAX-0015",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_GENERIC_PARAMETER {
        internal: 1020,
        stable: "MOTH-SYNTAX-0020",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_PARAMETER_CONTRACT {
        internal: 1038,
        stable: "MOTH-SYNTAX-0038",
        category: DiagnosticCategory::Syntax,
    }
}

diagnostic_family! {
    identity INVALID_THIS_USAGE {
        internal: 3040,
        stable: "MOTH-RULE-0040",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_RECEIVER_DECLARATION {
        internal: 3041,
        stable: "MOTH-RULE-0041",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_GENERIC_INSTANTIATION {
        internal: 3057,
        stable: "MOTH-RULE-0057",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_FUNCTION_SIGNATURE {
        internal: 3062,
        stable: "MOTH-RULE-0062",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity UNKNOWN_TRAIT {
        internal: 3069,
        stable: "MOTH-RULE-0069",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity DUPLICATE_TRAIT_REQUIREMENT {
        internal: 3070,
        stable: "MOTH-RULE-0070",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity TRAIT_PRIVATE_SURFACE_LEAK {
        internal: 3071,
        stable: "MOTH-RULE-0071",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity UNSUPPORTED_TRAIT_FEATURE {
        internal: 3072,
        stable: "MOTH-RULE-0072",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_TRAIT_CONFORMANCE {
        internal: 3073,
        stable: "MOTH-RULE-0073",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity GENERIC_BOUND_PRIVATE_SURFACE_LEAK {
        internal: 3074,
        stable: "MOTH-RULE-0074",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity TRAIT_NAME_USED_AS_TYPE {
        internal: 3075,
        stable: "MOTH-RULE-0075",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_TRAIT_KEYWORD_USAGE {
        internal: 3076,
        stable: "MOTH-RULE-0076",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity EXPORT_OUTSIDE_MODULE_ROOT {
        internal: 3077,
        stable: "MOTH-RULE-0077",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_EXPORT_TARGET {
        internal: 3080,
        stable: "MOTH-RULE-0080",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity DUPLICATE_PUBLIC_EXPORT {
        internal: 3081,
        stable: "MOTH-RULE-0081",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity PRIVATE_TYPE_IN_EXPORTED_API {
        internal: 3082,
        stable: "MOTH-RULE-0082",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity INVALID_TRAIT_INCOMPATIBILITY {
        internal: 3084,
        stable: "MOTH-RULE-0084",
        category: DiagnosticCategory::Rule,
    }
}

diagnostic_family! {
    identity DUPLICATE_EXPORT_BLOCK {
        internal: 3085,
        stable: "MOTH-RULE-0085",
        category: DiagnosticCategory::Rule,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    INVALID_GENERIC_APPLICATION,
    INVALID_GENERIC_PARAMETER,
    INVALID_PARAMETER_CONTRACT,
    INVALID_THIS_USAGE,
    INVALID_RECEIVER_DECLARATION,
    INVALID_GENERIC_INSTANTIATION,
    INVALID_FUNCTION_SIGNATURE,
    UNKNOWN_TRAIT,
    DUPLICATE_TRAIT_REQUIREMENT,
    TRAIT_PRIVATE_SURFACE_LEAK,
    UNSUPPORTED_TRAIT_FEATURE,
    INVALID_TRAIT_CONFORMANCE,
    GENERIC_BOUND_PRIVATE_SURFACE_LEAK,
    TRAIT_NAME_USED_AS_TYPE,
    INVALID_TRAIT_KEYWORD_USAGE,
    EXPORT_OUTSIDE_MODULE_ROOT,
    INVALID_EXPORT_TARGET,
    DUPLICATE_PUBLIC_EXPORT,
    PRIVATE_TYPE_IN_EXPORTED_API,
    INVALID_TRAIT_INCOMPATIBILITY,
    DUPLICATE_EXPORT_BLOCK,
];
