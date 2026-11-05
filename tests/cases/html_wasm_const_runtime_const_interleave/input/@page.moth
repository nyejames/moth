-- Prove const/runtime/const fragment ordering survives lowering for html_wasm.
-- The const fragments end up in index.html; the runtime fragment mounts via page.js.

render_middle || -> String:
    return [:runtime_middle_content]
;

middle = render_middle()
[: [middle]]
#[:entry]
