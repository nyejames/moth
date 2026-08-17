//! Core template data types and the durable `Template` handle.
//!
//! WHAT: defines slot keys, directive kinds, template node classifications, the
//!       durable `Template` handle, and shared formatter/style vocabulary consumed
//!       throughout AST template processing.
//! WHY: templates are a first-class Moth construct; this module owns the
//!      durable `Template` value and the shared vocabulary used by parsing,
//!      folding, slot routing, and runtime-handoff preparation.

use crate::compiler_frontend::ast::expressions::expression::ReactiveSource;
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterInput, FormatterOutput,
};
use crate::compiler_frontend::ast::templates::styles::whitespace::TemplateWhitespacePassProfile;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateTirReference, TemplateWrapperReference,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use std::sync::Arc;

// -------------------------
//  Slot Keys
// -------------------------

/// Unique identifier for a template slot.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SlotKey {
    /// The default unnamed slot (`$slot`).
    Default,
    /// A named slot (`$slot("name")`).
    Named(StringId),
    /// A positional slot used in composition.
    Positional(usize),
}

impl SlotKey {
    pub fn named(name: StringId) -> Self {
        Self::Named(name)
    }
}

// -------------------------
//  Directive Kinds
// -------------------------

/// Category of comment directive within a template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentDirectiveKind {
    Note,
    Todo,
    Doc,
}

/// High-level classification of a template node.
#[derive(Clone, Debug, PartialEq)]
pub enum TemplateType {
    /// A template that produces a string at runtime.
    StringFunction,

    /// Fully compile-time-resolved template content. This can still contain unresolved
    /// slots, which makes it a compile-time wrapper rather than a direct string value.
    String,

    /// `[$slot]` and `[$slot("name")]` parse as dedicated template nodes, then
    /// become structural slot placeholders in the parent parser TIR.
    SlotDefinition(SlotKey),

    /// `[$insert("name"): ...]` helpers carry contribution content that only an
    /// immediate parent template can consume during slot composition.
    SlotInsert(SlotKey),

    /// A comment or documentation directive.
    Comment(CommentDirectiveKind),
}

/// Classifies the context in which a template is being parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateParsingMode {
    /// Standard template parsing.
    Standard,
    /// Parsing inside a documentation comment (`$doc`), which has stricter constant requirements.
    DocComment,
}

/// Classifies how "constant" a template value is during AST evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateConstValueKind {
    /// Fully resolved final string value. Safe to materialize as a string slice before HIR.
    RenderableString,

    /// Structural `break` / `continue` signal inside a template loop body.
    ///
    /// It is compile-time foldable only when the enclosing loop consumes it; it
    /// must never be treated as a standalone renderable string.
    LoopControlSignal,

    /// A template that wraps other content, such as unresolved slot placeholders.
    /// This is not automatically a backend-facing constant string in runtime paths.
    WrapperTemplate,

    /// AST composition helper (e.g., `$insert(...)`) that must not escape as a
    /// backend-facing runtime value. Helper identity alone is not sufficient to
    /// prove validity when nested under a wrapper-owned final template value.
    SlotInsertHelper,

    /// Final template value still depends on runtime expressions.
    NonConst,
}

// -------------------------
//  Slot Placeholder
// -------------------------

/// Parser-side slot metadata converted immediately into a TIR placeholder.
#[derive(Clone, Debug)]
pub struct SlotPlaceholder {
    pub key: SlotKey,
    pub applied_child_wrappers: Vec<TemplateWrapperReference>,
    pub child_wrappers: Vec<TemplateWrapperReference>,
    pub skip_parent_child_wrappers: bool,
}

impl SlotPlaceholder {
    pub fn with_wrappers(
        key: SlotKey,
        applied_child_wrappers: Vec<TemplateWrapperReference>,
        child_wrappers: Vec<TemplateWrapperReference>,
        skip_parent_child_wrappers: bool,
    ) -> Self {
        Self {
            key,
            applied_child_wrappers,
            child_wrappers,
            skip_parent_child_wrappers,
        }
    }
}

// -------------------------
//  Template Segment Origin
// -------------------------

/// Identifies where a template segment originated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateSegmentOrigin {
    /// Head segments are values/configuration injected before the body starts.
    /// They must never be reformatted by the current template style.
    Head,
    /// Body segments are literal body content, so they are eligible for style
    /// formatters such as markdown when they are compile-time-known strings.
    Body,
}

/// Metadata for a V1 `$(source)` template subscription.
///
/// WHAT: records the resolved reactive source identity, ordinary underlying value type, and
/// authored source location without changing the segment expression's semantic `TypeId`.
/// WHY: subscriptions are template metadata, not a wrapper type or borrow. Later HIR/backend
/// stages can preserve this dependency while ordinary `[source]` head captures remain snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveSubscription {
    pub source: ReactiveSource,
    pub type_id: TypeId,
    pub location: SourceLocation,
}

// -------------------------
//  Formatting Traits & Types
// -------------------------

/// Trait for directive-owned output formatters (e.g. `$md`).
///
/// Formatters are stored in style directive registries, which are shared read-only across parallel
/// tokenization/header parsing workers.
pub trait TemplateFormatter: Send + Sync {
    fn format(
        &self,
        input: FormatterInput,
        string_table: &mut StringTable,
    ) -> Result<FormatterResult, CompilerMessages>;
}

impl std::fmt::Debug for dyn TemplateFormatter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TemplateFormatter")
    }
}

/// Bundles a core formatter with pre- and post-format whitespace passes.
#[derive(Clone, Debug)]
pub struct Formatter {
    /// Pre-format whitespace passes are run before parser-specific formatting.
    /// This allows directive-owned formatters (for example, `$md`) to opt into
    /// shared dedent/trim behavior while still operating over raw template body text.
    pub(crate) pre_format_whitespace_passes: Vec<TemplateWhitespacePassProfile>,

    /// Shared ownership keeps formatters cheap to clone when template styles are
    /// copied or explicitly inherited during AST construction.
    pub formatter: Arc<dyn TemplateFormatter>,

    /// Post-format passes run after formatter output is generated.
    pub(crate) post_format_whitespace_passes: Vec<TemplateWhitespacePassProfile>,
}

/// Controls how whitespace in the template body is handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyWhitespacePolicy {
    /// Plain templates (no style directive) keep the historical default dedent/trim flow.
    DefaultTemplateBehavior,
    /// Style directives own body whitespace behavior and receive raw body text unless
    /// their formatter explicitly opts into shared whitespace passes.
    StyleDirectiveControlled,
}

/// Result of a successful formatting pass.
#[derive(Clone, Debug)]
pub struct FormatterResult {
    pub output: FormatterOutput,
    pub warnings: Vec<CompilerDiagnostic>,
}

// -------------------------
//  Template Style Configuration
// -------------------------

/// Configuration passed into a template head to define how it should be parsed and rendered.
///
/// WHAT: non-recursive style metadata for a template: formatter, style ID,
///       whitespace policy, and suppress-child-template behavior.
/// WHY: parser-local wrapper references and TIR wrapper-context overlays own
///      `$children(..)` application, so `Style` remains a non-recursive config
///      shape that can be stored on TIR entries without recursive wrapper
///      payloads.
#[derive(Clone, Debug)]
pub struct Style {
    /// Semantic style label for this parsed template. Set by directive effects
    /// (`StyleDirectiveEffects.style_id`) or built-in directive handlers.
    pub id: &'static str,

    /// A callback function for how the string content of the template should be parsed
    /// If at all. Compiler will determine if this can be run at compile-time, or need a runtime call.
    pub formatter: Option<Formatter>,

    /// When true, nested child templates skip the parent-applied `$children(..)`
    /// wrappers while still allowing wrappers declared on the child itself.
    pub skip_parent_child_wrappers: bool,

    pub body_whitespace_policy: BodyWhitespacePolicy,

    /// When true, `[...]` brackets in the template body are treated as balanced
    /// literal text rather than parsed as nested child templates.
    pub suppress_child_templates: bool,
}

impl Style {
    pub fn default() -> Style {
        Style {
            id: "",
            formatter: None,
            skip_parent_child_wrappers: false,
            body_whitespace_policy: BodyWhitespacePolicy::DefaultTemplateBehavior,
            suppress_child_templates: false,
        }
    }
}

// -------------------------
//  Template AST Node
// -------------------------

/// The central template representation in the AST.
///
/// A `Template` is a narrow durable handle carrying its TIR identity and source
/// location. Effective style, kind and wrapper context are owned by the
/// `TemplateIr` entry resolved through `tir_reference`. The `Template` is the
/// durable value passed between parsing, composition, formatting, folding and
/// AST finalization.
#[derive(Clone, Debug)]
pub struct Template {
    /// Authoritative TIR reference.
    ///
    /// WHAT: holds the module-local root, pipeline phase, and value context.
    /// WHY: this long-lived reference carries the exact view and overlay identity
    ///      needed by consumers of the shared module store.
    pub(crate) tir_reference: TemplateTirReference,

    pub location: SourceLocation,
}
