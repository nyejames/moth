Response :: Err | message String |, Success;

make_err |msg String| -> Response:
    return Response::Err(msg)
;

result = make_err("bad")
if result is:
    Success => io.line("ok")
    Err(message) => io.line("err")
;
