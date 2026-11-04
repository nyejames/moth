can_error |ok Bool| -> String, Error!:
    if ok:
        return "ok"
    ;

    return! Error("boom")
;

value = can_error(true) catch |err|:
    io.line([: [err.message]])
    assert(false, "unreachable error path")
;

io.line("value catch assert false terminal")
io.line([: [value]])
