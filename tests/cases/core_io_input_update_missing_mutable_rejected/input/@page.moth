bad_update | | -> String, Error!:
    input ~= io.input.new()!
    io.input.update(input)
    io.input.close(~input)
    return "ok"
;

result = bad_update() catch:
    then "error"
;
[:result [result]]
