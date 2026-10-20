//! Family rows: the static schema each compact diagnostic family is checked against.
//!
//! WHAT: a `FamilyRow` names a family's internal code, stable code, category, fact layout, extra-data kind and
//! declared label codes. A pilot row also carries the legacy descriptor that the descriptor sites serve, and a
//! typed view that reads its records back.
//! WHY: one row per family is the single source of stable codes and titles for pilot families. Compaction
//! checks every draft against its row before it writes anything.

use super::extras::ExtraDataKind;
use super::facts::FactCodec;
use super::labels::LabelMessageCode;
use super::record::DiagnosticCode;
use super::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, RenderedPayload,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCategory, DiagnosticDescriptor, DiagnosticSeverity,
};

/// One fact's storage shape, read from its codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FactDescriptor {
    pub(super) words: u8,
    pub(super) pending: bool,
}

impl FactDescriptor {
    pub(super) const fn of<C: FactCodec>() -> Self {
        Self {
            words: C::WORDS as u8,
            pending: C::PENDING,
        }
    }
}

/// One family's renderer. It reads a record through the family's view and returns the message and guidance.
type FamilyRenderer = fn(&DiagnosticView<'_>, DiagnosticRenderContext<'_>) -> RenderedPayload;

/// A family's static schema.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FamilyRow {
    pub(super) code: DiagnosticCode,
    pub(super) stable_code: &'static str,
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.6 producer sweep reads family categories")
    )]
    pub(super) category: DiagnosticCategory,
    pub(super) reserved: bool,
    /// The legacy descriptor. Only pilot rows carry one.
    pub(super) descriptor: Option<DiagnosticDescriptor>,
    pub(super) layout: &'static [FactDescriptor],
    pub(super) extra: ExtraDataKind,
    /// Family-specific label codes that drafts of this family may declare. The universal codes apply to every family.
    pub(super) labels: &'static [LabelMessageCode],
    /// Renders the family's records from their typed view. Only pilot rows carry a renderer; identity rows have no
    /// records to render.
    pub(crate) render: Option<FamilyRenderer>,
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

    /// Bit `n` is set when fact word `n` holds a raw type id.
    pub(super) const fn pending_mask(&self) -> u8 {
        let mut mask = 0;
        let mut word = 0;
        let mut index = 0;
        while index < self.layout.len() {
            let fact = self.layout[index];
            let words = fact.words as usize;
            let mut offset = 0;
            while offset < words {
                if fact.pending {
                    mask |= 1 << (word + offset);
                }
                offset += 1;
            }
            word += words;
            index += 1;
        }
        mask
    }

    /// The legacy descriptor of a pilot row.
    pub(crate) const fn pilot_descriptor(&self) -> DiagnosticDescriptor {
        match self.descriptor {
            Some(descriptor) => descriptor,
            None => panic!("only pilot rows serve a legacy descriptor"),
        }
    }

    /// The severity that the family's descriptor gives its records. Only pilot rows carry a descriptor.
    pub(super) const fn default_severity(&self) -> DiagnosticSeverity {
        self.pilot_descriptor().default_severity
    }

    /// Whether drafts of this family may carry a label with `code`: one of its declared roles or a universal role.
    pub(super) fn permits_label(&self, code: LabelMessageCode) -> bool {
        code.is_universal() || self.labels.contains(&code)
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "reserved flag (D9): only schema tests read it")
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
        DiagnosticCategory::Infrastructure => {
            panic!("P4.1 defines no compact rows for Infrastructure")
        }
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

/// Declares a family row.
///
/// `pilot` rows serve the legacy descriptor and get a typed view. Their constructor is generated (`ctor: generated`)
/// or written by hand in the group module (`ctor: manual`). `render` names the renderer that takes the view, or
/// `none` for a reserved family with no records. `identity` rows declare only the code and category.
/// Attributes before `view:` go on the generated view struct. A reserved family uses one for its D9 allowance.
macro_rules! diagnostic_family {
    (
        pilot $name:ident {
            internal: $internal:literal,
            stable: $stable:literal,
            category: $category:expr,
            severity: $severity:expr,
            title: $title:literal,
            facts: [$($field:ident : $codec:ty),* $(,)?],
            labels: [$($label:expr),* $(,)?],
            extra: $extra:ident,
            ctor: generated $ctor:ident,
            $(#[$view_attr:meta])*
            view: $view:ident,
            render: $render:ident $(, $reserved:ident)? $(,)?
        }
    ) => {
        const _: () = assert!(
            ($internal / 1000) * 1000
                == $crate::compiler_frontend::compiler_messages::diagnostics::schema::band_of($category),
            "pilot family code must sit in its category's band"
        );

        pub(crate) const $name: $crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow =
            $crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow {
                code: $crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode::new($internal),
                stable_code: $stable,
                category: $category,
                reserved: $crate::compiler_frontend::compiler_messages::diagnostics::schema::reserved_flag!(
                    $($reserved)?
                ),
                descriptor: Some($crate::compiler_frontend::compiler_messages::DiagnosticDescriptor::new(
                    $stable,
                    $title,
                    $severity,
                )),
                layout: &[$(
                    $crate::compiler_frontend::compiler_messages::diagnostics::schema::FactDescriptor::of::<$codec>()
                ),*],
                extra: $crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind::$extra,
                labels: &[$($label),*],
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

        pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn $ctor(
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
        pilot $name:ident {
            internal: $internal:literal,
            stable: $stable:literal,
            category: $category:expr,
            severity: $severity:expr,
            title: $title:literal,
            facts: [$($field:ident : $codec:ty),* $(,)?],
            labels: [$($label:expr),* $(,)?],
            extra: $extra:ident,
            ctor: manual,
            $(#[$view_attr:meta])*
            view: $view:ident,
            render: $render:ident $(, $reserved:ident)? $(,)?
        }
    ) => {
        const _: () = assert!(
            ($internal / 1000) * 1000
                == $crate::compiler_frontend::compiler_messages::diagnostics::schema::band_of($category),
            "pilot family code must sit in its category's band"
        );

        pub(crate) const $name: $crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow =
            $crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow {
                code: $crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode::new($internal),
                stable_code: $stable,
                category: $category,
                reserved: $crate::compiler_frontend::compiler_messages::diagnostics::schema::reserved_flag!(
                    $($reserved)?
                ),
                descriptor: Some($crate::compiler_frontend::compiler_messages::DiagnosticDescriptor::new(
                    $stable,
                    $title,
                    $severity,
                )),
                layout: &[$(
                    $crate::compiler_frontend::compiler_messages::diagnostics::schema::FactDescriptor::of::<$codec>()
                ),*],
                extra: $crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind::$extra,
                labels: &[$($label),*],
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
    (
        identity $name:ident {
            internal: $internal:literal,
            stable: $stable:literal,
            category: $category:expr $(, $reserved:ident)? $(,)?
        }
    ) => {
        const _: () = assert!(
            ($internal / 1000) * 1000
                == $crate::compiler_frontend::compiler_messages::diagnostics::schema::band_of($category),
            "identity family code must sit in its category's band"
        );

        pub(crate) const $name: $crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow =
            $crate::compiler_frontend::compiler_messages::diagnostics::schema::FamilyRow {
                code: $crate::compiler_frontend::compiler_messages::diagnostics::record::DiagnosticCode::new($internal),
                stable_code: $stable,
                category: $category,
                reserved: $crate::compiler_frontend::compiler_messages::diagnostics::schema::reserved_flag!(
                    $($reserved)?
                ),
                descriptor: None,
                layout: &[],
                extra: $crate::compiler_frontend::compiler_messages::diagnostics::extras::ExtraDataKind::None,
                labels: &[],
                render: None,
            };
    };
}
pub(super) use diagnostic_family;
