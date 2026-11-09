Response ::
    Success,
    Err |
        message String,
    |,
;

r = Response::Success

if r is:
    Success() => io.line("ok")
    Err(message) => io.line([: [message]])
;
