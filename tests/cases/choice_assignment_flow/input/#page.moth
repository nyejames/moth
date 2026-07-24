Status :: Ready, Busy;

current ~= Status::Ready
current = Status::Busy
label ~= "unset"
if current is:
    Ready => label = "ready"
    Busy => label = "busy"
;
[:choice_assignment_flow result=[label]]
