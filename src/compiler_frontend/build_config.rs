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
//! Command-input value classification lives here so build, check and dev share one immediate
//! path. [`BuildConfigInputValue::from_command_text`] classifies the authored text from the Moth
//! literal grammar alone: it never consults a project or source contract and never materialises
//! a number. A numeric literal stays deferred until the contract that receives it is known.
//!
//! [`BuildConfigInputValue::materialise_for`] is the one acceptance rule for supplied values. A
//! deferred literal materialises directly at its contract's exact scalar through the shared
//! `moth_lexical` numeric helpers, so no intermediate width or precision exists. Typed values must
//! match the contract's exact primitive type.
//!
//! Exclusions: no `--input` argument parser or `#Config` grammar lives here. Command layers
//! split each argument at the first `=`, validate names and insert the resulting entries.
//! Optional absence is represented by omitting an input, never by a value in this vocabulary.
//! Resolution records that keep a typed value together with its origin arrive with the
//! resolution phases that produce them.
//!

use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId, SourceSpan};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identifier_policy::is_lowercase_with_underscores_name;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::{TokenizeFailure, tokenize};
use crate::compiler_frontend::tokenizer::tokens::{TokenIndex, TokenTag, TokenizerEntryMode};
use moth_lexical::identifier::is_identifier;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::grammar::NumericLiteralKind;
use moth_lexical::numeric::parse::{
    NumberLiteralErrorReason, literal_kind_initialises, materialize_normalized_fixed_scalar,
    parse_numeric_literal,
};

use crate::builder_surface::config_schema::ProjectFieldConfigPolicy;
use rustc_hash::FxHashMap;
use std::borrow::Borrow;
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
pub struct BuildConfigInputSet {
    entries: BTreeMap<BuildInputName, BuildConfigInputEntry>,
}

impl BuildConfigInputSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert one typed input, rejecting a second entry for the same name.
    ///
    /// On rejection the earlier entry keeps its place and the error carries the rejected entry
    /// back with the earlier location, so command diagnostics can underline both arguments.
    pub fn insert(
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

    pub fn get(&self, name: &BuildInputName) -> Option<&BuildConfigInputEntry> {
        self.entries.get(name)
    }

    /// Iterate every entry in deterministic name order.
    pub fn iter(&self) -> impl Iterator<Item = &BuildConfigInputEntry> {
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
pub struct BuildConfigInputDuplicate {
    rejected: BuildConfigInputEntry,
    existing_location: BuildConfigValueLocation,
}

impl BuildConfigInputDuplicate {
    pub fn rejected(&self) -> &BuildConfigInputEntry {
        &self.rejected
    }

    pub fn existing_location(&self) -> &BuildConfigValueLocation {
        &self.existing_location
    }
}

/// One explicit build-config input: a supplied value plus where the caller supplied it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildConfigInputEntry {
    name: BuildInputName,
    value: BuildConfigInputValue,
    location: BuildConfigValueLocation,
}

impl BuildConfigInputEntry {
    pub fn new(
        name: BuildInputName,
        value: impl Into<BuildConfigInputValue>,
        location: BuildConfigValueLocation,
    ) -> Self {
        Self {
            name,
            value: value.into(),
            location,
        }
    }

    pub fn name(&self) -> &BuildInputName {
        &self.name
    }

    pub fn value(&self) -> &BuildConfigInputValue {
        &self.value
    }

    pub fn location(&self) -> &BuildConfigValueLocation {
        &self.location
    }
}

/// One explicit input value before its receiving contract is known.
///
/// WHAT: a typed value when the authored text already fixes its type (`Bool`, `String` or `Char`),
///       or a deferred numeric literal whose exact scalar is chosen only by the contract it reaches.
/// WHY:  a command number has no intrinsic width or precision. Choosing one first would reject
///       `4294967296` for an unsigned contract or round a decimal for a float contract, so the
///       literal waits until its contract is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildConfigInputValue {
    Typed(PrimitiveBuildValue),
    Deferred(DeferredNumericLiteral),
}

impl From<PrimitiveBuildValue> for BuildConfigInputValue {
    fn from(value: PrimitiveBuildValue) -> Self {
        Self::Typed(value)
    }
}

/// One authored numeric literal that materialises only at its contract's exact scalar.
///
/// WHAT: the authored text for diagnostics, the normalized digits without separators, the authored
///       leading sign and the literal kind that decides which scalar families it can initialise.
/// WHY:  materialisation reuses the shared `moth_lexical` helpers, which take exactly these parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeferredNumericLiteral {
    authored_text: String,
    normalized: String,
    negative: bool,
    kind: NumericLiteralKind,
}

impl BuildConfigInputValue {
    /// Classify one command-input value from its authored text without materialising a number.
    ///
    /// WHAT: exact lowercase `true` and `false` infer Bool. An optional leading `-` followed by a
    ///       complete numeric literal defers the number with its literal kind. A single-quote- or
    ///       double-quote-leading value must be one complete ordinary Moth Char or String literal.
    ///       Every other value, including the empty value, `+1`, `NaN`, backtick text and malformed
    ///       numeric-looking text, falls back to String with its exact authored text preserved.
    /// WHY:  the primitive shape of a command value is decided from the authored text alone, so no
    ///       project or source contract is consulted and no second literal grammar exists.
    pub fn from_command_text(value: &str) -> Result<Self, BuildInputValueError> {
        if value == "true" {
            return Ok(Self::Typed(PrimitiveBuildValue::Bool(true)));
        }
        if value == "false" {
            return Ok(Self::Typed(PrimitiveBuildValue::Bool(false)));
        }

        let (negative, unsigned) = match value.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, value),
        };
        if unsigned.starts_with(|character: char| character.is_ascii_digit())
            && let Ok(literal) = parse_numeric_literal(unsigned)
        {
            return Ok(Self::Deferred(DeferredNumericLiteral {
                authored_text: value.to_owned(),
                normalized: literal.normalized_text,
                negative,
                kind: literal.kind,
            }));
        }

        if value.starts_with('\'') || value.starts_with('"') {
            let literal = parse_ordinary_quoted_literal(value)?;

            return Ok(Self::Typed(match literal {
                OrdinaryCommandLiteral::String(text) => PrimitiveBuildValue::String(text),
                OrdinaryCommandLiteral::Char(character) => PrimitiveBuildValue::Char(character),
            }));
        }

        Ok(Self::Typed(PrimitiveBuildValue::String(value.to_owned())))
    }

    /// Materialise this supplied value at the contract it initialises.
    ///
    /// WHAT: a typed value must match the contract's exact primitive type. A deferred literal becomes
    ///       the contract's exact scalar when its kind initialises that scalar and its text fits the
    ///       scalar's range.
    /// WHY:  the contract is the only thing that chooses a numeric scalar, so every supplied value
    ///       reaches the scalar it is stored and fingerprinted at without an intermediate width.
    pub(crate) fn materialise_for(
        &self,
        contract: BuildInputType,
    ) -> Result<PrimitiveBuildValue, ContractValueError> {
        match self {
            Self::Typed(value) => value.accept_for(contract),
            Self::Deferred(literal) => literal.materialise_for(contract),
        }
    }
}

impl DeferredNumericLiteral {
    /// The scalar this literal's kind takes when no contract selects one.
    ///
    /// WHAT: whole numbers name `I32`, and decimal-point or exponent literals name `F64`.
    /// WHY:  this names the literal's type in a mismatch diagnostic only. It never materialises.
    fn default_scalar(&self) -> FixedScalar {
        match self.kind {
            NumericLiteralKind::WholeNumber => FixedScalar::I32,
            NumericLiteralKind::DecimalPoint | NumericLiteralKind::Exponent => FixedScalar::F64,
        }
    }

    /// Build a deferred literal from its parsed parts.
    ///
    /// WHAT: the authored text, normalized digits, authored sign and literal kind as the lexer
    ///       reports them.
    /// WHY:  the command and header paths both hold these parts, and materialisation needs
    ///       nothing else.
    pub(crate) fn new(
        authored_text: String,
        normalized: String,
        negative: bool,
        kind: NumericLiteralKind,
    ) -> Self {
        Self {
            authored_text,
            normalized,
            negative,
            kind,
        }
    }

    /// Materialise this literal at the exact scalar of a fixed-scalar contract.
    ///
    /// WHAT: the literal kind must be able to initialise the contract's scalar, and the digits
    ///       must fit its range. Any other contract is a type mismatch.
    /// WHY:  a command number has no intrinsic width, so the receiving contract is the only
    ///       thing that chooses its scalar.
    pub(crate) fn materialise_for(
        &self,
        contract: BuildInputType,
    ) -> Result<PrimitiveBuildValue, ContractValueError> {
        let mismatch = || ContractValueError::TypeMismatch {
            provided: PrimitiveBuildInputType::Fixed(self.default_scalar()),
        };
        let PrimitiveBuildInputType::Fixed(scalar) = contract.primitive() else {
            return Err(mismatch());
        };
        if !literal_kind_initialises(self.kind, scalar) {
            return Err(mismatch());
        }

        materialize_normalized_fixed_scalar(&self.normalized, self.negative, scalar)
            .map(PrimitiveBuildValue::Fixed)
            .map_err(|reason| ContractValueError::LiteralOutOfRange {
                text: self.authored_text.clone(),
                reason,
            })
    }
}

/// One exact typed build-config value.
///
/// WHAT: the only resolved value vocabulary build inputs and `#Config` resolution transport. Every
///       numeric value is one exact fixed scalar, never an untyped or widened carrier.
/// WHY:  there is deliberately no raw, untyped or `none` variant. Optional absence is an omitted
///       input resolved through project and default rules, never a value. Authored String and Char
///       content is preserved exactly, without backend conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrimitiveBuildValue {
    String(String),
    Fixed(FixedScalarValue),
    Bool(bool),
    Char(char),
}

impl PrimitiveBuildValue {
    /// The exact primitive type this value carries.
    pub(crate) fn primitive_type(&self) -> PrimitiveBuildInputType {
        match self {
            Self::String(_) => PrimitiveBuildInputType::String,
            Self::Fixed(value) => PrimitiveBuildInputType::Fixed(value.scalar()),
            Self::Bool(_) => PrimitiveBuildInputType::Bool,
            Self::Char(_) => PrimitiveBuildInputType::Char,
        }
    }

    /// Accept this typed value at one contract, or report the exact type that does not fit.
    ///
    /// WHAT: the contract's own compatibility rule, so an optional contract accepts its primitive
    ///       as a present value.
    /// WHY:  a typed value is never converted, so acceptance is only an exact type check.
    pub(crate) fn accept_for(&self, contract: BuildInputType) -> Result<Self, ContractValueError> {
        let provided = self.primitive_type();
        if contract.accepts_primitive(provided) {
            Ok(self.clone())
        } else {
            Err(ContractValueError::TypeMismatch { provided })
        }
    }
}

/// Why a command-input value was rejected instead of classified.
///
/// WHAT: one rejection vocabulary for immediate command-input classification. A quote-leading
///       value that is not one complete ordinary Moth literal is rejected with the ordinary
///       tokenizer's stable diagnostic title. Infrastructure failures retain their separate
///       carrier until the command boundary renders them. Numeric range is decided later, when the
///       receiving contract is known.
/// WHY:  every command variant renders the same concrete rejection without a second wording
///       owner, and the compiler owns the inference semantics behind the message.
#[derive(Debug)]
pub enum BuildInputValueError {
    Infrastructure(Box<InfrastructureFailure>),
    /// A single-quote-leading value is not one complete ordinary Moth Char literal.
    MalformedCharLiteral {
        text: String,
        reason: &'static str,
    },
    /// A double-quote-leading value is not one complete ordinary Moth String literal.
    MalformedStringLiteral {
        text: String,
        reason: &'static str,
    },
}

impl fmt::Display for BuildInputValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Infrastructure(error) => {
                write!(formatter, "input tokenization failed: {}", error.message)
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

/// Reason text for a value that tokenizes but carries text past its complete literal.
const QUOTED_LITERAL_TRAILING_TEXT: &str = "text follows the literal";

/// The logical path and source identity supplied to the ordinary lexer for standalone command
/// values.
///
/// Command-input text belongs to the whole compilation rather than any file, so its token stream
/// carries [`SourceId::COMPILATION_ROOT`]. The path remains only a lexer scope for well-formed
/// diagnostics; command diagnostics use their stable titles rather than that location.
const COMMAND_INPUT_TOKENIZER_PATH: &str = "command-input";

/// Map a quoted-literal rejection to the typed error for the authored leading quote.
fn malformed_quoted_literal_error(value: &str, reason: &'static str) -> BuildInputValueError {
    if value.starts_with('\'') {
        BuildInputValueError::MalformedCharLiteral {
            text: value.to_owned(),
            reason,
        }
    } else {
        BuildInputValueError::MalformedStringLiteral {
            text: value.to_owned(),
            reason,
        }
    }
}

/// Parse one quote-leading command value through the ordinary Moth literal grammar.
///
/// WHAT: tokenizes the value with the ordinary file lexer and accepts only the exact
///       `ModuleStart + one matching StringSliceLiteral/CharLiteral + Eof` shape. The literal
///       must begin at the first value character and end at the tokenizer's final Eof position.
///       This deliberately rejects all terminal trivia, including comments and whitespace.
///       The command-value token stream is synthetic: its span rows land in a temporary
///       builder that is discarded with the tokens, and command diagnostics report their
///       stable titles rather than any source location.
/// WHY:  command inputs must reuse the one ordinary literal grammar — delimiter, escape and
///       rejection policy stay owned by the tokenizer instead of a second command parser. The
///       strict terminal-trivia policy prevents source-file comment handling from silently
///       discarding an authored command-value suffix.
fn parse_ordinary_quoted_literal(
    value: &str,
) -> Result<OrdinaryCommandLiteral, BuildInputValueError> {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let path = path_fork
        .try_intern_portable_path(COMMAND_INPUT_TOKENIZER_PATH, &mut string_table)
        .map_err(|error| {
            BuildInputValueError::Infrastructure(Box::new(match error {
                crate::compiler_frontend::symbols::path_interner::PathInternError::TableFull => {
                    InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly,
                        "path table exhausted while interning a command-input tokenizer path",
                    )
                }
                crate::compiler_frontend::symbols::path_interner::PathInternError::NonUtf8(_) => {
                    InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly,
                        "command-input tokenizer path should be valid UTF-8",
                    )
                }
                crate::compiler_frontend::symbols::path_interner::PathInternError::BaseMismatch {
                    ..
                } => InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly,
                    "logical path merge base is not a structural prefix of the destination table",
                ),
            }))
        })?;
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        value,
        path,
        TokenizerEntryMode::SourceFile,
        &StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .map_err(|failure| match failure {
        TokenizeFailure::Diagnosed(diagnostic) => {
            malformed_quoted_literal_error(value, diagnostic.descriptor().title)
        }
        TokenizeFailure::Infrastructure(error) => {
            BuildInputValueError::Infrastructure(Box::new(error))
        }
    })?;

    let source_tokens = lexed.tokens;
    if source_tokens.len() != 3 {
        return Err(malformed_quoted_literal_error(
            value,
            QUOTED_LITERAL_TRAILING_TEXT,
        ));
    }
    let token_at = |position: usize| {
        source_tokens
            .token(TokenIndex::try_from_index(position).expect("three-token literal fits"))
            .expect("validated lexer output has three token rows")
    };
    let module_start = token_at(0);
    let literal = token_at(1);
    let eof = token_at(2);

    let resolver = span_builder.resolver();
    let literal_range = literal.span().resolve_with(resolver);
    let eof_range = eof.span().resolve_with(resolver);
    if module_start.tag() != TokenTag::MODULE_START
        || eof.tag() != TokenTag::EOF
        || literal_range.start() != 0
        || literal_range.end() != eof_range.end()
    {
        return Err(malformed_quoted_literal_error(
            value,
            QUOTED_LITERAL_TRAILING_TEXT,
        ));
    }

    match literal.tag() {
        TokenTag::STRING_SLICE_LITERAL => {
            let id = literal.string_id().ok_or_else(|| {
                BuildInputValueError::Infrastructure(Box::new(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    "quoted command literal has a malformed string payload",
                )))
            })?;
            Ok(OrdinaryCommandLiteral::String(
                string_table.resolve(id).to_owned(),
            ))
        }
        TokenTag::CHAR_LITERAL => {
            let character = literal.char_value().ok_or_else(|| {
                BuildInputValueError::Infrastructure(Box::new(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    "quoted command literal has a malformed character payload",
                )))
            })?;
            Ok(OrdinaryCommandLiteral::Char(character))
        }
        _ => Err(malformed_quoted_literal_error(
            value,
            QUOTED_LITERAL_TRAILING_TEXT,
        )),
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
    /// value satisfies the matching optional as a present value. No other coercion exists: a
    /// numeric literal materialises only at the contract's exact scalar, and no primitive
    /// satisfies `String` implicitly.
    pub(crate) fn accepts_primitive(&self, primitive: PrimitiveBuildInputType) -> bool {
        self.primitive() == primitive
    }
}

/// The primitive types a build-config contract accepts.
///
/// WHAT: `Fixed` carries one exact fixed scalar, so `I32`, `U8`, `F64` and the other admitted
///       scalars are distinct contract types that never share a width or precision.
/// WHY:  a contract's numeric identity is its scalar, and a supplied numeric literal materialises at
///       exactly that scalar rather than at a width chosen elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PrimitiveBuildInputType {
    String,
    Fixed(FixedScalar),
    Bool,
    Char,
}

impl PrimitiveBuildInputType {
    /// The exact Moth type spelling used by diagnostics and contract reporting.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::String => "String",
            Self::Fixed(scalar) => scalar.name(),
            Self::Bool => "Bool",
            Self::Char => "Char",
        }
    }

    /// The build-input primitive for one fixed scalar, or `None` when the contract excludes it.
    ///
    /// WHAT: every signed and unsigned integer width and every binary float width is admitted.
    ///       The `Byte` octet is not a numeric build-input type.
    /// WHY: build inputs accept the exact fixed-width numeric domain, and each admitted scalar
    ///      keeps its own identity rather than widening to one shared carrier.
    pub(crate) fn from_fixed_scalar(scalar: FixedScalar) -> Option<Self> {
        match scalar.class() {
            FixedScalarClass::SignedInteger
            | FixedScalarClass::UnsignedInteger
            | FixedScalarClass::BinaryFloat => Some(Self::Fixed(scalar)),
            FixedScalarClass::Octet => None,
        }
    }
}

/// Why a build-config name was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildInputNameError {
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
pub struct BuildInputName {
    text: String,
}

impl BuildInputName {
    /// Validate and own one lower_snake_case build-config name.
    ///
    /// A digit-leading spelling such as `2fast` is rejected because it can never match a
    /// source declaration identifier; keyword-shadow reservation is owned separately by the
    /// canonical keyword policy.
    pub fn new(text: &str) -> Result<Self, BuildInputNameError> {
        if is_identifier(text) && is_lowercase_with_underscores_name(text) {
            Ok(Self {
                text: text.to_owned(),
            })
        } else {
            Err(BuildInputNameError::NotLowerSnakeCase)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl Borrow<str> for BuildInputName {
    fn borrow(&self) -> &str {
        self.text.as_str()
    }
}

/// WHAT: one carrier for source-span and command-input provenance. Retained compiler source spans
///       use the shared [`SourceSpan`]; command and programmatic inputs carry their argument
///       position instead because they have no source span.
/// WHY:  mismatch, duplicate and missing-input diagnostics must underline either the source
///       declaration or the exact command argument without a second location model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BuildConfigValueLocation {
    /// A retained compiler source span: a contract declaration, its default literal or a
    /// grouped-project field initializer.
    Source(SourceSpan),

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
pub struct BuildCommandLocation {
    argument_index: usize,
}

impl BuildCommandLocation {
    pub fn new(argument_index: usize) -> Self {
        Self { argument_index }
    }

    pub fn argument_index(&self) -> usize {
        self.argument_index
    }
}

/// How a resolved build-config value was produced.
///
/// WHAT: the resolution-origin provenance later `#Config` resolution records next to a typed
///       value and its location, covering both accepted resolution orders: fixed grouped-project
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

    /// A fixed grouped-project field, which is authoritative for its name and blocks overrides.
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
    match contract.primitive() {
        PrimitiveBuildInputType::String => fingerprint_feed(&mut hash, &[0]),
        PrimitiveBuildInputType::Fixed(scalar) => {
            fingerprint_feed(&mut hash, &[1]);
            fingerprint_feed_string(&mut hash, scalar.name());
        }
        PrimitiveBuildInputType::Bool => fingerprint_feed(&mut hash, &[2]),
        PrimitiveBuildInputType::Char => fingerprint_feed(&mut hash, &[3]),
    }

    // Exact scalar identity and the canonical value bits are the whole numeric meaning of a value.
    // Binary-float bits keep the sign of zero, so `-0.0` and `0.0` remain distinct authored values.
    match value {
        None => fingerprint_feed(&mut hash, &[0]),
        Some(PrimitiveBuildValue::String(value)) => {
            fingerprint_feed(&mut hash, &[1]);
            fingerprint_feed_string(&mut hash, value);
        }
        Some(PrimitiveBuildValue::Fixed(value)) => {
            fingerprint_feed(&mut hash, &[2]);
            fingerprint_feed_string(&mut hash, value.scalar().name());
            if let Some(value) = value.as_i64() {
                fingerprint_feed(&mut hash, &value.to_le_bytes());
            } else if let Some(value) = value.as_u64() {
                fingerprint_feed(&mut hash, &value.to_le_bytes());
            } else if let Some(value) = value.as_f64() {
                fingerprint_feed_u64(&mut hash, value.to_bits());
            }
        }
        Some(PrimitiveBuildValue::Bool(value)) => {
            fingerprint_feed(&mut hash, &[3, u8::from(*value)]);
        }
        Some(PrimitiveBuildValue::Char(value)) => {
            fingerprint_feed(&mut hash, &[4]);
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
/// Provenance retained for one declaration-owned bootstrap input.
///
/// The declaration name and any receiving project field remain separate identities. The input
/// contract is published privately; a folded project-field dependency carries the receiving field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigResolutionRecord {
    pub(crate) field_name: StringId,
    pub(crate) input_name: BuildInputName,
    pub(crate) contract: BuildInputType,
    /// Whether the authored contract had no satisfiable value by default.
    ///
    /// This is retained separately from the selected value: explicit inputs and builder globals
    /// replace the authored default, but boundary compatibility still compares the original
    /// required/default contract facts with source declarations.
    pub(crate) required: bool,
    /// The normalized authored default, if the contract declared one.
    pub(crate) default: Option<PrimitiveBuildValue>,
    pub(crate) value: Option<PrimitiveBuildValue>,
    pub(crate) origin: BuildConfigValueOrigin,
    pub(crate) fingerprint: BuildConfigFingerprint,
    pub(crate) qualifier_span: Option<SourceSpan>,
    pub(crate) value_location: Option<BuildConfigValueLocation>,
}

/// Folded value paired with the receiving project field at the compiler/build boundary.
///
/// The compiler service owns this join while field type identities, values and spans remain
/// available. Build validation receives only these owned records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FoldedConfigProjectFieldDependency {
    pub(crate) field_name: StringId,
    pub(crate) input_names: Vec<BuildInputName>,
    pub(crate) type_identity: CanonicalTypeIdentity,
    pub(crate) value: PublicFoldedValue,
    pub(crate) span: Option<SourceSpan>,
}

/// Compiler-owned inputs used while config constants are folded.
///
/// The service owns resolution; this carrier only snapshots typed inputs, builder globals and the
/// schema-derived project-field policy. It deliberately contains no build `Config`, target or
/// platform identity.
pub(crate) struct ConfigResolutionServices {
    explicit_inputs: BuildConfigInputSet,
    builder_globals: BuilderConfigGlobalSet,
    project_field_policies: crate::builder_surface::config_schema::ProjectFieldConfigPolicies,
    records: RefCell<Vec<ConfigResolutionRecord>>,
    input_provenances: RefCell<
        FxHashMap<
            BuildInputName,
            crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance,
        >,
    >,
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
            input_provenances: RefCell::new(FxHashMap::default()),
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

    pub(crate) fn record(&self, record: ConfigResolutionRecord) {
        self.records.borrow_mut().push(record);
    }

    pub(crate) fn take_records(&self) -> Vec<ConfigResolutionRecord> {
        std::mem::take(&mut *self.records.borrow_mut())
    }

    /// Borrow the bootstrap source-config resolution fact retained before the full value map exists.
    pub(crate) fn diagnostic_source_config_origin(
        &self,
        name: &str,
    ) -> Option<(BuildConfigValueOrigin, Option<SourceSpan>)> {
        self.records
            .borrow()
            .iter()
            .find(|record| record.input_name.as_str() == name)
            .map(|record| {
                let provider_span = match record.value_location.as_ref() {
                    Some(BuildConfigValueLocation::Source(span)) => Some(*span),
                    Some(BuildConfigValueLocation::Command(_)) | None => None,
                };
                (record.origin, provider_span.or(record.qualifier_span))
            })
    }

    /// Seed the stable provenance owned by one declaration-owned config input.
    ///
    /// The folded `ConstValue` metadata carries each project field's provenance through the
    /// ordinary fold, so this map is the only pre-fold dependency state the resolution
    /// service retains.
    pub(crate) fn record_input_provenance(
        &self,
        input_name: BuildInputName,
        provenance: crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance,
    ) {
        self.input_provenances
            .borrow_mut()
            .insert(input_name, provenance);
    }

    /// Recover every declaration-owned input represented in an aggregate folded provenance.
    ///
    /// Each retained input provenance is a subset of the folded field provenance when that input
    /// contributed to the value. Sort the result because the provenance map is hash-backed.
    pub(crate) fn input_names_for_provenance(
        &self,
        provenance: &crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance,
    ) -> Vec<BuildInputName> {
        let mut names = self
            .input_provenances
            .borrow()
            .iter()
            .filter(|(_, input_provenance)| input_provenance.is_subset_of(provenance))
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        names
    }
}

// ------------------------
//  Boundary Contract Resolution
// ------------------------

/// One normalized contract fact supplied to the project-wide build-config barrier.
///
/// Source shells, grouped-project fields and declaration-owned input contracts enter the barrier
/// through this one shape. The caller chooses whether a fact is a source contract, a fixed
/// project field or a declaration-owned input contract by placing it in the corresponding
/// resolver input slice; the fact itself never carries build-system or source-graph identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BuildConfigContractFact {
    name: BuildInputName,
    value_type: BuildInputType,
    required: bool,
    default: Option<PrimitiveBuildValue>,
    span: Option<SourceSpan>,
    resolved_provider: Option<ResolvedBuildConfigProvider>,
}

impl BuildConfigContractFact {
    /// Construct one already-normalized boundary fact.
    pub(crate) fn new(
        name: BuildInputName,
        value_type: BuildInputType,
        required: bool,
        default: Option<PrimitiveBuildValue>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self {
            name,
            value_type,
            required,
            default,
            span,
            resolved_provider: None,
        }
    }

    /// Attach the value already selected while a declaration-owned config contract folded.
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

    pub(crate) fn span(&self) -> Option<SourceSpan> {
        self.span
    }

    fn resolved_provider(&self) -> Option<&ResolvedBuildConfigProvider> {
        self.resolved_provider.as_ref()
    }
}

/// Value/provenance already selected by the config-folding owner.
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
        first: Box<BuildConfigContractFact>,
        conflicting: Box<BuildConfigContractFact>,
        reason: BuildConfigContractConflictReason,
    },
    DuplicateProjectContract {
        first: Box<BuildConfigContractFact>,
        conflicting: Box<BuildConfigContractFact>,
        reason: BuildConfigContractConflictReason,
    },
    ProjectSourceContractConflict {
        project: Box<BuildConfigContractFact>,
        source: Box<BuildConfigContractFact>,
        reason: BuildConfigContractConflictReason,
    },
    FixedProjectSourceTypeMismatch {
        fixed: Box<BuildConfigContractFact>,
        source: Box<BuildConfigContractFact>,
        reason: BuildConfigContractConflictReason,
    },
    DefaultTypeMismatch {
        contract: Box<BuildConfigContractFact>,
        provided: PrimitiveBuildInputType,
    },
    ValueTypeMismatch {
        contract: Box<BuildConfigContractFact>,
        provided: PrimitiveBuildInputType,
        value_location: Option<BuildConfigValueLocation>,
    },
    MissingRequiredValue {
        contract: Box<BuildConfigContractFact>,
    },
    /// A supplied numeric literal is outside the range of the exact scalar its contract selects.
    ///
    /// WHAT: the contract whose value was refused, the normalized literal text, the existing
    ///       number-literal reason code and where the supplied value came from.
    /// WHY:  the scalar's range is part of the contract, so a literal that does not fit reports the
    ///       existing number-literal reason, and a command value keeps its argument position.
    NumericLiteralOutOfRange {
        contract: Box<BuildConfigContractFact>,
        text: String,
        reason: NumberLiteralErrorReason,
        value_location: Option<BuildConfigValueLocation>,
    },
    /// A declaration-owned config contract reached the barrier without its folded provider payload.
    ///
    /// This is an internal handoff invariant: config selection belongs exclusively to the
    /// config-folding service and must never fall back to boundary resolution.
    DirectProjectProviderMissing {
        contract: Box<BuildConfigContractFact>,
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
            | Self::NumericLiteralOutOfRange {
                contract: source, ..
            }
            | Self::DirectProjectProviderMissing {
                contract: source, ..
            } => source.name(),
            Self::UnknownExplicitInput { input } => input.name(),
        }
    }

    /// Return the authored contract span when this error has one.
    #[cfg(test)]
    pub(crate) fn contract_span(&self) -> Option<SourceSpan> {
        match self {
            Self::SourceContractConflict { first, .. }
            | Self::DuplicateProjectContract { first, .. } => first.span(),
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
            | Self::NumericLiteralOutOfRange {
                contract: source, ..
            }
            | Self::DirectProjectProviderMissing {
                contract: source, ..
            } => source.span(),
            Self::UnknownExplicitInput { .. } => None,
        }
    }

    /// Return the command or source location of a supplied value, if one exists.
    pub(crate) fn value_location(&self) -> Option<&BuildConfigValueLocation> {
        match self {
            Self::ValueTypeMismatch { value_location, .. }
            | Self::NumericLiteralOutOfRange { value_location, .. } => value_location.as_ref(),
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
            } => Some((first.as_ref(), conflicting.as_ref())),
            Self::ProjectSourceContractConflict {
                project, source, ..
            }
            | Self::FixedProjectSourceTypeMismatch {
                fixed: project,
                source,
                ..
            } => Some((project.as_ref(), source.as_ref())),
            Self::DefaultTypeMismatch { .. }
            | Self::ValueTypeMismatch { .. }
            | Self::MissingRequiredValue { .. }
            | Self::NumericLiteralOutOfRange { .. }
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
            | Self::NumericLiteralOutOfRange {
                contract: source, ..
            }
            | Self::DirectProjectProviderMissing {
                contract: source, ..
            } => Some(source.as_ref()),
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
    span: Option<SourceSpan>,
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

    /// Origin and best retained source anchor for a source-config diagnostic label.
    ///
    /// Prefer the selected source provider/default location; command and builder providers have no
    /// source location, so their labels fall back to the authored contract span.
    pub(crate) fn diagnostic_origin_and_span(
        &self,
    ) -> (BuildConfigValueOrigin, Option<SourceSpan>) {
        let provider_span = match self.value_location.as_ref() {
            Some(BuildConfigValueLocation::Source(span)) => Some(*span),
            Some(BuildConfigValueLocation::Command(_)) | None => None,
        };
        (self.origin, provider_span.or(self.span))
    }
}

/// The deterministic resolved values for one project or package configuration boundary.
///
/// The map contains every known source, fixed-project and declaration-owned config contract name
/// exactly once. Its key order is lower_snake_case lexical order, independent of the order in
/// which the three fact slices were supplied.
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

    /// Find retained source-config provenance without allocating a validated key.
    pub(crate) fn diagnostic_origin_and_span(
        &self,
        name: &str,
    ) -> Option<(BuildConfigValueOrigin, Option<SourceSpan>)> {
        self.entries
            .get(name)
            .map(|value| value.diagnostic_origin_and_span())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Borrowed indexes for one already-validated canonical configuration boundary.
///
/// WHAT: retains the validated canonical value map and references to canonical source,
///       fixed-project and declaration-owned config facts without copying any fact payload. A
///       transient check-only unit then validates and resolves only its own source facts against
///       this index.
/// WHY: check-only compilation must not clone the canonical contract vector or reselect its
///       providers and defaults for every transient unit. The canonical resolver already validated
///       these facts and values before the index is built; only transient facts and any resolution
///       output are owned per unit.
pub(crate) struct BuildConfigResolutionIndex<'a> {
    canonical_values: &'a ResolvedBuildConfigMap,
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
        canonical_values: &'a ResolvedBuildConfigMap,
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
            canonical_values,
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
    /// Canonical values are cloned as the starting map, preserving the owning boundary's provider
    /// and default decisions. A transient duplicate must agree with its canonical namesake, but
    /// only names introduced by the transient unit are resolved from its private facts.
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
                        fixed: Box::new((**fixed).clone()),
                        source: Box::new(fact.clone()),
                        reason,
                    });
                }
                if let Some(project) = self.direct_project_by_name.get(fact.name())
                    && let Some(reason) = contract_conflict_reason(project, fact, true)
                {
                    return Err(BuildConfigResolutionError::ProjectSourceContractConflict {
                        project: Box::new((**project).clone()),
                        source: Box::new(fact.clone()),
                        reason,
                    });
                }
            }
        }

        let mut resolved = self.canonical_values.clone();

        // Finally resolve names introduced only by this transient source unit. Canonical names
        // and declaration-owned config names already retain their canonical result above.
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
            resolved.entries.insert((*name).clone(), value);
        }

        for input in explicit_inputs.iter() {
            if !resolved.entries.contains_key(input.name()) {
                return Err(BuildConfigResolutionError::UnknownExplicitInput {
                    input: input.clone(),
                });
            }
        }

        Ok(resolved)
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
                first: Box::new(first.clone()),
                conflicting: Box::new(fact.clone()),
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
/// is always name ordered. Fixed project fields are authoritative, declaration-owned config
/// contracts are resolved before source-only contracts, and unknown explicit inputs are checked
/// only after all known facts have been validated and resolved.
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
                    first: Box::new(first.clone()),
                    conflicting: Box::new(fact.clone()),
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
                first: Box::new(first.clone()),
                conflicting: Box::new(fact.clone()),
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
                contract: Box::new(fact.clone()),
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
                fixed: Box::new(fixed.clone()),
                source: Box::new(source.clone()),
                reason,
            });
        }

        if let Some(project) = direct_project_by_name.get(name)
            && let Some(reason) = contract_conflict_reason(project, source, true)
        {
            return Err(BuildConfigResolutionError::ProjectSourceContractConflict {
                project: Box::new(project.clone()),
                source: Box::new(source.clone()),
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

/// Why one supplied value cannot initialise the contract it reaches.
///
/// WHAT: a value whose exact primitive type does not match the contract, or a numeric literal
///       outside the range of the contract's exact scalar.
/// WHY:  the boundary and the AST resolver must materialise a supplied value by the same rule, and
///       each lane renders the refusal in its own diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContractValueError {
    TypeMismatch {
        provided: PrimitiveBuildInputType,
    },
    LiteralOutOfRange {
        text: String,
        reason: NumberLiteralErrorReason,
    },
}

/// Map one refused supplied value to the resolution error for the contract it reached.
fn contract_value_failure(
    error: ContractValueError,
    contract: &BuildConfigContractFact,
    value_location: Option<BuildConfigValueLocation>,
) -> BuildConfigResolutionError {
    match error {
        ContractValueError::TypeMismatch { provided } => {
            BuildConfigResolutionError::ValueTypeMismatch {
                contract: Box::new(contract.clone()),
                provided,
                value_location,
            }
        }
        ContractValueError::LiteralOutOfRange { text, reason } => {
            BuildConfigResolutionError::NumericLiteralOutOfRange {
                contract: Box::new(contract.clone()),
                text,
                reason,
                value_location,
            }
        }
    }
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
        return Ok(resolved_fixed_project_value(source, fixed_project));
    }

    if let Some(direct_project) = direct_project {
        let provider = direct_project.resolved_provider().ok_or_else(|| {
            BuildConfigResolutionError::DirectProjectProviderMissing {
                contract: Box::new(direct_project.clone()),
            }
        })?;
        return Ok(resolved_value_from_provider(direct_project, provider));
    }

    let contract = source.ok_or_else(|| {
        // `name` only comes from a source fact at this point. This branch is an internal invariant,
        // not user input; preserving a typed error is preferable to panicking if map assembly
        // changes that invariant.
        BuildConfigResolutionError::MissingRequiredValue {
            contract: Box::new(BuildConfigContractFact::new(
                name.clone(),
                BuildInputType::Primitive(PrimitiveBuildInputType::String),
                true,
                None,
                None,
            )),
        }
    })?;

    let (value, origin, value_location) = if let Some(input) = explicit_inputs.get(name) {
        let value = input
            .value()
            .materialise_for(contract.value_type())
            .map_err(|error| {
                contract_value_failure(error, contract, Some(input.location().clone()))
            })?;
        (
            Some(value),
            BuildConfigValueOrigin::ExplicitInput,
            Some(input.location().clone()),
        )
    } else if let Some(value) = builder_globals.get(name) {
        let value = value
            .accept_for(contract.value_type())
            .map_err(|error| contract_value_failure(error, contract, None))?;
        (Some(value), BuildConfigValueOrigin::BuilderGlobal, None)
    } else if let Some(value) = contract.default_value() {
        (
            Some(value.clone()),
            BuildConfigValueOrigin::DeclarationDefault,
            contract.span.map(BuildConfigValueLocation::Source),
        )
    } else if contract.required() {
        return Err(BuildConfigResolutionError::MissingRequiredValue {
            contract: Box::new(contract.clone()),
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
) -> ResolvedBuildConfigValue {
    resolved_value_from_contract(
        source,
        fixed_project.default.clone(),
        BuildConfigValueOrigin::FixedProjectField,
        fixed_project.span.map(BuildConfigValueLocation::Source),
    )
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
        span: contract.span,
        value_location: provider.value_location.clone(),
    }
}

/// Build one resolved value and fingerprint it under its exact contract.
///
/// WHAT: the single point where a selected value becomes a resolved value. Every selected value
///       already has its contract's exact primitive type: explicit inputs materialise against the
///       contract, and declaration and fixed-field defaults materialised when their contract facts
///       were built.
/// WHY:  the fingerprint then depends only on exact type identity and the canonical value, never on
///       origin or location.
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
        span: contract.span,
        value_location,
    }
}
