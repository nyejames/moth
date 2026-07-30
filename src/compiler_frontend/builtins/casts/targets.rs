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
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
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
}

/// Whether a builtin cast is infallible or fallible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum BuiltinCastFallibility {
    Infallible,
    Fallible,
}

/// Stable policy identifier for one row in the initial builtin evidence table.
///
/// WHAT: every initial builtin evidence row gets exactly one variant so policy
///      lookup stays table-driven rather than rebuilding match logic on every
///      call site. The single source of truth is
///      `compiler_frontend::builtins::casts::evidence::INITIAL_BUILTIN_EVIDENCE_ROWS`.
/// WHY: callers should ask the policy owner for a known policy and not re-derive
///      source/target combinations inline at AST or folding time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BuiltinCastPolicyId {
    IntToFloat,
    IntToString,
    FloatToString,
    BoolToString,
    CharToString,
    CharToInt,
    StringToError,
    ErrorToString,
    FloatToInt,
    IntToChar,
    StringToInt,
    StringToFloat,
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

/// Returns the builtin target classification for a type, if it is a supported
/// cast source or target.
///
/// WHAT: maps `Bool`, `Int`, `String`, `Char`, and `Float` to their builtin target
///      variants using `TypeEnvironment::builtins()`. Resolves `Error` by matching
///      the type's nominal path against the preseeded builtin error path.
/// WHY: classification is shared between cast target resolution and evidence
///      construction. Centralising it here means the policy table can stay
///      table-driven while still relying on one well-defined mapping rule.
pub(crate) fn builtin_cast_target_for_type(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
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

    let path = type_environment.nominal_path(type_id)?;

    if path.name_str(string_table) == Some(ERROR_TYPE_NAME) {
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
///      AST construction free of bespoke optional-detection logic.
pub(crate) fn cast_target_for_receiving_type(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
) -> Option<CastTargetResolution> {
    if let Some(target) = builtin_cast_target_for_type(type_id, type_environment, string_table) {
        return Some(CastTargetResolution {
            target,
            requires_optional_wrap_after_cast: false,
        });
    }

    let inner = type_environment.option_inner_type(type_id)?;
    let target = builtin_cast_target_for_type(inner, type_environment, string_table)?;

    Some(CastTargetResolution {
        target,
        requires_optional_wrap_after_cast: true,
    })
}
