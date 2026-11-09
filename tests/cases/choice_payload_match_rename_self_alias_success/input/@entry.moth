Response ::
    Success,
    Err |
        message String,
    |,
;

response = Response::Err("bad")

if response is:
    Err(message as message) => io.line([: match-self-alias-rename=[message]])
    Success => io.line("ok")
;
