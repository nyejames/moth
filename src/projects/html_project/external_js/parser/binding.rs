//! Binds JavaScript export declarations to their Moth annotations.
//!
//! WHAT: produces parsed Moth-facing signatures, constants and opaque types from the
//!       annotations and export facts collected by the neighbouring scanners.
//! WHY: annotation binding owns signature/type validation, literal materialisation and
//!      runtime-import deduplication while leaving source scanning to its dedicated modules.

use super::comment_extractor::{AnnotationKind, CommentExtractionResult};
use super::export_scanner::{ExportScanResult, JsExport, JsExportKind, annotation_precedes_export};
use super::parsed_js_module::{
    JsDiagnosticKind, JsParserDiagnostic, JsSourceSpan, ParsedJsConstant, ParsedJsFunction,
    ParsedJsModule, ParsedOpaqueType, ParsedRuntimeImport,
};
use super::signature_parser::{SignatureParseInput, parse_signature};
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::parse::{
    literal_kind_initialises, materialize_normalized_fixed_scalar, parse_numeric_literal,
};
use std::collections::{BTreeMap, BTreeSet};

/// Binds scanner results into the parser-owned module representation.
pub(super) fn bind_js_module(
    source: &str,
    registry: &RuntimeModuleRegistry,
    comment_result: CommentExtractionResult,
    export_result: ExportScanResult,
) -> ParsedJsModule {
    let CommentExtractionResult {
        annotations,
        diagnostics: comment_diagnostics,
    } = comment_result;
    let ExportScanResult {
        exports,
        runtime_imports,
        diagnostics: export_diagnostics,
    } = export_result;

    // Keep extraction and scanner diagnostics ahead of binding diagnostics.
    let mut diagnostics = comment_diagnostics;
    diagnostics.extend(export_diagnostics);

    let mut parsed = ParsedJsModule::empty();
    let mut seen_moth_names = Vec::new();
    let mut seen_js_names = Vec::new();

    // Collect opaque types, which are file-level annotations rather than export bindings.
    for annotation in &annotations {
        if let AnnotationKind::Opaque { type_name } = &annotation.kind {
            if seen_moth_names.contains(type_name) {
                diagnostics.push(JsParserDiagnostic {
                    message: format!("Duplicate Moth-facing name `{type_name}` in JS module."),
                    span: annotation.span.clone(),
                    kind: JsDiagnosticKind::DuplicateMothName,
                });
            } else {
                seen_moth_names.push(type_name.clone());
            }

            parsed.opaque_types.push(ParsedOpaqueType {
                name: type_name.clone(),
                span: annotation.span.clone(),
            });
        }
    }

    // Export names occupy one JS namespace, including exports that fail annotation binding.
    for export in &exports {
        if seen_js_names.contains(&export.js_name) {
            diagnostics.push(JsParserDiagnostic {
                message: format!(
                    "Duplicate JS export name `{}` in JS module.",
                    export.js_name
                ),
                span: export.span.clone(),
                kind: JsDiagnosticKind::DuplicateJsExportName,
            });
        } else {
            seen_js_names.push(export.js_name.clone());
        }
    }

    let mut annotated_exports = vec![false; exports.len()];
    let mut export_index = 0;
    for annotation in annotations {
        let moth_name = match &annotation.kind {
            AnnotationKind::Sig { moth_name, .. } | AnnotationKind::Const { moth_name, .. } => {
                moth_name
            }
            AnnotationKind::Opaque { .. } => continue,
        };
        if seen_moth_names.contains(moth_name) {
            diagnostics.push(JsParserDiagnostic {
                message: format!("Duplicate Moth-facing name `{moth_name}` in JS module."),
                span: annotation.span.clone(),
                kind: JsDiagnosticKind::DuplicateMothName,
            });
        } else {
            seen_moth_names.push(moth_name.clone());
        }

        let matched_index =
            find_next_export_after(&exports, annotation.span.byte_end, export_index).filter(
                |index| {
                    annotation_precedes_export(
                        source,
                        registry,
                        annotation.span.byte_end,
                        exports[*index].span.byte_start,
                    )
                },
            );
        let Some(index) = matched_index else {
            let (directive, kind) = match &annotation.kind {
                AnnotationKind::Const { .. } => {
                    ("@moth.const", JsDiagnosticKind::MissingExportAfterConst)
                }
                _ => ("@moth.sig", JsDiagnosticKind::MissingExportAfterSig),
            };
            diagnostics.push(JsParserDiagnostic {
                message: format!(
                    "`{directive}` for `{moth_name}` is not followed by a supported JS export declaration."
                ),
                span: annotation.span,
                kind,
            });
            continue;
        };
        export_index = index;
        let export = &exports[index];
        if annotated_exports[index] {
            diagnostics.push(JsParserDiagnostic {
                message: format!(
                    "JS export `{}` has more than one binding annotation.",
                    export.js_name
                ),
                span: annotation.span,
                kind: JsDiagnosticKind::AnnotationExportKindMismatch,
            });
            continue;
        }
        annotated_exports[index] = true;

        match (&annotation.kind, &export.kind) {
            (
                AnnotationKind::Sig { signature_text, .. },
                JsExportKind::Callable { parameter_count },
            ) => {
                let signature_result = parse_signature(SignatureParseInput {
                    text: signature_text.clone(),
                    base_byte: annotation.span.byte_start,
                });
                diagnostics.extend(signature_result.diagnostics);

                let abi_count = signature_result.signature.abi_parameter_count();
                if abi_count != *parameter_count {
                    diagnostics.push(JsParserDiagnostic {
                        message: format!(
                            "Annotated JS export `{}` has {} Moth ABI parameter(s) but {} JS parameter(s). \
                             Receiver `this` counts as the first JS parameter. Moth JS module exports must use one plain JS parameter per Moth ABI parameter.",
                            export.js_name, abi_count, parameter_count
                        ),
                        span: export.span.clone(),
                        kind: JsDiagnosticKind::ArityMismatch,
                    });
                }

                let parsed_function = ParsedJsFunction {
                    moth_name: moth_name.clone(),
                    js_name: export.js_name.clone(),
                    signature: signature_result.signature,
                    annotation_span: annotation.span,
                    export_span: export.span.clone(),
                };
                if parsed_function.signature.has_receiver() {
                    parsed.receiver_methods.push(parsed_function);
                } else {
                    parsed.free_functions.push(parsed_function);
                }
            }
            (AnnotationKind::Const { type_name, .. }, JsExportKind::Constant { literal_span }) => {
                if type_name != "U32" {
                    diagnostics.push(JsParserDiagnostic {
                        message: "`@moth.const` supports only the fixed `U32` type.".to_string(),
                        span: annotation.span,
                        kind: JsDiagnosticKind::InvalidConstant,
                    });
                    continue;
                }
                let Some(literal_span) = literal_span else {
                    // The scanner has already diagnosed an invalid declaration boundary.
                    continue;
                };
                let spelling = &source[literal_span.byte_start..literal_span.byte_end];
                match materialize_u32_constant(spelling) {
                    Ok(value) => parsed.constants.push(ParsedJsConstant {
                        moth_name: moth_name.clone(),
                        js_name: export.js_name.clone(),
                        value,
                        annotation_span: annotation.span,
                        export_span: export.span.clone(),
                        literal_span: literal_span.clone(),
                    }),
                    Err(message) => diagnostics.push(JsParserDiagnostic {
                        message,
                        span: literal_span.clone(),
                        kind: JsDiagnosticKind::InvalidConstant,
                    }),
                }
            }
            _ => diagnostics.push(JsParserDiagnostic {
                message: format!(
                    "Binding annotation for `{moth_name}` does not match JS export `{}`. \
                     Use `@moth.sig` for callable exports and `@moth.const` for literal constants.",
                    export.js_name
                ),
                span: annotation.span,
                kind: JsDiagnosticKind::AnnotationExportKindMismatch,
            }),
        }
    }

    for (index, export) in exports.iter().enumerate() {
        if !annotated_exports[index] {
            diagnostics.push(JsParserDiagnostic {
                message: format!(
                    "JavaScript export `{}` is not annotated with `@moth.sig` or `@moth.const`. \
                     Every export in a Moth JS module must be explicitly annotated. Keep private helpers unexported.",
                    export.js_name
                ),
                span: export.span.clone(),
                kind: JsDiagnosticKind::UnannotatedExport,
            });
        }
    }

    validate_signature_type_names(&parsed, &mut diagnostics);
    parsed.runtime_imports = deduplicate_runtime_imports(runtime_imports);
    parsed.diagnostics = diagnostics;
    parsed
}

/// Finds the next export at or after the end of an annotation's comment block.
fn find_next_export_after(
    exports: &[JsExport],
    byte_offset: usize,
    start_index: usize,
) -> Option<usize> {
    for (index, export) in exports.iter().enumerate().skip(start_index) {
        if export.span.byte_start >= byte_offset {
            return Some(index);
        }
    }
    None
}

fn validate_signature_type_names(
    parsed: &ParsedJsModule,
    diagnostics: &mut Vec<JsParserDiagnostic>,
) {
    let opaque_names = parsed
        .opaque_types
        .iter()
        .map(|opaque| opaque.name.as_str())
        .collect::<Vec<_>>();

    for function in parsed
        .free_functions
        .iter()
        .chain(parsed.receiver_methods.iter())
    {
        validate_function_signature_types(function, &opaque_names, diagnostics);
    }
}

fn validate_function_signature_types(
    function: &ParsedJsFunction,
    opaque_names: &[&str],
    diagnostics: &mut Vec<JsParserDiagnostic>,
) {
    if function.signature.has_unsupported_generic_parameters {
        return;
    }

    for parameter in &function.signature.parameters {
        validate_type_name(
            &parameter.type_name,
            &function.moth_name,
            &function.annotation_span,
            opaque_names,
            diagnostics,
        );
    }

    for return_type in &function.signature.returns {
        validate_type_name(
            &return_type.type_name,
            &function.moth_name,
            &function.annotation_span,
            opaque_names,
            diagnostics,
        );
    }
}

fn validate_type_name(
    type_name: &str,
    function_name: &str,
    span: &JsSourceSpan,
    opaque_names: &[&str],
    diagnostics: &mut Vec<JsParserDiagnostic>,
) {
    if !should_validate_known_type_name(type_name) {
        return;
    }

    if is_builtin_signature_type(type_name) || opaque_names.contains(&type_name) {
        return;
    }

    diagnostics.push(JsParserDiagnostic {
        message: format!(
            "Unknown external type `{type_name}` in `@moth.sig` for `{function_name}`. Declare it with `@moth.opaque {type_name}` before using it in Moth JS module signatures."
        ),
        span: span.clone(),
        kind: JsDiagnosticKind::UnknownExternalType,
    });
}

/// Enforce the intentionally narrower JS spelling before calling the shared numeric owner.
/// Moth separators and leading zeroes are not legal literals in this emitted ESM subset.
fn materialize_u32_constant(spelling: &str) -> Result<FixedScalarValue, String> {
    if spelling.is_empty()
        || !spelling.bytes().all(|byte| byte.is_ascii_digit())
        || (spelling.len() > 1 && spelling.starts_with('0'))
    {
        return Err(
            "`@moth.const` requires an unsigned decimal integer literal without leading zeroes, \
             separators, alternate radices or executable expressions."
                .to_string(),
        );
    }

    let literal = parse_numeric_literal(spelling)
        .map_err(|reason| format!("Invalid `@moth.const` numeric literal: {reason:?}."))?;
    if !literal_kind_initialises(literal.kind, FixedScalar::U32) {
        return Err("The authored numeric literal cannot initialise `U32`.".to_string());
    }
    materialize_normalized_fixed_scalar(&literal.normalized_text, false, FixedScalar::U32).map_err(
        |_| "`@moth.const` literal must be in the U32 range 0 through 4294967295.".to_string(),
    )
}

fn is_builtin_signature_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "Int" | "Uint" | "Float" | "U32" | "F32" | "Bool" | "String" | "Char"
    )
}

fn should_validate_known_type_name(type_name: &str) -> bool {
    !type_name.is_empty()
        && !matches!(
            type_name,
            "Void" | "void" | "None" | "none" | "Unit" | "unit" | "()"
        )
        && type_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Deduplicates imports so provider and backend consumers see one entry per runtime module.
fn deduplicate_runtime_imports(imports: Vec<ParsedRuntimeImport>) -> Vec<ParsedRuntimeImport> {
    let mut by_module: BTreeMap<String, (BTreeSet<String>, JsSourceSpan)> = BTreeMap::new();

    for import in imports {
        let entry = by_module
            .entry(import.module_name)
            .or_insert_with(|| (BTreeSet::new(), import.span));
        for name in import.imported_names {
            entry.0.insert(name);
        }
    }

    by_module
        .into_iter()
        .map(|(module_name, (names, span))| ParsedRuntimeImport {
            module_name,
            imported_names: names.into_iter().collect(),
            span,
        })
        .collect()
}
