read_input | | -> String, Error!:
    input ~= io.input.new()!
    io.input.update(~input)

    if io.input.key_pressed(input, "d"):
        return "key d"
    ;

    if io.input.pointer_down(input, "left"):
        x = io.input.pointer_x(input)
        y = io.input.pointer_y(input)
        if x > 0.0:
            return "pointer"
        ;
    ;

    io.input.close(~input)
    return "ok"
;

result = read_input() catch:
    then "error"
;
[:core_io_input_reads_success [result]]
