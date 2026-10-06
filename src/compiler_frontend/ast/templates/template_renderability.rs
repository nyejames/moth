//! Template-head value renderability classification.
//!
//! WHAT: determines whether a semantic type is allowed as a non-template,
//!       non-path expression in a template head.
//! WHY: template-head validation must use semantic `TypeId` identity through
//!      `TypeEnvironment`, not parse-time `DataType` representations.

use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use moth_lexical::numeric::fixed_scalar::FixedScalarClass;

/// Returns `true` if `type_id` is a scalar or textual type that can be
/// rendered directly into template output.
///
/// WHAT: accepts the built-in scalar/textual types that the compiler
///       supports for template rendering, plus the profile-sized `Uint`,
///       fixed numeric scalars (`I8`-`U64` and `F16`-`F64`) and lazily
///       interned `Dec` scales, which render through the canonical numeric
///       text contract.
/// WHY: positive list keeps the policy explicit and easy to extend.
///
/// Allowed: String, Int, Uint, Float, Bool, Char, fixed numeric scalars and Dec scales.
/// Rejected: `Byte`, structs, const records, choices, collections, functions,
///           external opaque types, trait names, generic instances,
///           generic parameters, and other builtin types such as Range and None.
pub(crate) fn is_template_renderable_type(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> bool {
    let builtins = type_environment.builtins();
    if type_id == builtins.string
        || type_id == builtins.int
        || type_id == builtins.uint
        || type_id == builtins.float
        || type_id == builtins.bool
        || type_id == builtins.char
    {
        return true;
    }
    if type_environment.number_scale(type_id).is_some() {
        return true;
    }

    // `Byte` is an octet, not a number, so it stays out of the numeric text contract.
    type_environment
        .fixed_scalar(type_id)
        .is_some_and(|scalar| scalar.class() != FixedScalarClass::Octet)
}
