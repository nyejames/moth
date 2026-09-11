//! Alias resolution and lexical vocabulary for the `$code` language profiles.
//!
//! The scanner shell owns cursor movement and shared role emission. This module
//! owns the profile registry and the non-Moth word tables so adding a language
//! does not require opening the Moth state machine.

use super::CodeHighlightRole;

/// Single-character operators kept for the non-Moth profiles.
///
/// WHAT: preserves the pre-scanner operator surface for languages whose profiles
///       do not define compound forms yet.
/// WHY: Moth owns the maximal-munch table; other profiles keep their current
///      lexical behaviour until they adopt the shared palette in the same pass.
pub(super) const NON_MOTH_OPERATOR_BYTES: &[u8] = b"=:-+*/%^!?|&<>~@#$`";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodeLanguage {
    Generic,
    Text,
    Moth,
    JavaScript,
    TypeScript,
    Python,
    Rust,
    Shell,
    Html,
    Markdown,
    Toml,
    Json,
    Yaml,
    Css,
    C,
    Sql,
}

/// Canonical short and long aliases for every supported `$code` language.
///
/// WHAT: one table owns alias resolution and the supported-values diagnostic,
///       so the two can never drift apart.
/// WHY: adding a language means extending this table, its formatter rules and
///      the documentation lists, not a second alias match somewhere else.
pub(crate) const LANGUAGE_ALIASES: &[(&str, CodeLanguage)] = &[
    ("txt", CodeLanguage::Text),
    ("text", CodeLanguage::Text),
    ("html", CodeLanguage::Html),
    ("md", CodeLanguage::Markdown),
    ("markdown", CodeLanguage::Markdown),
    ("toml", CodeLanguage::Toml),
    ("json", CodeLanguage::Json),
    ("yaml", CodeLanguage::Yaml),
    ("yml", CodeLanguage::Yaml),
    ("css", CodeLanguage::Css),
    ("c", CodeLanguage::C),
    ("sql", CodeLanguage::Sql),
    ("moth", CodeLanguage::Moth),
    ("js", CodeLanguage::JavaScript),
    ("javascript", CodeLanguage::JavaScript),
    ("ts", CodeLanguage::TypeScript),
    ("typescript", CodeLanguage::TypeScript),
    ("py", CodeLanguage::Python),
    ("python", CodeLanguage::Python),
    ("rs", CodeLanguage::Rust),
    ("rust", CodeLanguage::Rust),
    ("bash", CodeLanguage::Shell),
    ("sh", CodeLanguage::Shell),
    ("shell", CodeLanguage::Shell),
];

impl CodeLanguage {
    pub(crate) fn from_alias(alias: &str) -> Option<Self> {
        LANGUAGE_ALIASES
            .iter()
            .find(|(candidate, _)| *candidate == alias)
            .map(|(_, language)| *language)
    }

    /// Renders the supported alias groups in table order, for example
    /// `"txt"/"text", "html", ...`.
    ///
    /// WHY: the unsupported-language diagnostic should show the exact aliases
    ///      `from_alias` accepts without keeping a second hand-written list.
    pub(crate) fn supported_aliases() -> String {
        let mut groups: Vec<(CodeLanguage, Vec<&str>)> = Vec::new();

        for (alias, language) in LANGUAGE_ALIASES {
            match groups.last_mut() {
                Some((group_language, aliases)) if *group_language == *language => {
                    aliases.push(alias);
                }
                _ => groups.push((*language, vec![alias])),
            }
        }

        groups
            .into_iter()
            .map(|(_, aliases)| {
                aliases
                    .iter()
                    .map(|alias| format!("\"{alias}\""))
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub(super) fn comment_prefix(self) -> Option<&'static str> {
        match self {
            Self::Text | Self::Html | Self::Markdown | Self::Css => None,
            Self::Generic | Self::Json | Self::C => Some("//"),
            Self::Moth => Some("--"),
            Self::JavaScript | Self::TypeScript | Self::Rust => Some("//"),
            Self::Python | Self::Shell | Self::Toml | Self::Yaml => Some("#"),
            Self::Sql => Some("--"),
        }
    }

    /// True when capitalized words receive the nominal fallback role.
    ///
    /// WHY: code languages use the fallback to surface type-like names, while
    ///      prose-bearing profiles (HTML, Markdown, TOML) keep ordinary words
    ///      plain so content stays readable.
    pub(super) fn has_nominal_fallback(self) -> bool {
        matches!(
            self,
            Self::JavaScript | Self::TypeScript | Self::Python | Self::Rust | Self::Shell | Self::C
        )
    }
}

/// Direct non-Moth word classification result.
///
/// WHAT: carries the current word role and the optional role for the exact next
///       identifier, so one classifier owns all per-language vocabulary.
pub(super) struct NonMothWordClass {
    pub(super) role: Option<CodeHighlightRole>,
    pub(super) next_identifier_role: Option<CodeHighlightRole>,
}

/// Classifies one non-Moth word from direct per-language matches.
///
/// WHAT: returns the word role and optionally arms a declaration-name role for
///       the exact next identifier. `Generic` has no language vocabulary.
/// WHY: one local classifier replaces the previous keyword/type/literal helper
///      trio plus the loose pending-role state.
pub(super) fn classify_non_moth_word(language: CodeLanguage, word: &str) -> NonMothWordClass {
    let mut role = None;
    let mut next_identifier_role = None;

    match language {
        CodeLanguage::JavaScript => match word {
            "if" | "else" | "return" | "break" | "continue" | "for" | "while" | "in" | "const"
            | "let" | "var" => role = Some(CodeHighlightRole::Keyword),
            "true" | "false" | "null" | "undefined" => {
                role = Some(CodeHighlightRole::Literal);
            }
            "function" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Function);
            }
            _ => {}
        },
        CodeLanguage::TypeScript => match word {
            "if" | "else" | "return" | "break" | "continue" | "for" | "while" | "in" | "const"
            | "let" | "var" | "type" | "enum" => {
                role = Some(CodeHighlightRole::Keyword);
            }
            "number" | "string" | "boolean" | "unknown" | "never" | "void" | "any" => {
                role = Some(CodeHighlightRole::Type);
            }
            "true" | "false" | "null" | "undefined" => {
                role = Some(CodeHighlightRole::Literal);
            }
            "function" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Function);
            }
            "interface" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Contract);
            }
            _ => {}
        },
        CodeLanguage::Python => match word {
            "if" | "elif" | "else" | "return" | "break" | "continue" | "for" | "while" | "in"
            | "class" | "import" | "from" | "as" => {
                role = Some(CodeHighlightRole::Keyword);
            }
            "True" | "False" | "None" => role = Some(CodeHighlightRole::Literal),
            "def" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Function);
            }
            _ => {}
        },
        CodeLanguage::Rust => match word {
            "if" | "else" | "return" | "break" | "continue" | "for" | "while" | "in" | "let"
            | "mut" | "const" | "static" | "struct" | "enum" | "impl" | "mod" | "use" | "pub"
            | "crate" | "super" | "self" | "match" | "async" | "await" | "move" | "ref"
            | "type" | "where" | "unsafe" | "extern" | "dyn" => {
                role = Some(CodeHighlightRole::Keyword);
            }
            "i8" | "i16" | "i32" | "i64" | "i128" | "u8" | "u16" | "u32" | "u64" | "u128"
            | "isize" | "usize" | "f32" | "f64" | "bool" | "char" | "str" => {
                role = Some(CodeHighlightRole::Type);
            }
            "true" | "false" => role = Some(CodeHighlightRole::Literal),
            "fn" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Function);
            }
            "trait" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Contract);
            }
            _ => {}
        },
        CodeLanguage::Shell => match word {
            "if" | "then" | "else" | "elif" | "fi" | "for" | "while" | "do" | "done" | "in" => {
                role = Some(CodeHighlightRole::Keyword)
            }
            "true" | "false" => role = Some(CodeHighlightRole::Literal),
            "function" => {
                role = Some(CodeHighlightRole::Keyword);
                next_identifier_role = Some(CodeHighlightRole::Function);
            }
            _ => {}
        },
        CodeLanguage::Toml => match word {
            "true" | "false" => role = Some(CodeHighlightRole::Literal),
            _ => {}
        },
        CodeLanguage::Json => match word {
            "true" | "false" | "null" => role = Some(CodeHighlightRole::Literal),
            _ => {}
        },
        CodeLanguage::Yaml => {
            if is_yaml_literal(word) {
                role = Some(CodeHighlightRole::Literal);
            }
        }
        CodeLanguage::Css => {}
        CodeLanguage::C => match word {
            "if" | "else" | "for" | "while" | "do" | "switch" | "case" | "default" | "break"
            | "continue" | "return" | "goto" | "sizeof" | "struct" | "union" | "enum"
            | "typedef" | "static" | "const" | "extern" | "volatile" | "register" | "signed"
            | "unsigned" | "long" | "short" | "inline" => {
                role = Some(CodeHighlightRole::Keyword);
            }
            "int" | "char" | "float" | "double" | "void" | "bool" | "size_t" | "ssize_t"
            | "int8_t" | "int16_t" | "int32_t" | "int64_t" | "uint8_t" | "uint16_t"
            | "uint32_t" | "uint64_t" => {
                role = Some(CodeHighlightRole::Type);
            }
            "true" | "false" | "NULL" => role = Some(CodeHighlightRole::Literal),
            _ => {}
        },
        CodeLanguage::Sql => role = sql_word_role(word),
        CodeLanguage::Generic
        | CodeLanguage::Text
        | CodeLanguage::Moth
        | CodeLanguage::Html
        | CodeLanguage::Markdown => {}
    }

    NonMothWordClass {
        role,
        next_identifier_role,
    }
}

/// True for YAML boolean and null scalar spellings, case-insensitively.
fn is_yaml_literal(word: &str) -> bool {
    ["true", "false", "yes", "no", "on", "off", "null"]
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate))
}

/// Classifies one SQL word case-insensitively into the shared roles.
fn sql_word_role(word: &str) -> Option<CodeHighlightRole> {
    const KEYWORDS: &[&str] = &[
        "select",
        "from",
        "where",
        "insert",
        "into",
        "values",
        "update",
        "set",
        "delete",
        "create",
        "table",
        "database",
        "index",
        "drop",
        "alter",
        "add",
        "column",
        "join",
        "inner",
        "left",
        "right",
        "full",
        "outer",
        "on",
        "group",
        "by",
        "order",
        "having",
        "limit",
        "offset",
        "and",
        "or",
        "not",
        "primary",
        "key",
        "foreign",
        "references",
        "unique",
        "default",
        "check",
        "constraint",
        "as",
        "distinct",
        "union",
        "all",
        "exists",
        "between",
        "like",
        "in",
        "is",
        "case",
        "when",
        "then",
        "else",
        "end",
        "begin",
        "commit",
        "rollback",
        "transaction",
    ];
    const TYPES: &[&str] = &[
        "int",
        "integer",
        "bigint",
        "smallint",
        "tinyint",
        "real",
        "float",
        "double",
        "numeric",
        "decimal",
        "text",
        "varchar",
        "char",
        "boolean",
        "date",
        "time",
        "timestamp",
        "blob",
    ];

    if ["true", "false", "null"]
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate))
    {
        return Some(CodeHighlightRole::Literal);
    }

    if TYPES
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate))
    {
        return Some(CodeHighlightRole::Type);
    }

    KEYWORDS
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate))
        .then_some(CodeHighlightRole::Keyword)
}
