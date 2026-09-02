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

r = Response::Pending(3, message = "waiting")
if r is:
    Success => io.line("done")
    Err(message) => io.line([: [message]])
    Pending(retry_count, message) => io.line([: [retry_count] [message]])
;
