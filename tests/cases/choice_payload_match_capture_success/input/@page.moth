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

r = Response::Err("bad")

if r is:
    Success => io.line("ok")
    Err(message) => io.line([: match-capture=[message]])
    Pending(retry_count, message) => io.line([: [message]])
;
