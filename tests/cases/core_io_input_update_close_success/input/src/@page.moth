update_close_input | | -> String, Error!:
    input ~= io.input.new()!
    io.input.update(~input)
    io.input.close(~input)
    return "ok"
;

result = update_close_input() catch:
    then "error"
;
[:core_io_input_update_close_success [result]]
