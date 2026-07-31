parse_number |text String| -> Int, Error!:
    return! Error(text)
;

recover |use_zero Bool| -> Int:
    value = parse_number("bad") catch |err|:
        io.line([: [err.message]])
        if use_zero:
            then 0
        else
            then 1
        ;
    ;
    return value
;

value = recover(true)
io.line("value catch nested then if")
io.line([: [value]])
