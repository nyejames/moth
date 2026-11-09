Response :: Success, Err | message String |;

if Response::Err(message = "bad") is Response::Err(message = "bad"):
    io.line("equal")
;

if Response::Err(message = "bad") is Response::Err(message = "good"):
    io.line("different")
else
    io.line("not equal")
;
