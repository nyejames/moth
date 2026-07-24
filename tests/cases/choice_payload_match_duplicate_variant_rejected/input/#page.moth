Response ::
    Success,
    Err |
        message String,
    |,
;

r = Response::Err("bad")

if r is:
    Err(message) => io.line([: [message]])
    Err(other) => io.line([: [other]])
    Success => io.line("ok")
;
