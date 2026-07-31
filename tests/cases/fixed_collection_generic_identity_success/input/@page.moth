identity type A |value A| -> A:
    return value
;

render_generic || -> String:
    items {4 Int} = {1}
    same = identity(items)
    first = same.get(0) catch:
        then 0
    ;
    len = same.length()
    return [:fixed_collection_generic_identity_success value=[first] length=[len]]
;

[:[render_generic()]]
