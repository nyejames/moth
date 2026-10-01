//! Builtin cast target classification and resolution metadata.
//!
//! WHAT: defines the compact `BuiltinCastTarget` enum, the `BuiltinCastFallibility`
//!      classifier, the `BuiltinCastPolicyId` policy key, and the receiving-target
//!      resolution type. Also provides pure helpers that map a `TypeId` to its
//!      builtin target classification and to a receiving-type resolution that
//!      records whether the cast should land in the inner type before optional
//!      wrapping.
//! WHY: cast resolution must be a single-source decision so parser, AST, and folding
//!      cannot drift on what counts as a builtin cast target. Keeping these helpers
//!      pure and local to builtins means the policy owner can answer the same
//!      classification questions that the constant folder and later phases will ask
//!      without adding broad context-dependent APIs.

use crate::compiler_frontend::builtins::error_type::ERROR_TYPE_NAME;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::ids::{BuiltinTypeKey, TypeId};
use crate::compiler_frontend::datatypes::number::NumberScale;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// The set of builtin types that may be a cast source or target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum BuiltinCastTarget {
    Bool,
    Int,
    String,
    Char,
    Float,
    Error,
    /// An explicit-width scalar or the `Byte` octet type.
    ///
    /// WHAT: covers every `FixedScalar` identity, including `Byte`, as a builtin cast
    ///      source or target. `Int`/`Float` never alias a matching-width fixed type.
    /// WHY: numeric conversions between `Int`, `Float` and fixed scalars resolve through
    ///      this one classification so evidence and policy owners share the same vocabulary.
    Fixed(FixedScalar),
    /// One exact decimal `Dec` scale as a cast source or target.
    ///
    /// WHAT: classifies the lazy `Dec`/`Dec0`..`Dec256` identities so cast resolution
    ///      treats them like every other builtin target. The scale is part of the identity.
    /// WHY: the scale is part of identity, so per-pair evidence resolves `Dec` conversions
    ///      without enumerating a 257-scale matrix.
    Number(NumberScale),
}

impl BuiltinCastTarget {
    /// The bounded numeric domain of this target: `Int`, `Float` or a non-`Byte` fixed scalar.
    ///
    /// `BuiltinCastTarget::Number` deliberately stays outside this projection. Its exact
    /// conversions are classified pair-by-pair in the lazy evidence path, never by the
    /// bounded numeric matrices.
    pub(crate) fn numeric_scalar(self) -> Option<NumericScalar> {
        match self {
            BuiltinCastTarget::Int => Some(NumericScalar::Int),
            BuiltinCastTarget::Float => Some(NumericScalar::Float),
            BuiltinCastTarget::Fixed(FixedScalar::Byte) => None,
            BuiltinCastTarget::Fixed(scalar) => Some(NumericScalar::Fixed(scalar)),
            BuiltinCastTarget::Bool
            | BuiltinCastTarget::String
            | BuiltinCastTarget::Char
            | BuiltinCastTarget::Error
            | BuiltinCastTarget::Number(_) => None,
        }
    }
}

impl From<NumericScalar> for BuiltinCastTarget {
    fn from(scalar: NumericScalar) -> Self {
        match scalar {
            NumericScalar::Int => BuiltinCastTarget::Int,
            NumericScalar::Float => BuiltinCastTarget::Float,
            NumericScalar::Fixed(scalar) => BuiltinCastTarget::Fixed(scalar),
            NumericScalar::Number(scale) => BuiltinCastTarget::Number(scale),
        }
    }
}

/// Whether a builtin cast is infallible or fallible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum BuiltinCastFallibility {
    Infallible,
    Fallible,
}

/// Resolved compiler-owned builtin cast policy.
///
/// WHAT: carries the resolved builtin conversion policy and its complete numeric domains from
///       evidence selection through folding and HIR to runtime lowering.
/// WHY: callers consume the selected policy rather than reconstructing the source/target rules
///      owned by `casts::evidence` and implemented by `casts::policies`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BuiltinCastPolicyId {
    /// Numeric conversion between bounded scalars or eligible exact decimal `Dec` pairs.
    ///
    /// This reuses the bounded numeric policy for ordinary bounded pairs while
    /// `NumericScalar::Number` branches keep arbitrary-precision values out of `ExactNumericValue`.
    NumericConversion {
        source: NumericScalar,
        target: NumericScalar,
    },
    /// Infallible `Byte` to `U8` conversion.
    ByteToU8,
    /// Infallible `U8` to `Byte` conversion.
    U8ToByte,
    /// Infallible numeric text conversion for `Int`, `Float`, fixed numeric scalars and `Dec`.
    ///
    /// WHAT: carries the numeric domain whose canonical text the value formats to. Never holds
    ///       `Byte`, whose text conversion composes through `U8`.
    /// WHY: every numeric type shares one text contract, so the policy carries the domain and the
    ///      policy owner applies that domain's formatting rules.
    NumericToString(NumericScalar),
    /// Fallible numeric text parse for `Int`, `Float`, fixed numeric scalars and `Dec`.
    ///
    /// The destination domain owns the accepted grammar, range, precision or exact Dec scale.
    StringToNumeric(NumericScalar),
    BoolToString,
    CharToString,
    CharToInt,
    StringToError,
    ErrorToString,
    IntToChar,
    StringToBool,
    StringToChar,
}

impl BuiltinCastPolicyId {
    /// Returns whether AST constant folding can materialize this policy's result today.
    ///
    /// WHAT: marks the subset of pure builtin policies whose policy-space result maps back to an
    /// AST compile-time expression without extra nominal/const-record construction.
    /// WHY: `String -> Error` and `Error -> String` require a compile-time representation for the
    /// builtin `Error` struct before folding can be correct. Keeping that marker beside the policy
    /// id prevents the constant folder from silently preserving a runtime cast in const-required
    /// contexts.
    pub(crate) fn is_const_foldable(self) -> bool {
        !matches!(self, Self::StringToError | Self::ErrorToString)
    }
}

/// Resolution of an explicit cast target as seen from a receiving-type position.
///
/// WHAT: records the resolved builtin target alongside a flag describing whether
///      the cast should land in the inner type (because the receiving context is
///      `T?` or another optional wrapper) and is responsible for re-wrapping the
///      value afterwards.
/// WHY: optional receiving contexts must cast to the inner builtin type so existing
///      optional wrapping can finish the job. Flagging that here keeps the AST
///      builder from having to re-discover the optional structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CastTargetResolution {
    pub(crate) target: BuiltinCastTarget,
    pub(crate) requires_optional_wrap_after_cast: bool,
}

/// Classifies builtin type identities without consulting source names or path state.
///
/// Dec scales are resolved through their existing environment-owned identities and stay
/// exact; source nominal types are intentionally excluded.
pub(crate) fn builtin_cast_target_for_builtin_type(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Option<BuiltinCastTarget> {
    let builtins = type_environment.builtins();

    if type_id == builtins.bool {
        return Some(BuiltinCastTarget::Bool);
    }
    if type_id == builtins.int {
        return Some(BuiltinCastTarget::Int);
    }
    if type_id == builtins.string {
        return Some(BuiltinCastTarget::String);
    }
    if type_id == builtins.char {
        return Some(BuiltinCastTarget::Char);
    }
    if type_id == builtins.float {
        return Some(BuiltinCastTarget::Float);
    }

    // Fixed scalars (including `Byte`) are distinct seeded identities. A direct
    // definition check keeps optional unwrapping in the receiving-type caller below.
    if let Some(TypeDefinition::Builtin(builtin)) = type_environment.get(type_id)
        && let BuiltinTypeKey::FixedScalar(scalar) = builtin.key
    {
        return Some(BuiltinCastTarget::Fixed(scalar));
    }

    type_environment
        .number_scale(type_id)
        .map(BuiltinCastTarget::Number)
}

/// Returns the builtin target classification for a type, if it is a supported
/// cast source or target.
///
/// WHAT: maps builtin identities through [`builtin_cast_target_for_builtin_type`] and resolves
///      `Error` by matching the type's nominal path against the preseeded builtin error path.
///      `Int`/`Float` never alias a matching-width fixed type.
/// WHY: classification is shared between cast target resolution and evidence
///      construction. Centralising it here means the policy table can stay
///      table-driven while still relying on one well-defined mapping rule.
pub(crate) fn builtin_cast_target_for_type(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> Option<BuiltinCastTarget> {
    if let Some(target) = builtin_cast_target_for_builtin_type(type_id, type_environment) {
        return Some(target);
    }

    let path = type_environment.nominal_path(type_id)?;

    if path_fork
        .component(*path)
        .is_some_and(|name| string_table.resolve(name) == ERROR_TYPE_NAME)
    {
        return Some(BuiltinCastTarget::Error);
    }

    None
}

/// Resolves a receiving type to a builtin cast target and records whether the
/// cast must land in the inner type before optional wrapping re-applies.
///
/// WHAT: maps the receiving type to its builtin target and detects the optional
///      wrapper shape so callers know whether the cast should land in the inner
///      builtin type rather than in the optional itself.
/// WHY: optional receiving contexts cast to the inner builtin type and let the
///      existing optional wrapping path finish the job. Detecting that here keeps
pub(crate) fn cast_target_for_receiving_type(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> Option<CastTargetResolution> {
    if let Some(target) =
        builtin_cast_target_for_type(type_id, type_environment, string_table, path_fork)
    {
        return Some(CastTargetResolution {
            target,
            requires_optional_wrap_after_cast: false,
        });
    }

    let inner = type_environment.option_inner_type(type_id)?;
    let target = builtin_cast_target_for_type(inner, type_environment, string_table, path_fork)?;

    Some(CastTargetResolution {
        target,
        requires_optional_wrap_after_cast: true,
    })
}
