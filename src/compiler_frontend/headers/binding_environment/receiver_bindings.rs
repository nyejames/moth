//! Source receiver-method visibility helpers.
//!
//! WHAT: adds source receiver methods to file-local receiver-call visibility.
//! WHY: receiver methods live in a separate call namespace from ordinary value bindings, so
//! their visibility rules need dedicated helpers.
//! MUST NOT: register ordinary value/type dependencies or parse executable bodies.

use super::{
    BindingEnvironmentBuilder, FileVisibility, ReceiverMethodVisibility, SourceFunctionTarget,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

impl<'a> BindingEnvironmentBuilder<'a> {
    pub(super) fn add_visible_receiver_method(
        file_visibility: &mut FileVisibility,
        local_name: StringId,
        function_path: &InternedPath,
        location: SourceLocation,
    ) {
        let methods = file_visibility
            .visible_receiver_methods
            .entry(local_name)
            .or_default();

        if methods
            .iter()
            .any(|method| method.target.local_path() == function_path)
        {
            return;
        }

        methods.push(ReceiverMethodVisibility {
            target: SourceFunctionTarget::Local(function_path.clone()),
            location,
        });
    }
}
