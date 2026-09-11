# Moth codebase style/redundancy audit                                                                         
                                                                                                              
 Audited: the four highest-churn or largest production areas — ast/generic_functions/materialisation* (7 000  
 LOC across one 4 155-line file plus 5 children), build_system/create_project_modules Stage 0 (37 commits in  
 6 weeks on compilation.rs alone), compiler_messages (16 700 LOC), projects/html_project,                     
 analysis/borrow_checker/boracle, plus a repo-wide census of dead-code suppression and test-only production   
 API.                                                                                                         
                                                                                                              
 Governing docs: docs/src/developer-docs/style-guide/style-guide.mtf (one responsibility per module, ~2       
 000-line files, no forwarding shims, no test-only code in production files, justified lint suppression),     
 testing.mtf (integration cases own user-visible behaviour, one primary owner, tests never in production      
 files), docs/compiler-design-overview.md (one owner per semantic fact), docs/build-system-design.md (Stage 0 
 and the build/compiler split), index.md.                                                                     
                                                                                                              
 State: stage boundaries and diagnostic lanes hold up — I found no build-side code orchestrating compiler     
 stages and no CompilerError misused for user-facing source failures. The damage is concentrated in three     
 shapes: hand-written mirror/publish pipelines forked into two copies, blanket allow(dead_code) hiding        
 genuinely dead surfaces, and test-only API grown into production files. About 1 000–1 500 LOC is removable   
 without behaviour change, and two files need splitting.                                                      
                                                                                                              
 ────────────────────────────────────────────────────────────────────────────────                             
                                                                                                              
 1. Generic materialisation has two forked sidecar pipelines and 3 lanes in one file — MERGE then SPLIT       
                                                                                                              
 Issue. materialise_ast exists twice with the same 9-step body, and they have already drifted.                
                                                                                                              
 Where.                                                                                                       
 - src/compiler_frontend/ast/generic_functions/materialisation.rs:656-831 —                                   
   GenericTemplateArtefact::materialise_ast (frozen artefact lane)                                            
 - :3603-3832 — ModuleMaterialisationPreparation::materialise_ast (live preparation lane)                     
 - :2319-2360 — inherit_nominal_blueprints / inherit_artefact_nominal_blueprints                              
 - :2120-2142 and :4049-4068 — fallible-carrier success fold, twice in one file                               
 - :508-525 and :2001-2019 — two collect_namespace_source_paths recursive walkers                             
                                                                                                              
 Evidence. Both pipelines run fork_materialisation_string_table → StableBodySyntax::materialise →             
 generated_file_value_resolution_services → AstBuildContext{root_role: Support, …} → build environment →      
 intern_generated_canonical_type loop → install_generated_request_evidence → join_str("__generated_instance") 
 → AstEmitter::emit_generated_request → AstFinalizer::finalize → inherit blueprints. The request literals at  
 :783-792 and :3786-3795 are token-identical. Three verified divergences that are almost certainly            
 unintended:                                                                                                  
 - :800-802 attaches .with_generic_call_site_identity_handle(requester_context.frozen_identity_handle); :3802 
   does not.                                                                                                  
 - :817-824 inherits requester then artefact blueprints; :3818-3825 inherits self then requester.             
 - Error mapping at :770 uses string_table_ref, at :3771 uses &self.string_table.                             
                                                                                                              
 Only the preparation lane handles nested bodies (:3657-3689). The two blueprint-inherit functions differ     
 solely in parameter type and one word of an error string, and both source types already implement            
 MaterialisationNominalSource (:350-366).                                                                     
                                                                                                              
 Why it matters. This is the highest-churn file in the repo carrying a hand-synchronised fork of the          
 generated-function emission path. docs/compiler-design-overview.md names exactly one materialisation         
 contract; two copies means the call-site identity handle is present on one path and absent on the other with 
 nothing flagging it.                                                                                         
                                                                                                              
 Fix.                                                                                                         
 - MERGE: extract emit_materialised_sidecar(phase_context, environment, identity, type_arguments,             
   requester_context, call_span) owning steps 5–9. Each caller keeps only body materialisation and            
   environment construction. Reconcile the three divergences deliberately. ≈ −150 LOC.                        
 - MERGE: one generic inherit_nominal_blueprints<N: MaterialisationNominalSource> after adding an iterator to 
   the trait (−20); one shared fallible_carrier(success_ids, error_id, type_env) (−20);                       
   GenericFunctionInstanceKey::new + GenericFunctionInstantiationRequest::generated constructors in           
   instances.rs so "__generated_instance" appears once (−25).                                                 
 - SPLIT: materialisation.rs owns three lanes — stable capture types (:139-506), frozen-artefact emission     
   (:655-1680), visibility/namespace capture (:1684-2020), preparation freeze (:2144-3400), sidecar build +   
   evidence (:3400-4146). Move to                                                                             
   materialisation/{stable_types,artefact_emit,visibility,preparation_freeze,sidecar_build}.rs following the  
   existing child-module pattern the file header already describes. Net 0 LOC, max file ~1 250.               
                                                                                                              
 Leave local: the 31 Stable* mirror types themselves (grep count across materialisation.rs + children). They  
 exist because a frozen artefact must survive without the donor environment; collapsing them into the live    
 types would re-couple the lanes. The twin collect_namespace_source_paths walkers are a direct consequence —  
 LEAVE-LOCAL, but note the pairing in a comment.                                                              
                                                                                                              
 ────────────────────────────────────────────────────────────────────────────────                             
                                                                                                              
 2. #![allow(dead_code)] blankets hide four genuinely dead surfaces — REMOVE                                  
                                                                                                              
 Issue. 189 dead-code suppressions: 10 file-level #![allow(dead_code)] and 101 item-level allows with no      
 justification comment (excluding legitimate cfg_attr(not(feature = …)) gates). The style guide permits       
 allow(dead_code) "only for clearly identified planned work or test-only code". The blankets are suppressing  
 real dead code right now.                                                                                    
                                                                                                              
 Where and what they hide (all caller counts verified by repo-wide grep):                                     
                                                                                                              
 ┌─────────────────────────────────┬───────────────────────────────────┬────────────────────────────────────┐ 
 │ Suppression                     │ Hidden dead surface               │ Verified callers                   │ 
 ├─────────────────────────────────┼───────────────────────────────────┼────────────────────────────────────┤ 
 │ boracle/reducer.rs:11           │ reduce_problem (:160),            │ 0 production; only                 │ 
 │ (file-level)                    │ render_fixture_skeleton (:1839),  │ oracle/tests/reducer.rs (15 sites) │ 
 │                                 │ ReductionPass — 2 363 LOC         │ + oracle/tests/campaign.rs:130.    │ 
 │                                 │                                   │ Re-exported from                   │ 
 │                                 │                                   │ boracle/mod.rs:52-53.              │ 
 ├─────────────────────────────────┼───────────────────────────────────┼────────────────────────────────────┤ 
 │ boracle/differential.rs:11      │ compare_problem_parts             │ 0 production; reached only from    │ 
 │                                 │                                   │ unreachable reduce_problem + tests │ 
 ├─────────────────────────────────┼───────────────────────────────────┼────────────────────────────────────┤ 
 │ boracle/oracle/mod.rs:20        │ generated_problem                 │ 0 production; 5 test files         │ 
 │                                 │ (generator.rs:93)                 │                                    │ 
 ├─────────────────────────────────┼───────────────────────────────────┼────────────────────────────────────┤ 
 │ boracle/report.rs:9             │ final_use_candidate_for_place/_at │ 0 production; boracle/tests/mod.rs │ 
 │                                 │ /_for_origin_after_event          │ + projects/tests/boracle_tests.rs  │ 
 │                                 │ (:76-110)                         │                                    │ 
 ├─────────────────────────────────┼───────────────────────────────────┼────────────────────────────────────┤ 
 │ boracle/relations.rs:12         │ proven_disjoint (:174),           │ 0 anywhere for from_fresh_origins; │ 
 │                                 │ from_fresh_origins (:357)         │ 3 test sites for proven_disjoint   │ 
 ├─────────────────────────────────┼───────────────────────────────────┼────────────────────────────────────┤ 
 │ source/span.rs (19 bare item    │ overlaps_with, contains_with,     │ 0 production; only                 │ 
 │ allows)                         │ source_order_with, is_empty_with, │ source/tests/span_tests.rs.        │ 
 │                                 │ insertion_point,                  │ (resolve_with is production-live   │ 
 │                                 │ SourceSpanDatabase::{start,end,ov │ via build_config_contract.rs:297,  │ 
 │                                 │ erlaps,contains}                  │ declaration_shell.rs:274 — do not  │ 
 │                                 │                                   │ remove that one.)                  │ 
 ├─────────────────────────────────┼───────────────────────────────────┼────────────────────────────────────┤ 
 │ semantic_identity.rs (19 bare)  │ unaudited — needs the same pass   │                                    │ 
 ├─────────────────────────────────┼───────────────────────────────────┼────────────────────────────────────┤ 
 │ html_project/moth_template/mod. │ see finding 6                     │                                    │ 
 │ rs:14                           │                                   │                                    │ 
 └─────────────────────────────────┴───────────────────────────────────┴────────────────────────────────────┘ 
                                                                                                              
 differential.rs:8-11 confesses the state in its own header ("until those callers land");                     
 boracle/service.rs:453-463's dump match has no reduce arm, so the documented reduction workflow is           
 unreachable by the developers it exists for.                                                                 
                                                                                                              
 Why it matters. The suppressions disable exactly the signal that would have surfaced findings 1, 6 and 7     
 automatically. Every pub(crate) item in those ten files is now reachability-suspect, which makes any future  
 cleanup an archaeology exercise.                                                                             
                                                                                                              
 Fix.                                                                                                         
 - REMOVE the 10 file-level blankets. Re-annotate only individually-dead items, each naming its owning plan   
   or test module.                                                                                            
 - MOVE the span query API (overlaps_with, contains_with, source_order_with, is_empty_with, insertion_point,  
   the SourceSpanDatabase generics) out of source/span.rs into source/tests/span_tests.rs as free helpers     
   over the public range, or delete: ≈ −120 LOC.                                                              
 - MOVE proven_disjoint / from_fresh_origins into boracle/tests/relations.rs; REMOVE the three                
   final_use_candidate_* queries and the origin_last_use_after_event rows they force every report to compute: 
   ≈ −60 LOC.                                                                                                 
 - Decide the reducer toolchain: either wire a --dump reduced arm in boracle/service.rs:453-463 (+30 LOC,     
   makes 3 000 LOC reachable) or demote the whole toolchain to #[cfg(test)]. Leaving it pub(crate) behind a   
   blanket allow is the one option that should not survive.                                                   
                                                                                                              
 ────────────────────────────────────────────────────────────────────────────────                             
                                                                                                              
 3. Stage 0 publishes compiled modules through two copies of the same tail — MERGE                            
                                                                                                              
 Issue. The atomic module-publication contract has two implementations, and the string-merge idiom that       
 guards it has five.                                                                                          
                                                                                                              
 Where.                                                                                                       
 - create_project_modules/compilation/single_file.rs:511-552 vs compilation/canonical.rs:1011-1053            
 - compilation.rs:169-232 — three pure forwarders into compilation/single_file::*                             
 - merge_delta_from call sites: source_preparation.rs:228, module_preparation.rs:625, config_boundary.rs:56,  
   canonical.rs:1014, single_file.rs:514                                                                      
 - single_file.rs:178 — _mode: FrontendCompilationMode, never read in the 679-line file                       
                                                                                                              
 Evidence. Both tails run, in order: merge_delta_from(&compiled.string_table, base_len) → destructure         
 ModuleSemanticResult → if !remap.is_identity() { module.remap_string_ids(&remap);                            
 generated_delta.remap_string_ids(&remap); } → build CompiledModuleArtifact →                                 
 publish_module_and_generated(ModuleBoundaryPublication{…}), with identical Diagnosed arms (into_batch →      
 ModuleDiagnostics::from_batch → mark_diagnosed → push DiagnosedModule). ≈60 near-token-identical lines. The  
 compilation.rs forwarders are 7-line argument pass-throughs, giving two dispatch hops (mod.rs entry-dir      
 check → compilation.rs → child) for one decision; compilation.rs:168 carries its own #[allow(dead_code)].    
                                                                                                              
 FrontendCompilationMode::Check is threaded through the directory path (mod.rs:110-116) and silently          
 discarded on the synthetic path, so single-file check is Canonical mode with no diagnostic saying so.        
                                                                                                              
 Why it matters. docs/build-system-design.md makes publication atomic precisely so a half-merged result       
 cannot escape; a fix to remap-then-publish ordering currently has to land twice. The forwarders are exactly  
 the "forwarding wrappers" the style guide bans in Refactor moves.                                            
                                                                                                              
 Fix.                                                                                                         
 - MERGE: one publish_compiled_module(store, generated, materialisations, resource_inputs, module_id,         
   expected_origin, compiled, base_len, string_table) owned by compilation.rs, called by both lanes (−50).    
 - MERGE: one merge_module_string_delta(base, delta, base_len) -> StringIdRemap helper beside StringTable,    
   consumed at all five sites (−45).                                                                          
 - REMOVE: the three compilation.rs forwarders; call compilation::single_file from mod.rs (−65).              
 - Decide _mode: wire check-only through the synthetic path, or delete the parameter and state in the doc     
   comment that single-file check is canonical. A silently ignored mode parameter is the worst of the three   
   options.                                                                                                   
                                                                                                              
 Leave local: PreparedSourceInput, PreparedModule, GeneratedFunctionPublication, ModuleSuccessPublication.    
 Each field is read at a real decision point (e.g. PreparedModule.contains_moth_template gates implicit       
 template providers at canonical.rs:437); collapsing them would re-merge build-owned scheduling facts into    
 compiler inputs.                                                                                             
                                                                                                              
 ────────────────────────────────────────────────────────────────────────────────                             
                                                                                                              
 4. Diagnostics: 8 stale kinds contradict the docs, one payload shape has two constructors, and prose leaks   
 into payloads                                                                                                
                                                                                                              
 Issue. Three separate problems in one module, all cheap to fix.                                              
                                                                                                              
 (a) Eight Unused* rule kinds are stale scaffolding — REMOVE. diagnostic_kind_descriptors.rs:256-293 reserves 
 MOTH-RULE-0010 … MOTH-RULE-0017 for                                                                          
 UnusedVariable/Function/Type/Constant/FunctionArgument/FunctionReturnValue/FunctionParameter/FunctionParamet 
 erDefaultValue. Repo-wide grep: zero production constructors — compiler_diagnostic.rs and                    
 diagnostic_payload/ never mention them; the only references are 6 test fixtures using UnusedVariable +       
 DiagnosticPayload::UnusedName as a convenient warning stand-in. This contradicts the docs:                   
 docs/src/docs/getting-started/@page.moth:430-432 states unused-variable suggestions belong to the future     
 suggest command, not the compiler warning lane. Delete the 8 kinds, their descriptors, the UnusedName        
 payload and its remap.rs:32-38 arm; repoint the 6 fixtures at a live warning such as                         
 IdentifierNamingConvention. ≈ −60 LOC, and it frees a reserved code block that currently implies shipped     
 behaviour.                                                                                                   
                                                                                                              
 (b) malformed_css_template / malformed_html_template are the same constructor twice — MERGE.                 
 compiler_diagnostic.rs:787-803: both are with_severity(Syntax(kind), Warning, span,                          
 DiagnosticPayload::MalformedTemplate { message }), differing only in the kind. One caller each               
 (styles/css.rs:75, styles/html.rs:51). Collapse to malformed_template(kind, message, span). −15 LOC.         
                                                                                                              
 (c) Adding one reasoned diagnostic is an 8-file lockstep edit. Verified files that must change together:     
 reason enum in diagnostic_payload/types.rs, payload variant + dispatch in diagnostic_payload/mod.rs, macro   
 entry in reason_keys.rs, remap arm in diagnostic_payload/remap.rs (685 hand-written lines of per-variant     
 StringId walking), kind in diagnostic_kind.rs, descriptor in diagnostic_kind_descriptors.rs, constructor in  
 compiler_diagnostic.rs, render arm under render/. reason_keys.rs already proves the fix shape —              
 define_stable_reason_keys! generates the key mapping and the test inventory from one declaration. MERGE:     
 extend that macro (or add a sibling) to emit the payload variant, remap arm and descriptor from one registry 
 entry. This is the one place in the codebase where the style guide's "small declarative macros only where    
 they clearly reduce repetition" earns its keep; remap.rs is the highest-value target because a forgotten arm 
 is a silent wrong-string bug, not a compile error.                                                           
                                                                                                              
 (d) Pre-rendered prose in payloads — SPLIT. DiagnosticPayload::InvalidExternalModule{message} is formatted   
 at external_js/js_import_provider.rs:186-196; MalformedTemplate{message} is rendered verbatim;               
 InvalidArgumentReason detail is fed a formatter Err(String) via                                              
 template_head_parser/handler_directives.rs:59-66. The style guide requires payloads to carry "typed          
 DiagnosticPayload facts and stable diagnostic codes, not pre-rendered prose". Convert each to a typed reason 
 enum.                                                                                                        
                                                                                                              
 Verified clean: ErrorType::Config has no production constructor, so the CompilerError lane honours the       
 diagnostic-lane rule. Not a finding.                                                                         
                                                                                                              
 ────────────────────────────────────────────────────────────────────────────────                             
                                                                                                              
 5. HTML project: three path validators, two HTML escapers, three route parsers                               
                                                                                                              
 Issue. Four concerns with a documented single owner have two or three implementations each.                  
                                                                                                              
 (a) Portable path / URL encoding — MERGE. Canonical owner is build_system/output/output_path.rs:62-142       
 (parse_relative_path, output_path_identity, "owns the one portable relative-path parser", ASCII-case-folded  
 identity). structural_url_renderer.rs:166-203 reimplements segment validation (portable_segments) and        
 hand-rolls percent-encoding (encode_url_segment) with case-sensitive comparison;                             
 resource_output_plan.rs:562-606 adds a second hex encoder (package_output_prefix) plus a                     
 validate_output_path wrapper that only stringifies output_path_identity errors. Route both through the       
 canonical parser. ≈ −50 LOC, and it closes a real case-folding divergence.                                   
                                                                                                              
 (b) HTML escaping — MERGE. styles/escape_html.rs:32-51 documents itself as "the single allocation-free       
 writer for the five HTML-sensitive bytes". document_shell.rs:290-299 ignores it: escape_html_text chains     
 three .replace() calls (3 intermediate allocations, never escapes ') and escape_html_attribute adds a fourth 
 for ". 5 call sites, all in document_shell.rs:131-189. Note the blocker: push_escaped_html_text is           
 pub(super) to styles/, so the fix is to widen it to pub(crate) (or lift it beside path_policy.rs) and delete 
 both locals. −10 LOC, one escape table for the security-sensitive byte set.                                  
                                                                                                              
 (c) Route derivation — MERGE. html_project_builder.rs:150-152 states the invariant: "Derive the canonical    
 page route once… downstream code must not re-derive route semantics." Three violations:                      
 output_plan.rs:143-165 (derive_wasm_route_base re-parses .html/index.html), document_shell.rs:222-287        
 (route_title_fallback + extract_route_segment re-inspect file_name == "index.html", parent-folder vs         
 file-stem), and js_path.rs:581-586 — html_output_path is a pure forwarder to derive_logical_html_path whose  
 own doc comment says so. Delete the forwarder; have the route planner return a structured route (segments +  
 display title) that the shell and Wasm co-location consume. ≈ −60 LOC.                                       
                                                                                                              
 (d) styles/code.rs (2 023 lines) — SPLIT. One file holds the alias registry (:168-260, 16 languages), the    
 shared CodeScanner with 22 per-language dispatch branches, Moth-specific state machines (ContractState,      
 DependencyHighlightState, moth_word_role), and every other language's keyword table (classify_non_moth_word  
 at :1788-1906, sql_word_role, is_yaml_literal). Sibling precedent already exists (css.rs, html.rs,           
 escape_html.rs, validation.rs). Extract moth_scanner.rs and language_profiles.rs, leaving code.rs as scanner 
 shell + role spans. Net 0 LOC, and a Moth-highlighting change stops touching every other language's table.   
 highlight_code_html at :350 is a #[cfg(test)] production function — fold it into the test module during the  
 split (see finding 7).                                                                                       
                                                                                                              
 (e) Builder re-derives compiler-owned facts — MOVE. page_metadata.rs:100-145 reconstructs entry-scope        
 binding identity by portable-string strip_prefix plus a linear declaration-path comparison per constant;     
 js_path.rs:340-380 re-walks hir.blocks × reachability.backend_selection() × side_table.reactive_templates()  
 to re-decide reactive mount gating. docs/compiler-design-overview.md: "Each semantic fact has one source     
 owner. A later stage does not reconstruct the same fact." Publish a typed page-metadata selection and a      
 needs_reactive_mount link fact; the builder consumes both. −70 builder LOC, +30 compiler LOC.                
                                                                                                              
 ────────────────────────────────────────────────────────────────────────────────                             
                                                                                                              
 6. moth_template carries a type production cannot construct — REMOVE                                         
                                                                                                              
 Issue. Speculative scaffolding kept alive by a file-level blanket allow.                                     
                                                                                                              
 Where. moth_template/scope.rs:18-28 — MothTemplateScopeConstant { _private: () } whose only constructor is   
 #[cfg(test)] test_placeholder(). moth_template/input.rs:22-23 — default_module_constants and                 
 module_constants_by_path request fields; input.rs:88-119 — validate_no_caller_scope_constants makes any      
 non-empty value a hard error. moth_template/mod.rs:14 — #![allow(dead_code)] over the whole module.          
 compile.rs:230-260 — render_document_content's only structural branch forwards one line to render.rs:33-70.  
 resource_output_plan.rs:72 — ResourceUrlContext::Stylesheet under #[allow(dead_code)], constructed only in   
 two test files.                                                                                              
                                                                                                              
 Evidence. Production call sites pass Vec::new() for both fields at every site; a non-empty value is          
 rejected. The scope.rs header admits it: a "public conversion for arbitrary folded caller constants needs a  
 separate design". That design does not exist, so the API surface teaches future callers, via a validator,    
 that two of its three fields must always be empty.                                                           
                                                                                                              
 Why it matters. The style guide bans exactly this — no speculative scaffolding, no test-only constructors    
 shaping production types.                                                                                    
                                                                                                              
 Fix. REMOVE scope.rs (whole file), both request fields, the validator, the Stylesheet variant; MERGE         
 render_document_content's two-arm match into its single call site; delete the module blanket allow. ≈ −80    
 LOC, −1 file.                                                                                                
                                                                                                              
 ────────────────────────────────────────────────────────────────────────────────                             
                                                                                                              
 7. 187 cfg(test) items live in production files — MOVE                                                       
                                                                                                              
 Issue. Census of #[cfg(test)] functions, types and statics in non-test production files: 187, concentrated   
 as follows.                                                                                                  
                                                                                                              
 ┌───────────────────────────────────┬───────┬──────────────────────────────────────────────────────────────┐ 
 │ File                              │ Count │ Worst case                                                   │ 
 ├───────────────────────────────────┼───────┼──────────────────────────────────────────────────────────────┤ 
 │ timing/enabled/runtime.rs         │ 22    │ RecordAdmissionState + Condvar + RecordAdmissionPauseGuard + │ 
 │                                   │       │ 8 *_for_test fns (:574-710) — a whole test-synchronisation   │ 
 │                                   │       │ subsystem inside the production timing runtime               │ 
 ├───────────────────────────────────┼───────┼──────────────────────────────────────────────────────────────┤ 
 │ compiler_messages/diagnostic_kind │ 9     │ all() reflection helpers per kind family                     │ 
 │ .rs                               │       │                                                              │ 
 ├───────────────────────────────────┼───────┼──────────────────────────────────────────────────────────────┤ 
 │ compiler_frontend/pipeline.rs     │ 5     │ FILE_FRONTEND_PREPARE_COUNTS_FOR_TEST static Mutex<HashMap>  │ 
 │                                   │       │ plus record_file_frontend_prepare_for_test called            │ 
 │                                   │       │ unconditionally from the live prepare path at :297           │ 
 ├───────────────────────────────────┼───────┼──────────────────────────────────────────────────────────────┤ 
 │ generic_functions/materialisation │ 2     │ from_identities_for_test (:535-577, 43 lines constructing an │ 
 │ .rs                               │       │ artefact with no body payload) — 14 call sites in 4 other    │ 
 │                                   │       │ modules' test files                                          │ 
 ├───────────────────────────────────┼───────┼──────────────────────────────────────────────────────────────┤ 
 │ create_project_modules/source_loa │ —     │ read_source_code branches into source_loading_test_support   │ 
 │ ding.rs                           │       │ on every source read (:24-27)                                │ 
 ├───────────────────────────────────┼───────┼──────────────────────────────────────────────────────────────┤ 
 │ html_project/styles/code.rs       │ 1     │ highlight_code_html (:350)                                   │ 
 └───────────────────────────────────┴───────┴──────────────────────────────────────────────────────────────┘ 
                                                                                                              
 Why it matters. The style guide is explicit: "Test-only implementation belongs with tests. Production files  
 must not grow test-only constructors, mutators, semantic variants or convenience lookups just to support     
 fixtures." from_identities_for_test is the clearest cost — a second, unreviewed constructor for the          
 identity-indexed materialisation context that deliberately produces body-less artefacts, sitting in the      
 production file because the Stable* fields are module-private.                                               
                                                                                                              
 Fix. MOVE, not delete — these fixtures are load-bearing.                                                     
 - from_identities_for_test + contains_template → materialisation/test_support.rs gated #[cfg(test)]; a child 
   module reaches the private fields, so no visibility widening is needed. Drop the two #[cfg(test)] use      
   lines at materialisation.rs:112-115. −60 LOC out of the production file.                                   
 - timing/enabled/runtime.rs's admission-pause machinery → a #[cfg(test)] submodule beside it; the production 
   path keeps only the single hook it must expose.                                                            
 - pipeline.rs and source_loading.rs: replace the per-read *_for_test branch with a test-owned counting       
   wrapper around the reader, so the shipping read path has no test branch.                                   
 - Fold highlight_code_html into the styles/code.rs test module during the finding-5(d) split.                
                                                                                                              
 ────────────────────────────────────────────────────────────────────────────────                             
                                                                                                              
 8. Test suite: duplicated fixture ladders, wrong primary owners, four monolith files                         
                                                                                                              
 Issue. Four problems, ordered by cost.                                                                       
                                                                                                              
 (a) Duplicate fixture ladders — MERGE. compiler_frontend/tests/ast_fixture_support.rs:49-123 and             
 tests/type_id_fixture_support.rs:49-180,590-748 build the same AST nodes with different fixed arguments:     
 param vs param_declaration differ only in DataType-first vs TypeId-first plumbing; immutable_reference_expr  
 fixes ValueMode while inferred_type_reference_expr fixes DataType — the doc comments admit the pairing and   
 warn readers not to confuse them, which is the tell. success_return_slot repeats fresh_success_returns       
 element-wise. Collapse to one factory with with_datatype / with_type_id entry points. −120 to −200 LOC.      
 Separately, tokenizer/tests/lexer_tests.rs:27-30 keeps html_project_test_style_directives, a body-identical  
 copy of compiler_tests/test_support.rs:11-16's frontend_test_style_directives, which it already imports      
 three lines above. REMOVE, −6.                                                                               
                                                                                                              
 (b) Wrong primary owner — REMOVE the weaker copy. testing.mtf gives user-visible language behaviour to       
 tests/cases/. Duplicated in unit tests: lexer_tests.rs:360-430 (string escapes) and :1016-1130               
 (compound-assignment spacing) against cases arithmetic_operator_precedence, variables_and_assignment,        
 white_space; head_tests.rs:696-844,996-1462 (if/loop/else-if suffix grammar) against                         
 top_level_const_template*; parse_file_headers_tests.rs:1236-1470,2504-2989 (import/export and trait          
 rejections) against the *_rejected case directories. Keep only the hidden-invariant tests — span ownership,  
 remap identity, counters. −500 to −1 500 LOC and it removes the double edit on every grammar change.         
                                                                                                              
 (c) Implementation-shaped assertions — narrow in place. hir_expression_lowering_tests.rs looks up            
 .find(|block| block.id == BlockId(0)) at ~15 sites and asserts entry_block.statements.len();                 
 parse_file_headers_tests.rs:3444-3499 pins entry_runtime_fragment_count == 0/1/3;                            
 create_project_modules_tests.rs:8595 asserts error.msg.contains("unindexed source package @missing") where a 
 stable code exists. In boracle/tests/, mod.rs:230-244,316-322 pin OriginTraceRule variants and EventId(7)    
 ordinals — internal derivation choices, not legality. oracle/tests/properties.rs:8-16 already demonstrates   
 the right pattern (assert operational implications). LEAVE-LOCAL but rewrite: entry lookup by                
 region/terminator, assert kind not count, assert code not wording. Also delete one of the two                
 empty-AliasParams rejection fixtures (problem/tests/mod.rs:355-366 vs boracle/tests/mod.rs:131-143, same     
 contract, different event ordinal) — validation owns malformed input.                                        
                                                                                                              
 (d) Monolith test files — SPLIT. create_project_modules_tests.rs 9 468, parse_file_headers_tests.rs 6 779,   
 head_tests.rs 4 274, normalize_ast_tests.rs 3 952. Test names cluster by prefix with disjoint helper sets    
 (headers: prepared_/diagnosed_ vs legacy_/import_ vs trait_ vs entry_runtime_fragment_count_; normalize:     
 finalization_ vs synchronize_receiver_secondary_indexes_ vs const_template_projection_). Split along those   
 clusters; no behaviour change. Do this after (b), so the split is over surviving tests.                      
                                                                                                              
 ────────────────────────────────────────────────────────────────────────────────                             
                                                                                                              
 9. Boracle's duplicated solvers — LEAVE-LOCAL, and say so in the code                                        
                                                                                                              
 Issue. The reference solver and the bounded oracle re-implement the same four concepts, which reads as       
 textbook duplication and must not be merged.                                                                 
                                                                                                              
 Where. Place-ancestor prefix predicate: boracle/origins.rs:1505-1519 vs boracle/oracle/execute.rs:369-380    
 (near byte-identical three-conjunct check, inverted length direction). Four-way                              
 Fresh/AliasParams/Alias/Unknown dispatch at six independent sites. Loan liveness: loans.rs:986- vs           
 oracle/conflicts.rs:30-. Overlap trichotomy: OriginOverlapDecision vs DynamicOverlap vs PlaceOverlap.        
                                                                                                              
 Why leave it. boracle/oracle/mod.rs:5-6 states the oracle "never reuses the static origin, loan or overlap   
 solvers", and boracle-operational-oracle.mtf:22 forbids the oracle from naming any definition in             
 origins.rs/loans.rs/relations.rs — enforced by the oracle-static-solver-independence audit. Sharing a helper 
 would delete the only independent soundness check on the reference solver, which has already caught a real   
 shared-alias gap (pinned at boracle/tests/differential.rs:21-32). Abstraction here is strictly worse.        
                                                                                                              
 Fix. LEAVE-LOCAL, but add a // pointer at each of the four twin sites naming its counterpart and citing the  
 independence contract. +15 LOC that prevents a future reviewer from "fixing" this. Separately, SPLIT the two 
 oversized non-oracle files, which are a pipeline rather than a duplication: extract                          
 problem/builder.rs:1679-1774 (summary → CallResultProvenance translation) into problem/call_effects.rs, and  
 metadata.rs:675-830 (classify_function_return_alias) into its own module. Moves only, net 0.                 
                                                                                                              
 ────────────────────────────────────────────────────────────────────────────────                             
                                                                                                              
 Suggested order                                                                                              
                                                                                                              
 1. Finding 2 first — delete the blankets and the dead surfaces. It is pure subtraction, needs no design      
    decisions except the reducer call, and it makes every later step's dead-code signal trustworthy.          
 2. Findings 1 and 3 — the two forked pipelines. Highest bug risk (the materialisation fork has already       
    drifted three ways), and both are mechanical merges once the shared helper is named.                      
 3. Findings 4, 5, 6 — independent, parallelisable, mostly subtraction.                                       
 4. Finding 7, then 8 — move test-only API out, then prune and split tests against the reduced surface.       
 5. Finding 9 last — comments plus two file moves.                                                            
                                                                                                              
 Rough total: −1 000 to −2 000 LOC, three files dropping under the 2 000-line guideline, two forked contracts 
 collapsed to one owner, and one docs/code contradiction (MOTH-RULE-0010…0017) resolved.    

────────────────────────────────────────────────────────────────────────────────

# Implementation log

Branch `fix-open-audit-findings`. This section is the live record: status per finding,
corrections to the original report, and findings discovered while implementing.
Everything above this line is the original audit, kept unedited.

## Status

`Accepted for checkpoint` means the implemented change passed the final gate and independent
review. The commit containing this report records that checkpoint. It does not close any
explicit open finding below.

| Finding | Scope | State |
|---|---|---|
| 2 | boracle blanket allows and dead surfaces | accepted for checkpoint |
| 2b | `problem`/`last_use` compiled unconditionally (NEW) | accepted for checkpoint |
| 2 | `source/span.rs` + `semantic_identity.rs` bare allows | accepted for checkpoint |
| 3 | Stage 0 publish-tail merge, forwarders, `_mode` | accepted for checkpoint |
| 4a/4b | stale `Unused*` kinds, duplicate malformed ctors | accepted for checkpoint |
| 4c/4d | payload registry macro, typed reasons for prose payloads | accepted for checkpoint |
| 7 | test-only API out of production files | named concentrations accepted, tail closed this wave |
| 5a | portable path / URL encoding merge | accepted for checkpoint |
| 5b | HTML escaper merge | accepted for checkpoint |
| 5c | route derivation merge | corrected scope accepted for checkpoint |
| 5d | `styles/code.rs` split | accepted for checkpoint |
| 5e | builder re-derives compiler facts | rejected this wave: `$page` plan owns metadata; mount gating is JS-owned |
| 6 | moth_template scaffolding removal | accepted for checkpoint |
| 1 MERGE | materialisation sidecar pipelines | accepted for checkpoint |
| 1 SPLIT | materialisation file split | accepted for checkpoint |
| 8a | duplicate fixture ladders | accepted for checkpoint |
| 8b | owner-based prune against `tests/cases/` | accepted for checkpoint |
| 8c | implementation-shaped assertions | accepted for checkpoint |
| 8d | test monolith splits | implemented this wave |
| 9 | boracle independence comments + 2 file moves | accepted for checkpoint |
| — | dead-code suppression policy (NEW, see below) | accepted for checkpoint |
| — | integration diagnostic precision | implemented this wave; `ExpectedToken`/Colon rejected as unit-owned |
| — | output-path diagnostic-lane provenance | rejected this wave: no violation |
| — | production ownership follow-ups | rejected this wave: sidecar accessors and scanner sharing are leave-local |

## Corrections to the original report

**Finding 3, second fix bullet is wrong — dropped.** The report asked for one
`merge_module_string_delta(base, delta, base_len)` helper across five sites. There is
no duplication to remove: `symbols/string_interning.rs:585` `merge_delta_from` is
already the single owner, and the five call sites
(`source_preparation.rs:228`, `module_preparation.rs:625`, `config_boundary.rs:56`,
`canonical.rs:1014`, `single_file.rs:514`) merge different tables into different owners
and apply the resulting remap to different data. Only the *remap-application* idiom
duplicates, and that lives inside the two Stage 0 publish tails, which the first fix
bullet already merges. Adding a wrapper around a one-line method call would be pure
indirection.

**Finding 2, boracle framing needs context the report lacked.** `boracle` is an opt-in
Cargo feature (`Cargo.toml:31`) and its lane has a separate gate, `just boracle`, which
lints with `--all-targets --features boracle -- -D warnings`. Because `--all-targets`
compiles the lib both with and without `cfg(test)`, an item used only by a `#[cfg(test)]`
module is dead in the lib build. That is *why* the blanket allows exist, and it confirms
the prescribed fix: moving those items into `#[cfg(test)]` modules removes the warning and
the production surface at the same time. The blankets are still wrong; they just are not
arbitrary.

## Additional findings

### 2b. `problem/` and `last_use/` compile into every build with no consumer — REMOVE from default builds

**Issue.** `borrow_checker/mod.rs:21,23` declare `mod last_use;` and `mod problem;`
unconditionally, but their only consumer is the `#[cfg(feature = "boracle")]`
re-export at `mod.rs:48-52`. A repo-wide grep for `problem::` and `last_use::` outside
those two directories and `boracle/` returns exactly one hit: that re-export.

**Evidence.** ~3 600 production LOC in `problem/` (`builder.rs` 2 015, `validation.rs` 901,
`events.rs` 238, plus six small files) and 680 in `last_use/mod.rs` are compiled into every
default build with nothing reaching them. Two blanket `#![allow(dead_code)]` attributes
(`problem/mod.rs:13`, `last_use/mod.rs`) hide it completely. `mod.rs:11-14` describes both as
"shared future seams" for "future boracle-style analyses" — accurate, and exactly the
speculative-scaffolding shape the style guide bans from production builds.

**Why it matters.** It is the single largest dead surface the audit found, roughly three times
the next one, and it costs default-build compile time on every build in the repo. It also makes
the two blanket allows load-bearing: they cannot be removed without gating the modules, so the
dead-code signal stays off for 4 300 lines.

**Fix.** Gate both `#[cfg(any(feature = "boracle", test))]`, so the boracle lane and the
default `cargo test` configuration keep them (their `#[cfg(test)] mod tests` continue to run,
coverage unchanged) and shipping builds stop compiling them. Then both blanket allows can go.

### Reducer toolchain verdict: gate fixture entry points, retain production taxonomy

Finding 2's fix left this open. `reducer.rs` (~2 363 LOC) and `oracle/generator.rs`
are test infrastructure and should say so. `differential.rs` also contains comparison taxonomy
used by the non-test Boracle lane. Keep its `MalformedProblem` and `MalformedInput` variants
and severity conversion unconditional. Gate only fixture entry points.

The earlier CLI-arm question remains open. `service.rs:453-463` `render_dump` has no reduce arm,
so the documented reduction workflow remains unreachable from the developer CLI as shipped. The
campaign that consumes it remains gated by the `boracle_campaign` test feature. This log makes no
CLI-arm implementation promise.


## Wave 1 results

Earlier notes recorded a parent-reported gate at this checkpoint. The parent later reran the
complete gate after the review corrections. The later checkpoint evidence supports the implemented
rows in the status table. It does not close the explicit open findings.

The ownership and API corrections recorded in the following sections remain useful historical
narrative.


## Dead-code suppression policy (settled during wave 1)

Finding 2 said to delete the blankets but did not say what replaces them, and the first
implementation pass answered that badly enough to be worth recording as policy.

The failed approach was `#[cfg(test)]` on normalized-problem vocabulary — 49 markers threaded
through enum variants *and* the solver match arms that handle them in `boracle/loans.rs`,
`origins.rs`, `oracle/calls.rs`, `oracle/conflicts.rs`, `oracle/execute.rs`. Rejected on two
grounds. It reads as a configuration-dependent solver: an `EventKind::ExclusiveAlias` arm gated
on `cfg(test)` looks like borrow legality differing between test and shipping builds even when
the variant cannot be constructed there. And it scatters: when the builder starts emitting these
events, every one of those 49 sites needs finding and un-gating across five files.

The settled rule: `#[cfg(test)]` is for test-only *helpers and accessors*; planned vocabulary
the builder has not reached yet carries a per-item `#[allow(dead_code)]` with a one-line comment
naming its owning plan under `docs/roadmap/plans/boracle-next-research-plans/`. Per item, never a
file blanket, and never on a whole enum unless every variant is affected (`KillReason` is the one
such case). The evidence that this vocabulary is live design surface rather than dead code:
`problem/validation.rs:622-670` validates `Alias`, `ExclusiveAlias` and `Rebind` as part of the
documented normalized-input contract, while `problem/builder.rs:1561-1566` currently emits only
the `AliasFromPlace` and `ExclusiveAliasFromPlace` forms.

Two mechanical corrections applied by the parent for the same reason. `problem/ids.rs` had
`define_problem_id!(LoanId)` hand-expanded into 17 lines so that one accessor could be gated;
reverted to the macro with a single justified allow on `index()`, because hand-expanding one
member of a six-type family means the next method added to the macro silently skips `LoanId`.
And `boracle/loans.rs` had two helpers extracted
(`push_slot_rebind_provenance_loan`, `push_call_result_provenance_loans`) that added more code
than they replaced and needed `#[allow(clippy::too_many_arguments)]` to compile. The extraction
was reverted. A parameter list that trips the argument-count lint signals the seam is wrong rather
than a lint to suppress.

## Additional findings from wave 1

### `ProjectPathResolver::new` correction

An earlier note incorrectly described all 24 callsites as tests and said production used
`new_with_module_roots` exclusively. Two production callers exist in
`single_source_compilation/moth_template.rs` and `html_project/moth_template/bundle.rs`.
The deletion still stands on single-constructor grounds. Every site now calls
`new_with_module_roots(..., ModuleRootTable::empty())`, so no test pins an unreachable
configuration.

### `prepend_diagnostics_preserving_context` deletion rejected

An earlier note incorrectly described seven test-only callsites with no production diagnostic
path. Five real callers implement diagnostic ordering in the direct Moth template compilation
service. Retain the helper and its tests.

### `PreparedSourcePackageRoots::empty` — Finding 7, costed

12 callsites across 9 test modules; production uses `from_entries` and
`new_with_module_roots`. Relocation to test support is correct but touches all 9, so it was
annotated rather than moved in wave 1.

### Direct Moth template API diagnostics now sit behind a test gate

Finding 6 gated the direct template API behind `#[cfg(test)]` in `html_project/mod.rs`, which
left `moth_template_inputs_share_no_common_ancestor` and `duplicate_moth_template_input_path`
in `compiler_diagnostic.rs` reachable only from test-gated code. The parent deleted the sibling
`InvalidMothTemplateApiScopeItem` chain by hand across all seven files it touched — kind,
descriptor, payload variant, remap arm, two renderer arms, constructor — which is empirical proof
of Finding 4(c)'s lockstep cost. These two remain because they are documented service surface;
a later pass decides whether they follow the API's gating.

### `LoanSolver::solve` is test-only research surface

Production boracle calls `solve_with_rule_selection` through
`BoracleSolver::solve_with_rule_selection`. The generic `solve` and `LoanSolution::decisions`
are fixture support and are now `#[cfg(test)]`. This entry point split remains review context and
does not schedule another wave.

### Finding 5(c) scope correction

The route-derivation scout found the report conflated two owners: the "segments + display title"
struct belongs to page metadata, not routing. The three route re-derivations agree on every
current output, so the disagreements the report cited (trailing slash, uppercase, `./index.html`)
are latent, not live. The merge is still right; its justification is single ownership, not a bug.

## Wave 2 results

Earlier notes recorded wave 2 implementation and a parent-reported gate at that checkpoint. The
parent later reran the complete gate after the review corrections. The later checkpoint evidence
supports the implemented rows in the status table. It does not close the explicit open findings.

The structural outcomes remain recorded:

- **4(c)/4(d):** one diagnostics registry now owns payload variants, remap arms, stable keys and
  descriptors.
- **5(a):** one portable path parser and one RFC 3986 encoder now own the path and URL rules.
- **5(b):** HTML-sensitive bytes now use `push_escaped_html_text`.
- **5(c):** `CanonicalPageRoute` now feeds shell, JavaScript and Wasm consumers.
- **5(d):** language profiles, the Moth scanner and test support now sit outside the scanner
  shell.
- **1 SPLIT:** materialisation lanes now use explicit child imports rather than a shared parent
  scope.

Earlier per-slice LOC figures, projection comparisons and cumulative totals are withdrawn. They
mixed tracked documentation and deletion rows with untracked Rust children and lacked a clean
baseline. No whole-tree or wave LOC total appears in this log.

The subtraction findings that remain open are Finding 8(d), Finding 7's tail, Finding 5(e) and
integration diagnostic precision. Finding 8(b) is reviewed and validated, awaiting checkpoint.

### Integration corrections recorded after the slices reported green

The following integration corrections were recorded after slice reports. The later parent gate
covers the merged tree. The corrections remain useful implementation history.

- `sidecar_build.rs:387` `emit_materialised_sidecar` reached 10 parameters. Per the wave 1
  policy an argument list that trips the lint signals a wrong seam rather than a lint to suppress.
  The five requester facts became `GeneratedSidecarRequest` and `type_arguments` was dropped
  because both lanes passed `identity.type_arguments()`. The function now takes five parameters.
- `types.rs:850` `HtmlTemplateWarning`'s three variants all carried the `Unsafe` prefix which
  the enum name already implies. They were renamed to `ScriptTag`, `JavascriptUrl` and
  `InlineEventHandler`. Stable reason keys (`malformed_template.html.unsafe_*`) remain unchanged
  because they provide diagnostic identity.
- The parent corrected the route-derivation contract comment indentation in `output_plan.rs`.

## Open before closure

These are explicit open findings, not an ordered wave or an implementation promise. No automatic
next-wave work follows from this list.

1. **Finding 8(d)**: split the four monolith test files
   (`parse_file_headers_tests.rs`, `head_tests.rs`, `hir_expression_lowering_tests.rs` and
   `lexer_tests.rs`) by construct rather than line count.
2. **Integration diagnostic precision**: decide whether `expect.toml` contracts can carry the
   needed precision for the 19 total template structure reason variants. Six variants remain
   unit-only and 13 have case coverage in the current map. Four `ExpectedToken`/Colon
   rejections also remain unit-only.
3. **Finding 5(e)**: publish a typed reserved-page-metadata selection and a
   `needs_reactive_mount` link fact for the builder to consume rather than re-derive compiler
   facts.
4. **Finding 7 tail**: review the remaining `#[cfg(test)]` declarations in production files and
   move only test-owned helpers and accessors to child modules.

The parent must decide each item at closure. This section does not schedule another wave.

## Wave 3a results

Finding 7's four named concentrations and Finding 9 were implemented on the branch and remain
uncommitted. The parent later reran the complete gate after the review corrections. The later
checkpoint evidence supports these implemented rows. Finding 7's tail remains open.

The wave 3a census remains useful as a historical snapshot for Finding 7, not as closure evidence.
`#[cfg(test)]` declarations in non-test production files were recorded as 203 before the audit
and 183 after the named moves. The snapshot listed `timing/enabled/runtime.rs`,
`compiler_messages/diagnostic_kind.rs`, `compiler_frontend/pipeline.rs`, `materialisation.rs`,
`moth_template/scope.rs` and `styles/code.rs` as the main changed files. New test-owned files
included `runtime_test_support.rs`, `pipeline_test_support.rs` and
`artefact_emit/test_support.rs`.

Earlier wave 3a cost and cumulative LOC figures are withdrawn. Finding 7 moves test surface into
test support and Finding 9 adds module seams, so a whole-tree total would require a clean baseline.

Two implementation decisions were recorded. The frontend prepare counter could not move outside
production entirely because source preparation has no injectable provider and the existing
`FilePreparationPassCount` and `PreparedFileCount` counters are aggregate-only and gated behind
`benchmark_counters`. The counter state moved to a test-owned child, leaving one neutral observer
call with an inline `cfg(not(test))` no-op. `LoanSolver::solve` was deleted after caller review,
with tests repointed to `solve_with_liveness`. `LoanSolution::decisions` stays as
`#[cfg(test)]` fixture support.

### Corrections to the report, both material

**Finding 7's `prepend_diagnostics_preserving_context` deletion was wrong — rejected.** The
report claimed seven callsites, all under `#[cfg(test)]` module exports, with no production
diagnostic path. There are five real callers implementing diagnostic ordering:
`single_source_compilation/moth_template.rs:377,428,579`, `html_project/moth_template/bundle.rs:654`
and `compile.rs:258`. That is the direct Moth template compilation service, documented surface in
`docs/compiler-design-overview.md`; the `#[cfg(test)]` in that area gates the API entry point
(Finding 6), not this code. It is also the symmetric partner of `append_messages_preserving_context`,
which has a dozen production callers. The proposed replacement was three to four lines of
string-table cloning open-coded at five sites in place of one call. The deletion was made and
reverted; the helper and its tests are unchanged.

**`ProjectPathResolver::new` had production callers.** The wave-1 note said 24 callsites, all in
tests. Two are production: `single_source_compilation/moth_template.rs` and
`html_project/moth_template/bundle.rs`. The deletion still stands on single-constructor grounds,
and every site now calls `new_with_module_roots(..., ModuleRootTable::empty())`, the same entry
point single-file compilation uses at `compilation/single_file.rs:275-277`, so no test pins an
unreachable configuration. `PreparedSourcePackageRoots::empty` was deleted on the same reasoning:
it was literally `Self::default()`, so all callers use `default()` and no test helper replaced it.

### Concurrency incident: a codemod corrupted 25 lines across 8 files

A slice repointed the resolver with a structural codemod that replaced argument *values*, not
just the constructor name. Result: doubled commas at 17 sites, one call referencing a
non-existent local, and fully-qualified `crate::compiler_frontend::paths::module_roots::...`
spellings inlined where an import belongs, in files spanning three other slices. The parent took
the repoint over, repaired every site by hand and normalised all ten files to imported
spellings. Two lessons worth keeping: a codemod that rewrites call arguments needs every call
re-read afterwards, and a slice that breaks a shared API must land its callers in the same pass
or it blocks every sibling's validation.

### New finding: 183 `#[cfg(test)]` declarations still sit in production files

Finding 7 named only the concentrations. The long tail is real and now measured:
`timing/enabled/render.rs` 7, `compiler_frontend/project_globals.rs` 6,
`integration_test_runner/assertions/rendered_output.rs` 6, `build_config.rs` 5,
`datatypes/environment.rs` 5, plus production `*_for_test` APIs such as
`datatypes/environment.rs:985` `insert_function_type_for_test`, `render/terminal.rs:126`
`format_terminal_source_frame_for_test` and `tokenizer/tokens.rs:461`
The tail remains open in the status table. This log makes no automatic follow-up implementation
promise.

## Wave 3b results

Finding 8(a), 8(b) and 8(c) were implemented on the branch and remain uncommitted. The earlier
wording claimed a full gate but omitted `bench-ci` and `bench-scaling` execution proof. Parent
later supplied a complete checkpoint run that includes both benchmarks. That later evidence and the
independent review support these implemented rows. It does not close the open findings.

The earlier Wave 3b LOC total and workspace-count reconciliation are withdrawn. They mixed
tracked documentation and deletion rows with untracked Rust children and lacked a clean baseline.
The earlier 20 and 21 deletion figures, the 8 plus 12 breakdown and the cumulative Finding 8 and
Finding 9 arithmetic are also withdrawn. Restoration edits are present, but this log records no
final deletion total, per-file delta or whole-tree LOC total.

### Finding 8(b) review correction

Earlier headings and figures alternated between 20 and 21 deletions and then claimed an exact
8 plus 12 reconciliation. Those figures are withdrawn. They conflicted with the restored
branch-scope tests and did not use a clean baseline. The current tree records no final test
deletion total, per-file delta or whole-tree LOC total.

Independent review found that these branch-scope proofs had been overdeleted:
`template_option_capture_binding_is_not_visible_in_else_branch` and
`template_else_if_option_capture_binding_is_branch_local`. Their surviving cases did not prove
that a branch-local option capture becomes `UnknownName` with the `Value` namespace in the sibling
else or fallback. TestFix restored both assertions. The parent gate and the independent recheck
found no remaining blocker in these corrections.

The same review found that assigned-template and multiple-runtime-template assertions had been
narrowed from exact fragment counts to `> 0`, which weakened the contract. TestFix restored the
exact contracts: zero `0`, single `1`, assigned `1` and multiple `3`. The parent gate and the
independent recheck found no remaining blocker in these corrections.

### Diagnostic precision remains open

Template-structure errors remain more precise in unit tests than in integration cases. The cases
assert broad `MOTH-SYNTAX-0022` codes while the units pin an `InvalidTemplateStructureReason`.
That boundary means the prune cannot close until the parent decides which layer owns each
diagnostic contract.

The broad code has 19 total reason variants. Thirteen have a current case mapping:
`MissingCommaBeforeControlFlowSuffix`, `ControlFlowSuffixNotFinal`,
`MissingTemplateIfCondition`, `MissingTemplateLoopHeader`,
`TemplateMatchStyleControlFlowUnsupported`, `ElseInTemplateHead`, `DuplicateTemplateElse`,
`TemplateElseInLiteralBody`, `TemplateElseIfInLiteralBody`, `MalformedTemplateElse`,
`TemplateElseIfAfterElse`, `InlineTemplateElse` and `TemplateElseInLoopBody`.

Six variants remain unit-only with no case contract:
`OrphanTemplateElse`, `MalformedTemplateElseIf`, `MissingTemplateElseIfCondition`,
`OrphanTemplateElseIf`, `InlineTemplateElseIf` and `TemplateElseIfInLoopBody`. Four
`ExpectedToken { expected: Colon }` assertions also remain unit-only:
`export_alone_is_rejected`, `legacy_inline_export_declaration_is_rejected`,
`legacy_export_path_syntax_is_rejected` and `export_bare_path_rejected_as_deferred_namespace_export`.
`export_block_missing_colon_rejected` asserts only `MOTH-SYNTAX-0001`. The two literal-body
reasons share one case and need separate ownership review.

### 8(a): the projection assumed deletable bodies, but the duplication was a fixed argument

The two ladders looked like duplicate bodies but differed only in which argument each fixed. The
fix is one shared private core per node shape (`parameter`, `reference_expr`) behind typed entry
points that keep call sites flag-free: `param_with_datatype`, `param_with_type_id`,
`reference_expr_with_datatype`, `reference_expr_with_type_id`, `success_return_slot` and
`fresh_success_returns`. This remains a rename and core extraction rather than a subtraction.

Two leave-local decisions remain sound. `const_record_reference_expr` fixes
`ConstRecordState::ConstRecord`, a genuinely different construction mode. Sharing it would thread
that mode through runtime-reference callers for one function's worth of lines.
`public_interface/tests/declaration_record_tests.rs::param_declaration` has its own
`Expression::no_value_with_type_id` body and shares nothing with the merged factory. The two files
stay separate on a real ownership line: `ast_fixture_support` owns generic AST node construction
and `type_id_fixture_support` owns the HIR and TypeId lowering and registration harness, including
the AST walk it must build to register struct, choice and collection TypeIds.

`success_return_slot` was almost merged the wrong way round. The first plan used
`fresh_success_returns(vec![id]).pop()`, which allocates a vector and unwraps an `Option` to
retrieve one element. The single slot now provides the primitive and the vector factory maps over
it, so element construction has one definition and the common case allocates nothing.

### 8(c): an invalid fixture nine tests were asserting against

The largest find in this wave. `hir_builder_test_support.rs` `setup_builder` set
`start_function = Some(FunctionId(0))` with an empty `module.functions`, which
`hir/validation/structure.rs:66-80` rejects outright, so nine `runtime_template_*` tests were
asserting against a module shape the compiler would never accept — the same unreachable
configuration `ProjectPathResolver::new` showed in wave 3a. Production genuinely needs function 0
to exist (`hir_expression/numeric.rs:397` selects Trap through it), so the fixture now registers a
real `HirFunction(0)` with root region, `EntryStart` origin and empty provenance;
`structure.rs:82-94` and `:239-264` also require origin and provenance coverage, so both are
populated. The nine `functions.is_empty()` assertions became "exactly one function, and its id is
`module.start_function`", which still fails if lowering ever synthesizes a helper.

`expression_test_builder_produces_valid_hir_module_metadata` was added to drive the fixture
through full HIR `validate` and guard against the invalid shape returning. Its presence does not
provide validation for this log.

Also in 8(c): 13 hard-coded `BlockId(0)` entry lookups now resolve the declared start function's
`.entry`, which is the actual semantic relation rather than the region or terminator proxies the
report suggested; statement counts became kind assertions where the count was incidental to
lowering sequence; `create_project_modules_tests.rs` asserts `ErrorType::Compiler` instead of
matching prose; and the boracle aggregate and mixed-binding tests assert operational holder,
conflict and disjointness facts instead of `OriginTraceRule` variants and `EventId(7)` ordinals.
The duplicate empty-`AliasParams` rejection was deleted from the boracle side after confirming both
copies reach the same `BorrowProblem::new` → `validate` branch; `problem/tests/mod.rs` keeps it,
since validation owns malformed input.

### Corrections to parent instructions this wave

The earlier `has_non_trivial_root_body` suggestion would have replaced exact
`entry_runtime_fragment_count` contracts with a broader predicate that returns true for `x = 1`.
Independent review rejected that weakening. TestFix restored exact contracts for zero, single,
assigned and multiple runtime work. The parent gate and independent recheck found no remaining
blocker.

The earlier `to_owned()` approval for `BlockId` was also wrong because `BlockId` is a `Copy`
newtype. Plain dereference keeps the ownership clear. Finally, the earlier
`tests/cases/` mappings were guesses rather than verified coverage, which produced the
over-deletions recorded above.

### New gaps retained for closure

Production exposes no fact at the granularity tests need, so tests reach for prose or arithmetic.
`CompilerError` carries a prose `msg` and a broad `ErrorType` but no stable code or typed reason
(`compiler_messages/compiler_errors.rs:1057-1071`). The header API exposes
`entry_runtime_fragment_count` and the broader `has_non_trivial_root_body`
(`headers/types.rs:67-78`) but no typed boolean for runtime-template work. `tests/cases/` asserts
diagnostic codes without reason variants. These gaps explain why integration diagnostic precision
remains open. They do not authorise another implementation wave.

## Independent review disposition

Independent production review found a configuration-parity violation in Boracle. Test-gated match
arms changed non-test solver behaviour even though the event vocabulary remains production-facing.
`boracle/loans.rs` omitted `LoanIssue` lookup and Alias or ExclusiveAlias handling outside tests.
`boracle/differential.rs` gated `OracleComparisonClass::MalformedProblem`,
`OracleComparisonSeverity::MalformedInput` and the severity conversion. The `ProvenDisjoint`
vocabulary and arms in `boracle/relations.rs` needed the same parity treatment. BoracleFix restored
the vocabulary and handling unconditionally and added plan-named per-item allowances. The parent
gate and SliceReviewProd recheck found no remaining blocker.

Independent test review found overdeleted branch-scope proofs and narrowed contracts. TestFix
restored the option-capture `UnknownName` and `Value` assertions and restored exact runtime
fragment contracts for zero, single, assigned and multiple work. TestFix also removed the local
span overlap, containment, order and emptiness replicas that no longer exercised production
predicates. The parent gate and SliceReviewTests recheck found no remaining blocker.

The review also identified a test-local `proven_disjoint` helper and tests that only preserve
test-gated `ProvenDisjoint` rows. BoracleFix changed relation tests to use the production
constructor and removed the Debug prose pin. The parent gate and independent rechecks found no
remaining blocker. The narrowed loop-suffix plumbing tests and their stale covering comments were
removed.

The production review recorded these dispositions:

- `sidecar_build.rs:152-200` still contains preparation accessors and identity-index rebuilding
  called from `preparation_freeze.rs:211,216,313`. Review whether those accessors belong back in
  preparation freeze. No move is claimed.
- The Moth scanner retains shared number and operator dispatch alongside Moth-specific scanning.
  Further separation remains a review question rather than a required correction.
- The parent removed the stale `Wave 3 should` scheduling comment from `resource_output_plan.rs`.
  This comment fix has no diagnostic-lane behaviour change.
- Parent updated the constructor and `compiler_errors` comments to cite the
  `compiler-source-token-and-diagnostic-data-layout` plan and the retained direct-template
  source and warning boundary. The formerly unnamed-owner follow-up now has a named plan and the
  parent gate covers the change.
- Parent updated the index for three restructures and marked audit-log `tests.support` stale.
  These edits are included in the checkpoint.
- The output-path diagnostic lane remains an open provenance question. Review user or configuration
  paths separately from internally generated paths. No violation has been established.

## Final checkpoint evidence

Parent supplied the final gate record in `artifact://838`. `cargo fmt --check`, `just validate`
and `just boracle` exited successfully. The record reports:

- native clippy and feature-lane coverage with no findings
- source audit across 1 353 files with no findings
- first-party dependency audit across 21 files and 62 JavaScript sources with no findings
- workspace unit tests of 5 102, 836 and 17 with all tests passing
- integration cases 1 952 out of 1 952 correct
- docs check with no errors or warnings
- all 82 benchmark cases passing shared preflight
- `bench-ci` with no measurable change at `-1 ms` average across 7 of 8 comparable cases
  where the changed `docs_check` workload remains noted
- frontend benchmark movement of `-4 ms` average with 7 faster cases, 0 slower cases and 9 of
  10 comparable cases where the changed `docs_frontend` workload remains noted
- scaling fits of `n^0.98`, `n^0.78` and `n^1.63`, all within budget
- timers erasure with a clean 8 713 328-byte no-timer binary
- Boracle clippy plus 41 `borrow_problem`, 5 `last_use` and 251 `boracle` tests passing

SliceReviewProd and SliceReviewTests independently rechecked the corrections and reported no
remaining scoped blocker. CloseoutAudit subsequently returned PASS after inspecting the corrected
boundaries, plan rationale and gate evidence. These reviews ran no validation commands.
Its one non-blocking observation, stale overlap and containment prose in `source/span.rs`, was
corrected. A subsequent `cargo fmt --check` passed. No executable code changed after the gate.

This closes review of the implemented checkpoint, not the complete audit backlog. The standard
feature matrix and Boracle campaign were not run and are not claimed.

## Wave 4 results

This wave closed the remaining open items from the checkpoint. Original audit
text above the implementation log is unchanged.

### Finding 5(e) — rejected

The builder still joins HIR constant names to `const_facts` spans with portable-string
prefix stripping, and `html_module_uses_reactive_runtime_fragments` still walks
reachable HIR to decide mount-helper emission. Neither suggested compiler publication
is the correct current fix.

Page metadata is the operational reserved-`page_*` contract until the queued
`html-page-directives-and-runtime-title-plan`. That plan's Phase 3 is "Replace HIR-name
extraction with resolved module metadata" as part of `$page`. A compiler reserved-name
scan now would be a transitional artefact the plan deletes. Deferred work is not a
defect to implement incidentally.

Reactive mount gating is HTML-JS presentation policy
(`HTML-JS reactive mounting remains a JavaScript-owned concern`). `HirReachability`
already publishes `reachable_reactive_sinks` / `reachable_reactive_templates`. A
`needs_reactive_mount` link fact would bake one builder's bootstrap choice into a
shared compiler lane. The builder is consuming published reachability facts, not
reconstructing a compiler-owned semantic fact.

### Finding 7 tail — closed

Census of the named leftover clusters still held. Implemented:

- Moved the six timing-summary layout helpers from `timing/enabled/render.rs` into
  `timing/enabled/render_test_support.rs`. Promoted the duplicated accounting-note
  string to production `ACCOUNTING_NOTE` used by the renderer and tests.
- Deleted the four `ProjectGlobalsMemberMetadata` field-reader forwarders. Restored
  `ProjectGlobalsInterface::member` because `build_config_tests.rs` looks up metadata
  across a module boundary over the private `members` vector.
- Deleted `TypeEnvironment::{is_numeric, map_key_type, map_value_type, variant_for}`.
  Callers now use `get`/`map_shape`/`variants_for`. Deleted the test that only pinned
  the test-only `is_numeric` helper.

Leave-local, with the same private-field or harness-seam reasons as the census:
`insert_function_type_for_test`, `members` lookup helpers that would only move a
private field, `build_config.rs` one-line readers, terminal hermetic-root seams,
tokenizer freeze/new helpers, and integration-runner rendered-output wrappers.

### Finding 8(d) — implemented

The four monoliths were split into parent helper modules plus `#[path]` children.
Shared helpers stay in the parent. `parse_file_headers_tests` remains `pub(crate)`
with `parse_single_file_headers` / `parse_single_file_headers_with_table` /
`prepare_single_file` on the same path. One nested relative import in
`normalize_ast_runtime_tests.rs` gained a `super` after the extra module hop.

Line counts after the split:

- `create_project_modules_tests.rs` 1600 plus 10 children (195–1548)
- `parse_file_headers_tests.rs` 996 plus 16 children (69–1034)
- `head_tests.rs` 1113 plus 4 children (29–1611)
- `normalize_ast_tests.rs` 453 plus 4 children (243–1556)

The original "disjoint helper sets" claim was wrong for Stage 0: helpers are shared,
so children `use super::*` rather than each carrying a copy.

### Integration diagnostic precision — implemented, with one rejection

`tests/cases/` now owns the six previously unit-only `InvalidTemplateStructureReason`
variants via `diagnostic_assertions` reason keys. New cases:

- `template_else_if_orphan`, `template_else_if_malformed`,
  `template_else_if_missing_condition`, `template_else_if_inline_not_standalone`,
  `template_else_if_inside_loop_without_nested_if`,
  `template_control_flow_literal_body_else_if_rejected`,
  `template_else_body_orphan`

Existing sibling cases gained reason assertions. `template_else_orphan` (`[else]` as
the whole template) is `else_in_template_head`, not `orphan_template_else`; the body
orphan lives in `template_else_body_orphan`.

The four `ExpectedToken { expected: Colon }` unit tests stay unit-owned. There is no
compiler reason key for `ExpectedToken`, and cases already own those rejections as
`MOTH-SYNTAX-0001`.

### Output-path provenance — rejected

User/configuration paths already use typed `CompilerDiagnostic` / `InvalidConfigReason`.
Internally generated writer paths use `CompilerError` / `OutputRejectionReason`. No
lane-crossing constructor was found.

### Production ownership follow-ups — rejected

`sidecar_build.rs` preparation accessors are file organisation next to `materialise_ast`,
not an ownership violation. The Moth scanner's shared number/operator dispatch is the
documented post-5(d) housing; further separation would duplicate scan-loop plumbing.

## Wave 4 verification

- `cargo fmt --check` passed.
- `just validate` passed.
- Workspace tests: 5101 + 836 + 17, all passing. The unit count is one below the
  prior checkpoint because `type_environment_classifies_numeric_primitives` was
  deleted with the test-only `is_numeric` helper.
- Integration cases: 1959/1959 (seven new template-structure reason cases).
- Feature-lane coverage, source audit, first-party deps, documentation check,
  timer erasure, all 82 benchmark preflights, and all 3 scaling budgets passed.
- `just boracle` was not run: no borrow-checker sources changed.

## Open findings after wave 4

None from this report.

Whole-tree LOC and isolated-wave deletion totals remain withdrawn. This report stays
tracked at its requested `tmp/` path despite the directory ignore rule.