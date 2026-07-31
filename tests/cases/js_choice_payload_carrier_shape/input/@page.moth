Response ::
    Success,
    Err |
        message String,
    |,
    Pending |
        retry_count Int,
        message String,
    |,
;

u = Response::Success
p = Response::Err("bad")
n = Response::Pending(3, message = "waiting")

io.line("ok")
