//! Top-level const fragment collection and declaration-attached doc metadata.
//!
//! WHAT: collects folded top-level const string fragments and owns the AST doc-fragment records
//! attached by source-directive parsing.
//! WHY: builders consume ordered const fragments with runtime insertion indices, while module
//! metadata consumes documentation joined to its declaration identity. Runtime template handling
//! stays in the compiler-owned entry body via PushStartRuntimeFragment nodes.

use crate::compiler_frontend::ast::const_values::store::ConstStringValue;
use crate::compiler_frontend::builder_root_slots::ConstFragmentDestination;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::headers::parse_file_headers::TopLevelConstFragment;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;

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

/// One compile-time documentation record attached to its declaration.
///
/// WHAT: carries the target declaration path, the folded text and the directive operand span.
/// WHY: documentation is declaration metadata, not an unattached page fragment. The declaration
/// path is the stable module-local identity later consumers join on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AstDocFragment {
    pub target: PathId,
    pub value: StringId,
    pub span: Option<SourceSpan>,
}

// -------------------------
//  Fragment Collection
// -------------------------

/// Collects const top-level fragments from folded template result records.
///
/// WHAT: maps each header-parsed const fragment to its folded string value using the
/// const template result map produced during AST emission.
/// WHY: const fragments are folded during emit; this function gathers the results into the
/// ordered `AstConstTopLevelFragment` list. HIR never consumes fragments: module compilation
/// converts each one into an owned structural string on the module metadata, which builders read
/// to render entry fragments.
pub(crate) fn collect_const_top_level_fragments(
    top_level_const_fragments: &[TopLevelConstFragment],
    const_templates_by_path: &FxHashMap<PathId, FoldedConstTemplateResult>,
) -> Result<Vec<AstConstTopLevelFragment>, InfrastructureFailure> {
    let mut result = Vec::with_capacity(top_level_const_fragments.len());

    for fragment in top_level_const_fragments {
        let folded = const_templates_by_path
            .get(&fragment.header_path)
            .ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::AstConstruction,
                    "Top-level const fragment has no corresponding folded template value. This is a compiler bug.",
                )
            })?;

        let (value, destination) = match folded {
            FoldedConstTemplateResult::Output { value, destination } => {
                (value.clone(), destination.clone())
            }
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
