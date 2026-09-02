UserId as Int

render || -> String:
    by_name ~{String = Int} = {"Priya" = 1}
    by_id ~{Int = String} = {7 = "seven"}
    by_flag ~{Bool = String} = {true = "yes"}
    by_letter ~{Char = String} = {'a' = "letter"}
    by_alias ~{UserId = String} = {42 = "answer"}

    return [:hashmap_scalar_key_types_success lengths=[by_name.length],[by_id.length],[by_flag.length],[by_letter.length],[by_alias.length]]
;

[:[render()]]
