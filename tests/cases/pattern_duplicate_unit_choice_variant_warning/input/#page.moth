Status :: Ready, Busy, Offline;

current Status = Status::Busy
result ~= "unset"

if current is:
    Ready => result = "ready"
    Busy => result = "busy"
    Ready => result = "ready again"
    Offline => result = "offline"
;

[:pattern_duplicate_unit_choice_variant_warning result=[result]]
