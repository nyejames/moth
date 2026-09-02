render_loop_mutation || -> String:
    values ~= {0}
    i ~= 0
    loop i < 3:
        ~values.push(i)
        i = i + 1
    ;
    len = values.length()
    return [:collection_mutation_inside_loop len=[len]]
;

[:[render_loop_mutation()]]
