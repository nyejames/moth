Status :: Ready, Busy;

status Status = Status::Ready
label ~= "unset"

if status is:
    Ready => label = "ready"
    Busy => label = "busy"
;

suffix ~= "-done"
result = label + suffix
[:choice_match_wildcard_merge_continuation result=[result]]
