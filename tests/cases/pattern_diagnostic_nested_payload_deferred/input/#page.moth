-- Nested payload patterns should be rejected with a clear deferred diagnostic.

Response ::
    Success,
    Err |
        message String,
    |,
;

r = Response::Err("bad")

if r is:
    Err(message(nested)) => io.line([: [nested]])
    else => io.line("other")
;
