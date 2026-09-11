//! Shared identifier naming and reserved-keyword policy helpers.
//!
//! WHAT: centralizes naming-style warnings and keyword-shadow reservation checks used by
//! header parsing and AST binding creation.
//! WHY: identifier rules should not drift between frontend stages; one module keeps policy
//! and diagnostics consistent.

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, NamingConvention, ReservedNameOwner,
};
use crate::compiler_frontend::keywords::RESERVED_KEYWORD_SHADOWS;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdentifierNamingKind {
    TypeLike,
    ValueLike,
    TopLevelConstant,
}

/// Returns `name` without leading underscore characters.
pub(crate) fn strip_leading_underscores(name: &str) -> &str {
    name.trim_start_matches('_')
}

/// Returns the canonical keyword matched by this identifier shadow, if any.
///
/// Example shadows: `_true`, `FALSE`, `__LoOp`.
pub(crate) fn keyword_shadow_match(name: &str) -> Option<&'static str> {
    let stripped = strip_leading_underscores(name);
    if stripped.is_empty() {
        return None;
    }

    RESERVED_KEYWORD_SHADOWS
        .iter()
        .copied()
        .find(|keyword| stripped.eq_ignore_ascii_case(keyword))
}

pub(crate) fn is_keyword_shadow_identifier(name: &str) -> bool {
    keyword_shadow_match(name).is_some()
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
    if is_keyword_shadow_identifier(identifier) {
        return Err(reserved_keyword_shadow_error(name, span));
    }

    Ok(())
}
