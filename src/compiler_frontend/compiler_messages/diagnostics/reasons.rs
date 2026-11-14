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

/// Distinguishes unknown-type diagnostic sites so renderers can apply site-correct extras.
///
/// WHAT: type annotations compare builtin spellings; remaining namespace-member and
/// conformance sites do not.
/// WHY: `NamespaceMember` covers a missing namespace table, an unknown root, or a
/// recorded type member whose source declaration is absent. Conformance targets
/// cannot be builtin types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnknownTypeNameSite {
    TypeAnnotation,
    NamespaceMember,
    ConformanceTarget,
}

/// Distinguishes a source-file namespace record from an external package namespace.
///
/// WHAT: unknown-member diagnostics keep the origin of the exact resolved node.
/// WHY: source records stay shallow while external packages may expose child namespaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamespaceRecordKind {
    Source,
    ExternalPackage,
}

/// Role of the failing segment on a namespace path.
///
/// WHAT: filters candidate members so an intermediate `.` only suggests child
/// namespaces, a value-position leaf only suggests values, and a type-position
/// leaf only suggests types.
/// WHY: suggesting a type or child namespace as a value would teach invalid Moth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamespaceMemberRole {
    Value,
    Type,
    ChildNamespace,
}

/// Distinguishes a bare type name in value position from a generic type application.
///
/// WHAT: `TYPE_USED_AS_VALUE` keeps one family; this reason selects ordinary versus
///       `of`-application wording without a second stable code.
/// WHY: a generic application is still a type-as-value mistake, but the correction
///      is constructor syntax for a struct or a variant for a choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeUsedAsValueReason {
    BareTypeName,
    GenericStructApplication,
    GenericChoiceApplication,
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
    /// A map literal key coerced to the map's key type.
    MapKey,
    /// A map literal value coerced to the map's value type.
    MapValue,
    /// A collection element inside a `catch` recovery value.
    CatchRecoveryCollectionElement,
    /// A map key inside a `catch` recovery value.
    CatchRecoveryMapKey,
    /// A map value inside a `catch` recovery value.
    CatchRecoveryMapValue,
    StructFieldDefault,
    TemplateInterpolation,
    MatchScrutinee,
    MatchPattern,
    ErrorReturn,
    Pattern,
    General,
    CatchRecovery,
}

impl TypeMismatchContext {
    /// Whether this context names a value recovered by a `catch` handler.
    ///
    /// WHAT: recognises the direct recovery context and every literal slot nested inside it.
    /// WHY: recovery wording and guidance apply to the nested collection and map slots too, and a
    ///      ternary flag at each render or composition site would drift from the enum.
    pub(crate) fn is_catch_recovery(self) -> bool {
        matches!(
            self,
            Self::CatchRecovery
                | Self::CatchRecoveryCollectionElement
                | Self::CatchRecoveryMapKey
                | Self::CatchRecoveryMapValue
        )
    }
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

/// Why a declaration authored in one root file cannot be resolved from another source file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RootLocalDeclarationRole {
    /// The declaration is authored in `@mod.moth`, the normal API facade.
    NormalFacade,
    /// The declaration is authored in `@package.moth`, the support API facade.
    SupportFacade,
    /// The declaration is authored in `@entry.moth`, the start-bearing entry root.
    EntryRoot,
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
    PayloadFreeVariantValues,
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
            Self::PayloadFreeVariantValues => "payload-free variant values",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InvalidConfigReason {
    MissingKey,
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
    /// A `$config` input dependency targeted a grouped-project field whose schema policy is
    /// fixed-only.
    ConfigQualifierFixedField,
    /// A source or project field's name cannot become a build-config contract.
    ConfigContractNameInvalid,
    /// A `$config` modifier could not attach to a top-level source compile-time declaration.
    ConfigQualifierInvalidPlacement,
    /// A filesystem module or source-package prefix claimed the reserved `@project` root.
    ProjectGlobalsNameReserved,
    /// The marked binding's declared type is outside the primitive/optional contract vocabulary.
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
    EntryRootPackagePrefixCollision {
        prefix: StringId,
        entry_folder: StringId,
    },
    SourcePackageMissingRoot {
        prefix: StringId,
        root: StringId,
    },
    NoRootModuleEntries {
        entry_root: StringId,
    },
    AliasedModuleRootFiles {
        first_root: StringId,
        second_root: StringId,
        target: StringId,
    },
    ModuleRootFileOutsideClaimingModule {
        root_file: StringId,
        claiming_directory: StringId,
        target: StringId,
    },
    /// A canonical root filename names a directory or other non-file entry.
    ///
    /// The diagnostic keeps the authored marker, its containing directory and the canonical
    /// target so the malformed filesystem structure is visible without falling back to a
    /// missing-root error. Retired marker spellings are rejected earlier with
    /// [`InvalidConfigReason::InvalidModuleRootFileName`].
    ModuleRootFileNotRegularFile {
        root_file: StringId,
        directory: StringId,
        target: StringId,
    },
    ConfigImportUnsupported,
    FileValuePathUnsupported,
    SourceFileFolderCollision {
        file_name: StringId,
        folder_name: StringId,
        directory: StringId,
    },
    /// An authored `.moth` filename uses a special marker outside the accepted root vocabulary.
    ///
    /// The closed vocabulary is `@mod.moth`, `@package.moth` and `@entry.moth`; `config.moth`
    /// remains the one build-only special source name. Every other `@*.moth`, `+*.moth` and
    /// `#*.moth` marker is a structure error that names the file and lists the three accepted
    /// root files. The renderer never infers a replacement role from the filename.
    InvalidModuleRootFileName {
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
    /// A multi-component directory output path has no `_`-prefixed directory component.
    VisibleDirectoryOutputPath,
    ResolvesOutsideProjectRoot,
}

/// The annotation directive that owns one external JavaScript binding site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExternalJsAnnotationDirective {
    Sig,
    Const,
}

/// The JavaScript declaration form an external annotation binds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExternalJsDeclarationKind {
    Function,
    BlockArrow,
    /// An initialiser that starts with an unsigned decimal digit.
    Constant,
}

/// Structured facts for a malformed external JavaScript module.
///
/// Compiler-authored receiver diagnostics carry the offending Moth name. Parser diagnostics keep
/// their foreign tool detail opaque and interned so the compiler relays it without re-authoring it.
/// A source file that is not valid UTF-8 carries the decoder's description of the invalid sequence.
/// Annotation binding failures carry the directive and the nearest declaration form, so the
/// provider never flattens them into prose. Their authored JavaScript ranges travel as exact
/// source spans on the diagnostic rather than as line facts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InvalidExternalModuleReason {
    ReceiverMethod {
        moth_name: StringId,
    },
    ParserDiagnostic {
        parser_detail: StringId,
    },
    /// An external signature combines the whole Never contract with other return syntax.
    MixedNeverReturnContract,
    NonUtf8Source {
        utf8_detail: StringId,
    },
    /// Package paths reserve `#`, so a logical source path containing it cannot be registered.
    PathContainsPackageSeparator,
    /// An annotation binds to the nearest JavaScript declaration, which lacks `export`.
    MissingExportKeyword {
        directive: ExternalJsAnnotationDirective,
        declaration: ExternalJsDeclarationKind,
        moth_name: StringId,
        js_name: StringId,
    },
    /// No supported JavaScript declaration directly follows the annotation.
    OrphanedAnnotation {
        directive: ExternalJsAnnotationDirective,
        moth_name: StringId,
    },
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
    /// WHY: module-root filenames such as `@entry.moth` are fixed structural markers, not
    ///      dependency names. The module's dependency identity comes from its directory.
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
pub enum InvalidImportPathReason {
    PublicRoot,
    CurrentDirectorySegment,
    ParentDirectorySegment,
    EscapesProjectRoot,
    EscapesSourcePackageRoot,
    /// A proven intermediate dependency-path component names a hidden source directory.
    HiddenDirectory,
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
    /// The target lies inside a `_`-prefixed directory excluded from source discovery.
    HiddenPath,
    /// The target is the reserved `config.moth` source, which is not a module helper.
    ReservedConfigFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeAnnotationContext {
    DeclarationTarget,
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
    /// `$slot` received an invalid target declaration.
    InvalidSlotTarget,
    /// `$slot ... = ...` received a default that is not a compile-time `String`.
    InvalidSlotDefault,
    /// `$in` received an invalid slot name declaration.
    InvalidInsertTarget,
    /// `$before` or `$after` received a value that is not a compile-time
    /// `String` or constant template wrapper.
    InvalidChildDecoratorValue,
    /// A known directive appeared in a context its definition does not allow.
    WrongContext,
    /// The same source modifier was attached twice to one item.
    DuplicateModifier,
    /// A source modifier reached a blank line or file end before an eligible target.
    DanglingModifier,
    /// A source modifier attached to a declaration or dependency clause its contract does not
    /// accept.
    WrongTarget,
    /// Two same-line source directives were written without whitespace between them.
    MissingSeparator,
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

/// Which frontend formatter validator emitted a malformed-template warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MalformedTemplateReason {
    Css(CssTemplateWarning),
    Html(HtmlTemplateWarning),
}

/// Why two meaningful items cannot share one template head.
///
/// Each variant names one registered compatibility rule, so the diagnostic can explain the
/// semantic conflict instead of repeating the directive-list policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TemplateHeadConflictReason {
    /// Two formatter directives, including the same formatter twice, control how the template
    /// body is formatted.
    MultipleFormatters,
    /// A directive may appear at most once in one head.
    DuplicateExclusiveDirective,
    /// `$note` discards its body, so nothing else may be rendered in the head.
    DiscardedCommentDirectiveMustBeOnlyMeaningfulItem,
    /// A slot declaration must be the only meaningful item in its template head.
    SlotDirectiveMustBeExclusive,
}

/// Which item declared the compatibility rule that rejected the pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TemplateHeadConflictOwner {
    /// The rule that fires when an earlier item blocks a later one.
    EarlierItem,
    /// The rule that fires when the incoming item requires something to be absent.
    IncomingItem,
}

/// The kind of a meaningful template-head item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TemplateHeadItemKind {
    Directive,
    Expression,
}

/// One meaningful template-head item as a diagnostic names it.
///
/// Directive names stay interned identities; renderers add the `$` marker. Expressions cover
/// every non-directive item the head grammar accepts (values, paths, `this`, literals and
/// inlined template references).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TemplateHeadItemName {
    Directive(StringId),
    Expression,
}

impl TemplateHeadItemName {
    pub(crate) const fn kind(self) -> TemplateHeadItemKind {
        match self {
            Self::Directive(_) => TemplateHeadItemKind::Directive,
            Self::Expression => TemplateHeadItemKind::Expression,
        }
    }
}

/// Both sides of one rejected template-head item pair.
///
/// `owner` names the item whose registered rule rejected the pair, so the renderer can lead with
/// the directive that owns the restriction. The name list passed to the family constructor is
/// ordered incoming first, then earlier; each kind says whether its name is present.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TemplateHeadConflict {
    pub reason: TemplateHeadConflictReason,
    pub owner: TemplateHeadConflictOwner,
    pub earlier: TemplateHeadItemName,
    pub earlier_span: SourceSpan,
    pub incoming: TemplateHeadItemName,
    pub incoming_span: SourceSpan,
}

impl TemplateHeadConflict {
    /// The interned directive names in family storage order: incoming, then earlier.
    ///
    /// Expression items contribute no name. The stored conflict's kinds record which entries the
    /// family string list holds.
    pub(crate) fn head_item_names(self) -> Vec<StringId> {
        let mut names = Vec::with_capacity(2);
        if let TemplateHeadItemName::Directive(name) = self.incoming {
            names.push(name);
        }
        if let TemplateHeadItemName::Directive(name) = self.earlier {
            names.push(name);
        }
        names
    }
}

/// The compact shape of one template-head conflict: rule identity and both item kinds.
///
/// Directive names live in the family's string-id list; the kinds say which entries it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoredTemplateHeadConflict {
    pub reason: TemplateHeadConflictReason,
    pub owner: TemplateHeadConflictOwner,
    pub incoming_kind: TemplateHeadItemKind,
    pub earlier_kind: TemplateHeadItemKind,
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
    FallibleValueInTemplateHead,
    UnsupportedTypeInTemplateHead {
        type_id: TypeId,
    },
    RuntimeTemplateInConst,
    RuntimeValueInConstTemplateHead,
    EmptyPathInTemplateHead,
    IncompatibleHeadItem {
        conflict: TemplateHeadConflict,
    },
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
    TemplateLoopStepNotConst,
    TemplateLoopStepIsZero,
    TemplateLoopStepNoProgress,
    TemplateLoopConditionNotConst,
    TemplateConditionalLoopConstTrue,
    TemplateLoopBodyNotConst,
    TemplateConstLoopExpansionLimitExceeded {
        limit: usize,
    },
    /// A direct top-level `$in name` target is not registered by the active builder.
    BuilderRootSlotNotRegistered,
    /// A `$md` section marker does not follow the `@#id` grammar.
    InvalidMarkdownSectionMarker,
    /// Two section markers appear on the same Markdown heading, paragraph or list item.
    DuplicateMarkdownSectionMarker {
        earlier_span: Option<SourceSpan>,
    },
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
    NeverCannotCombine { found: DiagnosticToken },
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

/// Why an authored scope opener or closer violates the physical-line layout contract.
///
/// WHAT: names the one structural mistake the diagnostic reports, at its authored token.
/// WHY: `;` layout failures share one family but need different advice: a nested opener must move
///      to its own line, a multiline closer must stand alone, trailing code must move down, and a
///      stacked closer must be removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidScopeLayoutReason {
    /// A second semicolon-terminated scope opened on the same physical line as an active one.
    NestedScopeOpenerOnLine,
    /// A multiline scope's closer shares its line with authored code other than its opener.
    MultilineCloserAfterCode,
    /// Authored code follows a scope closer on the same physical line.
    ContentAfterCloser,
    /// More than one scope closer occupies one physical line.
    AdditionalCloserOnLine,
}

/// The authored source construct whose closing `;` the file ended without.
///
/// WHAT: names the still-open construct that an end-of-file report must describe.
/// WHY: the parser that recognises the opener already knows which construct it began. Carrying
///      that typed value beside the opening site stops the end-of-file path from guessing the
///      construct from the surrounding tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceScopeConstruct {
    /// A function body opened by the signature `:`.
    FunctionBody,
    /// An `if` body opened by its header `:`.
    IfBlock,
    /// A `loop` body opened by its header `:`.
    LoopBlock,
    /// A `catch` handler opened by its header `:`.
    CatchHandler,
    /// A match body opened by the `is:` of its header.
    MatchBlock,
    /// A choice declaration opened by `::`.
    ChoiceDeclaration,
    /// A trait declaration opened by its `must:` colon.
    TraitDeclaration,
    /// A declared region opened by the line-initial `name:`.
    DeclaredRegion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReservedNameOwner {
    BuiltinType,
    Keyword,
    CoreTrait,
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
    ConstFunctionCannotBeGeneric,
    ConstFunctionCannotBeReceiverMethod,
    ConstFunctionUnsupportedParameter { parameter_name: StringId },
    ConstFunctionCannotReturnError,
    ConstFunctionCannotReturnNever,
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
    ReturnContractMismatch {
        requirement_name: StringId,
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
    ScalarTypeCalledAsFunction,
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
    NonePatternRequiresOptionalScrutinee,
    BareCaptureOnOptionalScrutinee,
    OptionalPresentCaptureOnNonOptional,
    EmptyOptionalPresentCapture,
    OptionalPresentCaptureTypeAnnotation,
    MissingClosingPipe,
    ExpectedBindingInOptionalPresentCapture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NonExhaustiveMatchReason {
    MissingElseArm,
    MissingVariants,
    GuardedArmsRequireElse,
    MissingOptionalPatterns,
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
    FallibleSuffixOnAssertion,
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
    CatchHandlerCanFallThrough,
    InlineCatchMultiline,
    AssertionMessageCannotEscape,
    ThenWithNoActiveValueTarget,
    ThenCrossesBlockedConstruct,
    ThenInsideLoopBody,
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
    UnhandledBuiltinFailureInNeverFunction {
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

            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInNeverFunction { .. } => {
                "Implicit built-in failure cannot escape a `-> !` function. Recover locally with catch."
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
                "`catch` has nothing to recover here. This expression has no compatible recoverable \
                 failure, and it is not an authored safety-catch site such as a cast, arithmetic \
                 operation or more than one call at this expression level."
            }

            InvalidFallibleHandlingReason::CatchOnOptional => {
                "`catch` does not recover an optional value. Inspect the optional value with an `if` present-value capture."
            }

            InvalidFallibleHandlingReason::BangOnNonFallible => {
                "`!` propagates an `Error!` return, but this expression is not fallible."
            }

            InvalidFallibleHandlingReason::BangOnOptional => {
                "Postfix `!` propagates an `Error!` return, but this expression is an optional value. Use postfix `?` only inside a compatible optional-returning function, or inspect the optional value explicitly."
            }

            InvalidFallibleHandlingReason::FallibleSuffixOnAssertion => {
                "An assertion statement cannot take a `catch` handler or a `!` propagation suffix. Assertions report invariants rather than fallible values, so remove the suffix."
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
                "`then` is only valid inside a value-producing `if`, full match or `catch` recovery that has an active receiving site."
            }

            InvalidFallibleHandlingReason::ThenCrossesBlockedConstruct => {
                "`then` cannot cross a loop, catch handler or template to target a value-producing block."
            }

            InvalidFallibleHandlingReason::ThenInsideLoopBody => {
                "This loop is not a value-producing loop, so its body cannot produce a value with `then`. Assign the value or return it instead."
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
    NamedInsertOutsideParentSlot,
    ExtraLooseContentWithoutDefaultSlot,
    LooseContentWithoutDefaultSlot,
    InsertCannotTargetDefaultSlot,
    InsertTargetsUnknownNamedSlot,
    InsertTargetsUnknownPositionalSlot,
    SlotDefinitionOutsideTemplateBody,
    /// A `$before`/`$after` value contains the default `$slot`.
    DefaultSlotInChildDecorator,
    /// A `$before`/`$after` value contains a positional `$slot n`.
    PositionalSlotInChildDecorator,
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
    ConstCallRequiresConstantArguments,
    ConstFunctionRuntimeOperation,
    ConstFunctionExecutionUnsupported,
    ConstWorkLimitExceeded,
    ConstMemoryLimitExceeded,
    ConstHardSafetyLimitExceeded,
    ConstAssertionFailed,
    SameFileForwardConstantReference,
    ConstantInitializerNotFoldable,
    ExternalNonScalarConstantInConstantContext,
    NonCompileTimeFieldInConstantContext,
    NoneLiteralRequiresOptionalTypeContext,
    ExternalTypeConstructionNotSupported,
    StructFieldDefaultNotFoldable,
    EvaluationOutputBoundExceeded,
    StructuralStringRequiresFinalText,
    /// Integer overflow inside the `I32` exponent operand of a `Dec` power.
    ///
    /// WHAT: distinguishes the exponent's own `I32` arithmetic from an ordinary integer
    ///       overflow in the surrounding expression.
    /// WHY: a `Dec` power takes an `I32` exponent, so the exponent subtree is evaluated at
    ///      `I32` width before the `Dec` operation runs. The reader needs that rule, not a bare
    ///      integer overflow.
    DecExponentIntegerOverflow,
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

    /// Extra positional argument past the declared slot count.
    ///
    /// Compacted as a `CountPair` in the call-shape `count` word: expected in
    /// the low 16 bits, provided in the high 16 bits. The unused `parameter_name`
    /// slot stays an `OptStringFact` for other reasons.
    ExtraPositionalArgument {
        expected_count: usize,
        provided_count: usize,
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

    /// A call whose contract is `-> !` appeared where a value is required.
    ///
    /// WHAT: names a Never call in a declaration, return, argument, condition, operator,
    ///       constructor, collection, map, template, cast or assertion position.
    /// WHY: a Never call has no success value, so every value context must report the same
    /// targeted placement mistake instead of treating the call as an empty result.
    NeverCallUsedAsValue,

    /// A Never call was followed by postfix `!`, `?`, `catch` or member access.
    ///
    /// WHAT: names handling or continuation syntax authored after a call that never returns.
    /// WHY: there is no success, error, optional or receiver value to handle, so the syntax has no
    /// meaning and the call must stay a standalone terminating statement.
    NeverCallHandled,
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

    /// A body declared `-> !` can complete normally on a reachable active path.
    NeverBodyMayCompleteNormally,

    /// A body declared `-> !` has a reachable active path that returns to its caller.
    NeverBodyReturnsToCaller,

    /// An unannotated function body diverges on every reachable active path.
    ///
    /// WHAT: the omitted return contract states an ordinary completion the body never reaches.
    /// WHY: the author must choose the permanent `-> !` contract or the ordinary type this
    ///      temporary implementation will produce, so the omission cannot silently become Never.
    UnannotatedFunctionAlwaysDiverges,

    /// A `-> !` proof depends on its own unvalidated Never-call group.
    ///
    /// WHAT: names a self-edge or strongly connected group whose divergence proof needs the
    ///       contracts inside that same group.
    /// WHY: accepting the cycle would validate each callable against an unproven peer. The source
    ///      must make one member's divergence independent of the cycle before the group is valid.
    NeverProofDependsOnCycle,
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

/// Why one HTML builder root-slot contribution cannot be interpreted.
///
/// WHAT: the narrow set of structural-value rejections owned by the HTML builder.
/// WHY: the compiler routes the contribution and the builder validates slot-specific meaning;
///      a resource-bearing slot must explain which part of that meaning the contribution missed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidHtmlSlotContributionReason {
    /// The contribution carries no resource origin at all.
    MissingResource,
    /// The contribution carries literal text, such as a rendered path or external URL.
    NotAStaticResource,
    /// The contribution carries more than one resource origin.
    MultipleResources,
    /// The contribution uses the site root, which names no tracked resource.
    SiteRoot,
    /// The single resource origin does not name a CSS file.
    NonCssResource,
}

/// Which operand position one loop header rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoopOperandKind {
    Start,
    End,
    Step,
}

/// Why a loop header rejected one operand.
///
/// WHAT: the range bounds and the shared range or collection step rules.
/// WHY: both loop kinds take an independent signed step, so one loop-owned family reports every
///      operand rejection at its authored position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidLoopOperandReason {
    /// The bound has no integer or binary-float domain.
    BoundNotNumeric,
    /// The bound is numeric but no range domain combines it with the other bound.
    BoundIncompatibleDomain,
    /// An integer range's step or any collection step is not one of the independent signed
    /// fixed-width integer types.
    ///
    /// The step keeps its own type and never changes the unsigned or signed counter or the
    /// collection source type, so an unsigned, floating-point, `Byte`, `Dec` or non-numeric step
    /// is rejected at the step position.
    StepNotSignedInteger,
    /// A binary-float range's step cannot convert safely to the counter precision.
    ///
    /// The step must be a binary float at the same or narrower precision than the counter. A typed
    /// integer step never becomes a float step, and a wider float step would narrow the authored
    /// precision.
    StepNotConvertibleToFloatCounter,
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
    Comparison,
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
    InvalidArrow,
    ArmMustStartNewLine,
    ExpectedArmHeader,
    MissingBody,
    ExpressionBodyInStatementMatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidMatchGuardReason {
    SideEffectingPrelude,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidLoopHeaderReason {
    EmptyHeader,
    MissingColon,
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
    CollectionSourceNotCollection { found_type: Option<TypeId> },
    MissingRangeSeparator,
    MissingRangeEndBound,
    MissingRangeStep,
    FloatRangeMissingStep,
    ZeroLoopStep,
    MissingCollectionStep,
    RepeatedLoopStep,
    StepOnConditionalLoop,
    MisplacedLoopStep,
    BindingsOnConditionalLoop,
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
    UnexpectedScopeCloseAtTopLevel,
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
