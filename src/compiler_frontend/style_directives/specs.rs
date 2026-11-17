//! Data contracts for frontend-owned style directives.
//!
//! These types describe directive syntax, parser effects, formatter factories, and core
//! compiler-owned behavior. Frontend-owned directive values live in `builtins`.

use crate::compiler_frontend::ast::templates::template::Formatter;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::style_directives::compatibility::TemplateHeadCompatibility;
use crate::compiler_frontend::tokenizer::tokens::TemplateBodyMode;

/// The `$doc` source directive name.
///
/// Both the registry and AST metadata owners read the name from here so recognition and
/// diagnostic wording cannot drift apart.
pub(crate) const DOC_DIRECTIVE_NAME: &str = "doc";

/// The `$todo` source directive name.
///
/// Both the registry and AST validation owners read the name from here so recognition and
/// diagnostic wording cannot drift apart.
pub(crate) const TODO_DIRECTIVE_NAME: &str = "todo";

/// The core `$slot` directive name.
///
/// The symbol immediately after it names a slot in the receiving template's schema, not a
/// lexical value. Header-stage initializer scans read the name from here so they do not
/// record slot targets as constant references.
pub(crate) const SLOT_DIRECTIVE_NAME: &str = "slot";

/// The core `$supplied` directive name.
///
/// Its optional target names the same receiving slot key that `$slot` accepts. The directive
/// selects its body from that key's routed presence, so the target is a slot identity rather
/// than a lexical value. Header-stage initializer scans read the name from here so they do not
/// record guard targets as constant references.
pub(crate) const SUPPLIED_DIRECTIVE_NAME: &str = "supplied";

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

/// The `$export` directive name.
///
/// It is a source modifier, so the header walk reads the name from here to keep recognition and
/// file-role checks aligned.
pub(crate) const EXPORT_DIRECTIVE_NAME: &str = "export";

/// The `$config` directive name.
///
/// It is a source modifier, so the template-head parser and the header walk both read the name
/// from here to keep their context checks aligned.
pub(crate) const CONFIG_DIRECTIVE_NAME: &str = "config";

/// The `$fast_math` source-constraint directive name.
///
/// It selects fast semantics for fixed-integer and binary-float arithmetic in its lexical scope.
pub(crate) const FAST_MATH_DIRECTIVE_NAME: &str = "fast_math";

/// The `$safe_math` source-constraint directive name.
///
/// It guarantees that no reachable computation in its lexical scope uses fast numeric semantics.
pub(crate) const SAFE_MATH_DIRECTIVE_NAME: &str = "safe_math";

/// The `$infallible` source-constraint directive name.
///
/// It guarantees that no recoverable error or Moth-semantic trap escapes its lexical scope.
pub(crate) const INFALLIBLE_DIRECTIVE_NAME: &str = "infallible";

/// Attachment contexts a registered directive may appear in.
///
/// WHAT:
/// - `TEMPLATE_HEAD`: the directive prefixes a template and modifies that template.
/// - `SOURCE_MODIFIER`: the directive attaches to one following eligible source construct and may
///   share that attachment line with other source modifiers.
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

/// Which source position owns a directive name.
///
/// WHAT:
/// - `TemplateHead` prefixes a template and modifies that template.
/// - `Source` is every other position, including the inside of a code template body.
///
/// WHY:
/// - An unknown directive token and the names that could appear in its place depend on the
///   position, so the tokenizer needs the occurrence to choose accurate diagnostic wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectiveOccurrence {
    TemplateHead,
    Source,
}

/// Eligible target of one source-modifier directive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceModifierTarget {
    /// Exactly one explicitly typed top-level compile-time `#` binding.
    ExplicitTypedCompileTimeBinding,
    /// One authored public declaration or direct-selection dependency clause.
    ExportablePublicApiItem,
    /// One following declaration, including an eligible local declaration.
    DeclarationDocumentation,
    /// One following declaration or complete executable statement.
    DeclarationOrStatementTodo,
    /// One callable declaration (free function, receiver method or generic function) or one
    /// complete executable statement.
    ///
    /// WHAT: the three lexical source constraints share this eligibility set, so the variant
    ///       carries which constraint its directive names.
    /// WHY: the target contract stays one fact per eligibility set while each directive keeps its
    ///      own semantic kind; three identical target variants would make every target match
    ///      drift-prone without expressing any difference.
    SourceConstraint(SourceDirectiveKind),
}

/// Semantic role of one recognised source directive.
///
/// WHAT: separates the retained syntax kind from its target contract.
/// WHY: attachment handles every directive in one run, then each kind applies its own meaning
/// without re-resolving names or branching on spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceDirectiveKind {
    Config,
    Export,
    Doc,
    Todo,
    FastMath,
    SafeMath,
    Infallible,
}

impl SourceDirectiveKind {
    /// Whether this kind is one of the lexical source constraints.
    ///
    /// WHAT: the three kinds whose attachment changes the active expression contract for the
    ///       annotated function or statement.
    /// WHY: the header walk tests statement eligibility for exactly these three kinds, so the
    ///      grouping lives here instead of being spelled out as the same alternation again.
    pub(crate) fn is_source_constraint(self) -> bool {
        matches!(self, Self::FastMath | Self::SafeMath | Self::Infallible)
    }
}

impl SourceModifierTarget {
    pub(crate) fn directive_kind(self) -> SourceDirectiveKind {
        match self {
            Self::ExplicitTypedCompileTimeBinding => SourceDirectiveKind::Config,
            Self::ExportablePublicApiItem => SourceDirectiveKind::Export,
            Self::DeclarationDocumentation => SourceDirectiveKind::Doc,
            Self::DeclarationOrStatementTodo => SourceDirectiveKind::Todo,
            Self::SourceConstraint(kind) => kind,
        }
    }
}

/// Lexical source constraints active for one AST scope.
///
/// WHAT: the three source modifiers as one small copy value, inherited by every child scope of an
///       annotated function body or statement subtree.
/// WHY: numeric resolution and later constraint verification read one inherited fact instead of
///      re-walking directive syntax, and a plain value cannot leak the set into a callee body
///      whose own declaration did not opt in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceConstraintSet {
    fast_math: bool,
    safe_math: bool,
    infallible: bool,
}

impl SourceConstraintSet {
    /// Add one directive kind to this set.
    ///
    /// WHAT: constraint kinds set their field; every other kind leaves the set unchanged.
    /// WHY: attachment handles one annotation list whose entries may be documentation or TODO
    ///      directives as well as constraints.
    pub(crate) fn with_directive(self, kind: SourceDirectiveKind) -> Self {
        match kind {
            SourceDirectiveKind::FastMath => Self {
                fast_math: true,
                ..self
            },
            SourceDirectiveKind::SafeMath => Self {
                safe_math: true,
                ..self
            },
            SourceDirectiveKind::Infallible => Self {
                infallible: true,
                ..self
            },
            SourceDirectiveKind::Config
            | SourceDirectiveKind::Export
            | SourceDirectiveKind::Doc
            | SourceDirectiveKind::Todo => self,
        }
    }

    /// Smallest set containing this set and `other`.
    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            fast_math: self.fast_math || other.fast_math,
            safe_math: self.safe_math || other.safe_math,
            infallible: self.infallible || other.infallible,
        }
    }

    /// Whether no lexical constraint is active.
    pub(crate) fn is_empty(self) -> bool {
        !self.fast_math && !self.safe_math && !self.infallible
    }

    /// Whether fast numeric semantics are active in this scope.
    ///
    /// This is the accessor numeric resolution reads; it must never consult a callee body.
    pub(crate) fn fast_math_active(self) -> bool {
        self.fast_math
    }

    /// Whether the no-fast-use guarantee applies in this scope.
    pub(crate) fn safe_math_active(self) -> bool {
        self.safe_math
    }

    /// Whether the no-escaping-failure guarantee applies in this scope.
    pub(crate) fn infallible_active(self) -> bool {
        self.infallible
    }
}

/// Constraints attached to the next statement by one source-directive run.
///
/// WHAT: the union of the run's constraint annotations, the authored spans of the guarantees that
///       verification reports separately, and the target statement's first authored token.
/// WHY: the statement parser applies the set for exactly one statement and records the target's
///      authored range as a verification boundary without re-reading the directive run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatementSourceConstraintAttachment {
    pub(crate) constraints: SourceConstraintSet,
    pub(crate) target_span: SourceSpan,
    pub(crate) directive_spans: ConstraintDirectiveSpans,
}

/// Authored directive spans for the guarantees verification reports one by one.
///
/// WHAT: the run's `$safe_math` and `$infallible` spans, each `None` when that guarantee arrived
///       only by inheritance or through a reconstructed set.
/// WHY: `$fast_math` selects semantics but never fails verification, and one "first constraint"
///      span labels the wrong guarantee as soon as a run stacks them. Two optional spans keep each
///      violation at its own directive without a general annotation list or heap allocation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConstraintDirectiveSpans {
    pub(crate) safe_math: Option<SourceSpan>,
    pub(crate) infallible: Option<SourceSpan>,
}

impl ConstraintDirectiveSpans {
    /// Record one authored constraint directive's span when it can label a guarantee.
    ///
    /// WHAT: keeps the run's `$safe_math` or `$infallible` span and ignores every other kind,
    ///       including `$fast_math`.
    /// WHY: header and statement attachment share one projection, and the duplicate-directive
    ///      check already rejects two annotations of the same kind in a run.
    pub(crate) fn record(&mut self, kind: SourceDirectiveKind, span: SourceSpan) {
        match kind {
            SourceDirectiveKind::SafeMath => {
                self.safe_math.get_or_insert(span);
            }
            SourceDirectiveKind::Infallible => {
                self.infallible.get_or_insert(span);
            }
            SourceDirectiveKind::FastMath
            | SourceDirectiveKind::Config
            | SourceDirectiveKind::Export
            | SourceDirectiveKind::Doc
            | SourceDirectiveKind::Todo => {}
        }
    }
}

/// Operand shape a source directive requires after its name.
///
/// WHAT: `None` accepts no operand; `RequiredSingleString` requires exactly one compile-time
///       String expression whose tokens the AST owner parses and folds.
/// WHY: the header and statement attachment owners read one contract per directive instead of
///      hard-coding operand acceptance by directive name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceModifierOperand {
    None,
    RequiredSingleString,
}

/// Core language directive behavior handled directly by compiler-owned template parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreStyleDirectiveKind {
    Before,
    After,
    Fresh,
    Slot,
    Supplied,
    In,
    Note,
    Raw,
}

/// Supported optional single-operand types for handler-based directives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleDirectiveArgumentType {
    String,
}

/// Parsed value for an optional handler-based style operand.
#[derive(Clone, Debug)]
pub enum StyleDirectiveArgumentValue {
    String(String),
}

/// Semantic template-style state a handler-based directive applies when used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleDirectiveEffects {
    /// Optional semantic style label applied to the effective `TemplateIr.style.id`.
    ///
    /// This is intentionally distinct from `Formatter.id`:
    /// - `style_id` tags parsed template semantics.
    /// - `Formatter.id` identifies the concrete formatter implementation.
    pub style_id: Option<&'static str>,
    /// Optional toggle for parsing `[...]` as literal body text.
    pub suppress_child_templates: Option<bool>,
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
    /// A source-only directive whose eligible target and operand shape the spec states.
    SourceModifier {
        target: SourceModifierTarget,
        operand: SourceModifierOperand,
    },
}

/// Frontend-owned contract for one style directive.
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
    /// Declare a handler-based frontend directive.
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

    /// Internal helper for source-directive names.
    pub(crate) fn source_modifier(
        name: &str,
        target: SourceModifierTarget,
        operand: SourceModifierOperand,
    ) -> Self {
        Self {
            name: name.to_string(),
            body_mode: TemplateBodyMode::Normal,
            head_compatibility: TemplateHeadCompatibility::default(),
            contexts: StyleDirectiveContexts::SOURCE_MODIFIER,
            kind: StyleDirectiveKind::SourceModifier { target, operand },
        }
    }
}
