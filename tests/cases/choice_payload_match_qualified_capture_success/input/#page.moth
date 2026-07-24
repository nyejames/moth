Response ::
    Success,
    Err |
        message String,
    |,
;

r = Response::Err("bad")

if r is:
    Response::Err(message) => io.line([: match-qualified-capture=[message]])
    Response::Success => io.line("ok")
;
