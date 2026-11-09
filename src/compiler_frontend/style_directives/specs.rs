//! Data contracts for frontend-owned style directives.
//!
//! These types describe directive syntax, parser effects, formatter factories, and core
//! compiler-owned behavior. Frontend-owned directive values live in `builtins`.

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

/// The core `$slot` directive name.
///
/// The symbol immediately after it names a slot in the receiving template's schema, not a
/// lexical value. Header-stage initializer scans read the name from here so they do not
/// record slot targets as constant references.
pub(crate) const SLOT_DIRECTIVE_NAME: &str = "slot";

/// The core `$in` directive name.
///
/// Its required symbol target is a slot identity, not a lexical value. Header-stage
/// initializer scans read the name from here so they do not record insert targets as
/// constant references.
pub(crate) const INSERT_DIRECTIVE_NAME: &str = "in";

/// The core `$before` directive name.
///
/// Its assigned value wraps every direct child template on the prefix edge.
pub(crate) const BEFORE_DIRECTIVE_NAME: &str = "before";

/// The core `$after` directive name.
///
/// Its assigned value wraps every direct child template on the suffix edge.
pub(crate) const AFTER_DIRECTIVE_NAME: &str = "after";

/// The `$config` directive name.
///
/// It is a source modifier, so the template-head parser and the header walk both read the name
/// from here to keep their context checks aligned.
pub(crate) const CONFIG_DIRECTIVE_NAME: &str = "config";

/// Attachment contexts a registered directive may appear in.
///
/// WHAT:
/// - `TEMPLATE_HEAD`: the directive prefixes a template and modifies that template.
/// - `SOURCE_MODIFIER`: the directive occupies its own source line and modifies the next eligible
///   top-level declaration.
///
/// WHY:
/// - A known name in the wrong context is a typed contextual error, never an unknown-name or
///   invalid-character failure. One context fact on the spec keeps recognition and context checks
///   data-driven instead of adding name branches at each parse site.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleDirectiveContexts(u8);

impl StyleDirectiveContexts {
    pub const TEMPLATE_HEAD: Self = Self(1 << 0);
    pub const SOURCE_MODIFIER: Self = Self(1 << 1);

    pub const fn allows_template_head(self) -> bool {
        self.0 & Self::TEMPLATE_HEAD.0 != 0
    }

    pub const fn allows_source_modifier(self) -> bool {
        self.0 & Self::SOURCE_MODIFIER.0 != 0
    }
}

/// Eligible target of one source-modifier directive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceModifierTarget {
    /// Exactly one explicitly typed top-level compile-time `#` binding.
    ExplicitTypedCompileTimeBinding,
}

/// Core language directive behavior handled directly by compiler-owned template parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreStyleDirectiveKind {
    Before,
    After,
    Fresh,
    Slot,
    In,
    Note,
    Todo,
    Doc,
    Raw,
}

/// Supported optional single-operand types for handler-based directives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleDirectiveArgumentType {
    String,
    // Parser normalization still understands these operand kinds, but no frontend directive
    // currently declares them.
    #[allow(dead_code)]
    Template,
    #[allow(dead_code)]
    Number,
    #[allow(dead_code)]
    Bool,
}

/// Parsed value for an optional handler-based style operand.
#[derive(Clone, Debug)]
pub enum StyleDirectiveArgumentValue {
    String(String),
    // Produced by parser normalization for the dormant argument types above. Formatter
    // factories only consume `String` today, so these payloads stay unread.
    #[allow(dead_code)]
    Template(Box<Template>),
    #[allow(dead_code)]
    Number(f64),
    #[allow(dead_code)]
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
    /// Optional toggle that opts this template out of parent `$before`/`$after` wrappers.
    pub skip_parent_child_wrappers: Option<bool>,
}

/// Formatter factory used by handler-based directives.
///
/// Returns:
/// - `Ok(formatter)` to set/replace the active formatter.
/// - `Err(message)` for user-facing directive operand/configuration errors.
pub type FormatterFactory = fn(Option<&StyleDirectiveArgumentValue>) -> Result<Formatter, String>;

/// Full behavior contract for one handler-based style directive.
///
/// WHAT:
/// - Carries optional operand typing, style-state effects, and formatter factory behavior.
///
/// WHY:
/// - Frontend-owned formatter built-ins share one contract so parser dispatch stays generic.
#[derive(Clone, Debug)]
pub struct StyleDirectiveHandlerSpec {
    /// Optional single operand contract for `$name value`.
    pub argument_type: Option<StyleDirectiveArgumentType>,
    /// Template style-state toggles applied when this directive is parsed.
    pub effects: StyleDirectiveEffects,
    /// Optional formatter factory invoked with the parsed optional operand.
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
}

/// Directive class contract.
#[derive(Clone, Debug)]
pub enum StyleDirectiveKind {
    Core(CoreStyleDirectiveKind),
    Handler(StyleDirectiveHandlerSpec),
    /// A source-only modifier whose eligible target the spec states.
    SourceModifier(SourceModifierTarget),
}

/// Frontend registration contract for one style directive.
///
/// `body_mode` controls template-body tokenization after the directive appears in the template
/// head. `head_compatibility` controls parse-time compatibility against other meaningful head
/// items. `contexts` states which attachment location recognises the name, and `kind` controls
/// AST-level directive handling.
#[derive(Clone, Debug)]
pub struct StyleDirectiveSpec {
    pub name: String,
    pub body_mode: TemplateBodyMode,
    pub head_compatibility: TemplateHeadCompatibility,
    pub contexts: StyleDirectiveContexts,
    pub kind: StyleDirectiveKind,
}

impl StyleDirectiveSpec {
    /// Register a handler-based template directive.
    pub fn handler(
        name: impl Into<String>,
        body_mode: TemplateBodyMode,
        contexts: StyleDirectiveContexts,
        head_compatibility: TemplateHeadCompatibility,
        handler: StyleDirectiveHandlerSpec,
    ) -> Self {
        Self {
            name: name.into(),
            body_mode,
            head_compatibility,
            contexts,
            kind: StyleDirectiveKind::Handler(handler),
        }
    }

    /// Internal helper for compiler-owned core template directives.
    pub(crate) fn core(
        name: &str,
        body_mode: TemplateBodyMode,
        contexts: StyleDirectiveContexts,
        head_compatibility: TemplateHeadCompatibility,
        kind: CoreStyleDirectiveKind,
    ) -> Self {
        Self {
            name: name.to_string(),
            body_mode,
            head_compatibility,
            contexts,
            kind: StyleDirectiveKind::Core(kind),
        }
    }

    /// Internal helper for source-modifier directives.
    pub(crate) fn source_modifier(name: &str, target: SourceModifierTarget) -> Self {
        Self {
            name: name.to_string(),
            body_mode: TemplateBodyMode::Normal,
            head_compatibility: TemplateHeadCompatibility::default(),
            contexts: StyleDirectiveContexts::SOURCE_MODIFIER,
            kind: StyleDirectiveKind::SourceModifier(target),
        }
    }
}
