-- Unreachable helper that references both console and input functions.
-- HTML-Wasm reachability validation must not inspect it.
unused_io_wrapper || -> String, Error!:
    io.line("unused console")
    input ~= io.input.new()!
    io.input.update(~input)
    io.input.close(~input)
    return "ok"
;

[:core_io_unreachable_io_wrapper_wasm_ignored]
