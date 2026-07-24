Response ::
    Success,
    Err |
        message String,
    |,
;

message = "outer"
r = Response::Err("bad")

if r is:
    Err(message) => io.line([: [message]])
    Success => io.line("ok")
;
