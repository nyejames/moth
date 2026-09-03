//! Compiler-owned typed build-input vocabulary.
//!
//! WHAT: one carrier family for build-configuration input and resolution facts — validated
//!       lower_snake_case names ([`BuildInputName`]), primitive and optional contract types
//!       ([`BuildInputType`]), typed primitive values ([`PrimitiveBuildValue`]), source and
//!       command locations, resolution origins, and the deterministic duplicate-free
//!       [`BuildConfigInputSet`] map that command parsing fills.
//! WHY:  CLI parsing, programmatic commands and later `#Config` resolution must share one
//!       typed vocabulary instead of reparsing text or carrying string-backed fallback values.
//!       These carriers are semantic input facts owned by the frontend: they never carry target
//!       or platform identity and never depend on build-system or project settings, so build
//!       and project code constructs and consumes them without owning compiler semantics.
//!
//! Command-input value inference lives here so build, check and dev share one immediate
//! primitive-inference path. [`PrimitiveBuildValue::from_command_text`] composes the ordinary
//! Moth literal grammar and the shared `numeric_text` materialisation helpers — it never
//! consults a project or source contract and never re-implements a literal grammar.
//!
//! Exclusions: no `--input` argument parser or `#Config` grammar lives here. Command layers
//! split each argument at the first `=`, validate names and insert the resulting entries.
//! Optional absence is represented by omitting an input, never by a value in this vocabulary.
//! Resolution records that keep a typed value together with its origin arrive with the
//! resolution phases that produce them.
//!

use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason;
use crate::compiler_frontend::folded_value::FiniteFloat;
use crate::compiler_frontend::keywords::is_valid_identifier;
use crate::compiler_frontend::numeric_text::parse::{
    parse_numeric_text_to_f64, parse_numeric_text_to_i32,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identifier_policy::is_lowercase_with_underscores_name;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, TokenKind, TokenizerEntryMode};

use crate::builder_surface::config_schema::ProjectFieldConfigPolicy;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::rc::Rc;

/// The deterministic, duplicate-free set of explicit build-config inputs.
///
/// WHAT: the one map later command parsing fills with typed CLI and programmatic inputs, keyed
///       by validated name and iterated in name order. Insertion rejects a second entry for the
///       same name instead of overwriting the first typed value.
/// WHY:  duplicate explicit names are rejected deterministically by command parsing, and every
///       later resolution, diagnostic and fingerprint consumer must read the same stable order.
///       There is no untyped text fallback entry: a value enters the set only as a typed
///       [`PrimitiveBuildValue`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BuildConfigInputSet {
    entries: BTreeMap<BuildInputName, BuildConfigInputEntry>,
}

impl BuildConfigInputSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Insert one typed input, rejecting a second entry for the same name.
    ///
    /// On rejection the earlier entry keeps its place and the error carries the rejected entry
    /// back with the earlier location, so command diagnostics can underline both arguments.
    pub(crate) fn insert(
        &mut self,
        entry: BuildConfigInputEntry,
    ) -> Result<(), BuildConfigInputDuplicate> {
        let name = entry.name.clone();

        if let Some(existing) = self.entries.get(&name) {
            return Err(BuildConfigInputDuplicate {
                rejected: entry,
                existing_location: existing.location.clone(),
            });
        }

        self.entries.insert(name, entry);
        Ok(())
    }

    pub(crate) fn get(&self, name: &BuildInputName) -> Option<&BuildConfigInputEntry> {
        self.entries.get(name)
    }

    /// Iterate every entry in deterministic name order.
    #[allow(dead_code)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = &BuildConfigInputEntry> {
        self.entries.values()
    }

    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Typed primitive globals exposed by the selected builder.
///
/// This carrier intentionally contains no target, platform, backend or filesystem identity. A
/// builder may populate it with stable semantic values; the default surface is empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BuilderConfigGlobalSet {
    entries: BTreeMap<BuildInputName, PrimitiveBuildValue>,
}

/// A builder global name that is forbidden because it would expose implementation identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BuilderConfigGlobalError {
    PlatformIdentityName(BuildInputName),
}

impl BuilderConfigGlobalError {
    pub(crate) fn name(&self) -> &BuildInputName {
        match self {
            Self::PlatformIdentityName(name) => name,
        }
    }
}

impl fmt::Display for BuilderConfigGlobalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "builder config global '{}' exposes backend or platform identity",
            self.name().as_str()
        )
    }
}

impl BuilderConfigGlobalSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Insert one typed builder global unless its name describes the selected implementation.
    ///
    /// Backend and platform identity belong to builder capability resolution, not Moth source
    /// configuration. Rejecting these names at the carrier boundary prevents a builder surface
    /// from smuggling target selection through `#Config`.
    #[allow(dead_code)]
    pub(crate) fn insert(
        &mut self,
        name: BuildInputName,
        value: PrimitiveBuildValue,
    ) -> Result<Option<PrimitiveBuildValue>, BuilderConfigGlobalError> {
        if is_forbidden_builder_global_name(name.as_str()) {
            return Err(BuilderConfigGlobalError::PlatformIdentityName(name));
        }

        Ok(self.entries.insert(name, value))
    }

    pub(crate) fn get(&self, name: &BuildInputName) -> Option<&PrimitiveBuildValue> {
        self.entries.get(name)
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Names that would let source inspect a physical target or selected backend.
fn is_forbidden_builder_global_name(name: &str) -> bool {
    matches!(
        name,
        "target_os" | "target_arch" | "backend" | "is_wasm" | "is_javascript" | "is_browser"
    )
}

/// A rejected duplicate input name, carrying the rejected entry and the earlier location.
#[derive(Clone, Debug)]
pub(crate) struct BuildConfigInputDuplicate {
    rejected: BuildConfigInputEntry,
    existing_location: BuildConfigValueLocation,
}

impl BuildConfigInputDuplicate {
    pub(crate) fn rejected(&self) -> &BuildConfigInputEntry {
        &self.rejected
    }

    pub(crate) fn existing_location(&self) -> &BuildConfigValueLocation {
        &self.existing_location
    }
}

/// One explicit build-config input: a typed value plus where the caller supplied it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuildConfigInputEntry {
    name: BuildInputName,
    value: PrimitiveBuildValue,
    location: BuildConfigValueLocation,
}

impl BuildConfigInputEntry {
    pub(crate) fn new(
        name: BuildInputName,
        value: PrimitiveBuildValue,
        location: BuildConfigValueLocation,
    ) -> Self {
        Self {
            name,
            value,
            location,
        }
    }

    pub(crate) fn name(&self) -> &BuildInputName {
        &self.name
    }

    pub(crate) fn value(&self) -> &PrimitiveBuildValue {
        &self.value
    }

    pub(crate) fn location(&self) -> &BuildConfigValueLocation {
        &self.location
    }
}

/// One normalised typed primitive build-config value.
///
/// WHAT: the only value vocabulary build inputs and `#Config` resolution transport: one exact
///       typed primitive per value, with `Float` reusing the frontend's validated
///       [`FiniteFloat`] wrapper so non-finite values cannot enter resolution.
/// WHY:  there is deliberately no raw, untyped or `none` variant. Optional absence is an
///       omitted input resolved through project and default rules, never a value, and String
///       fallback is decided by command-text inference before a value is constructed, so no
///       string-backed marker survives into the typed vocabulary. Authored String and Char
///       content is preserved exactly, without backend conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PrimitiveBuildValue {
    String(String),
    Int(i32),
    Float(FiniteFloat),
    Bool(bool),
    Char(char),
}

impl PrimitiveBuildValue {
    /// Construct a finite `Float` value. Non-finite input is rejected; command parsing turns
    /// that rejection into a command-input diagnostic.
    pub(crate) fn float(value: f64) -> Result<Self, CompilerError> {
        Ok(Self::Float(FiniteFloat::new(value)?))
    }

    /// Infer one typed primitive value from raw command-input text.
    ///
    /// WHAT: the one immediate inference path every command variant shares. Exact lowercase
    ///       `true`/`false` infers Bool; a complete valid signed whole-number literal infers
    ///       Int; a complete valid decimal-point or exponent literal infers Float; a
    ///       single-quote-leading value must be one complete ordinary Moth Char literal; a
    ///       double-quote-leading value must be one complete ordinary Moth String literal; and
    ///       every other value — including the empty value, bare `none`, `+1`, `NaN`,
    ///       `Infinity`, backtick text and malformed numeric-looking text — falls back to
    ///       String with its exact authored remainder preserved.
    /// WHY:  a command value's primitive type is decided immediately from the authored text,
    ///       never by waiting for a project or source contract. Numeric materialisation and
    ///       quoted literal parsing compose the compiler's existing `numeric_text` and ordinary
    ///       literal-grammar owners, so no second grammar exists, and whole-number Int
    ///       overflow and non-finite Float results are diagnostics rather than fallbacks.
    pub(crate) fn from_command_text(value: &str) -> Result<Self, BuildInputValueError> {
        if value == "true" {
            return Ok(Self::Bool(true));
        }
        if value == "false" {
            return Ok(Self::Bool(false));
        }

        match parse_numeric_text_to_i32(value) {
            Ok(materialised) => return Ok(Self::Int(materialised)),
            Err(NumberLiteralErrorReason::OutsideIntRange) => {
                return Err(BuildInputValueError::IntOutOfRange {
                    text: value.to_owned(),
                });
            }
            Err(_) => {}
        }

        match parse_numeric_text_to_f64(value) {
            Ok(materialised) => {
                return Self::float(materialised).map_err(|_| {
                    BuildInputValueError::NonFiniteFloat {
                        text: value.to_owned(),
                    }
                });
            }
            Err(
                NumberLiteralErrorReason::NonFiniteFloat | NumberLiteralErrorReason::ParseOverflow,
            ) => {
                return Err(BuildInputValueError::NonFiniteFloat {
                    text: value.to_owned(),
                });
            }
            Err(_) => {}
        }

        if value.starts_with('\'') || value.starts_with('"') {
            let literal = match parse_ordinary_quoted_literal(value) {
                Ok(literal) => literal,
                Err(rejection) => {
                    return Err(malformed_quoted_literal_error(value, rejection));
                }
            };

            return Ok(match literal {
                OrdinaryCommandLiteral::String(text) => Self::String(text),
                OrdinaryCommandLiteral::Char(character) => Self::Char(character),
            });
        }

        Ok(Self::String(value.to_owned()))
    }

    /// The exact primitive type this value carries.
    pub(crate) fn primitive_type(&self) -> PrimitiveBuildInputType {
        match self {
            Self::String(_) => PrimitiveBuildInputType::String,
            Self::Int(_) => PrimitiveBuildInputType::Int,
            Self::Float(_) => PrimitiveBuildInputType::Float,
            Self::Bool(_) => PrimitiveBuildInputType::Bool,
            Self::Char(_) => PrimitiveBuildInputType::Char,
        }
    }
}

/// Why a command-input value was rejected instead of inferred.
///
/// WHAT: one rejection vocabulary for immediate command-input inference. Whole-number Int
///       overflow and non-finite Float materialisation are diagnostics; a quote-leading value
///       that is not one complete ordinary Moth literal is rejected with the ordinary
///       tokenizer's stable diagnostic title.
/// WHY:  every command variant renders the same concrete rejection without a second wording
///       owner, and the compiler owns the inference semantics behind the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BuildInputValueError {
    /// A whole-number-shaped value is outside the Int range.
    IntOutOfRange { text: String },
    /// A decimal-point or exponent-shaped value did not materialise as a finite Float.
    NonFiniteFloat { text: String },
    /// A single-quote-leading value is not one complete ordinary Moth Char literal.
    MalformedCharLiteral { text: String, reason: &'static str },
    /// A double-quote-leading value is not one complete ordinary Moth String literal.
    MalformedStringLiteral { text: String, reason: &'static str },
}

impl fmt::Display for BuildInputValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IntOutOfRange { text } => {
                write!(
                    formatter,
                    "whole-number value '{text}' is outside the Int range"
                )
            }
            Self::NonFiniteFloat { text } => {
                write!(formatter, "float value '{text}' is not finite")
            }
            Self::MalformedCharLiteral { text, reason } => write!(
                formatter,
                "value '{text}' is not a complete Moth Char literal ({reason})"
            ),
            Self::MalformedStringLiteral { text, reason } => write!(
                formatter,
                "value '{text}' is not a complete Moth String literal ({reason})"
            ),
        }
    }
}

/// One complete ordinary Moth literal parsed from standalone command text.
///
/// The ordinary grammar is deterministic about the delimiter: a single-quote-leading value
/// only ever completes as a Char literal and a double-quote-leading value only ever completes
/// as a String literal, so the parsed kind always matches the authored leading quote.
enum OrdinaryCommandLiteral {
    String(String),
    Char(char),
}

/// Why a quote-leading command value is not one complete ordinary literal.
struct QuotedLiteralRejection {
    reason: &'static str,
}

/// Reason text for a value that tokenizes but carries text past its complete literal.
const QUOTED_LITERAL_TRAILING_TEXT: &str = "text follows the literal";

/// The synthetic file identity given to the ordinary lexer for standalone command values.
///
/// Command inputs carry no source span, so the path only exists to keep the ordinary lexer's
/// diagnostics well-formed; command diagnostics use its stable titles, not the location.
const COMMAND_INPUT_TOKENIZER_PATH: &str = "command-input";

/// Map a quoted-literal rejection to the typed error for the authored leading quote.
fn malformed_quoted_literal_error(
    value: &str,
    rejection: QuotedLiteralRejection,
) -> BuildInputValueError {
    if value.starts_with('\'') {
        BuildInputValueError::MalformedCharLiteral {
            text: value.to_owned(),
            reason: rejection.reason,
        }
    } else {
        BuildInputValueError::MalformedStringLiteral {
            text: value.to_owned(),
            reason: rejection.reason,
        }
    }
}

/// Parse one quote-leading command value through the ordinary Moth literal grammar.
///
/// WHAT: tokenizes the value with the ordinary file lexer and accepts only the exact
///       `ModuleStart + one matching StringSliceLiteral/CharLiteral + Eof` shape. The literal
///       must begin at the first value character and end at the tokenizer's final Eof position.
///       This deliberately rejects all terminal trivia, including comments and whitespace.
/// WHY:  command inputs must reuse the one ordinary literal grammar — delimiter, escape and
///       rejection policy stay owned by the tokenizer instead of a second command parser. The
///       strict terminal-trivia policy prevents source-file comment handling from silently
///       discarding an authored command-value suffix.
fn parse_ordinary_quoted_literal(
    value: &str,
) -> Result<OrdinaryCommandLiteral, QuotedLiteralRejection> {
    let mut string_table = StringTable::new();
    let path = InternedPath::from_single_str(COMMAND_INPUT_TOKENIZER_PATH, &mut string_table);
    let file_tokens = tokenize(
        value,
        &path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        None,
    )
    .map_err(|diagnostic| QuotedLiteralRejection {
        reason: diagnostic.kind.descriptor().title,
    })?;

    let [module_start, literal, eof] = file_tokens.tokens.as_slice() else {
        return Err(QuotedLiteralRejection {
            reason: QUOTED_LITERAL_TRAILING_TEXT,
        });
    };

    if !matches!(module_start.kind, TokenKind::ModuleStart)
        || !matches!(eof.kind, TokenKind::Eof)
        || literal.location.start_pos
            != (CharPosition {
                line_number: 0,
                char_column: 1,
            })
        || literal.location.end_pos != eof.location.end_pos
    {
        return Err(QuotedLiteralRejection {
            reason: QUOTED_LITERAL_TRAILING_TEXT,
        });
    }

    match &literal.kind {
        TokenKind::StringSliceLiteral(id) => Ok(OrdinaryCommandLiteral::String(
            string_table.resolve(*id).to_owned(),
        )),
        TokenKind::CharLiteral(character) => Ok(OrdinaryCommandLiteral::Char(*character)),
        _ => Err(QuotedLiteralRejection {
            reason: QUOTED_LITERAL_TRAILING_TEXT,
        }),
    }
}

/// One accepted `#Config` contract type: a primitive or the matching optional primitive.
///
/// WHAT: the complete contract-type vocabulary for build configuration. `Optional` wraps the
///       same primitive, so `String?` is `Optional(String)` and not a separate aggregate type.
/// WHY:  contract checking accepts an exact primitive match plus a concrete value for the
///       matching optional, and nothing else; encoding only these two shapes here keeps
///       aggregate, nominal, generic and path types out of the build-input boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BuildInputType {
    Primitive(PrimitiveBuildInputType),
    Optional(PrimitiveBuildInputType),
}

impl BuildInputType {
    /// The primitive this contract carries, for required and optional forms alike.
    pub(crate) fn primitive(&self) -> PrimitiveBuildInputType {
        match self {
            Self::Primitive(primitive) | Self::Optional(primitive) => *primitive,
        }
    }

    /// True when this contract is the optional form.
    pub(crate) fn is_optional(&self) -> bool {
        matches!(self, Self::Optional(_))
    }

    /// True when a typed value of `primitive` satisfies this contract.
    ///
    /// The one build-input compatibility rule: exact primitive types match, and a concrete
    /// value satisfies the matching optional as a present value. No other coercion exists —
    /// `Int` does not satisfy `Float`, and no primitive satisfies `String` implicitly.
    pub(crate) fn accepts_primitive(&self, primitive: PrimitiveBuildInputType) -> bool {
        self.primitive() == primitive
    }
}

/// The primitive types a build-config contract accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PrimitiveBuildInputType {
    String,
    Int,
    Float,
    Bool,
    Char,
}

impl PrimitiveBuildInputType {
    /// The exact Moth type spelling used by diagnostics and contract reporting.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::String => "String",
            Self::Int => "Int",
            Self::Float => "Float",
            Self::Bool => "Bool",
            Self::Char => "Char",
        }
    }
}

/// Why a build-config name was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuildInputNameError {
    /// The name is empty or not lower_snake_case under the canonical identifier policy.
    NotLowerSnakeCase,
}

/// A validated lower_snake_case build-config name.
///
/// WHAT: owns the exact validated spelling of a `#Config` name or explicit input name as
///       stable owned text. Validation composes the canonical identifier policy helpers, so
///       build names cannot drift from source identifier rules.
/// WHY:  input names match source contract names by exact spelling, so one validated owner
///       lets command parsing, contract shells and resolution compare names without
///       revalidating at every boundary. Lexicographic `Ord` gives the deterministic ordering
///       every map and diagnostic in this module iterates in.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct BuildInputName {
    text: String,
}

impl BuildInputName {
    /// Validate and own one lower_snake_case build-config name.
    ///
    /// A digit-leading spelling such as `2fast` is rejected because it can never match a
    /// source declaration identifier; keyword-shadow reservation is owned separately by the
    /// canonical keyword policy.
    pub(crate) fn new(text: &str) -> Result<Self, BuildInputNameError> {
        if is_valid_identifier(text) && is_lowercase_with_underscores_name(text) {
            Ok(Self {
                text: text.to_owned(),
            })
        } else {
            Err(BuildInputNameError::NotLowerSnakeCase)
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }
}

/// Where a build-config value or contract fact came from.
///
/// WHAT: one location enum for diagnostics and provenance. Retained compiler source spans reuse
///       the shared [`SourceLocation`]; command and programmatic inputs carry their argument
///       position instead, because they have no source span.
/// WHY:  mismatch, duplicate and missing-input diagnostics must underline either the source
///       declaration or the exact command argument without a second location model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BuildConfigValueLocation {
    /// A retained compiler source span: a contract declaration, its default literal or a
    /// direct project field initializer.
    Source(SourceLocation),

    /// An explicit command or programmatic input at one argument position.
    Command(BuildCommandLocation),
}

/// The command-side position of one explicit build input.
///
/// WHAT: identifies one `--input name=value` argument (or programmatic input) by its zero-based
///       position in command order.
/// WHY:  command inputs have no source span, yet duplicate and mismatch diagnostics still need
///       a stable per-input identity; the argument position is that identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BuildCommandLocation {
    argument_index: usize,
}

impl BuildCommandLocation {
    pub(crate) fn new(argument_index: usize) -> Self {
        Self { argument_index }
    }

    pub(crate) fn argument_index(&self) -> usize {
        self.argument_index
    }
}

/// How a resolved build-config value was produced.
///
/// WHAT: the resolution-origin provenance later `#Config` resolution records next to a typed
///       value and its location, covering both accepted resolution orders: direct project
///       fields and project-wide source contracts.
/// WHY: resolved values retain their origin so diagnostics can explain explicit inputs, builder
///       globals, authoritative fixed fields and folded defaults. Origin is provenance only;
///       semantic fingerprints deliberately exclude it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BuildConfigValueOrigin {
    /// An explicit CLI or programmatic input.
    ExplicitInput,

    /// A compatible builder-provided primitive global.
    BuilderGlobal,

    /// A fixed direct project field, which is authoritative for its name and blocks overrides.
    FixedProjectField,
    /// The folded default of the declaration itself.
    DeclarationDefault,
}

/// Stable semantic fingerprint for one resolved build-config value.
///
/// The fingerprint covers only the field name, contract and effective value-or-absence. Source
/// locations and resolution origins are intentionally excluded: they describe provenance, while
/// the fingerprint answers whether the resolved configuration semantics changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BuildConfigFingerprint(pub(crate) u64);

pub(crate) fn build_config_fingerprint(
    field_name: &str,
    contract: BuildInputType,
    value: Option<&PrimitiveBuildValue>,
) -> BuildConfigFingerprint {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    let mut hash = OFFSET_BASIS;

    fingerprint_feed_string(&mut hash, field_name);
    fingerprint_feed(&mut hash, &[u8::from(contract.is_optional())]);
    fingerprint_feed(
        &mut hash,
        &[match contract.primitive() {
            PrimitiveBuildInputType::String => 0,
            PrimitiveBuildInputType::Int => 1,
            PrimitiveBuildInputType::Float => 2,
            PrimitiveBuildInputType::Bool => 3,
            PrimitiveBuildInputType::Char => 4,
        }],
    );

    match value {
        None => fingerprint_feed(&mut hash, &[0]),
        Some(PrimitiveBuildValue::String(value)) => {
            fingerprint_feed(&mut hash, &[1]);
            fingerprint_feed_string(&mut hash, value);
        }
        Some(PrimitiveBuildValue::Int(value)) => {
            fingerprint_feed(&mut hash, &[2]);
            fingerprint_feed(&mut hash, &value.to_le_bytes());
        }
        Some(PrimitiveBuildValue::Float(value)) => {
            fingerprint_feed(&mut hash, &[3]);
            fingerprint_feed_u64(&mut hash, value.value().to_bits());
        }
        Some(PrimitiveBuildValue::Bool(value)) => {
            fingerprint_feed(&mut hash, &[4, u8::from(*value)]);
        }
        Some(PrimitiveBuildValue::Char(value)) => {
            fingerprint_feed(&mut hash, &[5]);
            fingerprint_feed_u64(&mut hash, u64::from(u32::from(*value)));
        }
    }

    BuildConfigFingerprint(hash)
}

fn fingerprint_feed(hash: &mut u64, bytes: &[u8]) {
    const FNV_PRIME: u64 = 0x100000001b3;
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn fingerprint_feed_u64(hash: &mut u64, value: u64) {
    fingerprint_feed(hash, &value.to_le_bytes());
}

fn fingerprint_feed_string(hash: &mut u64, value: &str) {
    fingerprint_feed_u64(hash, value.len() as u64);
    fingerprint_feed(hash, value.as_bytes());
}

/// Provenance retained for one resolved direct-project `#Config` field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigResolutionRecord {
    pub(crate) field_name: StringId,
    pub(crate) contract: BuildInputType,
    /// Whether the authored direct-project contract had no satisfiable value by default.
    ///
    /// This is retained separately from the selected value: explicit inputs and builder globals
    /// replace the authored default, but boundary compatibility still compares the original
    /// required/default contract facts with source declarations.
    pub(crate) required: bool,
    /// The normalized authored default, if the direct-project contract declared one.
    pub(crate) default: Option<PrimitiveBuildValue>,
    pub(crate) value: Option<PrimitiveBuildValue>,
    pub(crate) origin: BuildConfigValueOrigin,
    pub(crate) fingerprint: BuildConfigFingerprint,
    pub(crate) qualifier_location: SourceLocation,
    pub(crate) value_location: Option<BuildConfigValueLocation>,
}

/// Compiler-owned inputs used while config constants are folded.
///
/// The service owns resolution; this carrier only snapshots typed inputs, builder globals and the
/// schema-derived direct-project policy. It deliberately contains no build `Config`, target or
/// platform identity.
#[derive(Clone, Debug)]
pub(crate) struct ConfigResolutionServices {
    explicit_inputs: BuildConfigInputSet,
    builder_globals: BuilderConfigGlobalSet,
    project_field_policies: crate::builder_surface::config_schema::ProjectFieldConfigPolicies,
    records: RefCell<Vec<ConfigResolutionRecord>>,
}

impl ConfigResolutionServices {
    pub(crate) fn new(
        explicit_inputs: &BuildConfigInputSet,
        builder_globals: &BuilderConfigGlobalSet,
        project_field_policies: crate::builder_surface::config_schema::ProjectFieldConfigPolicies,
    ) -> Rc<Self> {
        Rc::new(Self {
            explicit_inputs: explicit_inputs.clone(),
            builder_globals: builder_globals.clone(),
            project_field_policies,
            records: RefCell::new(Vec::new()),
        })
    }

    pub(crate) fn explicit_input(&self, name: &BuildInputName) -> Option<&BuildConfigInputEntry> {
        self.explicit_inputs.get(name)
    }

    pub(crate) fn builder_global(&self, name: &BuildInputName) -> Option<&PrimitiveBuildValue> {
        self.builder_globals.get(name)
    }

    pub(crate) fn project_field_policy(&self, field_name: &str) -> ProjectFieldConfigPolicy {
        self.project_field_policies.policy_for(field_name)
    }
    pub(crate) fn project_field_shape(
        &self,
        field_name: &str,
    ) -> Option<&crate::builder_surface::config_schema::ConfigFieldShape> {
        self.project_field_policies.shape_for(field_name)
    }

    pub(crate) fn record(&self, record: ConfigResolutionRecord) {
        self.records.borrow_mut().push(record);
    }

    pub(crate) fn take_records(&self) -> Vec<ConfigResolutionRecord> {
        std::mem::take(&mut *self.records.borrow_mut())
    }
}

// ------------------------
//  Boundary Contract Resolution
// ------------------------

/// One normalized contract fact supplied to the project-wide build-config barrier.
///
/// Source shells and project fields enter the barrier through this one shape. The caller chooses
/// whether a fact is a source contract, a fixed project field or a direct project `#Config` field
/// by placing it in the corresponding resolver input slice; the fact itself never carries
/// build-system or source-graph identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuildConfigContractFact {
    name: BuildInputName,
    value_type: BuildInputType,
    required: bool,
    default: Option<PrimitiveBuildValue>,
    location: SourceLocation,
    resolved_provider: Option<ResolvedBuildConfigProvider>,
}

impl BuildConfigContractFact {
    /// Construct one already-normalized boundary fact.
    pub(crate) fn new(
        name: BuildInputName,
        value_type: BuildInputType,
        required: bool,
        default: Option<PrimitiveBuildValue>,
        location: SourceLocation,
    ) -> Self {
        Self {
            name,
            value_type,
            required,
            default,
            location,
            resolved_provider: None,
        }
    }

    /// Attach the value already selected while a direct project contract folded.
    ///
    /// The barrier still validates this fact against source contracts, but must not select a
    /// second value or reconstruct its provenance from the authored contract alone.
    pub(crate) fn with_resolved_provider(
        mut self,
        value: Option<PrimitiveBuildValue>,
        origin: BuildConfigValueOrigin,
        fingerprint: BuildConfigFingerprint,
        value_location: Option<BuildConfigValueLocation>,
    ) -> Self {
        self.resolved_provider = Some(ResolvedBuildConfigProvider {
            value,
            origin,
            fingerprint,
            value_location,
        });
        self
    }

    pub(crate) fn name(&self) -> &BuildInputName {
        &self.name
    }

    pub(crate) fn value_type(&self) -> BuildInputType {
        self.value_type
    }

    pub(crate) fn required(&self) -> bool {
        self.required
    }

    pub(crate) fn default_value(&self) -> Option<&PrimitiveBuildValue> {
        self.default.as_ref()
    }

    pub(crate) fn location(&self) -> &SourceLocation {
        &self.location
    }

    fn resolved_provider(&self) -> Option<&ResolvedBuildConfigProvider> {
        self.resolved_provider.as_ref()
    }
}

/// Value/provenance already selected by the direct-project config folding owner.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedBuildConfigProvider {
    value: Option<PrimitiveBuildValue>,
    origin: BuildConfigValueOrigin,
    fingerprint: BuildConfigFingerprint,
    value_location: Option<BuildConfigValueLocation>,
}

/// Why two build-config contracts cannot describe one shared input.
///
/// The resolver reports the first difference in this order: primitive type, optionality, required
/// state, then normalized default. Keeping that order explicit makes conflict diagnostics stable
/// even when a malformed pair differs in more than one property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BuildConfigContractConflictReason {
    PrimitiveType {
        first: PrimitiveBuildInputType,
        conflicting: PrimitiveBuildInputType,
    },
    Optionality {
        first: bool,
        conflicting: bool,
    },
    Required {
        first: bool,
        conflicting: bool,
    },
    Default {
        first: Option<PrimitiveBuildValue>,
        conflicting: Option<PrimitiveBuildValue>,
    },
}

/// A typed failure found while collecting or resolving one boundary's config facts.
///
/// These are compiler-owned reason carriers. Build and project layers can map them into their
/// diagnostic bag later without reparsing names, values or command arguments, while focused
/// frontend tests can inspect the exact conflicting facts and locations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BuildConfigResolutionError {
    SourceContractConflict {
        first: BuildConfigContractFact,
        conflicting: BuildConfigContractFact,
        reason: BuildConfigContractConflictReason,
    },
    DuplicateProjectContract {
        first: BuildConfigContractFact,
        conflicting: BuildConfigContractFact,
        reason: BuildConfigContractConflictReason,
    },
    ProjectSourceContractConflict {
        project: BuildConfigContractFact,
        source: BuildConfigContractFact,
        reason: BuildConfigContractConflictReason,
    },
    FixedProjectSourceTypeMismatch {
        fixed: BuildConfigContractFact,
        source: BuildConfigContractFact,
        reason: BuildConfigContractConflictReason,
    },
    DefaultTypeMismatch {
        contract: BuildConfigContractFact,
        provided: PrimitiveBuildInputType,
    },
    ValueTypeMismatch {
        contract: BuildConfigContractFact,
        provided: PrimitiveBuildInputType,
        value_location: Option<BuildConfigValueLocation>,
    },
    MissingRequiredValue {
        contract: BuildConfigContractFact,
    },
    /// A direct project contract reached the barrier without its folded provider payload.
    ///
    /// This is an internal handoff invariant: direct project config selection belongs exclusively
    /// to the config-folding service and must never fall back to boundary resolution.
    DirectProjectProviderMissing {
        contract: BuildConfigContractFact,
    },
    UnknownExplicitInput {
        input: BuildConfigInputEntry,
    },
}

impl BuildConfigResolutionError {
    /// Return the name whose resolution failed.
    pub(crate) fn name(&self) -> &BuildInputName {
        match self {
            Self::SourceContractConflict { first, .. }
            | Self::DuplicateProjectContract { first, .. } => first.name(),
            Self::ProjectSourceContractConflict { source, .. }
            | Self::FixedProjectSourceTypeMismatch { source, .. }
            | Self::DefaultTypeMismatch {
                contract: source, ..
            }
            | Self::ValueTypeMismatch {
                contract: source, ..
            }
            | Self::MissingRequiredValue {
                contract: source, ..
            }
            | Self::DirectProjectProviderMissing {
                contract: source, ..
            } => source.name(),
            Self::UnknownExplicitInput { input } => input.name(),
        }
    }

    /// Return the authored contract location when this error has one.
    #[allow(dead_code)]
    pub(crate) fn contract_location(&self) -> Option<&SourceLocation> {
        match self {
            Self::SourceContractConflict { first, .. }
            | Self::DuplicateProjectContract { first, .. } => Some(first.location()),
            Self::ProjectSourceContractConflict { source, .. }
            | Self::FixedProjectSourceTypeMismatch { source, .. }
            | Self::DefaultTypeMismatch {
                contract: source, ..
            }
            | Self::ValueTypeMismatch {
                contract: source, ..
            }
            | Self::MissingRequiredValue {
                contract: source, ..
            }
            | Self::DirectProjectProviderMissing {
                contract: source, ..
            } => Some(source.location()),
            Self::UnknownExplicitInput { .. } => None,
        }
    }

    /// Return the typed mismatch reason, if this error came from a supplied value or default.
    pub(crate) fn provided_type(&self) -> Option<PrimitiveBuildInputType> {
        match self {
            Self::DefaultTypeMismatch { provided, .. }
            | Self::ValueTypeMismatch { provided, .. } => Some(*provided),
            Self::SourceContractConflict { .. }
            | Self::DuplicateProjectContract { .. }
            | Self::ProjectSourceContractConflict { .. }
            | Self::FixedProjectSourceTypeMismatch { .. }
            | Self::MissingRequiredValue { .. }
            | Self::DirectProjectProviderMissing { .. }
            | Self::UnknownExplicitInput { .. } => None,
        }
    }

    /// Return the command or source location of a supplied value, if one exists.
    pub(crate) fn value_location(&self) -> Option<&BuildConfigValueLocation> {
        match self {
            Self::ValueTypeMismatch { value_location, .. } => value_location.as_ref(),
            Self::UnknownExplicitInput { input } => Some(input.location()),
            Self::SourceContractConflict { .. }
            | Self::DuplicateProjectContract { .. }
            | Self::ProjectSourceContractConflict { .. }
            | Self::FixedProjectSourceTypeMismatch { .. }
            | Self::DefaultTypeMismatch { .. }
            | Self::MissingRequiredValue { .. }
            | Self::DirectProjectProviderMissing { .. } => None,
        }
    }
    /// Return the two authored facts involved in a contract conflict.
    pub(crate) fn conflict_facts(
        &self,
    ) -> Option<(&BuildConfigContractFact, &BuildConfigContractFact)> {
        match self {
            Self::SourceContractConflict {
                first, conflicting, ..
            }
            | Self::DuplicateProjectContract {
                first, conflicting, ..
            } => Some((first, conflicting)),
            Self::ProjectSourceContractConflict {
                project, source, ..
            }
            | Self::FixedProjectSourceTypeMismatch {
                fixed: project,
                source,
                ..
            } => Some((project, source)),
            Self::DefaultTypeMismatch { .. }
            | Self::ValueTypeMismatch { .. }
            | Self::MissingRequiredValue { .. }
            | Self::DirectProjectProviderMissing { .. }
            | Self::UnknownExplicitInput { .. } => None,
        }
    }

    /// Return the authored fact associated with a type or missing-value failure.
    pub(crate) fn contract_fact(&self) -> Option<&BuildConfigContractFact> {
        match self {
            Self::ProjectSourceContractConflict { source, .. }
            | Self::FixedProjectSourceTypeMismatch { source, .. }
            | Self::DefaultTypeMismatch {
                contract: source, ..
            }
            | Self::ValueTypeMismatch {
                contract: source, ..
            }
            | Self::MissingRequiredValue {
                contract: source, ..
            }
            | Self::DirectProjectProviderMissing {
                contract: source, ..
            } => Some(source),
            Self::SourceContractConflict { .. }
            | Self::DuplicateProjectContract { .. }
            | Self::UnknownExplicitInput { .. } => None,
        }
    }
}

/// One resolved build-config input retained by [`ResolvedBuildConfigMap`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedBuildConfigValue {
    name: BuildInputName,
    value_type: BuildInputType,
    required: bool,
    default: Option<PrimitiveBuildValue>,
    value: Option<PrimitiveBuildValue>,
    origin: BuildConfigValueOrigin,
    fingerprint: BuildConfigFingerprint,
    location: SourceLocation,
    value_location: Option<BuildConfigValueLocation>,
}

impl ResolvedBuildConfigValue {
    pub(crate) fn name(&self) -> &BuildInputName {
        &self.name
    }

    pub(crate) fn value_type(&self) -> BuildInputType {
        self.value_type
    }

    pub(crate) fn value(&self) -> Option<&PrimitiveBuildValue> {
        self.value.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn origin(&self) -> BuildConfigValueOrigin {
        self.origin
    }

    #[cfg(test)]
    pub(crate) fn fingerprint(&self) -> BuildConfigFingerprint {
        self.fingerprint
    }

    /// Location of the selected value, when that provider has a source or command location.
    #[cfg(test)]
    pub(crate) fn value_location(&self) -> Option<&BuildConfigValueLocation> {
        self.value_location.as_ref()
    }
}

/// The deterministic resolved values for one project or package configuration boundary.
///
/// The map contains every known source, fixed-project and direct-project contract name exactly
/// once. Its key order is lower_snake_case lexical order, independent of the order in which the
/// three fact slices were supplied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResolvedBuildConfigMap {
    entries: BTreeMap<BuildInputName, ResolvedBuildConfigValue>,
}

impl ResolvedBuildConfigMap {
    pub(crate) fn get(&self, name: &BuildInputName) -> Option<&ResolvedBuildConfigValue> {
        self.entries.get(name)
    }

    /// Iterate key/value pairs in deterministic name order.
    #[cfg(test)]
    pub(crate) fn iter(
        &self,
    ) -> impl Iterator<Item = (&BuildInputName, &ResolvedBuildConfigValue)> {
        self.entries.iter()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Borrowed indexes for one already-validated canonical configuration boundary.
///
/// WHAT: retains references to canonical source, fixed-project and direct-project facts without
/// copying any fact payload. A transient check-only unit can then validate and resolve only its
/// own source facts against this index.
/// WHY: check-only compilation must not clone the entire canonical contract vector for every
/// transient unit. The canonical resolver already validated these facts before the index is
/// built; only transient facts and any resolution output are owned per unit.
pub(crate) struct BuildConfigResolutionIndex<'a> {
    source_by_name: BTreeMap<&'a BuildInputName, &'a BuildConfigContractFact>,
    fixed_project_by_name: BTreeMap<&'a BuildInputName, &'a BuildConfigContractFact>,
    direct_project_by_name: BTreeMap<&'a BuildInputName, &'a BuildConfigContractFact>,
}

impl<'a> BuildConfigResolutionIndex<'a> {
    /// Index canonical facts after the owning boundary has resolved them successfully.
    ///
    /// Duplicate canonical facts are intentionally kept at their first occurrence here. The
    /// ordinary `resolve_build_config_values` path performs duplicate/conflict validation before
    /// this index is constructed, so rebuilding those owned errors would defeat the borrowed
    /// boundary and add no recovery path.
    pub(crate) fn from_validated(
        source_facts: &'a [BuildConfigContractFact],
        fixed_project_facts: &'a [BuildConfigContractFact],
        direct_project_facts: &'a [BuildConfigContractFact],
    ) -> Self {
        let mut source_by_name = BTreeMap::new();
        for fact in source_facts {
            source_by_name.entry(fact.name()).or_insert(fact);
        }
        let mut fixed_project_by_name = BTreeMap::new();
        for fact in fixed_project_facts {
            fixed_project_by_name.entry(fact.name()).or_insert(fact);
        }
        let mut direct_project_by_name = BTreeMap::new();
        for fact in direct_project_facts {
            direct_project_by_name.entry(fact.name()).or_insert(fact);
        }
        Self {
            source_by_name,
            fixed_project_by_name,
            direct_project_by_name,
        }
    }

    /// Keep only explicit inputs known to the canonical boundary or this transient unit.
    ///
    /// Fixed-only project fields remain absent by design: they are providers for matching source
    /// contracts, not standalone source names. The returned set owns only filtered input entries.
    pub(crate) fn filter_inputs_to_known_facts(
        &self,
        explicit_inputs: &BuildConfigInputSet,
        transient_source_facts: &[BuildConfigContractFact],
    ) -> BuildConfigInputSet {
        let transient_names = transient_source_facts
            .iter()
            .map(BuildConfigContractFact::name)
            .collect::<BTreeSet<_>>();
        let mut filtered = BuildConfigInputSet::new();
        for input in explicit_inputs.iter() {
            if self.source_by_name.contains_key(input.name())
                || self.direct_project_by_name.contains_key(input.name())
                || transient_names.contains(input.name())
            {
                filtered
                    .insert(input.clone())
                    .expect("filtered build-config inputs preserve unique names");
            }
        }
        filtered
    }

    /// Resolve one transient unit against borrowed canonical facts.
    ///
    /// Canonical facts are never cloned or concatenated with the transient slice. A transient
    /// duplicate must agree with its canonical namesake, but the canonical fact remains the
    /// provider of the resolved value; only names introduced by the transient unit are resolved
    /// from its private facts.
    pub(crate) fn resolve_with_transient_source_facts(
        &self,
        transient_source_facts: &[BuildConfigContractFact],
        explicit_inputs: &BuildConfigInputSet,
        builder_globals: &BuilderConfigGlobalSet,
    ) -> Result<ResolvedBuildConfigMap, BuildConfigResolutionError> {
        // Source-contract conflicts are checked over canonical-then-transient authored order before
        // defaults or project compatibility, matching the owning boundary resolver without
        // concatenating or cloning the canonical fact vector.
        let transient_by_name =
            collect_transient_source_facts(&self.source_by_name, transient_source_facts)?;
        validate_fact_defaults(transient_by_name.values().copied())?;
        for fact in transient_source_facts {
            if !self.source_by_name.contains_key(fact.name()) {
                if let Some(fixed) = self.fixed_project_by_name.get(fact.name())
                    && let Some(reason) = contract_conflict_reason(fixed, fact, false)
                {
                    return Err(BuildConfigResolutionError::FixedProjectSourceTypeMismatch {
                        fixed: (**fixed).clone(),
                        source: fact.clone(),
                        reason,
                    });
                }
                if let Some(project) = self.direct_project_by_name.get(fact.name())
                    && let Some(reason) = contract_conflict_reason(project, fact, true)
                {
                    return Err(BuildConfigResolutionError::ProjectSourceContractConflict {
                        project: (**project).clone(),
                        source: fact.clone(),
                        reason,
                    });
                }
            }
        }

        let mut resolved = BTreeMap::new();

        // Canonical source names retain canonical precedence over a matching transient
        // declaration, exactly as the former concatenated vector did.
        for (name, source) in &self.source_by_name {
            let value = resolve_one_build_config_value(
                name,
                self.fixed_project_by_name.get(name).copied(),
                self.direct_project_by_name.get(name).copied(),
                Some(*source),
                explicit_inputs,
                builder_globals,
            )?;
            resolved.insert((*name).clone(), value);
        }

        // Direct project contracts without a canonical source contract still belong to the
        // boundary map and must remain visible to the transient module.
        for (name, direct_project) in &self.direct_project_by_name {
            if self.source_by_name.contains_key(name) {
                continue;
            }
            let value = resolve_one_build_config_value(
                name,
                None,
                Some(*direct_project),
                None,
                explicit_inputs,
                builder_globals,
            )?;
            resolved.insert((*name).clone(), value);
        }

        // Finally resolve names introduced only by this transient source unit. Canonical names
        // and direct-project names above already own their result.
        for (name, source) in &transient_by_name {
            if self.source_by_name.contains_key(name)
                || self.direct_project_by_name.contains_key(name)
            {
                continue;
            }
            let value = resolve_one_build_config_value(
                name,
                self.fixed_project_by_name.get(name).copied(),
                None,
                Some(*source),
                explicit_inputs,
                builder_globals,
            )?;
            resolved.insert((*name).clone(), value);
        }

        for input in explicit_inputs.iter() {
            if !resolved.contains_key(input.name()) {
                return Err(BuildConfigResolutionError::UnknownExplicitInput {
                    input: input.clone(),
                });
            }
        }

        Ok(ResolvedBuildConfigMap { entries: resolved })
    }
}

/// Collect transient-only source facts after comparing them with the borrowed canonical index.
///
/// Canonical facts logically precede transient facts. This preserves the full resolver's
/// first-conflict ordering while retaining only facts introduced by this transient unit.
fn collect_transient_source_facts<'a>(
    canonical_by_name: &BTreeMap<&BuildInputName, &BuildConfigContractFact>,
    facts: &'a [BuildConfigContractFact],
) -> Result<BTreeMap<&'a BuildInputName, &'a BuildConfigContractFact>, BuildConfigResolutionError> {
    let mut transient_by_name: BTreeMap<&BuildInputName, &BuildConfigContractFact> =
        BTreeMap::new();
    for fact in facts {
        let first = canonical_by_name
            .get(fact.name())
            .copied()
            .or_else(|| transient_by_name.get(fact.name()).copied());
        if let Some(first) = first
            && let Some(reason) = contract_conflict_reason(first, fact, true)
        {
            return Err(BuildConfigResolutionError::SourceContractConflict {
                first: first.clone(),
                conflicting: fact.clone(),
                reason,
            });
        }
        if !canonical_by_name.contains_key(fact.name()) {
            transient_by_name.entry(fact.name()).or_insert(fact);
        }
    }
    Ok(transient_by_name)
}

/// Resolve all known build-config contracts for one project or package boundary.
///
/// Fact slices are already selected by the build-system barrier. Source facts are checked in the
/// supplied source order so the first conflicting declaration is stable; the resulting value map
/// is always name ordered. Fixed project fields are authoritative, direct project contracts are
/// resolved before source-only contracts, and unknown explicit inputs are checked only after all
/// known facts have been validated and resolved.
pub(crate) fn resolve_build_config_values(
    source_facts: &[BuildConfigContractFact],
    fixed_project_facts: &[BuildConfigContractFact],
    direct_project_facts: &[BuildConfigContractFact],
    explicit_inputs: &BuildConfigInputSet,
    builder_globals: &BuilderConfigGlobalSet,
) -> Result<ResolvedBuildConfigMap, BuildConfigResolutionError> {
    let source_by_name = collect_source_facts(source_facts)?;
    let fixed_project_by_name = collect_project_facts(fixed_project_facts)?;
    let direct_project_by_name = collect_project_facts(direct_project_facts)?;

    validate_fact_defaults(source_by_name.values())?;
    validate_fact_defaults(fixed_project_by_name.values())?;
    validate_fact_defaults(direct_project_by_name.values())?;
    validate_project_source_compatibility(
        source_facts,
        &source_by_name,
        &fixed_project_by_name,
        &direct_project_by_name,
    )?;

    // Fixed fields are providers, not build-config contracts. They are consulted only when a
    // selected source contract has the same name; a fixed-only name must not make an explicit
    // input known or appear in the resolved source namespace.
    let mut names = BTreeSet::new();
    names.extend(source_by_name.keys().cloned());
    names.extend(direct_project_by_name.keys().cloned());

    let mut resolved = BTreeMap::new();
    for name in names {
        let value = resolve_one_build_config_value(
            &name,
            source_by_name
                .get(&name)
                .and_then(|_| fixed_project_by_name.get(&name)),
            direct_project_by_name.get(&name),
            source_by_name.get(&name),
            explicit_inputs,
            builder_globals,
        )?;
        resolved.insert(name, value);
    }

    // This intentionally runs after fact validation and value resolution. In particular, a
    // check-only source fact that arrives late in collection still makes its name known before an
    // explicit input is classified as unknown.
    for input in explicit_inputs.iter() {
        if !resolved.contains_key(input.name()) {
            return Err(BuildConfigResolutionError::UnknownExplicitInput {
                input: input.clone(),
            });
        }
    }

    Ok(ResolvedBuildConfigMap { entries: resolved })
}

fn collect_source_facts(
    facts: &[BuildConfigContractFact],
) -> Result<BTreeMap<BuildInputName, BuildConfigContractFact>, BuildConfigResolutionError> {
    let mut by_name = BTreeMap::new();
    for fact in facts {
        if let Some(first) = by_name.get(fact.name()) {
            let reason = contract_conflict_reason(first, fact, true);
            if let Some(reason) = reason {
                return Err(BuildConfigResolutionError::SourceContractConflict {
                    first: first.clone(),
                    conflicting: fact.clone(),
                    reason,
                });
            }
        } else {
            by_name.insert(fact.name().clone(), fact.clone());
        }
    }
    Ok(by_name)
}

fn collect_project_facts(
    facts: &[BuildConfigContractFact],
) -> Result<BTreeMap<BuildInputName, BuildConfigContractFact>, BuildConfigResolutionError> {
    let mut by_name = BTreeMap::new();
    for fact in facts {
        if let Some(first) = by_name.get(fact.name()) {
            let reason = contract_conflict_reason(first, fact, true).unwrap_or(
                BuildConfigContractConflictReason::Default {
                    first: first.default.clone(),
                    conflicting: fact.default.clone(),
                },
            );
            return Err(BuildConfigResolutionError::DuplicateProjectContract {
                first: first.clone(),
                conflicting: fact.clone(),
                reason,
            });
        }
        by_name.insert(fact.name().clone(), fact.clone());
    }
    Ok(by_name)
}

fn validate_fact_defaults<'a>(
    facts: impl Iterator<Item = &'a BuildConfigContractFact>,
) -> Result<(), BuildConfigResolutionError> {
    for fact in facts {
        if let Some(default) = fact.default_value()
            && !fact
                .value_type()
                .accepts_primitive(default.primitive_type())
        {
            return Err(BuildConfigResolutionError::DefaultTypeMismatch {
                contract: fact.clone(),
                provided: default.primitive_type(),
            });
        }
    }
    Ok(())
}

fn validate_project_source_compatibility(
    source_facts: &[BuildConfigContractFact],
    source_by_name: &BTreeMap<BuildInputName, BuildConfigContractFact>,
    fixed_project_by_name: &BTreeMap<BuildInputName, BuildConfigContractFact>,
    direct_project_by_name: &BTreeMap<BuildInputName, BuildConfigContractFact>,
) -> Result<(), BuildConfigResolutionError> {
    // The maps provide the first deduplicated fact for each name, while this set preserves the
    // collected source order that determines which project/source conflict is reported first.
    let mut seen_names = BTreeSet::new();
    for fact in source_facts {
        if !seen_names.insert(fact.name().clone()) {
            continue;
        }
        let name = fact.name();
        let source = source_by_name
            .get(name)
            .expect("collected source facts must be present in the deduplicated map");
        if let Some(fixed) = fixed_project_by_name.get(name)
            && let Some(reason) = contract_conflict_reason(fixed, source, false)
        {
            return Err(BuildConfigResolutionError::FixedProjectSourceTypeMismatch {
                fixed: fixed.clone(),
                source: source.clone(),
                reason,
            });
        }

        if let Some(project) = direct_project_by_name.get(name)
            && let Some(reason) = contract_conflict_reason(project, source, true)
        {
            return Err(BuildConfigResolutionError::ProjectSourceContractConflict {
                project: project.clone(),
                source: source.clone(),
                reason,
            });
        }
    }
    Ok(())
}

fn contract_conflict_reason(
    first: &BuildConfigContractFact,
    conflicting: &BuildConfigContractFact,
    compare_required_and_default: bool,
) -> Option<BuildConfigContractConflictReason> {
    if first.value_type().primitive() != conflicting.value_type().primitive() {
        return Some(BuildConfigContractConflictReason::PrimitiveType {
            first: first.value_type().primitive(),
            conflicting: conflicting.value_type().primitive(),
        });
    }

    if first.value_type().is_optional() != conflicting.value_type().is_optional() {
        return Some(BuildConfigContractConflictReason::Optionality {
            first: first.value_type().is_optional(),
            conflicting: conflicting.value_type().is_optional(),
        });
    }

    if compare_required_and_default && first.required() != conflicting.required() {
        return Some(BuildConfigContractConflictReason::Required {
            first: first.required(),
            conflicting: conflicting.required(),
        });
    }

    if compare_required_and_default && first.default != conflicting.default {
        return Some(BuildConfigContractConflictReason::Default {
            first: first.default.clone(),
            conflicting: conflicting.default.clone(),
        });
    }

    None
}

fn resolve_one_build_config_value(
    name: &BuildInputName,
    fixed_project: Option<&BuildConfigContractFact>,
    direct_project: Option<&BuildConfigContractFact>,
    source: Option<&BuildConfigContractFact>,
    explicit_inputs: &BuildConfigInputSet,
    builder_globals: &BuilderConfigGlobalSet,
) -> Result<ResolvedBuildConfigValue, BuildConfigResolutionError> {
    if let (Some(fixed_project), Some(source)) = (fixed_project, source) {
        return resolved_fixed_project_value(source, fixed_project);
    }

    if let Some(direct_project) = direct_project {
        let provider = direct_project.resolved_provider().ok_or_else(|| {
            BuildConfigResolutionError::DirectProjectProviderMissing {
                contract: direct_project.clone(),
            }
        })?;
        return Ok(resolved_value_from_provider(direct_project, provider));
    }

    let contract = source.ok_or_else(|| {
        // `name` only comes from a source fact at this point. This branch is an internal invariant,
        // not user input; preserving a typed error is preferable to panicking if map assembly
        // changes that invariant.
        BuildConfigResolutionError::MissingRequiredValue {
            contract: BuildConfigContractFact::new(
                name.clone(),
                BuildInputType::Primitive(PrimitiveBuildInputType::String),
                true,
                None,
                SourceLocation::default(),
            ),
        }
    })?;

    let (value, origin, value_location) = if let Some(input) = explicit_inputs.get(name) {
        let value_type = input.value().primitive_type();
        if !contract.value_type().accepts_primitive(value_type) {
            return Err(BuildConfigResolutionError::ValueTypeMismatch {
                contract: contract.clone(),
                provided: value_type,
                value_location: Some(input.location().clone()),
            });
        }
        (
            Some(input.value().clone()),
            BuildConfigValueOrigin::ExplicitInput,
            Some(input.location().clone()),
        )
    } else if let Some(value) = builder_globals.get(name) {
        let value_type = value.primitive_type();
        if !contract.value_type().accepts_primitive(value_type) {
            return Err(BuildConfigResolutionError::ValueTypeMismatch {
                contract: contract.clone(),
                provided: value_type,
                value_location: None,
            });
        }
        (
            Some(value.clone()),
            BuildConfigValueOrigin::BuilderGlobal,
            None,
        )
    } else if let Some(value) = contract.default_value() {
        (
            Some(value.clone()),
            BuildConfigValueOrigin::DeclarationDefault,
            Some(BuildConfigValueLocation::Source(
                contract.location().clone(),
            )),
        )
    } else if contract.required() {
        return Err(BuildConfigResolutionError::MissingRequiredValue {
            contract: contract.clone(),
        });
    } else {
        (None, BuildConfigValueOrigin::DeclarationDefault, None)
    };

    Ok(resolved_value_from_contract(
        contract,
        value,
        origin,
        value_location,
    ))
}

fn resolved_fixed_project_value(
    source: &BuildConfigContractFact,
    fixed_project: &BuildConfigContractFact,
) -> Result<ResolvedBuildConfigValue, BuildConfigResolutionError> {
    Ok(resolved_value_from_contract(
        source,
        fixed_project.default.clone(),
        BuildConfigValueOrigin::FixedProjectField,
        Some(BuildConfigValueLocation::Source(
            fixed_project.location().clone(),
        )),
    ))
}
fn resolved_value_from_provider(
    contract: &BuildConfigContractFact,
    provider: &ResolvedBuildConfigProvider,
) -> ResolvedBuildConfigValue {
    ResolvedBuildConfigValue {
        name: contract.name().clone(),
        value_type: contract.value_type(),
        required: contract.required(),
        default: contract.default.clone(),
        value: provider.value.clone(),
        origin: provider.origin,
        fingerprint: provider.fingerprint,
        location: contract.location().clone(),
        value_location: provider.value_location.clone(),
    }
}

fn resolved_value_from_contract(
    contract: &BuildConfigContractFact,
    value: Option<PrimitiveBuildValue>,
    origin: BuildConfigValueOrigin,
    value_location: Option<BuildConfigValueLocation>,
) -> ResolvedBuildConfigValue {
    let fingerprint = build_config_fingerprint(
        contract.name().as_str(),
        contract.value_type(),
        value.as_ref(),
    );

    ResolvedBuildConfigValue {
        name: contract.name().clone(),
        value_type: contract.value_type(),
        required: contract.required(),
        default: contract.default.clone(),
        value,
        origin,
        fingerprint,
        location: contract.location().clone(),
        value_location,
    }
}
