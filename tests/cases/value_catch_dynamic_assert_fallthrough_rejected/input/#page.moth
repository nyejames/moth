can_error |ok Bool| -> String, Error!:
    if ok:
        return "ok"
    ;

    return! Error("boom")
;

value = can_error(true) catch |err|:
    assert(err.code > 0, "dynamic assertion can pass")
;
