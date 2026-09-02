-- An explicit copy of a map `get` result remains usable after the map is mutated.

observe |value String, before Int| -> String:
    return [:map_explicit_copy_independent_after_mutation observed=[value] length_before=[before]]
;

scores ~{String = String} = {"Priya" = "ten"}
score = scores.get("Priya") catch:
    then ""
;
snapshot = copy score
length_before = scores.length
-- The original alias reaches final use inside the call, which returns a fresh owned String.
part_before = observe(score, length_before)
-- Mutation succeeds because the shared alias is no longer live.
~scores.set("Linus", "seven") catch:
;
length_after = scores.length
updated = scores.get("Linus") catch:
    then ""
;
-- The independent copy remains usable after the map is mutated.
[:[part_before] copy=[snapshot] updated=[updated] length_after=[length_after]]
