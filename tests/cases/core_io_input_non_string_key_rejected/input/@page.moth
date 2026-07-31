bad_key | | -> String, Error!:
    input ~= io.input.new()!
    result = io.input.key_pressed(input, 1)
    io.input.close(~input)
    return "ok"
;

result = bad_key() catch:
    then "error"
;
[:result [result]]
