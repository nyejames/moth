-- An unreachable helper containing a reactive template must not block HTML-Wasm.

unused_helper || -> String:
    count $Int = 1
    return [:helper [$(count)]]
;

[:reactive_wasm_unreachable_helper_ignored]
