-- Success-path coverage for JS hashmap helpers and runtime ordering contracts.

render_hashmap_contract || -> String:
    scores ~= {"Priya" = 10, "Grace" = 12}

    has_ada_before = scores.contains("Priya")
    has_linus_before = scores.contains("Linus")

    ~scores.set("Linus", 7) catch:
    ;
    len_after_insert = scores.length

    ~scores.set("Priya", 11) catch:
    ;
    len_after_replace = scores.length

    removed = ~scores.remove("Grace") catch:
        then 0
    ;
    len_after_remove = scores.length

    ~scores.clear()
    len_after_clear = scores.length

    missing = scores.get("Missing") catch:
        then 99
    ;

    duplicate {String = Int} = {runtime_duplicate_key() = 1, runtime_duplicate_key() = 2}
    duplicate_length = duplicate.length
    duplicate_value = duplicate.get("One") catch:
        then 0
    ;

    return [:hashmap_runtime has_ada=[has_ada_before] has_linus=[has_linus_before] insert_len=[len_after_insert] replace_len=[len_after_replace] removed=[removed] remove_len=[len_after_remove] clear_len=[len_after_clear] missing=[missing] duplicate=[duplicate_value] duplicate_len=[duplicate_length]]
;

runtime_duplicate_key || -> String:
    return "One"
;

[:[render_hashmap_contract()]]
