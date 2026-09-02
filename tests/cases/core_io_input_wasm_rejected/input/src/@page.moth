new_input || -> String, Error!:
    input ~= io.input.new()!
    io.input.close(~input)
    return "ok"
;

result = new_input() catch:
    then "error"
;
[:core_io_input_wasm_rejected [result]]
