find_name |present Bool| -> String?:
    if present is true:
        return "Priya"
    ;

    return none
;

display_name |present Bool| -> String?:
    name = find_name(present)?
    return [:Hello [name]]
;

display_name_fallible |present Bool| -> String?, Error!:
    name = find_name(present)?
    return [:Fallible [name]]
;

io.line("option_postfix_propagation_success")
if display_name(true) is |name|:
    io.line([: [name]])
else
    io.line("unexpected missing")
;

if display_name(false) is |name|:
    io.line([: [name]])
else
    io.line("missing")
;

fallible_present = display_name_fallible(true) catch then none
if fallible_present is |name|:
    io.line([: [name]])
else
    io.line("unexpected fallible missing")
;

fallible_missing = display_name_fallible(false) catch then none
if fallible_missing is |name|:
    io.line([: [name]])
else
    io.line("fallible missing")
;
