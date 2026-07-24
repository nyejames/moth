can_error |name String| -> String, Int, Error!:
    return! Error("boom")
;

rethrow |name String| -> String, Int, Error!:
    value, number = can_error(name) catch |err|:
        return! err
    ;
    return value, number
;

first, count = rethrow("Ana") catch:

    then "fallback", 0

;
io.line([:bubbled-recovery-first=[first]])
io.line([:bubbled-recovery-count=[count]])
