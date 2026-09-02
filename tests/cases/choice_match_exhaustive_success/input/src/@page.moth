Status :: Ready, Busy, Offline;

current Status = Status::Busy
match_result ~= "unset"

if current is:
    Ready => match_result = "ready"
    Status::Busy => match_result = "busy"
    Offline => match_result = "offline"
;

[:choice_match_exhaustive_success result=[match_result]]
