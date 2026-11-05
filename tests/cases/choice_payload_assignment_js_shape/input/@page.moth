Response :: Err | message String |, Success;

e = Response::Err("bad")
other = e
if other is:
    Success => io.line("ok")
    Err(message) => io.line("err")
;
