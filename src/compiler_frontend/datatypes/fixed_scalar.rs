//! Explicit-width builtin scalar identities.
//!
//! WHAT: the fixed-width numeric spellings `I8`..`F64` and the octet type `Byte`, each a
//!       distinct builtin identity that carries its own width, signedness or octet meaning.
//! WHY:  these types are declared and compared directly, so width and octet meaning must be
//!       semantic identity instead of something later stages re-derive from a rendered name.
//!
//! Exclusions: `Int` and `Float` remain separate profile-dependent identities and never alias a
//! fixed width that happens to match, even under a matching `NumericProfile`. `Number`/`NumberN`
//! and the inactive `Decimal` scaffold are not part of this vocabulary. No arithmetic, range,
//! conversion or storage-layout policy lives here; those consume these identities elsewhere.

/// One fixed-width builtin scalar or the `Byte` octet type.
///
/// Discriminants follow `ALL` order so `builtin_type_ids::fixed_scalar` can derive the seeded
/// `TypeId` layout from this enum without a second ordering table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum FixedScalar {
    I8 = 0,
    I16 = 1,
    I32 = 2,
    I64 = 3,
    U8 = 4,
    U16 = 5,
    U32 = 6,
    U64 = 7,
    F16 = 8,
    F32 = 9,
    F64 = 10,
    Byte = 11,
}

impl FixedScalar {
    /// Every fixed scalar in seeded `TypeId` order.
    ///
    /// `TypeEnvironment::new` seeds this sequence and `builtin_type_ids::fixed_scalar` mirrors it,
    /// so the order is a stable identity fact rather than a local convenience.
    pub(crate) const ALL: [FixedScalar; 12] = [
        FixedScalar::I8,
        FixedScalar::I16,
        FixedScalar::I32,
        FixedScalar::I64,
        FixedScalar::U8,
        FixedScalar::U16,
        FixedScalar::U32,
        FixedScalar::U64,
        FixedScalar::F16,
        FixedScalar::F32,
        FixedScalar::F64,
        FixedScalar::Byte,
    ];

    /// The exact source spelling of this scalar, which is also its keyword token spelling.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            FixedScalar::I8 => "I8",
            FixedScalar::I16 => "I16",
            FixedScalar::I32 => "I32",
            FixedScalar::I64 => "I64",
            FixedScalar::U8 => "U8",
            FixedScalar::U16 => "U16",
            FixedScalar::U32 => "U32",
            FixedScalar::U64 => "U64",
            FixedScalar::F16 => "F16",
            FixedScalar::F32 => "F32",
            FixedScalar::F64 => "F64",
            FixedScalar::Byte => "Byte",
        }
    }

    /// Resolves an exact source spelling back to its fixed scalar identity.
    ///
    /// `Int` and `Float` deliberately return `None`: they are profile-dependent identities with no
    /// fixed width, so a matching `NumericProfile` width never makes them alias an `I*`/`U*`/`F*`
    /// spelling.
    pub(crate) fn from_name(name: &str) -> Option<FixedScalar> {
        FixedScalar::ALL
            .into_iter()
            .find(|scalar| scalar.name() == name)
    }
}
