//! Build-system command profile policy.
//!
//! WHAT: owns the single build-system `BuildProfile` and the one profile-selection helper shared
//! by output resolution and the HTML builder.
//! WHY: output roots, manifest ownership and builder codegen must agree on the selected profile
//! without each layer re-deriving it from `Flag::Release`.

use crate::compiler_frontend::Flag;

/// One build-system profile used for command policy.
///
/// WHAT: distinguishes development and release builds for output policy and the HTML builder.
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
    /// WHY: this is the single profile-selection helper shared by output resolution and the HTML
    /// builder.
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
