//! Symbol map construction for the JavaScript backend.
//!
//! Builds deterministic JS identifier names for every HIR function, local, and
//! field before code emission begins.

use crate::backends::js::JsEmitter;
use crate::backends::js::identifiers::{is_js_reserved, sanitize_identifier};
use crate::compiler_frontend::builtins::error_type::{ERROR_FIELD_CODE, ERROR_FIELD_MESSAGE};

const BUILTIN_ERROR_MESSAGE_FIELD_ID: u32 = 0;
const BUILTIN_ERROR_CODE_FIELD_ID: u32 = 1;

/// Deterministic JS field name for the builtin `Error.message` field.
///
/// WHAT: mirrors the symbol shape generated for the canonical builtin `Error` struct field.
/// WHY: backend-created `Error` values must use the same lowered field names as source-authored
/// `Error(...)` structs, including compact release symbols.
pub(crate) fn builtin_error_message_js_field_name(release_build: bool) -> String {
    build_symbol_raw(
        "fld",
        BUILTIN_ERROR_MESSAGE_FIELD_ID,
        ERROR_FIELD_MESSAGE,
        release_build,
    )
}

/// Deterministic JS field name for the builtin `Error.code` field.
pub(crate) fn builtin_error_code_js_field_name(release_build: bool) -> String {
    build_symbol_raw(
        "fld",
        BUILTIN_ERROR_CODE_FIELD_ID,
        ERROR_FIELD_CODE,
        release_build,
    )
}

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn build_symbol_maps(&mut self) {
        self.build_function_symbols();
        self.build_local_symbols();
        self.build_field_symbols();
    }

    fn build_function_symbols(&mut self) {
        let mut function_ids = self
            .hir
            .functions
            .iter()
            .map(|function| function.id)
            .collect::<Vec<_>>();
        function_ids.sort_by_key(|function_id| function_id.0);

        for function_id in function_ids {
            let leaf_name_hint = self
                .hir
                .side_table
                .resolve_function_name(function_id, self.string_table)
                .unwrap_or("fn");
            let raw_name = self.build_symbol_raw("fn", function_id.0, leaf_name_hint);
            let js_name = self.assign_unique_identifier(&raw_name);
            self.function_name_by_id.insert(function_id, js_name);
        }
    }

    fn build_local_symbols(&mut self) {
        let mut local_specs = Vec::new();

        for block in &self.hir.blocks {
            for local in &block.locals {
                let raw_name = self
                    .hir
                    .side_table
                    .local_name_path(local.id)
                    .and_then(|path| path.name_str(self.string_table))
                    .map(|leaf_name| self.build_symbol_raw("l", local.id.0, leaf_name))
                    .unwrap_or_else(|| self.build_symbol_raw("l", local.id.0, "local"));

                local_specs.push((local.id, raw_name));
            }
        }

        local_specs.sort_by_key(|(local_id, _)| local_id.0);
        local_specs.dedup_by_key(|(local_id, _)| local_id.0);

        for (local_id, raw_name) in local_specs {
            let js_name = self.assign_unique_identifier(&raw_name);
            self.local_name_by_id.insert(local_id, js_name);
        }
    }

    fn build_field_symbols(&mut self) {
        let mut field_specs = Vec::new();

        for hir_struct in &self.hir.structs {
            for field in &hir_struct.fields {
                let raw_name = self
                    .hir
                    .side_table
                    .field_name_path(field.id)
                    .and_then(|path| path.name_str(self.string_table))
                    .map(|leaf_name| self.build_symbol_raw("fld", field.id.0, leaf_name))
                    .unwrap_or_else(|| self.build_symbol_raw("fld", field.id.0, "field"));

                field_specs.push((field.id, raw_name));
            }
        }

        field_specs.sort_by_key(|(field_id, _)| field_id.0);
        field_specs.dedup_by_key(|(field_id, _)| field_id.0);

        for (field_id, raw_name) in field_specs {
            let js_name = self.assign_unique_identifier(&raw_name);
            self.field_name_by_id.insert(field_id, js_name);
        }
    }

    fn assign_unique_identifier(&mut self, raw: &str) -> String {
        let mut identifier = sanitize_identifier(raw);

        if is_js_reserved(&identifier) {
            identifier = format!("_{identifier}");
        }

        if identifier.is_empty() {
            identifier = "_value".to_owned();
        }

        let mut candidate = identifier.clone();
        let mut suffix = 1usize;

        while self.used_identifiers.contains(&candidate) {
            candidate = format!("{identifier}_{suffix}");
            suffix += 1;
        }

        self.used_identifiers.insert(candidate.clone());
        candidate
    }

    fn build_symbol_raw(&self, kind_tag: &str, id: u32, leaf_name_hint: &str) -> String {
        build_symbol_raw(
            kind_tag,
            id,
            leaf_name_hint,
            self.use_release_symbol_names(),
        )
    }

    fn use_release_symbol_names(&self) -> bool {
        !self.config.pretty
    }
}

fn build_symbol_raw(
    kind_tag: &str,
    id: u32,
    leaf_name_hint: &str,
    release_symbol_names: bool,
) -> String {
    if release_symbol_names {
        format!("b_{kind_tag}{id}")
    } else {
        format!("moth_{leaf_name_hint}_{kind_tag}{id}")
    }
}
