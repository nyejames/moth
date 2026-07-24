import @choices { Response as R }

a R = R::Err("bad")
b R = R::Err("bad")
c R = R::Err("good")

if a is b:
    io.line("equal")
;

if a is c:
    io.line("different")
else
    io.line("not equal")
;
