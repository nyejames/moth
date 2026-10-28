//! Family rows: the static schema every compact diagnostic family is checked against.
//!
//! WHAT: a `FamilyRow` names its internal and stable codes, category, descriptor, fact layout,
//! extra-data kind and label codes. Its typed view reads records back for rendering.
//! WHY: one row per family is the single source of codes and titles. Compaction checks every
//! draft against its row before it writes anything.

use super::extras::ExtraDataKind;
use super::facts::{FactCapacityError, FactCodec, IdRemap};
use super::labels::LabelMessageCode;
use super::record::DiagnosticCode;
use super::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, RenderedPayload,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticDescriptor, DiagnosticSeverity};

/// The diagnostic domain used to assign stable code bands and render metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DiagnosticCategory {
    Syntax,
    Type,
    Rule,
    Import,
    Borrow,
    Config,
    DeferredFeature,
}

/// One fact's storage shape, read from its codec.
#[derive(Clone, Copy, Debug)]
pub(super) struct FactDescriptor {
    pub(super) words: u8,
    /// Relative words that every value of this codec must leave as raw type ids.
    pub(super) required_pending_mask: u8,
    /// Relative words that some value of this codec may leave as raw type ids.
    pub(super) possible_pending_mask: u8,
    /// Rewrites this codec's identity words through a remap. Other words are left as they are.
    pub(super) remap_ids: fn(&mut [u32], IdRemap<'_>) -> Result<(), FactCapacityError>,
    /// Appends path IDs encoded by this fact.
    pub(super) append_path_ids:
        fn(&[u32], &mut Vec<crate::compiler_frontend::symbols::path_interner::PathId>),
}

impl FactDescriptor {
    pub(super) const fn of<C: FactCodec>() -> Self {
        Self {
            words: C::WORDS as u8,
            required_pending_mask: C::REQUIRED_PENDING_MASK,
            possible_pending_mask: C::PENDING_MASK,
            remap_ids: C::remap_ids,
            append_path_ids: C::append_path_ids,
        }
    }
}

/// One family's renderer. It reads a record through the family's view and returns the message and guidance.
type FamilyRenderer = fn(&DiagnosticView<'_>, DiagnosticRenderContext<'_>) -> RenderedPayload;

/// A family's static schema.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FamilyRow {
    pub(super) code: DiagnosticCode,
    pub(super) category: DiagnosticCategory,
    pub(super) reserved: bool,
    /// The descriptor owned by this family row.
    pub(super) descriptor: DiagnosticDescriptor,
    pub(super) layout: &'static [FactDescriptor],
    pub(super) extra: ExtraDataKind,
    /// Family-specific label codes that drafts of this family may declare. The universal codes apply to every family.
    pub(super) labels: &'static [LabelMessageCode],
    /// Renders records from the family's typed view. Reserved families have no records to render.
    pub(crate) render: Option<FamilyRenderer>,
    /// The stable reason key the family's view reports; `None` when the family has no reason fact.
    pub(super) reason_key: fn(&DiagnosticView<'_>) -> Option<&'static str>,
}

impl FamilyRow {
    /// Total fact words that this family's constructor must write.
    pub(super) const fn word_count(&self) -> usize {
        let mut total = 0;
        let mut index = 0;
        while index < self.layout.len() {
            total += self.layout[index].words as usize;
            index += 1;
        }
        total
    }

    /// Bit `n` is set when fact word `n` may hold a raw type id for this family.
    pub(super) const fn pending_mask(&self) -> u8 {
        let mut mask = 0;
        let mut word = 0;
        let mut index = 0;
        while index < self.layout.len() {
            let fact = self.layout[index];
            let words = fact.words as usize;
            let mut offset = 0;
            while offset < words {
                if fact.possible_pending_mask & (1 << offset) != 0 {
                    mask |= 1 << (word + offset);
                }
                offset += 1;
            }
            word += words;
            index += 1;
        }
        mask
    }
    /// Bit `n` is set when fact word `n` must hold a raw type id for every value of this family.
    pub(super) const fn required_pending_mask(&self) -> u8 {
        let mut mask = 0;
        let mut word = 0;
        let mut index = 0;
        while index < self.layout.len() {
            let fact = self.layout[index];
            let words = fact.words as usize;
            let mut offset = 0;
            while offset < words {
                if fact.required_pending_mask & (1 << offset) != 0 {
                    mask |= 1 << (word + offset);
                }
                offset += 1;
            }
            word += words;
            index += 1;
        }
        mask
    }

    /// The stable descriptor owned by this family row.
    pub(crate) const fn descriptor(&self) -> DiagnosticDescriptor {
        self.descriptor
    }

    /// The severity that the family's descriptor gives its records.
    pub(super) const fn default_severity(&self) -> DiagnosticSeverity {
        self.descriptor().default_severity
    }

    /// Whether drafts of this family may carry a label with `code`: one of its declared roles or a universal role.
    pub(super) fn permits_label(&self, code: LabelMessageCode) -> bool {
        code.is_universal() || self.labels.contains(&code)
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "reserved flag: only schema tests read it")
    )]
    pub(super) const fn is_reserved(&self) -> bool {
        self.reserved
    }
}

/// The thousands band that codes in `category` occupy. For example, `Rule` owns `3000..=3999`.
pub(super) const fn band_of(category: DiagnosticCategory) -> u16 {
    match category {
        DiagnosticCategory::Syntax => 1000,
        DiagnosticCategory::Type => 2000,
        DiagnosticCategory::Rule => 3000,
        DiagnosticCategory::Import => 4000,
        DiagnosticCategory::Borrow => 5000,
        DiagnosticCategory::Config => 6000,
        DiagnosticCategory::DeferredFeature => 7000,
    }
}

/// Expands to `false`, or to `true` when a family is marked `reserved`: its code is held, but nothing produces it.
macro_rules! reserved_flag {
    () => {
        false
    };
    (reserved) => {
        true
    };
}
pub(super) use reserved_flag;

/// Expands to a family's renderer slot. `none` marks a reserved family with no records; any other renderer
/// decodes the record into the family view and passes it to the renderer.
macro_rules! family_renderer {
    (none, $view:ident) => {
        None
    };
    ($render:ident, $view:ident) => {
        Some({
            fn render_family(
                diagnostic: &$crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView<'_>,
                context: $crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext<'_>,
            ) -> $crate::compiler_frontend::compiler_messages::render::RenderedPayload {
                let facts = diagnostic
                    .family_view::<$view>()
                    .expect("a family row decodes only records of its own code");
                $render(diagnostic, &facts, context)
            }
            render_family
        })
    };
}
pub(super) use family_renderer;

/// Declares a family row with its descriptor and typed view. Its constructor is generated
/// (`ctor: generated`) or written by hand in the group module (`ctor: manual`). `render` names the
/// renderer that takes the view, or `none` for a reserved family with no records. Attributes
/// before `view:` go on the generated view struct. A reserved family uses one for its
/// `dead_code` allowance.
macro_rules! diagnostic_family {
    (
        $name:ident {
            internal: $internal:literal,
            stable: $stable:literal,
            category: $category:expr,
            severity: $severity:expr,
            title: $title:literal,
            facts: [$($field:ident : $codec:ty),* $(,)?],
            labels: [$($label:expr),* $(,)?],
            extra: $extra:ident,
            ctor: generated $ctor:ident,
            $(reason_key: $reason:ident,)?
            $(#[$view_attr:meta])*
            view: $view:ident,
            render: $render:ident $(, $reserved:ident)? $(,)?
        }
    ) => {
        const _: () = assert!(
            ($internal / 1000) * 1000
                == $crate::compiler_frontend::compiler_messages::diagnostics::schema::band_of($category),
            "family code must sit in its category's band"
        );

        pub(crate) const $name: $crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow =
            $crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow {
                code: $crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode::new($internal),
                category: $category,
                reserved: $crate::compiler_frontend::compiler_messages::diagnostics::schema::reserved_flag!(
                    $($reserved)?
                ),
                descriptor: $crate::compiler_frontend::compiler_messages::DiagnosticDescriptor::new(
                    $stable,
                    $title,
                    $severity,
                ),
                layout: &[$(
                    $crate::compiler_frontend::compiler_messages::diagnostics::schema::FactDescriptor::of::<$codec>()
                ),*],
                extra: $crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind::$extra,
                labels: &[$($label),*],
                reason_key: $crate::compiler_frontend::compiler_messages::diagnostics::view::reason_key_through::<$view>,
                render: $crate::compiler_frontend::compiler_messages::diagnostics::schema::family_renderer!(
                    $render,
                    $view
                ),
            };

        $(#[$view_attr])*
        pub(crate) struct $view {
            $(pub(crate) $field: <$codec as $crate::compiler_frontend::compiler_messages::diagnostics::facts::FactCodec>::Durable,)*
        }

        impl $crate::compiler_frontend::compiler_messages::diagnostics::view::FamilyView for $view {
            $(
                fn reason_key(&self) -> Option<&'static str> {
                    use $crate::compiler_frontend::compiler_messages::diagnostics::reason_keys::OptionalStableReasonKey as _;
                    self.$reason.optional_stable_reason_key()
                }
            )?
            fn from_record(
                record: $crate::compiler_frontend::compiler_messages::diagnostics::store::RecordRef<'_>,
            ) -> Option<Self> {
                if record.code() != $name.code {
                    return None;
                }
                let facts = record.facts();
                $(let $field = facts.read::<$codec>();)*
                facts.finish();
                Some(Self {
                    $($field,)*
                })
            }
        }

        pub(crate) fn $ctor(
            $($field: <$codec as $crate::compiler_frontend::compiler_messages::diagnostics::facts::FactCodec>::Draft,)*
            span: Option<$crate::compiler_frontend::source::SourceSpan>,
        ) -> $crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft {
            // The cursor is underscore-prefixed so that a family with no facts does not warn as unused.
            $crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft::with_facts(
                &$name,
                span,
                |_cursor| {
                    $(_cursor.push::<$codec>(&$field)?;)*
                    Ok(())
                },
            )
        }
    };
    (
        $name:ident {
            internal: $internal:literal,
            stable: $stable:literal,
            category: $category:expr,
            severity: $severity:expr,
            title: $title:literal,
            facts: [$($field:ident : $codec:ty),* $(,)?],
            labels: [$($label:expr),* $(,)?],
            extra: $extra:ident,
            ctor: manual,
            $(reason_key: $reason:ident,)?
            $(#[$view_attr:meta])*
            view: $view:ident,
            render: $render:ident $(, $reserved:ident)? $(,)?
        }
    ) => {
        const _: () = assert!(
            ($internal / 1000) * 1000
                == $crate::compiler_frontend::compiler_messages::diagnostics::schema::band_of($category),
            "family code must sit in its category's band"
        );

        pub(crate) const $name: $crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow =
            $crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow {
                code: $crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode::new($internal),
                category: $category,
                reserved: $crate::compiler_frontend::compiler_messages::diagnostics::schema::reserved_flag!(
                    $($reserved)?
                ),
                descriptor: $crate::compiler_frontend::compiler_messages::DiagnosticDescriptor::new(
                    $stable,
                    $title,
                    $severity,
                ),
                layout: &[$(
                    $crate::compiler_frontend::compiler_messages::diagnostics::schema::FactDescriptor::of::<$codec>()
                ),*],
                extra: $crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind::$extra,
                labels: &[$($label),*],
                reason_key: $crate::compiler_frontend::compiler_messages::diagnostics::view::reason_key_through::<$view>,
                render: $crate::compiler_frontend::compiler_messages::diagnostics::schema::family_renderer!(
                    $render,
                    $view
                ),
            };

        $(#[$view_attr])*
        pub(crate) struct $view {
            $(pub(crate) $field: <$codec as $crate::compiler_frontend::compiler_messages::diagnostics::facts::FactCodec>::Durable,)*
        }

        impl $crate::compiler_frontend::compiler_messages::diagnostics::view::FamilyView for $view {
            $(
                fn reason_key(&self) -> Option<&'static str> {
                    use $crate::compiler_frontend::compiler_messages::diagnostics::reason_keys::OptionalStableReasonKey as _;
                    self.$reason.optional_stable_reason_key()
                }
            )?
            fn from_record(
                record: $crate::compiler_frontend::compiler_messages::diagnostics::store::RecordRef<'_>,
            ) -> Option<Self> {
                if record.code() != $name.code {
                    return None;
                }
                let facts = record.facts();
                $(let $field = facts.read::<$codec>();)*
                facts.finish();
                Some(Self {
                    $($field,)*
                })
            }
        }
    };
}
pub(super) use diagnostic_family;
