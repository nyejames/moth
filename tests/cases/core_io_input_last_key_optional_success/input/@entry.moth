read_last | | -> String, Error!:
    input ~= io.input.new()!
    io.input.update(~input)

    last = io.input.last_key_pressed(input)
    if last is |key|:
        return "last key"
    else
        return "no last key"
    ;

    io.input.close(~input)
    return "ok"
;

result = read_last() catch:
    then "error"
;
[:core_io_input_last_key_optional_success [result]]
