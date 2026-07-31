can_error || -> String, Error!:
    return! Error("boom")
;

recover || -> String:
    value = can_error() catch |err|:
        if 1 is:
            1 => io.line("one")
            else => io.line("other")
        ;
        then "fallback"
    ;

    return value
;

output = recover()
io.line([: [output]])
