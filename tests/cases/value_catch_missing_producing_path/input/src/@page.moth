parse_number || -> Int, Error!:
    return! Error("bad")
;

recover |log_error Bool| -> Int:
    return parse_number() catch:
        if log_error:
            io.line("bad")
        ;
    ;
;
