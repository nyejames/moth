//! Supporting diagnostic payload reason and context types.
//!
//! WHAT: stores the typed reason enums and small payload helper records used by
//! DiagnosticPayload variants.
//! WHY: separating these supporting facts keeps the top-level payload enum readable
//! while preserving structured diagnostics across compiler stages.

use super::*;

// -------------------------------
//  Diagnostic Payload Supporting Types
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
    Path(InternedPath),
    RenderedText(StringId),
    Unknown,
}

impl DiagnosticPlace {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            DiagnosticPlace::Local(name) | DiagnosticPlace::RenderedText(name) => {
                *name = remap.get(*name);
            }

            DiagnosticPlace::Path(path) => path.remap_string_ids(remap),

            DiagnosticPlace::Unknown => {}
        }
    }
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
    HashmapConstruction,
    HashmapOperation,
    ReactiveTemplateRuntime,
    RuntimeCasts,
    CheckedNumericOperations,
    FloatFormatting,
    FloatBoundaryValidation,
    GenericRuntimeValues,
    ReactiveExternalCallSink,
    CrossModuleCalls,
    RuntimeAssertionMessages,
}

impl UnsupportedBackendFeatureReason {
    pub(crate) fn feature_name(self) -> &'static str {
        match self {
            Self::HashmapConstruction => "hashmap construction",
            Self::HashmapOperation => "hashmap operation",
            Self::ReactiveTemplateRuntime => "reactive template runtime",
            Self::RuntimeCasts => "runtime casts",
            Self::CheckedNumericOperations => "checked numeric operations",
            Self::FloatFormatting => "Float formatting",
            Self::FloatBoundaryValidation => "Float boundary validation",
            Self::GenericRuntimeValues => "generic runtime values",
            Self::ReactiveExternalCallSink => "reactive external-call sink",
            Self::CrossModuleCalls => "cross-module calls",
            Self::RuntimeAssertionMessages => "runtime assertion messages",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InvalidConfigReason {
    MissingKey,
    DuplicateKey,
    FunctionUnsupported,
    TraitDeclarationUnsupported,
    TraitConformanceUnsupported,
    TraitIncompatibilityUnsupported,
    MutableBindingUnsupported,
    PlainBindingUnsupported,
    UnsupportedStatement,
    StandaloneTemplateUnsupported,
    MissingValue,
    UnsupportedScalarValue,
    NotCompileTimeConstant,
    ValueCouldNotFold,
    UnsupportedPackageFoldersValue,
    DuplicatePackageFolder {
        folder: StringId,
    },
    InvalidPackageFolder {
        folder: Option<StringId>,
        reason: InvalidPackageFolderReason,
    },
    EmptyProjectSetting,
    UnknownKey {
        key: StringId,
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

impl InvalidConfigReason {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            Self::DuplicatePackageFolder { folder } => {
                *folder = remap.get(*folder);
            }

            Self::InvalidPackageFolder { folder, .. } => {
                if let Some(folder) = folder {
                    *folder = remap.get(*folder);
                }
            }

            Self::InvalidProjectSettingValue { value, expected } => {
                *value = remap.get(*value);
                *expected = remap.get(*expected);
            }

            Self::MissingHtmlHomepage { entry_root } => {
                *entry_root = remap.get(*entry_root);
            }

            Self::DuplicateHtmlOutputPath {
                output_path,
                entry_point,
                existing_entry_point,
            } => {
                *output_path = remap.get(*output_path);
                *entry_point = remap.get(*entry_point);
                *existing_entry_point = remap.get(*existing_entry_point);
            }

            Self::ResourceOutputPathCollision {
                output_path,
                existing_origin,
                conflicting_origin,
            } => {
                *output_path = remap.get(*output_path);
                *existing_origin = remap.get(*existing_origin);
                *conflicting_origin = remap.get(*conflicting_origin);
            }

            Self::ResourceOutputPathReserved {
                output_path,
                origin,
                artefact_kind,
            } => {
                *output_path = remap.get(*output_path);
                *origin = remap.get(*origin);
                *artefact_kind = remap.get(*artefact_kind);
            }

            Self::ConfiguredEntryRootMissing { entry_root }
            | Self::NoRootModuleEntries { entry_root } => {
                *entry_root = remap.get(*entry_root);
            }

            Self::MultipleModuleRootFiles {
                directory,
                candidates,
            } => {
                *directory = remap.get(*directory);
                for candidate in candidates {
                    *candidate = remap.get(*candidate);
                }
            }

            Self::EntryRootPackagePrefixCollision {
                prefix,
                entry_folder,
            } => {
                *prefix = remap.get(*prefix);
                *entry_folder = remap.get(*entry_folder);
            }

            Self::SourcePackageMissingRoot { prefix, root } => {
                *prefix = remap.get(*prefix);
                *root = remap.get(*root);
            }

            Self::SourcePackageMultipleRoots {
                prefix,
                root,
                candidates,
            } => {
                *prefix = remap.get(*prefix);
                *root = remap.get(*root);
                for candidate in candidates {
                    *candidate = remap.get(*candidate);
                }
            }

            Self::SourceFileFolderCollision {
                file_name,
                folder_name,
                directory,
            } => {
                *file_name = remap.get(*file_name);
                *folder_name = remap.get(*folder_name);
                *directory = remap.get(*directory);
            }

            Self::LegacyModuleRootFileName {
                file_name,
                directory,
            } => {
                *file_name = remap.get(*file_name);
                *directory = remap.get(*directory);
            }

            Self::InvalidOutputFolder { folder, .. } => {
                if let Some(folder) = folder {
                    *folder = remap.get(*folder);
                }
            }

            Self::OutputFoldersNotDistinct {
                dev_folder,
                release_folder,
            } => {
                *dev_folder = remap.get(*dev_folder);
                *release_folder = remap.get(*release_folder);
            }

            Self::OutputManifestOwnerConflict {
                output_root,
                existing_builder,
                existing_profile,
                active_builder,
                active_profile,
            } => {
                *output_root = remap.get(*output_root);
                *existing_builder = remap.get(*existing_builder);
                *existing_profile = remap.get(*existing_profile);
                *active_builder = remap.get(*active_builder);
                *active_profile = remap.get(*active_profile);
            }

            Self::UnknownKey { key } => {
                *key = remap.get(*key);
            }

            Self::InvalidConfigValueShape { expected } => {
                *expected = remap.get(*expected);
            }

            Self::MissingKey
            | Self::DuplicateKey
            | Self::FunctionUnsupported
            | Self::TraitDeclarationUnsupported
            | Self::TraitConformanceUnsupported
            | Self::TraitIncompatibilityUnsupported
            | Self::MutableBindingUnsupported
            | Self::PlainBindingUnsupported
            | Self::UnsupportedStatement
            | Self::StandaloneTemplateUnsupported
            | Self::MissingValue
            | Self::UnsupportedScalarValue
            | Self::NotCompileTimeConstant
            | Self::ValueCouldNotFold
            | Self::UnsupportedPackageFoldersValue
            | Self::EmptyProjectSetting
            | Self::ConfigImportUnsupported
            | Self::FileValuePathUnsupported => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidPackageFolderReason {
    Empty,
    AbsolutePath,
    ParentDirectorySegment,
    NestedPath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumberLiteralErrorReason {
    SeparatorNotBetweenDigits,
    MultipleDecimalPoints,
    DecimalPointNotAfterDigit,
    EndsWithSeparator,
    MissingFractionalDigits,
    UppercaseExponentMarker,
    MissingExponentDigits,
    InvalidExponentSignPlacement,
    InvalidSeparatorPlacement,
    OutsideIntRange,
    NonFiniteFloat,
    ParseOverflow,
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

impl InvalidImportPathReason {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            Self::CaseMismatch { provided, expected } => {
                *provided = remap.get(*provided);
                *expected = remap.get(*expected);
            }

            Self::PublicRoot
            | Self::CurrentDirectorySegment
            | Self::ParentDirectorySegment
            | Self::EscapesProjectRoot
            | Self::EscapesSourcePackageRoot => {}
        }
    }
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

impl InvalidCompileTimePathReason {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if let Self::CaseMismatch { provided, expected } = self {
            *provided = remap.get(*provided);
            *expected = remap.get(*expected);
        }
    }
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
    ReactiveAccessNotAllowed,
    InvalidTokenAfterName { token: TokenKind },
    ExpectedTypeAnnotation { found: TokenKind },
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
    /// Capacity expression did not fold to an `Int` value.
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

impl InvalidMapTypeReason {
    pub(crate) fn remap_string_ids(&mut self, _remap: &StringIdRemap) {
        match self {
            Self::UnsupportedKeyType { .. }
            | Self::ExcessiveInlineNesting { .. }
            | Self::EmptyMapKeyType
            | Self::EmptyMapValueType
            | Self::MultipleMapSeparators
            | Self::FixedCapacityNotAllowed => {}
        }
    }
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

impl InvalidMapLiteralReason {
    pub(crate) fn remap_string_ids(&mut self, _remap: &StringIdRemap) {
        // All variants are unit-like; no string IDs to remap.
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InvalidGenericParameterReason {
    EmptyParameterList,
    BoundsMustUseIs,
    ListMustStayWithHeader,
    InvalidToken { found: TokenKind },
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
    ReactiveSubscriptionEmpty,
    ReactiveSubscriptionMultipleSources,
    ReactiveSubscriptionComplexExpression,
    ReactiveSubscriptionNonReactiveSource,
    ReactiveSubscriptionInConstTemplate,
    ReactiveSubscriptionOutsideTemplate,
    EmptyPathInTemplateHead,
    IncompatibleHeadItem,
    HelperOutsideWrapperSlot,
    RuntimeControlFlowUnresolvedInsert,
    MissingCommaBeforeControlFlowSuffix,
    ControlFlowSuffixNotFinal,
    MissingTemplateIfCondition,
    MissingTemplateLoopHeader,
    ElseInTemplateHead,
    OrphanTemplateElse,
    OrphanTemplateElseIf,
    OrphanTemplateBreak,
    OrphanTemplateContinue,
    DuplicateTemplateElse,
    TemplateElseIfAfterElse,
    MalformedTemplateElse,
    MalformedTemplateElseIf,
    MalformedTemplateBreak,
    MalformedTemplateContinue,
    MissingTemplateElseIfCondition,
    InlineTemplateElse,
    InlineTemplateElseIf,
    InlineTemplateBreak,
    InlineTemplateContinue,
    TemplateElseInLiteralBody,
    TemplateElseIfInLiteralBody,
    TemplateLoopControlInLiteralBody,
    TemplateElseInLoopBody,
    TemplateElseIfInLoopBody,
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
    ReactiveAccessNotAllowed,
    ReactiveParameterDefaultValue,
    /// An authored `=` introduced a parameter or struct-field default but no value
    /// followed it before a top-level comma, closing pipe, newline, block end or EOF.
    MissingDefaultValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InvalidFunctionSignatureReason {
    MissingArrowOrColon { found: TokenKind },
    UnexpectedEndAfterParameters,
    MissingReturnType,
    MissingTraitRequirementReturnType,
    TrailingCommaInReturns,
    UnexpectedEndAfterComma,
    UnexpectedEndInReturns,
    MissingColonAfterReturns,
    UnexpectedArrowInReturns,
    MissingCommaOrColon { found: TokenKind },
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
        type_name: StringId,
    },
    UnsupportedType {
        function_name: StringId,
        type_name: StringId,
    },
    ReceiverMethodImportOrExportNotAllowed,
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
    RecursiveRuntimeStruct { cycle: String },
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
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
}

impl InvalidTraitConformanceReason {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            Self::MissingMethod { requirement_name }
            | Self::ReceiverMutabilityMismatch { requirement_name }
            | Self::ParameterCountMismatch {
                requirement_name, ..
            }
            | Self::ParameterModeMismatch {
                requirement_name, ..
            }
            | Self::ParameterTypeMismatch {
                requirement_name, ..
            }
            | Self::ReturnCountMismatch {
                requirement_name, ..
            }
            | Self::ReturnTypeMismatch {
                requirement_name, ..
            }
            | Self::ReturnChannelMismatch {
                requirement_name, ..
            } => {
                *requirement_name = remap.get(*requirement_name);
            }

            Self::ImportedModuleRoot
            | Self::AliasTarget
            | Self::NonCanonicalTarget
            | Self::NonlocalSourceTarget
            | Self::BuiltinTarget
            | Self::ExternalOpaqueTarget
            | Self::DuplicateCanonicalEvidence
            | Self::BuiltinEvidenceOverride => {}

            Self::IncompatibleTraitEvidence {
                incompatible_trait_name,
            } => {
                *incompatible_trait_name = remap.get(*incompatible_trait_name);
            }
        }
    }
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
    MutableMarkerOnAssignmentTarget,
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
    DuplicateTarget,
    UnsupportedRhs,
    ExistingTargetMutableMarker,
    ExistingTargetImmutable,
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
    OperandIsFallible,
    OperandArityMismatch,
    TargetArityMismatch,
    FallibleEvidenceRequiresHandling,
    InfallibleEvidenceCannotUseFallibleForm,
    PropagationRequiresErrorReturn,
    PropagationAndRecoveryConflict,
    BangMustAttachToCast,
    ScalarConstructorRemoved,
    NoEvidence,
    BuiltinEvidenceNotConstFoldable,
    UserDefinedEvidenceNotConstFoldable,
    GenericBoundEvidenceNotConstFoldable,
    BuiltinCastFailedInConst,
    CatchHandlerNotConstFoldable,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    NotOptionExpression,
    FunctionHasNoOptionalReturn,
    OptionPropagationReturnTypeMismatch,
    OptionPropagationCatchConflict,
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
}

impl InvalidFallibleHandlingReason {
    pub(crate) fn message(self) -> &'static str {
        match self {
            InvalidFallibleHandlingReason::CatchOutsideBoundary => {
                "`catch` can only handle a fallible expression at an assignment, declaration, return, or statement boundary."
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
                "`catch` handles fallible `Error!` expressions, but this expression is not fallible."
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

            InvalidFallibleHandlingReason::NotOptionExpression => {
                "The '?' option-propagation suffix is only valid for optional expressions."
            }

            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn => {
                "This expression uses '?' propagation, but the surrounding function does not return an optional success value."
            }

            InvalidFallibleHandlingReason::OptionPropagationReturnTypeMismatch => {
                "This '?' propagation expression is not compatible with the surrounding function's optional return type."
            }

            InvalidFallibleHandlingReason::OptionPropagationCatchConflict => {
                "`catch` handles fallible expressions. Optional values must use explicit option inspection instead of `? catch`."
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
    IntegerDivisionOnlyIntInt,
    ConstantSelfReference,
    ConstantNotVisible,
    NonConstantReferenceInConstant,
    SameFileForwardConstantReference,
    ConstantInitializerNotFoldable,
    ExternalNonScalarConstantInConstantContext,
    ExternalFunctionCallInConstantContext,
    NonCompileTimeFieldInConstantContext,
    NoneLiteralRequiresOptionalTypeContext,
    ExternalTypeConstructionNotSupported,
    StructFieldDefaultNotFoldable,
    StructuralStringRequiresFinalText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnsupportedOperatorCategory {
    Arithmetic,
    Comparison,
    Range,
    Logical,
    Unary,
    Other,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvalidFallibleOperandReason {
    FallibleValueNotHandled,
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

    ReactiveSourceRequired {
        parameter_name: Option<StringId>,
        parameter_index: usize,
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
}

impl DeferredFeatureReason {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if let Self::NamedFeature { feature } = self {
            *feature = remap.get(*feature);
        }
    }
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
    CollectionSourceNotCollection { found_type: TypeId },
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

/// WHAT: structured facts for a symbolic spacing diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SymbolicSpacingError {
    pub construct: SymbolicSpacingConstruct,
    pub missing: MissingWhitespace,
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
    InvalidReactiveBindingSpacing,
    InvalidSymbolicSpacing { error: SymbolicSpacingError },
    InvalidUnaryNegationSpacing,
    UnsupportedUnaryPlus,
}

impl CommonSyntaxMistakeReason {
    pub(super) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            CommonSyntaxMistakeReason::FunctionKeyword { keyword }
            | CommonSyntaxMistakeReason::StructKeyword { keyword } => {
                *keyword = remap.get(*keyword);
            }

            CommonSyntaxMistakeReason::EqualityOperator
            | CommonSyntaxMistakeReason::InequalityOperator
            | CommonSyntaxMistakeReason::LogicalAndOperator
            | CommonSyntaxMistakeReason::LogicalOrOperator
            | CommonSyntaxMistakeReason::BooleanBangNegation
            | CommonSyntaxMistakeReason::ExpressionAssignment
            | CommonSyntaxMistakeReason::RustBorrowPrefix
            | CommonSyntaxMistakeReason::InvalidAsOperator
            | CommonSyntaxMistakeReason::StatementLineComment
            | CommonSyntaxMistakeReason::LetOrVarKeyword
            | CommonSyntaxMistakeReason::ConstKeyword
            | CommonSyntaxMistakeReason::MatchKeyword
            | CommonSyntaxMistakeReason::SignatureParenthesisDelimiter
            | CommonSyntaxMistakeReason::SignatureAsKeyword
            | CommonSyntaxMistakeReason::InvalidCompileTimeBindingSpacing
            | CommonSyntaxMistakeReason::InvalidMutableBindingSpacing
            | CommonSyntaxMistakeReason::InvalidReactiveBindingSpacing
            | CommonSyntaxMistakeReason::InvalidSymbolicSpacing { .. }
            | CommonSyntaxMistakeReason::InvalidUnaryNegationSpacing
            | CommonSyntaxMistakeReason::UnsupportedUnaryPlus => {}
        }
    }
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
        current_evidence_location: SourceLocation,
        previous_evidence_location: Option<SourceLocation>,
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
