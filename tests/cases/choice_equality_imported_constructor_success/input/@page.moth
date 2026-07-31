import @choices { Response }

a Response = Response::Err("bad")
b Response = Response::Err("bad")
c Response = Response::Err("good")

if a is b:
    io.line("equal")
;

if a is c:
    io.line("different")
else
    io.line("not equal")
;
