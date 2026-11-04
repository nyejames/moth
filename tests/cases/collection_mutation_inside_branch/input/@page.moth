render_branch_mutation || -> String:
    values ~= {1, 2, 3}
    condition = true
    if condition:
        ~values.push(4)
    else
        removed = ~values.remove(0) catch:
            then 0
        ;
    ;
    ~values.set(0, 10) catch:
    ;
    first = values.get(0) catch:
        then 0
    ;
    len = values.length()
    return [:collection_mutation_inside_branch first=[first] len=[len]]
;

[:[render_branch_mutation()]]
