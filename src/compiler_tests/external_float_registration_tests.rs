//! Real-source regression for directly registered raw Fixed F64 external boundaries.
//!
//! WHAT: compiles a real Moth project whose only external function is registered directly on the
//!       builder surface as a raw Fixed F64 boundary (a JS runtime helper with no Wasm lowering),
//!       lowers the canonical module HIR through the production JavaScript backend entry, and
//!       executes the emitted bundle in Node under an ambient Float32 profile. The host helper
//!       returns a driver-set raw payload, so no invalid value crosses a Moth parameter before the
//!       boundary itself.
//! WHY:  the Phase 3 exit contract is that a raw fixed F64 boundary keeps binary64 width, that the
//!       finite guard cannot be bypassed, and that a declared `Error!` route delivers the shared
//!       boundary code as a returned error. Those are runtime properties of the generated
//!       JavaScript, so only executing the emitted bundle proves them, and the boundary must enter
//!       through the normal compilation service with an injected custom external registry rather
//!       than an annotated provider wrapper.
//!
//! The HIR-side invariants are deliberately not re-asserted here: the selection fact is owned by
//! `ast/expressions/tests/external_call_float_selection_tests.rs`, the exact-type guard allocation
//! by `hir/tests/hir_expression_lowering_tests.rs` (fixed F64 call allocates and validates its
//! exact type), and the carrier matrix (signed zero, coercion rejection, the full finite and
//! non-finite set) by `backends/js/tests/numeric_statements.rs`. This owner adds the real-source
//! registration route and the runtime outcomes that route can lose.
//!
//! The fixture follows the canonical project path: `compile_project_frontend_with_inputs` loads
//! `config.moth` against the builder surface carrying the registered function, then runs the
//! production Stage 0 plus module compilation sequence (AST, HIR validation, borrow validation,
//! numeric proofs). `lower_hir_to_js` then produces the JavaScript exactly as the HTML builder's
//! JS path calls it.

use crate::backends::js::{JsLoweringConfig, lower_hir_to_js};
use crate::build_system::BuildProfile;
use crate::build_system::create_project_modules::{
    FrontendCompilationMode, compile_project_frontend_with_inputs,
};
use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalFunctionDef, ExternalFunctionLowerings, ExternalJsLowering,
    ExternalReturnSlot, ExternalSignatureType,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{CONFIG_FILE_NAME, Config};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

/// Ambient numeric widths both fixtures compile under.
///
/// WHY: the regression exists because the ambient Float32 profile must never round, widen or
///      mis-type a boundary whose registered return type is fixed F64.
const AMBIENT_FLOAT32_PROFILE: NumericProfile = NumericProfile {
    int_width: IntWidth::Bits32,
    float_precision: FloatPrecision::Bits32,
};

/// Fixture project config: the entry root the fixture module is written into.
const CONFIG_SOURCE: &str = concat!(
    "project #= (\n",
    "    name = \"raw_fixed_f64_registration\",\n",
    "    entry_root = \"src\",\n",
    ")\n",
    "html #= ()\n",
);

/// Infallible lane: the raw boundary carries no declared error slot, so its finite guard must trap.
const FINITE_SOURCE: &str = concat!(
    "@test/default raw_f64_finite\n",
    "\n",
    "finite_width || -> F64:\n",
    "    return raw_f64_finite()\n",
    ";\n",
);

/// Fallible lane: the raw boundary declares the builtin `Error!` slot and the source forwards the
/// raw success value with a direct postfix return, so its finite guard must stay recoverable.
const FORWARD_SOURCE: &str = concat!(
    "@test/default raw_f64_forward\n",
    "\n",
    "forward || -> F64, Error!:\n",
    "    return raw_f64_forward()!\n",
    ";\n",
);

/// One directly registered raw fixed F64 external function for a fixture.
struct RawFixedF64Function {
    /// Leaf name in `@test/default`, which is also the name the source imports.
    name: &'static str,
    /// JS runtime helper the registered lowering must call.
    js_helper: &'static str,
    /// Whether the registered signature declares the builtin `Error!` slot.
    declares_error_slot: bool,
}

impl RawFixedF64Function {
    /// The registration metadata a real builder would publish for this boundary.
    fn definition(&self) -> ExternalFunctionDef {
        ExternalFunctionDef {
            name: self.name.to_owned(),
            parameters: Vec::new(),
            returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::Fixed(
                FixedScalar::F64,
            ))],
            error_return_type: self
                .declares_error_slot
                .then_some(ExternalSignatureType::BuiltinError),
            lowerings: ExternalFunctionLowerings {
                js: Some(ExternalJsLowering::RuntimeFunction(
                    self.js_helper.to_owned(),
                )),
                // A raw Wasm lowering would keep the guard too, but this fixture declares no Wasm
                // lowering at all: the JS runtime helper is the only lowering, so HIR owns the
                // guard for the raw Fixed F64 result.
                wasm: None,
            },
        }
    }
}

/// A compiled fixture project's emitted JavaScript and the emitted name of its source function.
struct CompiledRawFixedF64Fixture {
    js_source: String,
    function_name: String,
}

/// Compile one fixture project through the canonical services and lower its HIR to JavaScript.
///
/// WHY: the fixture must exercise the normal compilation service with an injected custom external
///      registry, not a hand-built AST/HIR or a second production orchestrator. The registered
///      function lives on `BuilderSurface::binding_packages`, the same surface Stage 0 clones into
///      the module's effective registry that backend lowering consumes.
fn compile_raw_fixed_f64_fixture(
    source: &str,
    source_function: &str,
    function: &RawFixedF64Function,
) -> CompiledRawFixedF64Fixture {
    let temporary_project = tempfile::tempdir().expect("fixture should create a temporary project");
    let root = temporary_project.path().to_path_buf();
    fs::write(root.join(CONFIG_FILE_NAME), CONFIG_SOURCE)
        .expect("fixture should write config.moth");
    fs::create_dir_all(root.join("src")).expect("fixture should create the entry root");
    fs::write(root.join("src/@page.moth"), source).expect("fixture should write the entry module");

    // The registered boundary lives on the builder surface's external registry, the same surface
    // the canonical service clones into each module's effective registry for frontend resolution
    // and backend lowering.
    let mut surface = BuilderSurface::with_mandatory_core();
    surface
        .binding_packages
        .register_function(function.definition())
        .expect("fixture external function registration should not collide");

    let style_directives = StyleDirectiveRegistry::built_ins();
    let build_config_inputs = BuildConfigInputSet::new();
    let mut config = Config::new(root);
    let mut string_table = StringTable::new();
    let mut project_source_files = None;
    let frontend = compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        AMBIENT_FLOAT32_PROFILE,
        None,
        &style_directives,
        &mut surface,
        &mut string_table,
        &mut project_source_files,
        &build_config_inputs,
        FrontendCompilationMode::Canonical,
    )
    .expect("the raw fixed F64 registration fixture must compile through the canonical service");

    let module = frontend
        .project
        .successful_artefacts_in_module_id_order()
        .find(|artefact| {
            artefact
                .module
                .metadata
                .entry_point
                .ends_with(Path::new("src/@page.moth"))
        })
        .map(|artefact| &artefact.module)
        .expect("the fixture project should compile its entry module");

    let mut js_config = JsLoweringConfig::direct_js(false, AMBIENT_FLOAT32_PROFILE);
    js_config.external_package_registry = Arc::clone(&module.link_facts.external_package_registry);
    let js = lower_hir_to_js(
        &module.executable.hir,
        &module.executable.numeric_proofs,
        &string_table,
        js_config,
        &module.executable.type_environment,
        &module.executable.path_table,
    )
    .expect("the validated fixture HIR should lower to JavaScript");

    // Resolution of the emitted symbol stays inside the fixture so tests bind behavior, not names.
    let function_name = module
        .executable
        .hir
        .functions
        .iter()
        .find(|hir_function| {
            module
                .executable
                .hir
                .side_table
                .function_name_path(hir_function.id)
                .and_then(|path| module.executable.path_table.component(path))
                .is_some_and(|component| string_table.resolve(component) == source_function)
        })
        .and_then(|hir_function| js.function_name_by_id.get(&hir_function.id).cloned())
        .unwrap_or_else(|| {
            panic!("the fixture source function `{source_function}` must emit a JavaScript name")
        });

    CompiledRawFixedF64Fixture {
        js_source: js.source,
        function_name,
    }
}

/// Runs one generated JavaScript program through Node and returns its stdout.
///
/// WHY: exact boundary precision and guard delivery are properties of the executed module, not of
///      the emitted source text.
fn run_javascript(source: &str) -> String {
    let output = Command::new("node")
        .args(["--eval", source])
        .output()
        .expect("Node.js is required for the raw fixed F64 runtime regression");
    assert!(
        output.status.success(),
        "Node.js runtime failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Node.js output is UTF-8")
}

/// Verifies an infallible raw fixed F64 boundary keeps binary64 width under a Float32 profile.
///
/// WHAT: a zero-argument host helper returns a driver-set raw payload at the registered boundary.
///       A finite binary64-range payload must arrive unchanged, and a non-finite payload must take
///       the infallible boundary's trap route.
/// WHY:  the boundary is raw (no Wasm lowering), so HIR owns the finite guard, and the resolved
///      fixed F64 type owns the width. Rounding the guard at the ambient Float32 profile would
///      overflow `1e300` to infinity, and skipping the guard would deliver `NaN`.
#[test]
fn infallible_raw_fixed_f64_boundary_keeps_binary64_width_under_ambient_float32() {
    let fixture = compile_raw_fixed_f64_fixture(
        FINITE_SOURCE,
        "finite_width",
        &RawFixedF64Function {
            name: "raw_f64_finite",
            js_helper: "__moth_raw_f64_finite",
            declares_error_slot: false,
        },
    );

    let driver = format!(
        r#"
function attempt() {{
    try {{
        return "value:" + String({function_name}());
    }} catch (error) {{
        return error instanceof Error ? "trap" : "threw:" + typeof error;
    }}
}}
const rows = [];
rows.push("finite=" + attempt());
rawBoundaryPayload = NaN;
rows.push("nonfinite=" + attempt());
console.log(rows.join("\n"));
"#,
        function_name = fixture.function_name,
    );
    let stub = concat!(
        "let rawBoundaryPayload = 1e300;\n",
        "function __moth_raw_f64_finite() { return rawBoundaryPayload; }\n",
    );

    assert_eq!(
        run_javascript(&format!("{stub}\n{}\n{driver}", fixture.js_source)),
        "finite=value:1e+300\n\
         nonfinite=trap\n",
        "a raw infallible fixed F64 boundary must return a finite binary64 payload unchanged and \
         trap on a non-finite payload"
    );
}

/// Verifies a fallible raw fixed F64 direct-forward boundary delivers the declared boundary code.
///
/// WHAT: a zero-argument host helper returns a driver-set raw payload through the registered
///       builtin `Error!` slot, and the source forwards the raw success value with a direct postfix
///       return. A finite binary64-range payload must arrive unchanged on the success carrier, and
///       a non-finite payload must become a returned error carrying the shared boundary code.
/// WHY:  a source-declared `Error!` route is a contract: recovery callers branch on a returned
///      error code, so the guard failure must not become a trap, and the raw success value must
///      keep its resolved width instead of rounding to the ambient Float32 profile.
#[test]
fn fallible_raw_fixed_f64_direct_forward_delivers_declared_boundary_under_ambient_float32() {
    let fixture = compile_raw_fixed_f64_fixture(
        FORWARD_SOURCE,
        "forward",
        &RawFixedF64Function {
            name: "raw_f64_forward",
            js_helper: "__moth_raw_f64_forward",
            declares_error_slot: true,
        },
    );

    let driver = format!(
        r#"
function attempt() {{
    let produced;
    try {{
        produced = {function_name}();
    }} catch (error) {{
        return error instanceof Error ? "trap" : "threw:" + typeof error;
    }}
    if (produced && produced.tag === "ok") {{
        return "ok:" + String(produced.value);
    }}
    return "err:" + __moth_error_code(produced.value);
}}
const rows = [];
rows.push("finite=" + attempt());
rawBoundaryPayload = NaN;
rows.push("nonfinite=" + attempt());
console.log(rows.join("\n"));
"#,
        function_name = fixture.function_name,
    );
    let stub = concat!(
        "let rawBoundaryPayload = 1e300;\n",
        "function __moth_raw_f64_forward() { return { tag: \"ok\", value: rawBoundaryPayload }; }\n",
    );
    let rejected = format!("err:{}", BuiltinErrorCode::FloatBoundaryNonFinite.as_u32());

    assert_eq!(
        run_javascript(&format!("{stub}\n{}\n{driver}", fixture.js_source)),
        format!(
            "finite=ok:1e+300\n\
             nonfinite={rejected}\n"
        ),
        "a declared Error! boundary must return the shared Float boundary code for a non-finite \
         payload while keeping a finite binary64 payload on the success carrier"
    );
}
