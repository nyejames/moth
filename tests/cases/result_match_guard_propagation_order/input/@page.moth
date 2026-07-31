guard_skip || -> Bool, Error!:
    io.line("skipped-guard")
    return true
;

guard_fail || -> Bool, Error!:
    io.line("failing-guard")
    return! Error("guard failed")
;

compute |subject Int| -> String, Error!:
    result ~= "unset"

    if subject is:
        1 if guard_skip()! => result = "one"
        2 if guard_fail()! => result = "two"
        else => result = "fallback"
    ;

    return result
;

value = compute(2) catch:

    then "caught"

;
io.line([:match guard propagation result=[value]])
