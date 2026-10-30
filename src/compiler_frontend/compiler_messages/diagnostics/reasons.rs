//! Constructor inputs and semantic classification facts used by diagnostic families.

//! These types describe facts owned by producers before a compact family draft is built.

use crate::compiler_frontend::compiler_messages::diagnostic_token::DiagnosticToken;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;

// -------------------------------
//  Diagnostic Family Inputs
// -------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameNamespace {
    Value,
    Type,
    Module,
    Field,
    Variant,
    Function,
    Method,
    TemplateSlot,
    ConfigKey,
}
/// Which compact source-owned table rejected an authored request.
///
/// Span resources keep the source identity on the diagnostic's primary span; this fact records
/// the exhausted source-owned capacity without allocating another row. Path/identity resources
/// name compact identity tables whose exhaustion has no meaningful authored span, so the
/// diagnostic stays spanless rather than manufacturing source provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceSpanCapacityResource {
    ExtendedSpan,
    LogicalPath,
    SourceIdentity,
    /// Source-owned authored path syntax rows.
    PathSyntax,
    /// Source-owned numeric literal side-store rows.
    NumericLiteral,
    /// Source-owned token shapes/spans for one source.
    Token,
    /// Authored physical source bytes that cannot be addressed by u32 offsets.
    SourceSnapshot,
}

/// Which compact HIR row or edge buffer could not represent another source-owned value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HirCapacityResource {
    ExpressionRows,
    ValueEdges,
    StructFields,
    VariantFields,
    MapEntries,
    StringPieces,
    PlaceProjections,
}

/// Which compact AST expression or constant-value buffer could not represent another
/// source-owned value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AstCapacityResource {
    /// Completed AST semantic expression rows.
    ExpressionRows,
    /// Typed AST expression ranges and semantic side records.
    ExpressionSideEntries,
    /// Constant value rows in the constant store.
    ConstValues,
    /// Choice variant tags stored on choice construction expressions.
    ChoiceTags,
    /// Distances between flat expression grouping markers.
    GroupDistances,
}

/// Which store arena or fact slot of the compact diagnostic store could not hold another row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticCapacityResource {
    /// Fixed-size diagnostic records.
    Records,
    /// Extra-data rows that point at labels, foreign sites, and string lists.
    Extras,
    /// Secondary label rows.
    Labels,
    /// Typed label data rows.
    LabelData,
    /// String identifiers carried by list facts.
    StringIds,
    /// Generic substitution rows referenced by label data.
    Substitutions,
    /// Foreign-domain site rows.
    ForeignSites,
    /// Frozen identity contexts shared by foreign sites.
    IdentityContexts,
    /// A fact value that does not fit its compact encoding.
    FactValue,
    /// A type display id that does not fit its compact encoding.
    TypeDisplays,
}

/// The compact buffer that overflowed, tagged with the stage that owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CapacityResource {
    Hir(HirCapacityResource),
    Ast(AstCapacityResource),
    PathNodes,
    Store(DiagnosticCapacityResource),
}
impl From<HirCapacityResource> for CapacityResource {
    fn from(resource: HirCapacityResource) -> Self {
        Self::Hir(resource)
    }
}

impl From<AstCapacityResource> for CapacityResource {
    fn from(resource: AstCapacityResource) -> Self {
        Self::Ast(resource)
    }
}

/// Why project-context-dependent semantic facts cannot cross a package facade boundary.
///
/// The reason is intentionally independent of the rendered wording so build, check and tooling
/// can distinguish direct public declarations from executable reachability failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProjectContextEscapeReason {
    ExportedDeclaration,
    ReachableExecutable,
}

/// The semantic receiving boundary that requires a producer's success slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueReceiverContext {
    Declaration,
    Assignment,
    Return,
    Then,
    FunctionArgument,
    ConstructorArgument,
    OperatorOperand,
    Condition,
    ReceiverBase,
    CollectionElement,
    MapKey,
    MapValue,
    TemplateInterpolation,
    CastSource,
    MatchScrutinee,
    AssertionArgument,
    General,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeMismatchContext {
    Assignment,
    Declaration,
    ReturnValue,
    FunctionArgument,
    AssertionArgument,
    ConstructorArgument,
    ReceiverArgument,
    Operator,
    Condition,
    CollectionElement,
    StructFieldDefault,
    TemplateInterpolation,
    MatchScrutinee,
    MatchPattern,
    ErrorReturn,
    Pattern,
    General,
    CatchRecovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamingConvention {
    CamelCase,
    LowercaseWithUnderscores,
    UppercaseWithUnderscores,
    LowercaseOrUppercaseWithUnderscores,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportPublicSurfaceType {
    SourcePackage,
    ModuleRoot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticPlace {
    Local(StringId),
    Path(PathId),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BorrowAccessKind {
    Shared,
    Mutable,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidMutableAccessReason {
    ImmutablePlace,
    OverlappingAccess,
    AliasedValueRequiresExclusiveAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnsupportedBackendFeatureReason {
    ConstructorRoutes,
    HashmapConstruction,
    HashmapOperation,
    RuntimeCasts,
    CheckedNumericOperations,
    RecoverableNumericFailure,
    FloatFormatting,
    FloatBoundaryValidation,
    GenericRuntimeValues,
    FixedWidthScalarValues,
    CrossModuleCalls,
    RuntimeAssertionMessages,
    ErrorValues,
    FallibleControlFlow,
    MutableFunctionParameters,
    MutableLocalBindingAliases,
    NumberValues,
    VariantPayloadValues,
    UnaryOperators,
    NonEmptyCollections,
    StructRangeVariantPayloadExpressions,
    TemplateChunkTypes,
    ProjectedPlaces,
    LinkedFunctionTargets,
    MatchTerminators,
    ProjectedAssignmentTargets,
}

impl UnsupportedBackendFeatureReason {
    pub(crate) fn feature_name(self) -> &'static str {
        match self {
            Self::ConstructorRoutes => "constructor routes",
            Self::HashmapConstruction => "hashmap construction",
            Self::HashmapOperation => "hashmap operation",
            Self::RuntimeCasts => "runtime casts",
            Self::CheckedNumericOperations => "checked numeric operations",
            Self::RecoverableNumericFailure => "recoverable numeric failure",
            Self::FloatFormatting => "binary-float formatting",
            Self::FloatBoundaryValidation => "binary-float boundary validation",
            Self::GenericRuntimeValues => "generic runtime values",
            Self::FixedWidthScalarValues => "fixed-width numeric and Byte values",
            Self::CrossModuleCalls => "cross-module calls",
            Self::RuntimeAssertionMessages => "runtime assertion messages",
            Self::ErrorValues => "Moth Error values",
            Self::FallibleControlFlow => "fallible control flow",
            Self::MutableFunctionParameters => "mutable function parameters",
            Self::MutableLocalBindingAliases => "mutable local binding aliases",
            Self::NumberValues => "Dec values",
            Self::VariantPayloadValues => "variant payload fields",
            Self::UnaryOperators => "unary operators",
            Self::NonEmptyCollections => "collection literals other than empty String collections",
            Self::StructRangeVariantPayloadExpressions => {
                "struct literals, ranges, and variant payload reads"
            }
            Self::TemplateChunkTypes => {
                "string template chunks of type Bool, Char, Byte, float, or unit"
            }
            Self::ProjectedPlaces => "projected place reads",
            Self::LinkedFunctionTargets => "module-private and generated function calls",
            Self::MatchTerminators => "match control flow",
            Self::ProjectedAssignmentTargets => "projected assignment targets",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InvalidConfigReason {
    MissingKey,
    MissingConfigFile,
    MissingProjectRecord,
    MissingActiveBuilderSection {
        section: StringId,
    },
    DuplicateKey,
    FunctionUnsupported,
    TraitDeclarationUnsupported,
    TraitConformanceUnsupported,
    TraitIncompatibilityUnsupported,
    NamedTypeUnsupported,
    MutableBindingUnsupported,
    PlainBindingUnsupported,
    UnsupportedStatement,
    StandaloneTemplateUnsupported,
    MissingValue,
    UnsupportedScalarValue,
    NotCompileTimeConstant,
    ValueCouldNotFold,
    EmptyProjectSetting,
    /// A declaration-owned `#Config` dependency targeted a grouped-project field whose schema
    /// policy is fixed-only.
    ConfigQualifierFixedField,
    /// A source or project field's name cannot become a build-config contract.
    ConfigContractNameInvalid,
    /// A `#Config` qualifier appeared outside a top-level source declaration.
    ConfigQualifierInvalidPlacement,
    /// A filesystem module or source-package prefix claimed the reserved `@project` root.
    ProjectGlobalsNameReserved,
    /// The qualifier's `of` type is outside the primitive/optional contract vocabulary.
    ConfigQualifierUnsupportedType,
    /// An already-typed build-config value did not agree with another contract for the same name.
    ///
    /// The two interned descriptions include primitive type, optionality and default/required
    /// facts; source locations are carried by the primary and secondary diagnostic labels.
    ConfigContractConflict {
        first: StringId,
        conflicting: StringId,
    },
    /// An explicit command input name was not present in the complete boundary contract set.
    ///
    /// `provided_argument_index` retains the command position even though the diagnostic's
    /// source anchor falls back to the project/config context.
    UnknownBuildConfigInput {
        key: StringId,
        provided_argument_index: Option<usize>,
    },
    /// An explicit or builder-provided primitive did not exactly match the contract type.
    ///
    /// `provided_argument_index` retains command provenance when the supplied value came from
    /// outside the authored qualifier declaration.
    ConfigInputTypeMismatch {
        provided: StringId,
        expected: StringId,
        provided_argument_index: Option<usize>,
    },
    /// A supplied numeric literal does not fit the exact scalar its contract selects.
    ///
    /// `provided_argument_index` retains command provenance, because a command value has no
    /// source span of its own and the primary anchor is the receiving declaration.
    ConfigInputOutOfRange {
        literal: StringId,
        reason: NumberLiteralErrorReason,
        provided_argument_index: Option<usize>,
    },
    /// A required declaration-owned config contract had no input, global or default.
    MissingConfigInput,
    UnknownKey {
        key: StringId,
    },
    UnknownRecordField {
        record: StringId,
        field: StringId,
    },
    MissingRequiredRecordField {
        record: StringId,
        field: StringId,
    },
    InvalidConfigValueShape {
        expected: StringId,
    },
    InvalidProjectSettingValue {
        value: StringId,
        expected: StringId,
    },
    MissingHtmlHomepage {
        entry_root: StringId,
    },
    DuplicateHtmlOutputPath {
        output_path: StringId,
        entry_point: StringId,
        existing_entry_point: StringId,
    },
    ResourceOutputPathCollision {
        output_path: StringId,
        existing_origin: StringId,
        conflicting_origin: StringId,
    },
    ResourceOutputPathReserved {
        output_path: StringId,
        origin: StringId,
        artefact_kind: StringId,
    },
    InvalidArtefactOutputPath {
        output_path: StringId,
        artefact_kind: StringId,
        reason: InvalidOutputFolderReason,
    },
    ConfiguredEntryRootMissing {
        entry_root: StringId,
    },
    EntryRootPackagePrefixCollision {
        prefix: StringId,
        entry_folder: StringId,
    },
    SourcePackageMissingRoot {
        prefix: StringId,
        root: StringId,
    },
    SourcePackageMultipleRoots {
        prefix: StringId,
        root: StringId,
        candidates: Vec<StringId>,
    },
    NoRootModuleEntries {
        entry_root: StringId,
    },
    MultipleModuleRootFiles {
        directory: StringId,
        candidates: Vec<StringId>,
    },
    ConfigImportUnsupported,
    FileValuePathUnsupported,
    SourceFileFolderCollision {
        file_name: StringId,
        folder_name: StringId,
        directory: StringId,
    },
    LegacyModuleRootFileName {
        file_name: StringId,
        directory: StringId,
    },
    InvalidOutputFolder {
        folder: Option<StringId>,
        reason: InvalidOutputFolderReason,
    },
    OutputFoldersNotDistinct {
        dev_folder: StringId,
        release_folder: StringId,
    },
    OutputManifestOwnerConflict {
        output_root: StringId,
        existing_builder: StringId,
        existing_profile: StringId,
        active_builder: StringId,
        active_profile: StringId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidOutputFolderReason {
    Empty,
    NonUtf8,
    AbsolutePath,
    RootOrPrefix,
    ParentDirectorySegment,
    CurrentDirectory,
    InvalidPathComponent,
    InsideOrEqualToEntryRoot,
    ResolvesOutsideProjectRoot,
}
/// Structured facts for a malformed external JavaScript module.
///
/// Compiler-authored receiver diagnostics carry the offending Moth name. Parser diagnostics keep
/// their foreign tool detail opaque and interned so the compiler relays it without re-authoring it.
/// A source file that is not valid UTF-8 carries the decoder's description of the invalid sequence.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InvalidExternalModuleReason {
    ReceiverMethod {
        moth_name: StringId,
    },
    ParserDiagnostic {
        parser_detail: StringId,
    },
    NonUtf8Source {
        utf8_detail: StringId,
    },
    /// Package paths reserve `#`, so a logical source path containing it cannot be registered.
    PathContainsPackageSeparator,
}

/// WHAT: structured reason for an invalid quoted-string escape.
/// WHY: the tokenizer rejects unsupported escapes, physical-newline continuation and trailing
/// backslashes with one diagnostic family while keeping the exact cause structured for render.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidStringEscapeReason {
    UnsupportedEscape { escaped: char },
    PhysicalNewline,
    TrailingBackslash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenericApplicationErrorReason {
    OnNonNamedType,
    EmptyArgumentList,
    MissingArgumentAfterComma,
    NestedApplication,
}

/// Declaration kind that owns a public generic whose trait bound is private at
/// its owner. The diagnostic names this kind so the remediation matches the
/// declaration the author wrote; the leak rule itself is the same for every kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PublicGenericOwnerKind {
    Function,
    ReceiverMethod,
    Struct,
    Choice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParameterContractKind {
    Wire,
    Route,
}

impl ParameterContractKind {
    /// Every authored parameter-contract kind, in spelling order.
    pub const ALL: [ParameterContractKind; 2] =
        [ParameterContractKind::Wire, ParameterContractKind::Route];

    /// The one compiler-owned head spelling for this contract kind.
    ///
    /// WHAT: the single definition of the `Wire` and `Route` literals used by the contract
    ///       parser, the reserved-name registry, declaration validation and diagnostics.
    /// WHY: the spelling is syntax with one owner; copies drift and let one site accept a name
    ///      the others reject.
    pub const fn spelling(self) -> &'static str {
        match self {
            ParameterContractKind::Wire => "Wire",
            ParameterContractKind::Route => "Route",
        }
    }

    /// The contract kind for one authored head spelling, if it is compiler-owned.
    pub fn from_spelling(spelling: &str) -> Option<ParameterContractKind> {
        ParameterContractKind::ALL
            .into_iter()
            .find(|kind| kind.spelling() == spelling)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidParameterContractReason {
    MissingSpecification {
        contract: ParameterContractKind,
    },
    InvalidPosition {
        contract: ParameterContractKind,
    },
    WireArrowNotAllowed,
    RouteOutputMissing,
    RouteRepeatedArrow,
    NestedContract {
        outer: ParameterContractKind,
        inner: ParameterContractKind,
    },
    MutableParameter {
        contract: ParameterContractKind,
    },
    ParameterDefault {
        contract: ParameterContractKind,
    },
    ReceiverContract {
        contract: ParameterContractKind,
    },
    MultipleSpecificationTypes {
        contract: ParameterContractKind,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathKind {
    Empty,
    TrailingSeparator,
    InvalidRoot,
    InvalidComponent,
    OnlyRootSlashSupported,
    EmptyComponent,
    WhitespaceMustBeQuoted,
    MissingSeparator,
    MissingClosingQuote,
    InvalidEscape,
    /// A path component starts with `@` after the dependency introducer was consumed.
    ///
    /// WHAT: the first `@` introduces the dependency path. A second `@` in
    ///       any component (such as `@@pages` or `@helper/@home`) is not a valid module name.
    /// WHY: normal module-root filenames such as `@page.moth` are cosmetic filesystem markers,
    ///      not dependency names. The module's dependency identity comes from its directory.
    LeadingAtInPathComponent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DependencyClauseKind {
    Namespace,
    NamespaceAlias,
    DirectSelection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidDependencyClauseReason {
    MissingPath,
    ExpectedPath,
    ExpectedSelectionName,
    MissingAlias,
    ExpectedAliasName,
    DuplicateSelectionName,
    DuplicateSelectionLocalName,
    LegacyBraceSelections,
    MissingSelectionAfterComma,
    MissingCommaBetweenSelections,
    NamespaceAliasWithSelections,
    InvalidSelectionDelimiter,
    DependencyClauseNotAllowed,
    /// The reserved `@project` namespace cannot be used as a nested dependency path.
    ProjectGlobalsPathReserved,
    /// The project package facade cannot declare a dependency on its own `@project` provider.
    ProjectGlobalsFacadeDependencyNotAllowed,
    /// The reserved `@project` synthetic interface cannot be directly re-exported.
    ProjectGlobalsReexportNotAllowed,
    ProviderRequiresBinding,
    /// A continuation comma consumed a declaration-start token as a dependency selection.
    ///
    /// WHAT: the comma after a selection kept the clause open, so the following declaration
    ///       name was parsed as another selection. The diagnostic points at that name with a
    ///       secondary label on the continuation comma.
    ContinuationEnteredStatement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LegacyDependencyClauseReason {
    ImportKeyword,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidImportPathReason {
    PublicRoot,
    CurrentDirectorySegment,
    ParentDirectorySegment,
    EscapesProjectRoot,
    EscapesSourcePackageRoot,
    CaseMismatch {
        provided: StringId,
        expected: StringId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidCompileTimePathReason {
    MissingTarget,
    TargetIsDirectory,
    TargetNotRegular,
    CurrentDirectorySegment,
    ParentDirectorySegment,
    CaseMismatch {
        provided: StringId,
        expected: StringId,
    },
    EscapesModuleBoundary,
    EscapesSymlink,
    EscapesProjectRoot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeAnnotationContext {
    DeclarationTarget,
    /// Required primitive/optional type after a declaration-owned `#Config of` qualifier.
    BuildConfigContract,
    SignatureParameter,
    SignatureReturn,
    TypeAliasTarget,
    TraitRequirement,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InvalidTypeAnnotationReason {
    NoneNotAllowed,
    ReservedTraitKeyword,
    TraitThisMustBeDirect,
    AsNotValidHere,
    UnexpectedColon,
    InvalidTokenAfterName { token: DiagnosticToken },
    ExpectedTypeAnnotation { found: DiagnosticToken },
    DuplicateOptional,
    NestedOptional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidCollectionTypeReason {
    NegativeCapacity,
    /// Capacity-only shorthand `{N}` is not allowed in type signatures, aliases,
    /// struct fields, or return types. It is only valid for declaration targets
    /// where the initializer provides the element type.
    ShorthandCapacityNotAllowed,
    /// Fixed collection capacity must be greater than zero.
    ZeroCapacity,
    /// Capacity expression did not fold to an integer value.
    CapacityNotInt,
    /// Capacity expression references a non-constant value or contains runtime-only syntax.
    CapacityNotConstant,
    /// Folded capacity value does not fit in `usize`.
    CapacityOverflow,
    /// Collection literal has more items than the fixed collection capacity allows.
    InitializerExceedsFixedCapacity {
        capacity: usize,
        length: usize,
    },
    /// Immutable binding initialized with an empty fixed collection literal.
    EmptyImmutableFixedCollection,
    /// Capacity-only shorthand declaration requires a non-empty collection literal
    /// so the element type can be inferred.
    ShorthandEmptyLiteralAmbiguous,
    /// Capacity-only shorthand declaration requires a collection literal initializer.
    ShorthandNonLiteralRhs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidMapTypeReason {
    /// The key type is not one of the supported scalar types.
    UnsupportedKeyType { key_type: TypeId },
    /// Map types are nested too deeply inline; use a type alias instead.
    ExcessiveInlineNesting { depth: usize },
    /// Map type is missing the key type before the '=' separator.
    EmptyMapKeyType,
    /// Map type is missing the value type after the '=' separator.
    EmptyMapValueType,
    /// Map type contains more than one top-level '=' separator.
    MultipleMapSeparators,
    /// Fixed or capacity map syntax is outside the builtin hashmap design.
    FixedCapacityNotAllowed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidMapLiteralReason {
    /// A literal was classified as a map but an entry lacks a top-level `=`.
    MixedCollectionMapEntries,
    /// A foldable literal key appears more than once in the same map literal.
    DuplicateKnownKey,
    /// A map entry has `=` before any key expression.
    MissingKeyExpression,
    /// A map entry ends before a value expression appears after `=`.
    MissingValueExpression,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InvalidGenericParameterReason {
    EmptyParameterList,
    BoundsMustUseIs,
    ListMustStayWithHeader,
    InvalidToken { found: DiagnosticToken },
    DuplicateTraitBound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidTemplateDirectiveReason {
    UnknownDirective,
    MissingArgument,
    /// Optional helpful detail from the directive's own argument validation,
    /// for example an unsupported language name plus the supported aliases.
    InvalidArgument {
        detail: Option<StringId>,
    },
    DirectiveNotAllowedHere,
    /// Directive received parenthesized arguments it does not accept.
    UnexpectedArguments,
    /// Directive parentheses are empty.
    EmptyArguments,
    /// `$slot` received an invalid target argument.
    InvalidSlotTarget,
    /// `$insert` received an invalid slot name argument.
    InvalidInsertTarget,
    /// `$children` received an invalid wrapper argument.
    InvalidChildrenArgument,
}

impl InvalidTemplateDirectiveReason {
    pub(crate) const fn invalid_argument() -> Self {
        Self::InvalidArgument { detail: None }
    }

    pub(crate) const fn invalid_argument_with_detail(detail: StringId) -> Self {
        Self::InvalidArgument {
            detail: Some(detail),
        }
    }
}
/// Structured warning facts produced while validating a CSS template.
///
/// Values that appear in the warning text stay as source facts here; renderers own their prose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CssTemplateWarning {
    MismatchedClosingBrace { expected: char },
    UnexpectedClosingBrace,
    MismatchedClosingParenthesis { expected: char },
    UnexpectedClosingParenthesis,
    UnclosedDelimiter { opening: char },
    InlineSelectorBlock,
    MissingSelectorOrAtRulePrelude,
    NestedBlock,
    TopLevelContent,
    MalformedDeclaration,
    InvalidPropertyName { name: StringId },
    MissingDeclarationValue,
}

/// Structured warning facts produced while validating an HTML template.
///
/// Every current variant reports potentially unsafe authored markup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HtmlTemplateWarning {
    ScriptTag,
    JavascriptUrl,
    InlineEventHandler,
}

/// Which project-owned validator emitted a malformed-template warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MalformedTemplateReason {
    Css(CssTemplateWarning),
    Html(HtmlTemplateWarning),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidTemplateStructureReason {
    MissingClosingBracket,
    SlotInHead,
    MissingHandlerBody,
    InvalidChildDirective,
    NestedTemplateNotAllowed,
    HelperInConstTemplate,
    NonFoldableConstTemplate,
    NonFoldableDocComment,
    FallibleValueInTemplateHead,
    UnsupportedTypeInTemplateHead { type_id: TypeId },
    RuntimeTemplateInConst,
    RuntimeValueInConstTemplateHead,
    EmptyPathInTemplateHead,
    IncompatibleHeadItem,
    HelperOutsideWrapperSlot,
    RuntimeControlFlowUnresolvedInsert,
    MissingCommaBeforeControlFlowSuffix,
    ControlFlowSuffixNotFinal,
    MissingTemplateIfCondition,
    MissingTemplateLoopHeader,
    UnexpectedTokenAfterControlFlowSuffix,
    TemplateMatchStyleControlFlowUnsupported,
    TemplateIfConditionNotConst,
    TemplateIfBranchNotConst,
    TemplateOptionCaptureConstDeferred,
    TemplateLoopRangeBoundsNotConst,
    TemplateLoopSourceNotConst,
    TemplateLoopConditionNotConst,
    TemplateConditionalLoopConstTrue,
    TemplateLoopBodyNotConst,
    TemplateConstLoopExpansionLimitExceeded { limit: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidSignatureMemberReason {
    ChoicePayloadMutable,
    ChoicePayloadDefaultValue,
    CompileTimeParameterDeferred,
    ThisNotAllowed,
    TrailingComma,
    TraitReceiverMustBeThis,
    TraitMutableThisOnlyFirstParameter,
    TraitBareThisOnlyReceiver,
    TraitRequirementDefaultValue,
    /// An authored `=` introduced a parameter or struct-field default but no value
    /// followed it before a top-level comma, closing pipe, newline, block end or EOF.
    MissingDefaultValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InvalidFunctionSignatureReason {
    MissingArrowOrColon { found: DiagnosticToken },
    UnexpectedEndAfterParameters,
    MissingReturnType,
    MissingTraitRequirementReturnType,
    TrailingCommaInReturns,
    UnexpectedEndAfterComma,
    UnexpectedEndInReturns,
    MissingColonAfterReturns,
    UnexpectedArrowInReturns,
    MissingCommaOrColon { found: DiagnosticToken },
    VoidNotAllowed,
    MultipleErrorReturnSlots,
    ErrorSlotNotLast,
    GenericWhereConstraintsUnsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidChoiceVariantReason {
    EmptyRecordBody,
    RecursiveDeclaration,
    ConstructorStyleNotSupported,
    PayloadShorthandNotSupported,
    UnexpectedSeparator,
    MissingVariants,
    UnknownVariant,
    UnitVariantWithParentheses,
    UnitVariantAsConstructor,
    PayloadVariantMissingArguments,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReservedNameOwner {
    BuiltinType,
    Keyword,
    CoreTrait,
    ImplicitStart,
    ParameterContract,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidThisUsageReason {
    NotInReceiverMethod,
    Reassignment,
    LoopBinding,
    DeclarationBinding,
    DuplicateThis { function_name: StringId },
    NotFirstParameter { function_name: StringId },
    OutsideTraitDeclaration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidTraitKeywordUsageReason {
    MustOutsideTraitSyntax,
    ThisOutsideTraitSyntax,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidReceiverDeclarationReason {
    UnknownStructTarget,
    NonlocalSourceType,
    BuiltinScalarType,
    ExternalOpaqueType,
    FieldNameConflict,
    DuplicateMethod,
    DuplicateVisibleMethod,
    GenericReceiverType {
        function_name: StringId,
        /// Receiver type identity; rendered through the diagnostic render context.
        type_id: TypeId,
    },
    UnsupportedType {
        function_name: StringId,
        /// Receiver type identity; rendered through the diagnostic render context.
        type_id: TypeId,
    },
    ReceiverMethodImportOrExportNotAllowed,
    GenericOwnerAlignment,
    GenericParameterAlignment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidControlFlowStatementReason {
    ElseOutsideIfOrMatch,
    ElseIfUnsupported,
    BreakOutsideLoop,
    ContinueOutsideLoop,
    ReturnOutsideFunction,
    ReturnBangOutsideErrorFunction,
    ExpectedColonAfterCondition,
    ExpectedConditionAfterIf,
    UnexpectedEndOfFileInMatch,
    CaseRequiredBeforeElse,
    DuplicateElseArm,
    ExpectedFatArrow,
    InlineValueIfMultiline,
    InlineValueIfElseThen,
    ValueIfMissingElse,
    ValueIfBranchFallsThrough,
    ValueIfNoProducingPath,
    ValueBlockOutsideReceiver,
    ValueIfOptionNonePredicate,
    ValueIfOptionLiteralPredicate,
    ExpectedValueAfterThen,
    ExpectedValueAfterElse,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InvalidDeclarationReason {
    ReservedBuiltinName,
    ConstantCannotBeMutable,
    ExternalTypeLiteralConstruction,
    ParameterizedGenericTypeAlias,
    UnusedGenericParameter { parameter_name: StringId },
    RecursiveGenericType,
    RecursiveRuntimeStruct { cycle: Vec<StringId> },
    ExternalTypeAlias { type_name: StringId },
    InvalidGenericParameterName { parameter_name: StringId },
    DuplicateGenericParameter { parameter_name: StringId },
    GenericParameterNameCollision { parameter_name: StringId },
    ReservedGenericParameterName { parameter_name: StringId },
    GenericTraitsUnsupported,
    InvalidTraitName,
    TraitConformanceMissingTrait,
    TraitConformanceSemicolon,
    TraitIncompatibilityMissingTrait,
    TraitIncompatibilitySemicolon,
    MissingInitializerExpression,
    DeclarationRequiresModuleTopLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidTraitIncompatibilityReason {
    SelfIncompatible,
    UnknownTrait,
    DuplicateRelation,
    PrivateTraitSurfaceLeak,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InvalidTraitConformanceReason {
    ImportedModuleRoot,
    AliasTarget,
    NonCanonicalTarget,
    NonlocalSourceTarget,
    BuiltinTarget,
    ExternalOpaqueTarget,
    DuplicateCanonicalEvidence,
    BuiltinEvidenceOverride,
    IncompatibleTraitEvidence {
        incompatible_trait_name: StringId,
    },
    MissingMethod {
        requirement_name: StringId,
    },
    ReceiverMutabilityMismatch {
        requirement_name: StringId,
    },
    ParameterCountMismatch {
        requirement_name: StringId,
        expected: usize,
        found: usize,
    },
    ParameterModeMismatch {
        requirement_name: StringId,
        parameter_index: usize,
    },
    ParameterContractMismatch {
        requirement_name: StringId,
        parameter_index: usize,
    },
    ParameterTypeMismatch {
        requirement_name: StringId,
        parameter_index: usize,
        expected_type: TypeId,
        found_type: TypeId,
    },
    ReturnCountMismatch {
        requirement_name: StringId,
        expected: usize,
        found: usize,
    },
    ReturnTypeMismatch {
        requirement_name: StringId,
        return_index: usize,
        expected_type: TypeId,
        found_type: TypeId,
    },
    ReturnChannelMismatch {
        requirement_name: StringId,
        return_index: usize,
    },
    InferredFailureInInfallibleCast {
        requirement_name: StringId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidAssignmentTargetReason {
    TemporaryNotAssignable,
    ImmutableBinding,
    ImmutableFieldRoot,
    UnavailableInCatchRecovery,
    CollectionGetTargetNotWritable,
    MapGetTargetNotWritable,
    ReadOnlyMapProperty,
    ExpectedAssignmentOperator,
    MissingWritePrefix {
        operator: Option<DiagnosticCompoundAssignmentOperator>,
    },
    MutableMarkerRequiresExistingPlace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidMultiBindReason {
    ThisTargetReserved,
    ExpectedTargetName,
    MissingTargetAfterComma,
    MissingAssignmentOperator,
    InvalidTokenAfterTarget,
    MissingRightHandExpression,
    MultipleRightHandExpressions,
    MutableTargetNeedsExplicitType,
    CompileTimeMarkerOnTarget,
    DuplicateTarget,
    UnsupportedRhs,
    ExistingTargetMutableMarker,
    ExistingTargetImmutable,
    ExistingTargetNeedsPrefix,
    UnknownPrefixedTarget,
    ArityMismatch { expected: usize, found: usize },
    RhsNotMultiValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidBuiltinCallReason {
    MissingParentheses,
    TakesNoArguments,
    NamedArgumentsNotSupported,
    UnhandledFallibleCall,
    DoesNotAcceptMutableAccess,
    CastMissingParentheses,
    CastMissingArgument,
    CastTooManyArguments,
    CastMissingClosingParenthesis,
    MissingArgument,
    TooManyArguments,
    ExpressionPositionNotAllowed,
    MapLengthIsProperty,
    ScalarConstructorRemoved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidCastReason {
    MissingExplicitTarget,
    TargetNotBuiltin,
    TargetIsGenericParameter,
    SameSourceAndTarget,
    SourceIsOptional,
    OperandArityMismatch,
    TargetArityMismatch,
    CastPropagationRemoved,
    ScalarConstructorRemoved,
    NoEvidence,
    BuiltinEvidenceNotConstFoldable,
    UserDefinedEvidenceNotConstFoldable,
    GenericBoundEvidenceNotConstFoldable,
    BuiltinCastFailedInConst,
}

/// Which receiver-call surface produced a receiver-access diagnostic.
///
/// WHAT: distinguishes user source receiver methods from compiler-owned collection and map
///       builtins so the shared renderer can name the receiver kind without per-kind reason
///       variants.
/// WHY: collection builtins, map builtins and source methods share one access classifier, and
///      the kind only affects the rendered noun, not the source-state logic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReceiverCallKind {
    SourceMethod,
    CollectionBuiltin,
    MapBuiltin,
}

/// Structured reasons behind the stable invalid-receiver-call diagnostic family.
///
/// WHAT: encodes the receiver source state and whether `~` was authored, independent of the
///       receiver kind. The receiver kind is carried as a separate payload fact.
/// WHY: a temporary receiver, an immutable existing place and a mutable place missing `~` are
///      distinct user mistakes with different repairs, so they must not share one reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidReceiverCallReason {
    CalledAsFreeFunction,
    MustUseParentheses,
    ConstRecordNoRuntimeCalls,
    /// An existing mutable receiver used without the required `~` marker.
    MutableReceiverMissingMarker,
    /// An existing immutable receiver used for a mutable-requiring call without `~`.
    ImmutableReceiverMutableMethod,
    /// A temporary or non-place receiver used for a mutable-requiring call without `~`.
    NonPlaceReceiverMutableMethod,
    /// `~` authored on an existing immutable receiver for a mutable-requiring call.
    MutableMarkerOnImmutableReceiver,
    /// `~` authored on a temporary or non-place receiver for a mutable-requiring call.
    MutableMarkerOnNonPlaceReceiver,
    /// `~` authored on a call that does not require mutable access.
    UnneededMutableAccessMarker,
    /// `~` authored with no receiver call following it.
    MutableMarkerOnNonReceiverCall,
    AmbiguousGenericBoundMethod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidCopyTargetReason {
    FunctionName,
    FunctionCall,
    NonPlace,
    MutableMarkerNotAllowed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidFieldAccessReason {
    ExpectedNameAfterDot,
    FieldNotMethod,
    ChoicePayloadMutation,
    ChoicePayloadDeferred,
    UnknownExternalMember,
    UnknownMember,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidMatchPatternReason {
    WildcardNotSupported,
    AsNotValid,
    NegativeLiteralNotNumeric,
    LiteralTypeUnsupported,
    ScrutineeTypeUnsupportedForRelational,
    UnitVariantHasPayload,
    PayloadVariantNeedsBindings,
    CaptureBindingMustBeFieldName,
    ExpectedLocalBindingAfterAs,
    AliasMustBeLocalBinding,
    DuplicateCaptureBinding,
    TooManyCaptureBindings,
    CaptureBindingNameMismatch,
    TooFewCaptureBindings,
    QualifierDoesNotMatchScrutinee,
    ExpectedVariantNameAfterQualifier,
    MustUseVariantNamesNotLiterals,
    MustStartWithVariantName,
    UnknownVariant,
    CaptureBindingShadowsVariable,
    NonePatternRequiresOptionalScrutinee,
    OptionValuePatternRequiresEquality,
    BareCaptureOnOptionalScrutinee,
    OptionPresentCaptureOnNonOptional,
    EmptyOptionPresentCapture,
    OptionPresentCaptureTypeAnnotation,
    MissingClosingPipe,
    ExpectedBindingInOptionPresentCapture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NonExhaustiveMatchReason {
    MissingElseArm,
    MissingVariants,
    GuardedArmsRequireElse,
    MissingOptionPatterns,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvalidFallibleHandlingReason {
    CatchOutsideBoundary,
    ExpectedCatchBlockOrHandler,
    ExpectedCatchHandlerOpeningPipe,
    ExpectedCatchHandlerIdentifier,
    ExpectedCatchHandlerClosingPipe,
    ExpectedCatchHandlerColon,
    EmptyCatchHandlerBinding,
    MultipleCatchHandlerBindings,
    FallbackValuesForErrorOnlyResult,
    CatchOnNonFallible,
    CatchOnOptional,
    BangOnNonFallible,
    BangOnOptional,
    FunctionHasNoErrorSlot,
    EntryHasIncompatibleError {
        produced_error_type: TypeId,
        entry_error_type: TypeId,
    },
    NotOptionExpression,
    FunctionHasNoOptionalReturn,
    FunctionHasMultipleSuccessReturns {
        count: usize,
    },
    OptionPropagationReturnTypeMismatch,
    OptionPropagationCatchConflict,
    ExplicitPropagationCatchConflict,
    CatchHandlerConflicts,
    CatchHandlerCanFallThrough,
    InlineCatchMultiline,
    AssertionMessageCannotEscape,
    ThenWithNoActiveValueTarget,
    ThenCrossesBlockedConstruct,
    ThenRequiresValues,
    DirectOptionFallbackSyntax,
    UnhandledErrorReturn,
    SuccessValueDiscarded,
    IncompatibleCatchErrorTypes {
        first_error_type_id: TypeId,
        second_error_type_id: TypeId,
        first_producer_span: Option<SourceSpan>,
        second_producer_span: Option<SourceSpan>,
    },
    CustomErrorMixedWithImplicitFailure {
        error_type_id: TypeId,
        typed_producer_span: Option<SourceSpan>,
        implicit_producer_span: Option<SourceSpan>,
    },
    UnhandledBuiltinFailureInCustomErrorFunction {
        error_type_id: TypeId,
        witness: Box<BuiltinFailureWitness>,
    },
    UnhandledBuiltinFailureInExportedFunction {
        witness: Box<BuiltinFailureWitness>,
    },
}

/// Boundary-inward evidence for one escaping built-in failure.
///
/// Producers retain at most the first three private call sites, then the terminal numeric/range
/// operation separately. An unresolved or cyclic chain has no invented origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltinFailureWitness {
    pub call_spans: Vec<Option<SourceSpan>>,
    pub origin_span: Option<SourceSpan>,
    pub origin: BuiltinFailureOriginKind,
    pub elided_call_hops: u32,
}

/// Which terminal producer a failure witness identifies.
///
/// WHAT: distinguishes an ordinary numeric or range operation from a compound-assignment
///       write-back that runs after already-handled RHS work, carrying the canonical
///       write-back target type for the label renderer.
/// WHY: the origin span alone cannot tell handled arithmetic from the later conversion
///      into the target; the kind keeps that distinction structured without a new
///      diagnostic family or reason code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinFailureOriginKind {
    Operation,
    CompoundWriteBack { target: TypeId },
}

impl InvalidFallibleHandlingReason {
    pub(crate) fn message(&self) -> &'static str {
        match self {
            InvalidFallibleHandlingReason::IncompatibleCatchErrorTypes { .. } => {
                "This catch protects producers with incompatible typed errors. Split the work into separately handled expressions or convert the errors explicitly."
            }

            InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure { .. } => {
                "This catch mixes a custom typed error with implicit built-in failure. Split the work into separately handled expressions or convert the errors explicitly."
            }

            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                ..
            } => {
                "Implicit built-in failure cannot escape through a custom error slot. Recover locally with catch or convert the failure explicitly with catch and return!."
            }

            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { .. } => {
                "This exported function has no error slot but implicit built-in failure may escape. Recover locally with catch or declare a final Error! return slot."
            }

            InvalidFallibleHandlingReason::CatchOutsideBoundary => {
                "`catch` is only allowed at an assignment, declaration, return or statement boundary."
            }

            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler => {
                "Expected `:` or `|err|:` catch-block syntax after `catch`."
            }

            InvalidFallibleHandlingReason::ExpectedCatchHandlerOpeningPipe => {
                "Expected `|` to start the catch handler binding."
            }

            InvalidFallibleHandlingReason::ExpectedCatchHandlerIdentifier => {
                "Expected a catch handler identifier between `|` markers."
            }

            InvalidFallibleHandlingReason::ExpectedCatchHandlerClosingPipe => {
                "Expected `|` after the catch handler identifier."
            }

            InvalidFallibleHandlingReason::ExpectedCatchHandlerColon => {
                "Expected ':' to start the catch handler scope."
            }

            InvalidFallibleHandlingReason::EmptyCatchHandlerBinding => {
                "`catch ||:` is invalid. Use `catch:` when the error value is unused."
            }

            InvalidFallibleHandlingReason::MultipleCatchHandlerBindings => {
                "`catch` accepts one optional error binding. Use `catch |err|:`."
            }

            InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult => {
                "This fallible expression has no success return values, so `catch then` fallback values are not allowed here."
            }

            InvalidFallibleHandlingReason::CatchOnNonFallible => {
                "`catch` needs a compatible recoverable failure or an authored cast, arithmetic operation or \
                 multiple calls at this expression level; this expression has none."
            }

            InvalidFallibleHandlingReason::CatchOnOptional => {
                "`catch` does not recover an optional value. Inspect the optional value with an `if ... is |present| ... else ...` expression."
            }

            InvalidFallibleHandlingReason::BangOnNonFallible => {
                "`!` propagates an `Error!` return, but this expression is not fallible."
            }

            InvalidFallibleHandlingReason::BangOnOptional => {
                "Postfix `!` propagates an `Error!` return, but this expression is optional. Use postfix `?` only inside a compatible optional-returning function, or inspect the option explicitly."
            }

            InvalidFallibleHandlingReason::FunctionHasNoErrorSlot => {
                "This expression uses '!' propagation, but the surrounding function does not declare an error return slot."
            }

            InvalidFallibleHandlingReason::EntryHasIncompatibleError { .. } => {
                "This '!' propagation produces an error incompatible with the entry's built-in start Error! boundary. Handle it locally with `catch` or recover explicitly."
            }

            InvalidFallibleHandlingReason::NotOptionExpression => {
                "The '?' option-propagation suffix is only valid for optional expressions."
            }

            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn => {
                "This expression uses '?' propagation, but the surrounding function does not return an optional success value."
            }

            InvalidFallibleHandlingReason::FunctionHasMultipleSuccessReturns { .. } => {
                "This '?' propagation requires exactly one optional success return slot. Inspect the option locally or use a single optional success return slot."
            }

            InvalidFallibleHandlingReason::OptionPropagationReturnTypeMismatch => {
                "This '?' propagation expression is not compatible with the surrounding function's optional return type."
            }

            InvalidFallibleHandlingReason::OptionPropagationCatchConflict => {
                "`catch` handles fallible expressions. Optional values must use explicit option inspection instead of `? catch`."
            }

            InvalidFallibleHandlingReason::ExplicitPropagationCatchConflict => {
                "Explicit propagation leaves the enclosing function and cannot target a surrounding catch. Split the work into statements before recovering locally."
            }

            InvalidFallibleHandlingReason::CatchHandlerConflicts => {
                "Catch handler conflicts with an existing visible declaration."
            }

            InvalidFallibleHandlingReason::CatchHandlerCanFallThrough => {
                "Catch handler without fallback can fall through while success values are required."
            }

            InvalidFallibleHandlingReason::InlineCatchMultiline => {
                "Inline `catch then` recovery must fit on a single logical line."
            }

            InvalidFallibleHandlingReason::AssertionMessageCannotEscape => {
                "Assertion messages must be ordinary values. Message construction cannot escape its evaluation through error or option propagation, return, `break`, or `continue`."
            }

            InvalidFallibleHandlingReason::ThenWithNoActiveValueTarget => {
                "`then` is only valid inside a value-producing block."
            }

            InvalidFallibleHandlingReason::ThenCrossesBlockedConstruct => {
                "`then` cannot target a value-producing block across this construct."
            }

            InvalidFallibleHandlingReason::ThenRequiresValues => "Expected a value after 'then'.",

            InvalidFallibleHandlingReason::DirectOptionFallbackSyntax => {
                "Optional values do not support direct fallback syntax. Use `if option is |value| ... else ...`."
            }

            InvalidFallibleHandlingReason::UnhandledErrorReturn => {
                "Calls to error-returning functions must be explicitly handled with postfix `!` or `catch`."
            }

            InvalidFallibleHandlingReason::SuccessValueDiscarded => {
                "This fallible expression returns success values, so it cannot be used as a standalone statement."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidTemplateSlotReason {
    InsertOutsideParentSlot,
    ExtraLooseContentWithoutDefaultSlot,
    LooseContentWithoutDefaultSlot,
    InsertCannotTargetDefaultSlot,
    InsertTargetsUnknownNamedSlot,
    InsertTargetsUnknownPositionalSlot,
    SlotDefinitionOutsideTemplateBody,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompileTimeEvaluationErrorReason {
    IntegerOverflow,
    FloatOverflow,
    DivideByZero,
    InvalidExponent,
    InvalidOperatorForType,
    ConstantSelfReference,
    ConstantNotVisible,
    NonConstantReferenceInConstant,
    SameFileForwardConstantReference,
    ConstantInitializerNotFoldable,
    ExternalNonScalarConstantInConstantContext,
    NonCompileTimeFieldInConstantContext,
    NoneLiteralRequiresOptionalTypeContext,
    ExternalTypeConstructionNotSupported,
    StructFieldDefaultNotFoldable,
    EvaluationOutputBoundExceeded,
    StructuralStringRequiresFinalText,
}

/// Diagnostic-owned exact source operator for unsupported-operand diagnostics.
///
/// WHAT: carries the authored source spelling of the operator that failed type checking,
///      independent of AST storage so the diagnostic layer never depends on `Operator`.
/// WHY: replacing the broad operator-category payload with the exact operator lets rendered
///      messages name the specific source construct the user wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    IntDivide,
    Modulus,
    Exponent,
    And,
    Or,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Equality,
    NotEqual,
    Not,
    Range,
}

impl DiagnosticOperator {
    /// Authored source spelling used in rendered diagnostics.
    pub fn source_spelling(&self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::IntDivide => "//",
            Self::Modulus => "%",
            Self::Exponent => "^",
            Self::And => "and",
            Self::Or => "or",
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => ">=",
            Self::LessThan => "<",
            Self::LessThanOrEqual => "<=",
            Self::Equality => "is",
            Self::NotEqual => "is not",
            Self::Not => "not",
            Self::Range => "to",
        }
    }
}

/// Diagnostic-owned compound-assignment operator.
///
/// WHAT: carries only the seven operators that have an authored compound-assignment form.
/// WHY: a dedicated type prevents impossible spacing facts such as `and=` from reaching the
/// renderer and keeps user input away from internal-invariant panic paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticCompoundAssignmentOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    IntDivide,
    Modulus,
    Exponent,
}

impl DiagnosticCompoundAssignmentOperator {
    /// Authored source spelling used in rendered diagnostics.
    pub fn source_spelling(&self) -> &'static str {
        match self {
            Self::Add => "+=",
            Self::Subtract => "-=",
            Self::Multiply => "*=",
            Self::Divide => "/=",
            Self::IntDivide => "//=",
            Self::Modulus => "%=",
            Self::Exponent => "^=",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IncompatibleChoiceComparisonReason {
    DifferentChoiceTypes,
    ChoiceWithNonChoice,
    PayloadEqualityNotSupported {
        field_name: StringId,
        field_type: TypeId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidRouteOutputReason {
    /// The signature's resolved `Route` output is neither a choice type nor a generic parameter,
    /// so no variant could ever satisfy the future-input contract.
    NotChoiceOrGeneric,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InvalidCallShapeReason {
    MissingArgument {
        parameter_name: Option<StringId>,
        parameter_index: usize,
    },

    ExtraPositionalArgument {
        expected_count: usize,
    },

    DuplicateArgument {
        parameter_name: Option<StringId>,
        parameter_index: usize,
    },

    NamedArgumentNotFound {
        name: StringId,
        known_parameters: Vec<StringId>,
    },

    PositionalAfterNamed,

    NamedArgumentsNotSupported,
    WireArgumentOrigin,
    WireArgumentMutableMarker,
    RouteArgumentNotDirect,
    RouteForwardingContractMismatch,
    RouteParameterRead,
    RouteInputFieldMismatch,

    MutableAccessRequired {
        parameter_name: Option<StringId>,
        parameter_index: usize,
    },

    MutableAccessNotAllowed {
        parameter_name: Option<StringId>,
        parameter_index: usize,
    },

    MutableAccessOnNonPlace {
        parameter_name: Option<StringId>,
        parameter_index: usize,
    },

    MutableAccessOnImmutablePlace {
        parameter_name: Option<StringId>,
        parameter_index: usize,
        binding_name: Option<StringId>,
    },

    /// An immutable existing place passed to a mutable parameter without `~`.
    ///
    /// WHAT: distinct from `MutableAccessOnImmutablePlace`, which covers an authored `~`.
    /// WHY: the missing marker and the authored marker are different user mistakes, so the
    /// diagnostic points at the value expression here and at the `~` marker there.
    ImmutablePlaceMutableAccessRequired {
        parameter_name: Option<StringId>,
        parameter_index: usize,
        binding_name: Option<StringId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidReturnShapeReason {
    BareReturnWithExpectedValues {
        expected_count: usize,
    },

    ReturnValuesWithBareSignature,

    TooManyReturnValues {
        expected_count: usize,
    },

    TooFewReturnValues {
        expected_count: usize,
        provided_count: usize,
    },

    MissingReturnBangValue,

    FunctionMayFallThrough,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DeferredFeatureReason {
    NamedFeature { feature: StringId },
    CaptureTaggedPattern,
    NegatedMatchPattern,
    NamedPayloadPatternAssignment,
    NestedPayloadPattern,
    ChoiceVariantDefaultValue,
    GenericReceiverMethod,
    DeclaredRegion,
    CheckedBlock,
    AsyncBlock,
    RuntimeAnonymousRecord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidPageMetadataReason {
    NotAString,
    DuplicateDeclaration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RangeOperandKind {
    Start,
    End,
    Step,
}

/// Why a range header rejected one operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidRangeOperandReason {
    /// The operand has no integer or binary-float domain.
    NotNumeric,
    /// The operand is numeric but no range domain combines it with the other operands.
    IncompatibleDomain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperatorOperandPosition {
    Unary,
    BinaryLeft,
    BinaryRight,
}

/// Structured reasons behind the stable invalid-expression diagnostic family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidExpressionReason {
    /// Two operands appeared with no operator between them.
    ExpectedOperatorBeforeExpression,
    /// Defensive evaluator fallback after structural parser checks.
    UnresolvedStackShape,
    /// A `.moth` path in expression position has no file value.
    MothFileHasNoValue,
    /// A value-position path is missing the explicit file extension the language requires.
    ExtensionlessFileValue,
    /// A runtime expression used the empty anonymous-record shape, which is not in the initial
    /// runtime surface.
    EmptyRuntimeAnonymousRecord,
    /// A record literal field was not written as `name = value`.
    AnonymousRecordFieldNotNamed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidStandaloneStatementReason {
    FieldRead,
    Expression,
    StandaloneTemplate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamespaceTypeValueMisuseKind {
    Namespace,
    Type,
    Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidMatchArmReason {
    SemicolonDelimiter,
    LegacyColonSyntax,
    LegacyElseSyntax,
    InvalidArrow,
    ArmMustStartNewLine,
    ExpectedArmHeader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidMatchGuardReason {
    SideEffectingPrelude,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidLoopHeaderReason {
    EmptyHeader,
    MissingColon,
    RemovedInSyntax,
    MissingClosingPipe,
    MalformedBindingPipes,
    MissingSourceBeforeBindings,
    EmptyBindingList,
    ThisBinding,
    BindingMustBeSymbol,
    MissingBindingComma,
    TrailingBindingComma,
    BareSingleBinding,
    BareDualBinding,
    TooManyBindings,
    DuplicateBindingName,
    BindingAlreadyDeclared,
    CollectionSourceNotCollection { found_type: Option<TypeId> },
    MissingRangeSeparator,
    MissingRangeEndBound,
    MissingRangeStep,
    FloatRangeMissingStep,
    ZeroRangeStep,
    ExpectedHeaderExpression,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidStatementPositionReason {
    UnexpectedComma,
    UnexpectedCloseParenthesis,
    UnexpectedCloseCurly,
    UnexpectedPipe,
    UnexpectedArrow,
    UnexpectedWildcard,
    AnonymousDeclaredRegion,
    GenericParameterOutsideDeclarationHeader,
    UnexpectedOf,
    UnexpectedScopeCloseInExpression,
    UnexpectedScopeCloseInTemplate,
}

/// WHAT: the exact symbolic construct whose whitespace is wrong.
/// WHY: the construct enum owns the exact operator so impossible construct/operator combinations
/// cannot be formed. Plain assignment and mutable declaration carry no operator because they are
/// not binary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolicSpacingConstruct {
    BinaryOperator {
        operator: DiagnosticOperator,
    },
    Assignment,
    CompoundAssignment {
        operator: DiagnosticCompoundAssignmentOperator,
    },
    MutableDeclaration,
}

impl SymbolicSpacingConstruct {
    /// Authored source spelling used in rendered diagnostics.
    pub fn source_spelling(&self) -> &'static str {
        match self {
            Self::BinaryOperator { operator } => operator.source_spelling(),
            Self::Assignment => "=",
            Self::CompoundAssignment { operator } => operator.source_spelling(),
            Self::MutableDeclaration => "~=",
        }
    }
}

/// WHAT: which side of a symbolic construct is missing required whitespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissingWhitespace {
    Before,
    After,
    Both,
}

/// Structured facts for a symbolic spacing diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SymbolicSpacingError {
    pub construct: SymbolicSpacingConstruct,
    pub missing: MissingWhitespace,
}

/// Which gap separates `::` from a qualified choice name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChoiceVariantSeparatorGap {
    Before,
    After,
    Both,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CommonSyntaxMistakeReason {
    EqualityOperator,
    InequalityOperator,
    LogicalAndOperator,
    LogicalOrOperator,
    BooleanBangNegation,
    ExpressionAssignment,
    RustBorrowPrefix,
    InvalidAsOperator,
    StatementLineComment,
    FunctionKeyword { keyword: StringId },
    LetOrVarKeyword,
    ConstKeyword,
    MatchKeyword,
    StructKeyword { keyword: StringId },
    SignatureParenthesisDelimiter,
    SignatureAsKeyword,
    InvalidCompileTimeBindingSpacing,
    InvalidConfigQualifierSpacing,
    InvalidMutableBindingSpacing,
    InvalidSymbolicSpacing { error: SymbolicSpacingError },
    InvalidChoiceVariantSpacing { gap: ChoiceVariantSeparatorGap },
    InvalidUnaryNegationSpacing,
    UnsupportedUnaryPlus,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InvalidGenericInstantiationReason {
    WrongArgumentCount {
        expected: usize,
        found: usize,
    },
    TypeDoesNotAcceptArguments,
    OptionTypeSyntaxNotSupported,
    ResultTypeSyntaxNotSupported,
    ExternalTypeArgumentsUnsupported,
    MissingTypeArguments,
    CannotInferArguments {
        missing_parameters: Vec<StringId>,
    },
    CannotInferFunctionArguments {
        missing_parameters: Vec<StringId>,
    },
    ConflictingInference {
        subject: GenericInferenceSubject,
        parameter_id: GenericParameterId,
        parameter_name: StringId,
        existing_type_id: TypeId,
        replacement_type_id: TypeId,
    },
    MissingTraitEvidence {
        parameter_name: StringId,
        trait_name: StringId,
        concrete_type_id: TypeId,
    },
    MissingNominalTraitEvidence {
        parameter_name: StringId,
        trait_name: StringId,
        concrete_type_id: TypeId,
    },
    RecursiveFunctionInstantiation,
    ExplicitCallTypeArgumentsUnsupported,
    GenericFunctionValueDeferred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenericInferenceSubject {
    Function,
    NominalType,
}
