Response ::
    Success,
    Err |
        message String,
    |,
;

response = Response::Err("bad")

if response is:
    Err(text as error_message) => io.line([: [error_message]])
    Success => io.line("ok")
;
