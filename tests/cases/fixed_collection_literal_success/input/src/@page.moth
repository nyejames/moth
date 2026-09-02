render_literal || -> String:
    items {4 Int} = {1, 2}
    value = items.get(1) catch:
        then 0
    ;
    return [:fixed_collection_literal_success value=[value]]
;

[:[render_literal()]]
