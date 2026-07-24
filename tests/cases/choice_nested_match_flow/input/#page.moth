Status :: Ready, Busy, Offline;
Phase :: Start, Run, End;

status Status = Status::Busy
phase Phase = Phase::Run

result ~= "unset"

if status is:
    Busy => result = "busy"
    Ready => result = "ready"
    Offline => result = "offline"
;

if phase is:
    Run => result = result + "-run"
    Start => result = result + "-start"
    End => result = result + "-end"
;

[:choice_nested_match_flow result=[result]]
