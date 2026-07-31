Status :: Ready, Busy, Offline;

get_status || -> Status:
    return Status::Busy
;

current Status = get_status()
label ~= "unset"
if current is:
    Ready => label = "ready"
    Busy => label = "busy"
    Offline => label = "offline"
;
[:choice_function_return_flow result=[label]]
