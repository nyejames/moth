mutate |items ~{Int}| -> Int:
    ~items.push(3)
    return items.length()
;

render_mutable_alias || -> String:
    values ~= {1, 2}
    len = mutate(~values)
    return [:collection_mutation_through_mutable_alias len=[len]]
;

[:[render_mutable_alias()]]
