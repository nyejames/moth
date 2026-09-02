-- Positive: aliases, struct fields, function parameter/return maps.

Scores as {String = Int}

Person = |
    name String,
    scores {String = Int},
|

get_score |p Person, key String| -> Int:
    return p.scores.get(key) catch:
        then 0
    ;
;

scores ~Scores = {"Priya" = 10}
p ~= Person("Grace", scores)
priya_score = get_score(p, "Priya")

-- Mutate the local mutable map directly
~scores.set("Linus", 7) catch:
;
linus_score = get_score(p, "Linus")
p2 = Person("Bo", scores)
bo_score = get_score(p2, "Linus")

[:alias_struct_param priya=[priya_score] linus=[linus_score] bo=[bo_score]]
