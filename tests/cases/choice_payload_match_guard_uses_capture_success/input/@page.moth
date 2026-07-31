Response ::
    Success,
    Err |
        message String,
    |,
;

r = Response::Err("bad")

if r is:
    Err(message) if message is not "" => io.line([: match-guarded-capture=[message]])
    Success => io.line("ok")
    else => io.line("else")
;
