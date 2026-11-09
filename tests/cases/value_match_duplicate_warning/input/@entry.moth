Status :: Ready, Busy, Offline;

label_for |current Status| -> String:
    return if current is:
        Ready => then "ready"
        Busy => then "busy"
        Ready => then "ready again"
        Offline => then "offline"
    ;
;

label = label_for(Status::Busy)

[:value_match_duplicate_warning result=[label]]
