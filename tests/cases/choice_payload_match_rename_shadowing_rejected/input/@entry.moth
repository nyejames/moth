Response ::
    Success,
    Err |
        message String,
    |,
;

error_message = "outer"
response = Response::Err("bad")

if response is:
    Err(message as error_message) => io.line([: [error_message]])
    Success => io.line("ok")
;
