SmallError = |
    message String,
|

inner |fail Bool| -> Int, SmallError!:
    if fail:
        return! SmallError("inner failed")
    ;

    return 42
;

outer |fail Bool| -> Int, SmallError?!:
    value = inner(fail)!
    return value
;

ok = outer(false) catch:
    then 0
;
fallback = outer(true) catch |err|:
    if err is not none:
        io.line("wrapped optional error")
    ;
    then -1
;

[:ok=[ok] fallback=[fallback]]
