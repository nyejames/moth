Status :: Ready, Busy;

check_equal |a Status, b Status| -> Bool:
    return a is b
;

a Status = Status::Ready
b Status = Status::Ready
c Status = Status::Busy

if check_equal(a, b):
    io.line("same=true")
;

if check_equal(a, c):
    io.line("different=true")
else
    io.line("different=false")
;
