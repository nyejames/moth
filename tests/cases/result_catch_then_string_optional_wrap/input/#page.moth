find_name || -> String?, Error!:
    return! Error("boom")
;

recover || -> String?:
    return find_name() catch:
        then "fallback"
    ;
;

value = recover()
label ~= "unset"
if value is:
    none => label = "none"
    "fallback" => label = "fallback"
    else => label = "other"
;

io.line([: [label]])
