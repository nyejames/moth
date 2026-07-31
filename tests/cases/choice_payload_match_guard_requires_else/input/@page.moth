Response ::
    Success,
    Err |
        message String,
    |,
;

r = Response::Err("bad")

if r is:
    Err(message) if message is not "" => io.line([: [message]])
    Success => io.line("ok")
;
