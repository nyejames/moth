poll |input ~io.input.Input|:
    io.input.update(~input)
;

new_input | | -> String, Error!:
    input ~= io.input.new()!
    poll(~input)
    io.input.close(~input)
    return "ok"
;

result = new_input() catch:
    then "error"
;
[:core_io_input_signature_success [result]]
