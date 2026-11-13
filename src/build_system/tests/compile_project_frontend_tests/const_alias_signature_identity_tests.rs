//! Focused identity check for repeated const-function imports.
//!
//! WHY: an imported const function can be materialised once and then reused under a second local
//!      alias. That alias must own a signature whose parameter paths are children of the alias
//!      itself, exactly like a freshly materialised signature; keeping the first entry's
//!      parameter paths is an identity discrepancy even when positional calls still fold.

use super::*;
use crate::compiler_frontend::ast::generic_functions::reuse_probe;

/// WHAT: one imported const function is called through a fresh import and through a second
///       provider-export alias, using positional arguments, an omitted default and named
///       arguments.
/// WHY:  the reuse fast path must alias the prepared executable while presenting the same
///       parameter identity a fresh import would install. The test-only path invariant rejects an
///       alias that keeps the first entry's parameter parents, and the folded values prove the
///       aliased executable still receives the right arguments and defaults.
#[test]
fn repeated_const_import_alias_owns_alias_local_parameter_paths() {
    let provider_name = "const_alias_identity_probe_marker";
    let alias_name = "const_alias_identity_probe_alias";
    let entry = format!(
        "@helper {provider_name}\n@helper {alias_name}\n\n\
         canonical #I32 = {provider_name}(2, 4)\n\
         aliased #I32 = {alias_name}(8, 9)\n\
         positional_default #I32 = {alias_name}(6, 7)\n\
         defaulted #I32 = {alias_name}(value = 5)\n\
         named #I32 = {alias_name}(prefix = 1, value = 2, suffix = 9)\n\
         canonical_named #I32 = {provider_name}(value = 5)\n\n\
         [: canonical=[canonical] aliased=[aliased] positional_default=[positional_default] \
         defaulted=[defaulted] named=[named] canonical_named=[canonical_named] ]\n"
    );
    let provider_module = format!(
        "$export\n@internal {provider_name}\n$export\n@internal {provider_name} as {alias_name}\n"
    );
    let provider_body = format!(
        "{provider_name} #|prefix I32 = 7, value I32, suffix I32 = 3| -> I32:\n    return prefix * 100 + value * 10 + suffix\n;\n"
    );

    let compiled = compile_template_fixture(&[
        ("src/@entry.moth", entry.as_str()),
        ("src/helper/@mod.moth", provider_module.as_str()),
        ("src/helper/internal.moth", provider_body.as_str()),
    ]);

    assert_eq!(
        entry_folded_output(&compiled),
        " canonical=243 aliased=893 positional_default=673 defaulted=753 named=129 canonical_named=753 ",
        "canonical and aliased calls must fold the same exact values"
    );
    assert_eq!(
        reuse_probe::preparations(provider_name),
        1,
        "one provider origin must prepare its body exactly once"
    );
    assert_eq!(
        reuse_probe::rebases(provider_name),
        1,
        "the second import alias must reuse the completed origin before context rebase"
    );
    assert!(
        reuse_probe::signature_checks(provider_name) >= 2,
        "fresh consumer and provider entry signatures must both be validated: {:?}",
        reuse_probe::checked_signatures()
    );
    assert!(
        reuse_probe::signature_checks(alias_name) >= 1,
        "the reused alias signature must validate parameters under the alias path: {:?}",
        reuse_probe::checked_signatures()
    );
}
