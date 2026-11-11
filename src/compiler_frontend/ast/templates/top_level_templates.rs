//! Top-level const fragment collection.
//!
//! WHAT: collects folded top-level const string fragments.
//! WHY: builders consume ordered const fragments with runtime insertion indices. Declaration
//! documentation is source metadata owned by the AST result, not template output. Runtime
//! template handling stays in the compiler-owned entry body via PushStartRuntimeFragment nodes.

use crate::compiler_frontend::ast::const_values::store::ConstStringValue;
use crate::compiler_frontend::builder_root_slots::ConstFragmentDestination;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::headers::parse_file_headers::TopLevelConstFragment;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

use rustc_hash::FxHashMap;

// -------------------------
//  Fragment Types
// -------------------------

/// A top-level const template that has been folded to a string at compile time.
///
/// WHAT: carries the folded string value, including any unresolved structural pieces, its
///       insertion index relative to runtime fragments, its authored span and its builder
///       destination.
/// WHY: builders merge const fragments with the runtime fragment list using the insertion index
///      to reconstruct source-order interleaving, while named builder contributions stay out of
///      the body lane and retain their authored provenance for builder interpretation.
#[derive(Clone, Debug)]
pub struct AstConstTopLevelFragment {
    /// Number of runtime fragments preceding this const fragment in source order.
    pub runtime_insertion_index: usize,
    /// Exact authored source span of this const fragment.
    pub span: SourceSpan,
    /// Default body lane or one builder-registered named destination.
    pub destination: ConstFragmentDestination,
    pub value: ConstStringValue,
}

/// Folded value for a top-level const template.
///
/// WHAT: carries the already-folded string value and validated destination, or records that the
///       template produced no output at all.
/// WHY: `[$note: ...]` must disappear rather than become an empty String fragment. The explicit
///      no-output variant lets fragment collection skip it without inventing a value.
#[derive(Clone, Debug)]
pub(crate) enum FoldedConstTemplateResult {
    Output {
        value: ConstStringValue,
        destination: ConstFragmentDestination,
    },
    NoOutput,
}

impl FoldedConstTemplateResult {
    pub(crate) fn new(value: ConstStringValue, destination: ConstFragmentDestination) -> Self {
        Self::Output { value, destination }
    }
}

// -------------------------
//  Fragment Collection
// -------------------------

/// Collects const top-level fragments from folded template result records.
///
/// WHAT: removes each header-parsed const fragment's folded result from the emitting map and
/// returns the ordered `AstConstTopLevelFragment` list.
/// WHY: const fragments are folded during emit; this function transfers their owned values into
/// the ordered list. HIR never consumes fragments: module compilation converts each one into an
/// owned structural string on the module metadata, which builders read to render entry fragments.
/// The map has no later consumer on this path, so removing an entry moves its value instead of
/// cloning the string pieces and destination.
pub(crate) fn collect_const_top_level_fragments(
    top_level_const_fragments: &[TopLevelConstFragment],
    const_templates_by_path: &mut FxHashMap<PathId, FoldedConstTemplateResult>,
) -> Result<Vec<AstConstTopLevelFragment>, InfrastructureFailure> {
    let mut result = Vec::with_capacity(top_level_const_fragments.len());

    for fragment in top_level_const_fragments {
        let folded = const_templates_by_path
            .remove(&fragment.header_path)
            .ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::AstConstruction,
                    "Top-level const fragment has no corresponding folded template value. This is a compiler bug.",
                )
            })?;

        let (value, destination) = match folded {
            FoldedConstTemplateResult::Output { value, destination } => (value, destination),
            FoldedConstTemplateResult::NoOutput => continue,
        };

        result.push(AstConstTopLevelFragment {
            runtime_insertion_index: fragment.runtime_insertion_index,
            span: fragment.span,
            destination,
            value,
        });
    }

    Ok(result)
}

#[cfg(test)]
#[path = "tests/template_tests.rs"]
mod template_tests;
