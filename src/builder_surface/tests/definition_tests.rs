//! Builder surface definition prose tests.
//!
//! WHAT: pins the damaged-package-set report's installed-root spelling and registration rule.
//! WHY: `BUILTIN_PACKAGES_ROOT` is canonicalized, so Windows releases would otherwise expose a
//!      verbatim `\\?\` prefix in the reinstall message. Registration provenance, not a path
//!      check, decides whether the report applies.

#[cfg(windows)]
#[test]
fn damaged_package_set_report_renders_the_installed_root_portably() {
    use crate::builder_surface::definition::{BUILTIN_PACKAGES_ROOT, damaged_package_set_report};
    use crate::builder_surface::package_metadata::PackageSetMembership;
    use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;

    let message = damaged_package_set_report(PackageSetMembership::CompilerShipped)
        .expect("a compiler-shipped registration should be classified as damaged");

    assert!(
        message.contains(portable_path_text(&*BUILTIN_PACKAGES_ROOT).as_str()),
        "the damaged package set report must use portable display text: {message}"
    );
    assert!(
        !message.contains("\\\\?\\"),
        "the report must not expose a Windows verbatim prefix: {message}"
    );
}

#[test]
fn ordinary_registration_never_reports_a_damaged_package_set() {
    use crate::builder_surface::definition::damaged_package_set_report;
    use crate::builder_surface::package_metadata::PackageSetMembership;

    assert_eq!(
        damaged_package_set_report(PackageSetMembership::Ordinary),
        None,
        "a custom or project-local registration never makes a read failure an installed-set defect"
    );
}
