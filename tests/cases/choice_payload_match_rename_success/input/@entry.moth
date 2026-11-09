Response ::
    Success,
    Err |
        message String,
    |,
;

response = Response::Err("bad")

if response is:
    Err(message as error_message) => io.line([: match-renamed-capture=[error_message]])
    Success => io.line("ok")
;
