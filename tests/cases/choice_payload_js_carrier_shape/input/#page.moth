Response :: Err | message String |, Success;

e = Response::Err("bad")
if e is:
    Success => io.line("ok")
    Err(message) => io.line("err")
;
