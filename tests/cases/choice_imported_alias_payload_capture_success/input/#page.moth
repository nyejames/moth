import @choices/Response { Response as Reply }

e = Reply::Err("captured")

if e is:
    Reply::Success => io.line("ok")
    Reply::Err(message) => io.line([:imported-alias-message=[message]])
;
