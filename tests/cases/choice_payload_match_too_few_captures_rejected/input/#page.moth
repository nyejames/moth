Response ::
    Success,
    Pending |
        retry_count Int,
        message String,
    |,
;

r = Response::Pending(3, "waiting")

if r is:
    Pending(retry_count) => io.line("ok")
    Success => io.line("ok")
;
