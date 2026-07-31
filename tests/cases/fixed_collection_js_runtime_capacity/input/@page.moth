render_fixed_runtime || -> String:
    items ~{2 Int} = {}
    initial_length = items.length()

    ~items.push(1) catch:
    ;
    after_first_length = items.length()

    ~items.push(2) catch:
    ;

    ~items.push(3) catch:
    ;
    after_overflow_length = items.length()

    removed = ~items.remove(0) catch:
        then 0
    ;
    after_remove_length = items.length()

    ~items.push(3) catch:
    ;
    final_length = items.length()

    first = items.get(0) catch:
        then 0
    ;
    second = items.get(1) catch:
        then 0
    ;

    return [:fixed_collection_js_runtime lengths=[initial_length],[after_first_length],[after_overflow_length],[after_remove_length],[final_length] removed=[removed] values=[first],[second]]
;

[:[render_fixed_runtime()]]
