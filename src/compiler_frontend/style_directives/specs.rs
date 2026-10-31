//! Data contracts for frontend and project-provided style directives.
//!
//! These types describe directive syntax, parser effects, formatter factories, and core
//! compiler-owned behavior. Registry merge policy lives in `registry`; frontend-owned directive
//! values live in `builtins`.

use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{BodyWhitespacePolicy, Formatter};
use crate::compiler_frontend::style_directives::compatibility::TemplateHeadCompatibility;
use crate::compiler_frontend::tokenizer::tokens::TemplateBodyMode;

/// The core `$doc` directive name.
///
/// A `$doc` body suppresses child templates, so header-stage token scans must recognise this
/// name to treat its brackets as literal text. Both the directive registry and those scans read
/// the name from here so they cannot drift apart.
pub(crate) const DOC_DIRECTIVE_NAME: &str = "doc";

/// Core language directive behavior handled directly by compiler-owned template parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreStyleDirectiveKind {
    Children,
    Fresh,
    Slot,
    Insert,
    Note,
    Todo,
    Doc,
    Raw,
}

/// Supported optional single-argument types for handler-based directives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleDirectiveArgumentType {
    String,
    Template,
    Number,
    Bool,
}

/// Parsed value for an optional handler-based style argument.
#[derive(Clone, Debug)]
pub enum StyleDirectiveArgumentValue {
    String(String),
    Template(Box<Template>),
    Number(f64),
    Bool(bool),
}

/// Template-style toggles that a handler-based directive can apply when used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleDirectiveEffects {
    /// Optional semantic style label applied to the effective `TemplateIr.style.id`.
    ///
    /// This is intentionally distinct from `Formatter.id`:
    /// - `style_id` tags parsed template semantics.
    /// - `Formatter.id` identifies the concrete formatter implementation.
    pub style_id: Option<&'static str>,
    /// Optional body-whitespace policy override.
    pub body_whitespace_policy: Option<BodyWhitespacePolicy>,
    /// Optional toggle for parsing `[...]` as literal body text.
    pub suppress_child_templates: Option<bool>,
    /// Optional toggle that opts this template out of parent `$children(..)` wrappers.
    pub skip_parent_child_wrappers: Option<bool>,
}

/// Formatter factory used by handler-based directives.
///
/// Returns:
/// - `Ok(formatter)` to set/replace the active formatter.
/// - `Err(message)` for user-facing directive argument/configuration errors.
pub type FormatterFactory = fn(Option<&StyleDirectiveArgumentValue>) -> Result<Formatter, String>;

/// Full behavior contract for one handler-based style directive.
///
/// WHAT:
/// - Carries optional argument typing, style-state effects, and formatter factory behavior.
///
/// WHY:
/// - The same contract is used for frontend-owned formatter built-ins and project-owned
///   directives, so file location determines ownership while parser dispatch stays generic.
#[derive(Clone, Debug)]
pub struct StyleDirectiveHandlerSpec {
    /// Optional single argument contract for `$name(...)`.
    pub argument_type: Option<StyleDirectiveArgumentType>,
    /// Template style-state toggles applied when this directive is parsed.
    pub effects: StyleDirectiveEffects,
    /// Optional formatter factory invoked with the parsed optional argument.
    pub formatter_factory: Option<FormatterFactory>,
}

impl StyleDirectiveHandlerSpec {
    pub fn new(
        argument_type: Option<StyleDirectiveArgumentType>,
        effects: StyleDirectiveEffects,
        formatter_factory: Option<FormatterFactory>,
    ) -> Self {
        Self {
            argument_type,
            effects,
            formatter_factory,
        }
    }

    pub fn no_op() -> Self {
        Self {
            argument_type: None,
            effects: StyleDirectiveEffects::default(),
            formatter_factory: None,
        }
    }
}

/// Directive class contract.
#[derive(Clone, Debug)]
pub enum StyleDirectiveKind {
    Core(CoreStyleDirectiveKind),
    Handler(StyleDirectiveHandlerSpec),
}

/// Frontend registration contract for one style directive.
///
/// `body_mode` controls template-body tokenization after the directive appears in the template
/// head. `head_compatibility` controls parse-time compatibility against other meaningful head
/// items. `kind` controls AST-level directive handling.
#[derive(Clone, Debug)]
pub struct StyleDirectiveSpec {
    pub name: String,
    pub body_mode: TemplateBodyMode,
    pub head_compatibility: TemplateHeadCompatibility,
    pub kind: StyleDirectiveKind,
}

impl StyleDirectiveSpec {
    /// Register a handler-based directive.
    pub fn handler(
        name: impl Into<String>,
        body_mode: TemplateBodyMode,
        head_compatibility: TemplateHeadCompatibility,
        handler: StyleDirectiveHandlerSpec,
    ) -> Self {
        Self {
            name: name.into(),
            body_mode,
            head_compatibility,
            kind: StyleDirectiveKind::Handler(handler),
        }
    }

    /// Register an explicit no-op handler-based directive.
    pub fn handler_no_op(name: impl Into<String>, body_mode: TemplateBodyMode) -> Self {
        Self::handler(
            name,
            body_mode,
            TemplateHeadCompatibility::fully_compatible_meaningful(),
            StyleDirectiveHandlerSpec::no_op(),
        )
    }

    /// Internal helper for compiler-owned core directives.
    pub(crate) fn core(
        name: &str,
        body_mode: TemplateBodyMode,
        head_compatibility: TemplateHeadCompatibility,
        kind: CoreStyleDirectiveKind,
    ) -> Self {
        Self {
            name: name.to_string(),
            body_mode,
            head_compatibility,
            kind: StyleDirectiveKind::Core(kind),
        }
    }

    pub(crate) fn is_core(&self) -> bool {
        matches!(self.kind, StyleDirectiveKind::Core(_))
    }
}
