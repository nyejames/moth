use crate::build_system::BuildProfile;
use crate::compiler_frontend::Flag;

#[test]
fn from_flags_maps_release_once() {
    assert_eq!(BuildProfile::from_flags(&[]), BuildProfile::Dev);
    assert_eq!(
        BuildProfile::from_flags(&[Flag::Release]),
        BuildProfile::Release
    );
    // Unrelated flags must not change profile selection.
    assert_eq!(
        BuildProfile::from_flags(&[Flag::HtmlWasm]),
        BuildProfile::Dev
    );
    assert_eq!(
        BuildProfile::from_flags(&[Flag::HtmlWasm, Flag::Release]),
        BuildProfile::Release
    );
}

#[test]
fn is_release_reports_the_selected_profile() {
    assert!(!BuildProfile::Dev.is_release());
    assert!(BuildProfile::Release.is_release());
}
