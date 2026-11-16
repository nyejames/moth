//! Builder surface definition prose tests.
//!
//! WHAT: pins the damaged-package-set report's installed-root spelling.
//! WHY: `BUILTIN_PACKAGES_ROOT` is canonicalized, so Windows releases would otherwise expose a
//!      verbatim `\\?\` prefix in the reinstall message.

#[cfg(windows)]
#[test]
fn damaged_package_set_report_renders_the_installed_root_portably() {
    use crate::builder_surface::definition::{BUILTIN_PACKAGES_ROOT, damaged_package_set_report};
    use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;

    let missing = BUILTIN_PACKAGES_ROOT.join("missing-package");
    let message = damaged_package_set_report(&missing)
        .expect("a path below the installed package root should be classified as damaged");

    assert!(
        message.contains(portable_path_text(&*BUILTIN_PACKAGES_ROOT).as_str()),
        "the damaged package set report must use portable display text: {message}"
    );
    assert!(
        !message.contains("\\\\?\\"),
        "the report must not expose a Windows verbatim prefix: {message}"
    );
}
