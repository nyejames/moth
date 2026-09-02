-- A live shared alias from map `get` blocks a later `set` while the alias is still used.
scores ~{String = Int} = {"Priya" = 10}
score = scores.get("Priya") catch:
    then 0
;
~scores.set("Linus", 7) catch:
;
io.line([: [score]])
