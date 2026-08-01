//! Build-system command profile policy.
//!
//! WHAT: owns the single build-system `BuildProfile` and the one profile-selection helper shared
//! by frontend profile conversion, the HTML builder and output resolution.
//! WHY: output roots, manifest ownership and builder codegen must agree on the selected profile
//! without each layer re-deriving it from `Flag::Release`.

use crate::compiler_frontend::Flag;

/// One build-system profile used for command policy.
///
/// WHAT: distinguishes development and release builds for frontend profile conversion, HTML
/// builder policy and output resolution.
/// WHY: output roots and builder codegen must agree on the selected profile without each layer
/// re-deriving it from `Flag::Release`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    Dev,
    Release,
}

impl BuildProfile {
    /// Select the build profile from the command flag slice.
    ///
    /// WHAT: maps the presence of `Flag::Release` to [`BuildProfile::Release`].
    /// WHY: this is the single profile-selection helper shared by frontend profile conversion,
    /// the HTML builder and output resolution.
    pub fn from_flags(flags: &[Flag]) -> Self {
        if flags.contains(&Flag::Release) {
            Self::Release
        } else {
            Self::Dev
        }
    }

    /// Report whether this is a release build.
    pub fn is_release(self) -> bool {
        matches!(self, Self::Release)
    }
}
