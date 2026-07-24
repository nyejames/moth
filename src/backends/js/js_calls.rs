//! Call-target and call-statement lowering for the JavaScript backend.
//!
//! WHAT: owns call-target resolution, inline-expression substitution, and call-statement emission.
//! WHY: call-target lowering (user function, runtime helper, inline expression, or external
//! module export with generated glue) is a distinct concern from general statement orchestration.

use crate::backends::js::JsEmitter;
use crate::backends::js::value_use::JsValueUse;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalJsLowering};
use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::ids::LocalId;

/// Result of lowering a call target for the JS backend.
///
/// WHAT: distinguishes between a regular function call (emit as `name(args)`) and an inline
/// expression (emit as a substituted expression template without call wrapping).
pub(crate) enum LoweredCallTarget {
    FunctionName(String),
    InlineExpression { template: String },
}

impl<'hir> JsEmitter<'hir> {
    /// Lower a HIR call target into the JS name or inline template to emit.
    pub(crate) fn lower_call_target(
        &mut self,
        target: &CallTarget,
    ) -> Result<LoweredCallTarget, CompilerError> {
        match target {
            CallTarget::UserFunction(function_id) => Ok(LoweredCallTarget::FunctionName(
                self.function_name(*function_id)?.to_owned(),
            )),
            CallTarget::ExternalFunction(id) => {
                self.referenced_external_functions.insert(*id);
                let function_def = self
                    .config
                    .external_package_registry
                    .get_function_by_id(*id)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "JavaScript backend: unknown external function '{}'",
                            id.name()
                        ))
                    })?;
                let lowering = function_def.lowerings.js.as_ref().ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "JavaScript backend: no JS lowering registered for external function '{}'",
                        id.name()
                    ))
                })?;
                match lowering {
                    ExternalJsLowering::RuntimeFunction(name) => {
                        Ok(LoweredCallTarget::FunctionName(name.clone()))
                    }
                    ExternalJsLowering::InlineExpression(template) => {
                        Ok(LoweredCallTarget::InlineExpression {
                            template: template.clone(),
                        })
                    }
                    ExternalJsLowering::ExternalModuleExport { export_name } => {
                        if self.config.external_module_export_glue_enabled {
                            let glue_name =
                                crate::backends::js::external_module_export_glue_function_name(*id);
                            Ok(LoweredCallTarget::FunctionName(glue_name))
                        } else {
                            Err(CompilerError::compiler_error(format!(
                                "JavaScript backend: external module export '{}' requires generated HTML glue before lowering.",
                                export_name
                            )))
                        }
                    }
                }
            }
        }
    }

    /// Emit a complete `HirStatementKind::Call` as JS source lines.
    ///
    /// WHAT: lowers the target, lowers each argument with the correct value-use context,
    /// builds the call expression, and emits the result assignment if present.
    pub(crate) fn emit_call_statement(
        &mut self,
        target: &CallTarget,
        args: &[HirExpression],
        result: &Option<LocalId>,
    ) -> Result<(), CompilerError> {
        let lowered_target = self.lower_call_target(target)?;

        let args = if matches!(target, CallTarget::ExternalFunction(_)) {
            args.iter()
                .map(|arg| self.lower_expression_for_use(arg, JsValueUse::HostCallArgument))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            args.iter()
                .map(|arg| self.lower_expression_for_use(arg, JsValueUse::MothCallArgument))
                .collect::<Result<Vec<_>, _>>()?
        };

        let call = match &lowered_target {
            LoweredCallTarget::FunctionName(name) => {
                format!("{name}({})", args.join(", "))
            }
            LoweredCallTarget::InlineExpression { template } => {
                substitute_inline_expression(template, &args)?
            }
        };

        if let Some(result_local) = result {
            let result_name = self.local_name(*result_local)?;
            if self.call_returns_alias_reference(target) {
                self.emit_line(&format!("__moth_assign_borrow({result_name}, {call});"));
            } else {
                self.emit_line(&format!("__moth_assign_value({result_name}, {call});"));
            }
        } else {
            self.emit_line(&format!("{call};"));
        }

        Ok(())
    }

    /// Whether a call target returns an alias reference that should use borrow assignment.
    fn call_returns_alias_reference(&self, target: &CallTarget) -> bool {
        let CallTarget::UserFunction(function_id) = target else {
            return false;
        };

        self.hir
            .functions
            .iter()
            .find(|function| function.id == *function_id)
            .is_some_and(|function| {
                // Fallible calls return a fresh backend carrier. Any aliasing belongs to the
                // success payload inside that carrier, not to the carrier local itself.
                if self
                    .type_environment
                    .fallible_carrier_slots(function.return_type)
                    .is_some()
                {
                    return false;
                }

                function.return_aliases.len() == 1 && function.return_aliases[0].is_some()
            })
    }
}

/// Substitute lowered argument expressions into an inline expression template.
///
/// WHAT: replaces positional placeholders `#0`, `#1`, ... in the template with the corresponding
/// lowered argument string.
/// WHY: inline expressions are raw JS snippets; arguments are spliced in positionally so the
/// backend emits a single expression instead of a helper call.
pub(super) fn substitute_inline_expression(
    template: &str,
    args: &[String],
) -> Result<String, CompilerError> {
    let mut result = String::new();
    let mut seen_placeholders = vec![0usize; args.len()];
    let mut last_copied_byte = 0usize;
    let mut chars = template.char_indices().peekable();

    while let Some((start_byte, character)) = chars.next() {
        if character != '#' {
            continue;
        }

        let digit_start_byte = start_byte + character.len_utf8();
        let mut digit_end_byte = digit_start_byte;
        while let Some(&(next_byte, next_character)) = chars.peek() {
            if !next_character.is_ascii_digit() {
                break;
            }

            digit_end_byte = next_byte + next_character.len_utf8();
            chars.next();
        }

        if digit_end_byte == digit_start_byte {
            continue;
        }

        let placeholder = &template[start_byte..digit_end_byte];
        let argument_index = template[digit_start_byte..digit_end_byte]
            .parse::<usize>()
            .map_err(|_| {
                CompilerError::compiler_error(format!(
                    "JavaScript backend: inline expression template contains invalid placeholder '{placeholder}'"
                ))
            })?;

        let Some(argument) = args.get(argument_index) else {
            return Err(CompilerError::compiler_error(format!(
                "JavaScript backend: inline expression template contains placeholder '{placeholder}' but only {} argument(s) were provided.",
                args.len()
            )));
        };

        seen_placeholders[argument_index] += 1;
        if seen_placeholders[argument_index] > 1 {
            return Err(CompilerError::compiler_error(format!(
                "JavaScript backend: inline expression template contains duplicate placeholder '{placeholder}'. Each argument must be referenced at most once."
            )));
        }

        result.push_str(&template[last_copied_byte..start_byte]);
        result.push_str(argument);
        last_copied_byte = digit_end_byte;
    }

    result.push_str(&template[last_copied_byte..]);

    for (index, count) in seen_placeholders.iter().enumerate() {
        if *count == 0 {
            let placeholder = format!("#{index}");
            return Err(CompilerError::compiler_error(format!(
                "JavaScript backend: inline expression template is missing placeholder '{placeholder}'"
            )));
        }
    }

    Ok(result)
}
