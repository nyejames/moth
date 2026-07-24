Response ::
    Success,
    Err |
        message String,
    |,
;

r = Response::Err("bad")

if r is:
    Err(message, extra) => io.line("ok")
    Success => io.line("ok")
;
