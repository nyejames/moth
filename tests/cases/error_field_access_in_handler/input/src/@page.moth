-- Tests accessing error fields inside a catch handler.

can_error |name String| -> String, Float, Error!:
    return! Error("something went wrong")
;

recover |name String| -> String, Float:
    value, score = can_error(name) catch |err|:
        io.line([:handler-error-message=[err.message]])
        then "fallback", 0.0
    ;
    return value, score
;

name, score = recover("test")
io.line([:handler-recovery-value=[name]])
io.line([:handler-recovery-score=[score]])
