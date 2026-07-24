can_error || -> Int, Error?!:
    return! Error("optional_error_boom")
;

recover || -> Int:
    value = can_error() catch |err|:
        if err is not none:
            io.line("optional_error_present")
        ;
        then 0
    ;
    return value
;

io.line([: [recover()]])
