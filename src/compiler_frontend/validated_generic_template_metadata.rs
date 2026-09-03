//! Materialisation-context validation for exported generic callables.
//!
//! WHAT: joins public callable origins to the declaring module's retained generic templates and
//! installs each stable origin directly on the complete [`ModuleMaterialisationContext`] payload.
//! WHY: the complete context is the sole durable template owner. This module validates that join
//! without constructing the deleted body-only template store or another compatibility lane.

use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::public_interface::CallableSeed;
use crate::compiler_frontend::public_interface::{
    PublicDeclarationRecord, PublicDeclarationSemantics, PublicInterfaceDraft,
    PublicReceiverMethodSemantics,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, OriginDeclarationId, OriginFunctionId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;

use rustc_hash::{FxHashMap, FxHashSet};

const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<GenericFunctionTemplate>();
};

/// Validate that a complete materialisation context retains every exported generic body.
///
/// The context remains the sole durable owner. This compatibility-free validation wrapper uses
/// the existing total join rules while the former body-only store is removed from module
/// metadata.
pub(in crate::compiler_frontend) fn validate_materialisation_context_templates(
    draft: &PublicInterfaceDraft,
    callable_seeds: &[CallableSeed],
    templates: &mut FxHashMap<InternedPath, GenericFunctionTemplate>,
) -> Result<(), CompilerError> {
    let expected_callables = collect_public_callable_origins(draft)?;
    validate_callable_seeds(&expected_callables, callable_seeds)?;

    for (path, template) in templates.iter() {
        if *path != template.function_path {
            return Err(CompilerError::compiler_error(
                "Materialisation context generic template map key does not match its template path",
            ));
        }
    }

    for seed in callable_seeds.iter().filter(|seed| seed.generic_template) {
        let template = templates.get_mut(&seed.path).ok_or_else(|| {
            CompilerError::compiler_error(
                "Materialisation context omitted an exported generic callable template",
            )
        })?;
        if template.body_tokens.is_none() {
            return Err(CompilerError::compiler_error(
                "Declaring materialisation context retained an imported contract instead of its generic body",
            ));
        }
        template.declaration_identity =
            Some(GeneratedDeclarationIdentity::Public(seed.origin.clone()));
    }

    for seed in callable_seeds.iter().filter(|seed| !seed.generic_template) {
        if templates.contains_key(&seed.path) {
            return Err(CompilerError::compiler_error(
                "Materialisation context retained a generic template for a non-generic public callable",
            ));
        }
    }

    Ok(())
}

fn collect_public_callable_origins(
    draft: &PublicInterfaceDraft,
) -> Result<FxHashMap<OriginFunctionId, bool>, CompilerError> {
    let mut callables = FxHashMap::default();

    for PublicDeclarationRecord {
        origin, semantics, ..
    } in &draft.declarations
    {
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
