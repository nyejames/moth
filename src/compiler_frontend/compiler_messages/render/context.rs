//! Shared diagnostic render context and syntax-name helpers.
//!
//! WHAT: owns the render-boundary lookup context plus token/name spelling helpers used
//! by terminal, terse, and dev-server renderers.
//! WHY: diagnostic facts stay structured until this boundary, where stable IDs and token
//! kinds become user-facing prose.

use super::*;

/// Render-boundary data needed to turn diagnostic facts into user-facing text.
///
/// WHAT: carries shared lookup tables by reference while diagnostics keep only stable IDs.
/// WHY: type diagnostics should store semantic `TypeId`s, not rendered strings or owned
/// `TypeEnvironment` snapshots. The render boundary decides how those IDs become names.
#[derive(Clone, Copy)]
pub(crate) struct DiagnosticRenderContext<'a> {
    pub(crate) string_table: &'a StringTable,
    pub(crate) type_environment: Option<&'a TypeEnvironment>,
}

impl<'a> DiagnosticRenderContext<'a> {
    pub(crate) fn new(string_table: &'a StringTable) -> Self {
        Self {
            string_table,
            type_environment: None,
        }
    }

    pub(crate) fn with_optional_type_environment(
        mut self,
        type_environment: Option<&'a TypeEnvironment>,
    ) -> Self {
        self.type_environment = type_environment;
        self
    }
}

pub(crate) fn diagnostic_type_name(
    type_id: TypeId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    match context.type_environment {
        Some(type_environment) if type_environment.get(type_id).is_some() => {
            display_type(type_id, type_environment, context.string_table)
        }
        _ => format!("TypeId({})", type_id.0),
    }
}

pub(crate) fn type_mismatch_context_name(
    context: crate::compiler_frontend::compiler_messages::TypeMismatchContext,
) -> &'static str {
    match context {
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::Assignment => {
            "assignment"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::Declaration => {
            "declaration"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::ReturnValue => {
            "return value"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::FunctionArgument => {
            "function argument"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::AssertionArgument => {
            "assertion argument"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::ConstructorArgument => {
            "constructor argument"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::ReceiverArgument => {
            "receiver argument"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::Operator => "operator",
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::Condition => "condition",
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::CollectionElement => {
            "collection element"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::StructFieldDefault => {
            "struct field default"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::TemplateInterpolation => {
            "template interpolation"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::MatchScrutinee => {
            "match scrutinee"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::MatchPattern => {
            "match pattern"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::ErrorReturn => {
            "error return"
        }
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::Pattern => "pattern",
        crate::compiler_frontend::compiler_messages::TypeMismatchContext::General => "general",
    }
}

/// Render a token as source-facing syntax rather than Rust enum debug output.
///
/// WHAT: gives syntax diagnostics one spelling source across terminal, terse, dev-server, and
/// contextless compiler-error fallback while the last bridge call sites are retired.
/// WHY: parser diagnostics carry `TokenKind` facts, but user output should show Moth syntax
/// such as `(` or `name`, not implementation names such as `OpenParenthesis`.
pub(crate) fn token_kind_name(token_kind: &TokenKind, string_table: &StringTable) -> String {
    match token_kind {
        TokenKind::ModuleStart => "module start".to_owned(),
        TokenKind::Eof => "end of file".to_owned(),
        TokenKind::Export => "`export`".to_owned(),
        TokenKind::Hash => "`#`".to_owned(),
        TokenKind::Reactive => "`$`".to_owned(),
        TokenKind::Arrow => "`->`".to_owned(),
        TokenKind::Symbol(name) => format!("name `{}`", string_table.resolve(*name)),
        TokenKind::StyleDirective(name) => {
            format!("style directive `${}`", string_table.resolve(*name))
        }
        TokenKind::StringSliceLiteral(value) => {
            format!("string literal \"{}\"", string_table.resolve(*value))
        }
        // `TokenKind::Path` now carries only a dense handle into a file-owned
        // `PathSyntaxTable`; that table is not part of stable diagnostic facts, so
        // the renderer names the token class without a file-local spelling.
        TokenKind::Path(_) => "path".to_owned(),
        TokenKind::NumericLiteral(token) => {
            let text = string_table.resolve(token.normalized_text);
            match token.kind {
                crate::compiler_frontend::numeric_text::token::NumericLiteralKind::WholeNumber => {
                    format!("integer literal `{text}`")
                }
                _ => format!("float literal `{text}`"),
            }
        }
        TokenKind::CharLiteral(value) => format!("character literal `{value}`"),
        TokenKind::RawStringLiteral(value) => {
            format!("raw string literal `{}`", string_table.resolve(*value))
        }
        TokenKind::BoolLiteral(value) => format!("boolean literal `{value}`"),
        TokenKind::OpenCurly => "`{`".to_owned(),
        TokenKind::CloseCurly => "`}`".to_owned(),
        TokenKind::TypeParameterBracket => "`|`".to_owned(),
        TokenKind::Newline => "newline".to_owned(),
        TokenKind::End => "`;`".to_owned(),
        TokenKind::StartTemplateBody => "`:`".to_owned(),
        TokenKind::Comma => "`,`".to_owned(),
        TokenKind::Dot => "`.`".to_owned(),
        TokenKind::Colon => "`:`".to_owned(),
        TokenKind::DoubleColon => "`::`".to_owned(),
        TokenKind::Assign => "`=`".to_owned(),
        TokenKind::This => "`this`".to_owned(),
        TokenKind::Must => "`must`".to_owned(),
        TokenKind::TraitThis => "`This`".to_owned(),
        TokenKind::OpenParenthesis => "`(`".to_owned(),
        TokenKind::CloseParenthesis => "`)`".to_owned(),
        TokenKind::As => "`as`".to_owned(),
        TokenKind::Type => "`type`".to_owned(),
        TokenKind::Of => "`of`".to_owned(),
        TokenKind::Variadic => "`..`".to_owned(),
        TokenKind::Mutable => "`~`".to_owned(),
        TokenKind::DatatypeNone => "`None` type".to_owned(),
        TokenKind::NoneLiteral => "`none`".to_owned(),
        TokenKind::DatatypeInt => "`Int`".to_owned(),
        TokenKind::DatatypeFloat => "`Float`".to_owned(),
        TokenKind::DatatypeBool => "`Bool`".to_owned(),
        TokenKind::DatatypeTrue => "`True`".to_owned(),
        TokenKind::DatatypeFalse => "`False`".to_owned(),
        TokenKind::DatatypeString => "`String`".to_owned(),
        TokenKind::DatatypeChar => "`Char`".to_owned(),
        TokenKind::Bang => "`!`".to_owned(),
        TokenKind::QuestionMark => "`?`".to_owned(),
        TokenKind::Negative => "unary `-`".to_owned(),
        TokenKind::Exponent => "`^`".to_owned(),
        TokenKind::Multiply => "`*`".to_owned(),
        TokenKind::Divide => "`/`".to_owned(),
        TokenKind::Modulus => "`%`".to_owned(),
        TokenKind::IntDivide => "`//`".to_owned(),
        TokenKind::ExponentAssign => "`^=`".to_owned(),
        TokenKind::MultiplyAssign => "`*=`".to_owned(),
        TokenKind::DivideAssign => "`/=`".to_owned(),
        TokenKind::ModulusAssign => "`%=`".to_owned(),
        TokenKind::IntDivideAssign => "`//=`".to_owned(),
        TokenKind::Add => "`+`".to_owned(),
        TokenKind::Subtract => "`-`".to_owned(),
        TokenKind::AddAssign => "`+=`".to_owned(),
        TokenKind::SubtractAssign => "`-=`".to_owned(),
        TokenKind::Not => "`not`".to_owned(),
        TokenKind::Is => "`is`".to_owned(),
        TokenKind::LessThan => "`<`".to_owned(),
        TokenKind::LessThanOrEqual => "`<=`".to_owned(),
        TokenKind::GreaterThan => "`>`".to_owned(),
        TokenKind::GreaterThanOrEqual => "`>=`".to_owned(),
        TokenKind::And => "`and`".to_owned(),
        TokenKind::Or => "`or`".to_owned(),
        TokenKind::If => "`if`".to_owned(),
        TokenKind::Else => "`else`".to_owned(),
        TokenKind::Return => "`return`".to_owned(),
        TokenKind::ReturnBang => "`return!`".to_owned(),
        TokenKind::Catch => "`catch`".to_owned(),
        TokenKind::Then => "`then`".to_owned(),
        TokenKind::Checked => "`checked`".to_owned(),
        TokenKind::Async => "`async`".to_owned(),
        TokenKind::Loop => "`loop`".to_owned(),
        TokenKind::By => "`by`".to_owned(),
        TokenKind::Break => "`break`".to_owned(),
        TokenKind::Continue => "`continue`".to_owned(),
        TokenKind::ExclusiveRange => "`to`".to_owned(),
        TokenKind::Ampersand => "`&`".to_owned(),
        TokenKind::FatArrow => "`=>`".to_owned(),
        TokenKind::Wildcard => "`_`".to_owned(),
        TokenKind::Copy => "`copy`".to_owned(),
        TokenKind::TemplateClose => "`]`".to_owned(),
        TokenKind::TemplateHead => "`[`".to_owned(),
        TokenKind::ChannelSend => "`>>`".to_owned(),
        TokenKind::ChannelReceive => "`<<`".to_owned(),
        TokenKind::Yield => "`yield`".to_owned(),
        TokenKind::Cast => "`cast`".to_owned(),
        TokenKind::CastBang => "`cast!`".to_owned(),
        TokenKind::Assert => "`assert`".to_owned(),
    }
}

pub(crate) fn expected_token_message(
    expected: &TokenKind,
    found: Option<&TokenKind>,
    string_table: &StringTable,
) -> String {
    let expected = token_kind_name(expected, string_table);

    if let Some(found) = found {
        let found = token_kind_name(found, string_table);
        format!("Expected {expected}, but found {found}.")
    } else {
        format!("Expected {expected}.")
    }
}

pub(crate) fn unexpected_token_message(found: &TokenKind, string_table: &StringTable) -> String {
    let found = token_kind_name(found, string_table);
    format!("Unexpected token {found}.")
}

pub(crate) fn unknown_name_message(
    name: StringId,
    namespace: NameNamespace,
    string_table: &StringTable,
) -> String {
    let name = string_table.resolve(name);
    let namespace = namespace_name(namespace);

    format!("Unknown {namespace} name '{name}'.")
}

pub(crate) fn duplicate_declaration_message(name: StringId, string_table: &StringTable) -> String {
    let name_str = string_table.resolve(name);

    format!(
        "Cannot declare '{name_str}' because that name is already visible in this scope. Moth does not allow duplicate names or shadowing."
    )
}
