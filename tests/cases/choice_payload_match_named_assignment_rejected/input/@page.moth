Response ::
    Success,
    Err |
        message String,
    |,
;

r = Response::Err("bad")

if r is:
    Err(message = text) => io.line([: [text]])
    Success => io.line("ok")
;
