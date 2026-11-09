Response ::
    Success,
    Err |
        message String,
    |,
;

response = Response::Err("bad")

if response is:
    Err(message as 1) => io.line("bad")
    Success => io.line("ok")
;
