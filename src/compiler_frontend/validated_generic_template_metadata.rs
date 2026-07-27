//! Validated generic-function template body artefacts retained as compiler metadata.
//!
//! WHAT: owns the one total extraction/join owner that moves the already-validated
//! [`GenericFunctionTemplate`] body payloads for directly exported generic free functions and
//! receiver methods out of the donor-local AST template map and into one deterministic store
//! keyed by the stable
//! [`OriginFunctionId`] already retained by the [`PublicInterfaceDraft`].
//!
//! WHY: locked decision 10 in the canonical-module plan separates consumer-visible generic
//! semantic identity (the draft's [`PublicGenericTemplateDescriptor`]) from the declaring
//! module's retained template body. This store is one validated body-artefact checkpoint for the
//! future build-owned generated-sidecar worklist (R5D-R5G), not public semantic identity and not
//! a backend-consumed artefact yet.
//!
//! The retained [`GenericFunctionTemplate`] is the one existing body payload produced during
//! signature resolution and body validation. It carries the body tokens, resolved signature,
//! generic parameter list handle, source identity and declaration location. It is TIR-free,
//! contains no `Rc`, `RefCell`, `Ast`, `TemplateIrStore` or `TypeEnvironment`, and is `Send`
//! across directory compilation.
//!
//! This is a body-artefact checkpoint only, not the complete materialisation context. Complete
//! materialisation also needs declaration, file-visibility, generic/type and related frontend
//! context that this slice intentionally does not retain; that context is deferred to a later
//! bounded R5D-R5G slice.
//!
//! Boundary: the store lives on [`ModuleCompilerMetadata`] during compilation. The legacy
//! flat-module handoff drops it before string-table remap because the retained
//! [`FunctionSignature`] carries donor-local `StringId`s whose remap owner is not in scope for
//! this slice. The drop is deliberate; R5D-R5G will replace this body-only checkpoint with the
//! complete artefact store and worklist that own generated-sidecar materialisation.

use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::public_interface::CallableSeed;
use crate::compiler_frontend::public_interface::{
    PublicDeclarationRecord, PublicDeclarationSemantics, PublicInterfaceDraft,
    PublicReceiverMethodSemantics,
};
use crate::compiler_frontend::semantic_identity::{OriginDeclarationId, OriginFunctionId};
use crate::compiler_frontend::symbols::interned_path::InternedPath;

use rustc_hash::{FxHashMap, FxHashSet};

// Verifies the retained template payload is `Send` across directory compilation. The
// `GenericFunctionTemplate` carries only owned token, signature, path, location and `u32`
// handle data: no `Rc`, `RefCell`, `Ast`, `TemplateIrStore` or `TypeEnvironment`.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<GenericFunctionTemplate>();
    assert_send::<ValidatedGenericTemplateArtefact>();
    assert_send::<ValidatedGenericTemplateStore>();
};

/// One validated generic-function template body artefact retained as compiler metadata.
///
/// WHAT: pairs the stable [`OriginFunctionId`] of one directly exported generic callable
/// with the one existing [`GenericFunctionTemplate`] body payload produced during signature
/// resolution and body validation. The template carries the body tokens, resolved signature,
/// generic parameter list handle, source identity and declaration location: a validated
/// body-artefact checkpoint, not the complete materialisation context (see module docs).
///
/// WHY: keyed by the exact stable origin already retained by the draft so aliases, declaration
/// order, local paths and source positions never define artefact identity. The template is moved,
/// not cloned: there is no second body-token representation.
#[derive(Debug, Clone)]
pub(crate) struct ValidatedGenericTemplateArtefact {
    pub(crate) origin: OriginFunctionId,
    // The template body payload is retained as compiler metadata for the future R5D-R5G
    // generated-sidecar worklist. Production code moves the artefact into the store and drops it
    // at the legacy handoff; test accessors read the field until R5D-R5G replaces this checkpoint.
    #[allow(dead_code)]
    pub(crate) template: GenericFunctionTemplate,
}

/// Deterministic store of validated generic-template body artefacts for one compiled module.
///
/// WHAT: owns zero or one artefact per directly exported generic free-function or receiver-method
/// origin and none for non-generic or private callables. Artefacts are sorted by full stable origin
/// for deterministic iteration independent of hash-map or declaration-scheduling order.
///
/// WHY: gives [`crate::build_system::build::ModuleCompilerMetadata`] one owned lane for
/// validated generic-template body artefacts. R5D-R5G will replace this checkpoint with a
/// complete artefact that retains the declaration, file-visibility, generic/type and related
/// frontend context this slice intentionally omits, so concrete instances can be materialised
/// without reopening donor AST state.
#[derive(Debug, Clone, Default)]
pub(crate) struct ValidatedGenericTemplateStore {
    // The artefacts vector is retained as compiler metadata for the future R5D-R5G
    // generated-sidecar worklist. Production code constructs and drops the store at the legacy
    // handoff; test accessors read the field until R5D-R5G replaces this checkpoint.
    #[allow(dead_code)]
    artefacts: Vec<ValidatedGenericTemplateArtefact>,
}

impl ValidatedGenericTemplateStore {
    /// Construct a store from already-validated, already-joined artefacts.
    ///
    /// Compiler-internal: the extraction/join owner is the sole caller. It passes artefacts in
    /// the documented deterministic origin order.
    pub(crate) fn from_artefacts(artefacts: Vec<ValidatedGenericTemplateArtefact>) -> Self {
        Self { artefacts }
    }

    /// Whether the store contains no artefacts.
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.artefacts.is_empty()
    }

    /// The number of retained artefacts.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.artefacts.len()
    }

    /// Read-only access to the retained artefacts in deterministic origin order.
    #[cfg(test)]
    pub(crate) fn artefacts(&self) -> &[ValidatedGenericTemplateArtefact] {
        &self.artefacts
    }
}

/// Extract validated generic-template body artefacts for directly exported generic callables.
///
/// WHAT: the sole total extraction/join owner. It runs after generic body validation and before
/// HIR consumes AST state. It validates the exact transient declaration-path-to-stable-origin
/// seed for every directly exported callable, joins each generic seed to the one existing
/// [`GenericFunctionTemplate`] by full [`InternedPath`], moves the template out of the donor-local
/// map, and returns one deterministic store keyed by stable origin.
///
/// WHY: the draft is the authority for which exported free functions are generic (via
/// [`PublicGenericTemplateDescriptor`]) and which receiver methods are generic (via their
/// receiver-method semantics). The transient callable seeds are the exact donor-local path
/// relationship for both generic and non-generic public callables, so leaf names, aliases,
/// declaration order and hash-map order never define a join.
///
/// Total validation:
/// - A missing or extra public callable seed, a duplicate generic seed path or origin, or a seed
///   whose generic flag disagrees with the draft is a `CompilerError`. Non-generic receiver
///   methods may share a donor-local leaf path because their receiver identity is carried by the
///   stable origin and they do not join the generic-template body map.
/// - A draft generic callable with no matching template is a `CompilerError`: the declaring
///   module validated a generic body, so a missing template is an internal invariant violation.
/// - A template map key that does not equal the template's own `function_path` is a
///   `CompilerError`: the donor-local map and the template disagree on identity.
/// - A template whose exact path matches an exported non-generic callable is a `CompilerError`:
///   the draft and validated template state disagree.
/// - A template whose exact path matches no public callable is a private generic function and
///   remains an intentional exclusion.
///
/// [`PublicGenericTemplateDescriptor`]: crate::compiler_frontend::public_interface::PublicGenericTemplateDescriptor
pub(crate) fn extract_validated_generic_template_artefacts(
    draft: &PublicInterfaceDraft,
    callable_seeds: &[CallableSeed],
    templates: FxHashMap<InternedPath, GenericFunctionTemplate>,
) -> Result<ValidatedGenericTemplateStore, CompilerError> {
    let expected_callables = collect_public_callable_origins(draft)?;
    validate_callable_seeds(&expected_callables, callable_seeds)?;

    // Validate every donor-local map entry before consuming any body. The map key is the exact
    // transient declaration path; the template repeats it as its own identity.
    let mut templates_by_path = templates;

    for (path, template) in &templates_by_path {
        // The map key must equal the template's own `function_path`. The donor-local template
        // map is keyed by the same path stored on the template, so a disagreement is an internal
        // invariant violation. The stable origin remains the artefact identity; the path is used
        // only for this donor-local exact join inside the defining module.
        if *path != template.function_path {
            return Err(CompilerError::compiler_error(format!(
                "validated generic-template extraction: a generic function template map key {:?} \
                 does not equal the template's own function_path {:?}; a map-key/template-path \
                 mismatch is an internal invariant violation",
                path, template.function_path
            )));
        }
    }

    // Join each exported generic callable origin to its template by exact path. A missing
    // template for a draft-identified generic export is an internal invariant violation.
    let mut artefacts: Vec<ValidatedGenericTemplateArtefact> =
        Vec::with_capacity(expected_callables.len());

    for seed in callable_seeds.iter().filter(|seed| seed.generic_template) {
        let template = templates_by_path.remove(&seed.path).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "validated generic-template extraction: exported generic callable origin {:?} \
                 has no validated template at exact declaration path {:?}; a missing exported \
                 template is an internal invariant violation",
                seed.origin, seed.path
            ))
        })?;

        artefacts.push(ValidatedGenericTemplateArtefact {
            origin: seed.origin.clone(),
            template,
        });
    }

    // Any remaining template whose exact path matches an exported non-generic callable is a
    // generic/non-generic mismatch. A remaining template whose path does not match a public
    // callable is a private generic function and remains an intentional exclusion.
    for seed in callable_seeds.iter().filter(|seed| !seed.generic_template) {
        if templates_by_path.contains_key(&seed.path) {
            return Err(CompilerError::compiler_error(format!(
                "validated generic-template extraction: exported non-generic callable origin \
                 {:?} has a validated template at exact declaration path {:?}; a \
                 generic/non-generic mismatch is an internal invariant violation",
                seed.origin, seed.path
            )));
        }
    }

    // Deterministic order independent of hash-map iteration. The complete stable origin is the
    // only ordering key, including receiver identity for same-named methods.
    artefacts.sort_by(|left, right| left.origin.cmp(&right.origin));

    Ok(ValidatedGenericTemplateStore::from_artefacts(artefacts))
}

fn collect_public_callable_origins(
    draft: &PublicInterfaceDraft,
) -> Result<FxHashMap<OriginFunctionId, bool>, CompilerError> {
    let mut callables = FxHashMap::default();

    for PublicDeclarationRecord { origin, semantics } in &draft.declarations {
        match semantics {
            PublicDeclarationSemantics::Function(function) => {
                let OriginDeclarationId::Function(function_origin) = origin else {
                    return Err(CompilerError::compiler_error(
                        "validated generic-template extraction found free-function semantics under a non-function declaration origin",
                    ));
                };
                if function_origin.receiver().is_some() {
                    return Err(CompilerError::compiler_error(format!(
                        "validated generic-template extraction found receiver origin {:?} in a free-function declaration record",
                        function_origin
                    )));
                }
                insert_public_callable_origin(
                    &mut callables,
                    function_origin.clone(),
                    matches!(
                        function.category,
                        crate::compiler_frontend::public_interface::PublicFunctionCategory::GenericTemplate(_)
                    ),
                )?;
            }
            PublicDeclarationSemantics::Struct(struct_semantics) => {
                insert_receiver_callable_origins(
                    &mut callables,
                    origin,
                    &struct_semantics.receiver_methods,
                )?;
            }
            PublicDeclarationSemantics::Choice(choice_semantics) => {
                insert_receiver_callable_origins(
                    &mut callables,
                    origin,
                    &choice_semantics.receiver_methods,
                )?;
            }
            PublicDeclarationSemantics::TransparentAlias(_)
            | PublicDeclarationSemantics::Constant(_)
            | PublicDeclarationSemantics::Trait(_) => {}
        }
    }

    Ok(callables)
}

fn insert_receiver_callable_origins(
    callables: &mut FxHashMap<OriginFunctionId, bool>,
    receiver_declaration: &OriginDeclarationId,
    methods: &[PublicReceiverMethodSemantics],
) -> Result<(), CompilerError> {
    let OriginDeclarationId::Type(receiver_origin) = receiver_declaration else {
        return Err(CompilerError::compiler_error(
            "validated generic-template extraction found receiver methods under a non-type declaration origin",
        ));
    };

    for method in methods {
        if method.method_origin.receiver() != Some(receiver_origin) {
            return Err(CompilerError::compiler_error(format!(
                "validated generic-template extraction found receiver method origin {:?} attached to {:?}",
                method.method_origin, receiver_origin
            )));
        }
        insert_public_callable_origin(
            callables,
            method.method_origin.clone(),
            matches!(
                method.category,
                crate::compiler_frontend::public_interface::PublicReceiverMethodCategory::GenericTemplate
            ),
        )?;
    }

    Ok(())
}

fn insert_public_callable_origin(
    callables: &mut FxHashMap<OriginFunctionId, bool>,
    origin: OriginFunctionId,
    generic_template: bool,
) -> Result<(), CompilerError> {
    if callables.insert(origin.clone(), generic_template).is_some() {
        return Err(CompilerError::compiler_error(format!(
            "validated generic-template extraction found duplicate public callable origin {:?}",
            origin
        )));
    }
    Ok(())
}

fn validate_callable_seeds(
    expected_callables: &FxHashMap<OriginFunctionId, bool>,
    seeds: &[CallableSeed],
) -> Result<(), CompilerError> {
    let mut seen_paths: FxHashMap<InternedPath, bool> = FxHashMap::default();
    let mut seen_origins = FxHashSet::default();

    for seed in seeds {
        if let Some(previous_generic) = seen_paths.insert(seed.path.clone(), seed.generic_template)
            && (previous_generic || seed.generic_template)
        {
            return Err(CompilerError::compiler_error(format!(
                "validated generic-template extraction found duplicate generic public callable declaration path {:?}",
                seed.path
            )));
        }
        if !seen_origins.insert(seed.origin.clone()) {
            return Err(CompilerError::compiler_error(format!(
                "validated generic-template extraction found duplicate public callable origin {:?}",
                seed.origin
            )));
        }

        let Some(expected_generic) = expected_callables.get(&seed.origin) else {
            return Err(CompilerError::compiler_error(format!(
                "validated generic-template extraction found public callable seed {:?} with no matching draft origin",
                seed.origin
            )));
        };
        if *expected_generic != seed.generic_template {
            return Err(CompilerError::compiler_error(format!(
                "validated generic-template extraction found generic/non-generic mismatch for public callable origin {:?}",
                seed.origin
            )));
        }
    }

    if seeds.len() != expected_callables.len() {
        let missing = expected_callables
            .keys()
            .find(|origin| !seen_origins.contains(*origin));
        return Err(CompilerError::compiler_error(format!(
            "validated generic-template extraction is missing the exact public callable seed for origin {:?}",
            missing
        )));
    }

    Ok(())
}

#[cfg(test)]
#[path = "tests/validated_generic_template_metadata_tests.rs"]
mod tests;
