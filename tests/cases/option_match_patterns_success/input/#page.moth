name String? = "Ana"
name_label ~= "unset"

if name is:
    none => name_label = "missing"
    "Ana" => name_label = "ana"
    else => name_label = "other"
;

User = | name String |
maybe_user User? = none
user_label ~= "unset"

if maybe_user is:
    none => user_label = "missing"
    else => user_label = "present"
;

score Int? = 7
score_label ~= "unset"

if score is:
    < 5 => score_label = "low"
    < 10 => score_label = "medium"
    |amount| => score_label = [:high=[amount]]
    none => score_label = "missing"
;

[:option_match_patterns_success name=[name_label] user=[user_label] score=[score_label]]
