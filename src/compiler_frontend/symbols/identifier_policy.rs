//! Compiler-local identifier naming and diagnostic helpers.
//!
//! WHAT: owns naming-style checks, warning policy and structured diagnostic construction for
//! identifiers; shared character and reserved-name rules come from moth-lexical.
//! WHY: compiler presentation and diagnostics must remain local without duplicating neutral name
//! policy across frontend stages.

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, NamingConvention, ReservedNameOwner,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use moth_lexical::identifier::is_reserved_user_name;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentifierNamingKind {
    TypeLike,
    ValueLike,
    TopLevelConstant,
}

/// Returns true for CamelCase-style type identifiers with an uppercase first character followed by
/// alphanumeric characters.
pub(crate) fn is_camel_case_type_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    first.is_uppercase() && chars.all(|ch| ch.is_alphanumeric())
}

/// Returns true for lowercase_with_underscores identifiers.
///
/// Rule: lowercase letters/digits/underscores only, and at least one lowercase letter.
pub(crate) fn is_lowercase_with_underscores_name(name: &str) -> bool {
    let mut has_lowercase = false;

    for ch in name.chars() {
        if ch.is_lowercase() {
            has_lowercase = true;
            continue;
        }

        if ch.is_ascii_digit() || ch == '_' {
            continue;
        }

        return false;
    }

    has_lowercase
}

/// Returns true for UPPER_CASE constant identifiers.
///
/// Rule: uppercase letters/digits/underscores only, and at least one uppercase letter.
pub(crate) fn is_uppercase_constant_name(name: &str) -> bool {
    let mut has_uppercase = false;

    for ch in name.chars() {
        if ch.is_uppercase() {
            has_uppercase = true;
            continue;
        }

        if ch.is_ascii_digit() || ch == '_' {
            continue;
        }

        return false;
    }

    has_uppercase
}

pub(crate) fn naming_warning_for_identifier(
    name: crate::compiler_frontend::symbols::string_interning::StringId,
    span: Option<SourceSpan>,
    naming_kind: IdentifierNamingKind,
    string_table: &StringTable,
) -> Option<CompilerDiagnostic> {
    let identifier = string_table.resolve(name);
    match naming_kind {
        IdentifierNamingKind::TypeLike => {
            if is_camel_case_type_name(identifier) {
                return None;
            }

            Some(CompilerDiagnostic::identifier_naming_convention(
                name,
                NamingConvention::CamelCase,
                span,
            ))
        }
        IdentifierNamingKind::ValueLike => {
            if is_lowercase_with_underscores_name(identifier) {
                return None;
            }

            Some(CompilerDiagnostic::identifier_naming_convention(
                name,
                NamingConvention::LowercaseWithUnderscores,
                span,
            ))
        }
        IdentifierNamingKind::TopLevelConstant => {
            if is_lowercase_with_underscores_name(identifier)
                || is_uppercase_constant_name(identifier)
            {
                return None;
            }

            Some(CompilerDiagnostic::identifier_naming_convention(
                name,
                NamingConvention::LowercaseOrUppercaseWithUnderscores,
                span,
            ))
        }
    }
}

pub(crate) fn reserved_keyword_shadow_error(
    name: StringId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::reserved_name_collision(name, ReservedNameOwner::Keyword, span)
}

/// Direct diagnostic result for reserved-identifier checks.
///
/// Most parser callers preserve this source diagnostic on their own typed boundary.
type IdentifierPolicyResult<T> = Result<T, CompilerDiagnostic>;

pub(crate) fn ensure_not_keyword_shadow_identifier(
    name: StringId,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> IdentifierPolicyResult<()> {
    let identifier = string_table.resolve(name);
    if is_reserved_user_name(identifier) {
        return Err(reserved_keyword_shadow_error(name, span));
    }

    Ok(())
}
