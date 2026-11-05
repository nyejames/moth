-- Verify that choice equality does not double-evaluate expressions with side effects.
-- Each operand should be evaluated exactly once.

Response :: Ok, Err | message String |;

make_err || -> Response:
    io.line("call")
    return Response::Err("test")
;

if make_err() is make_err():
    io.line("equal")
;
