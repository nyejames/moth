-- A map `get` alias reaches final use before `set` mutates the same map.

observe |value String, before Int| -> String:
    return [:map_alias_final_use_allows_mutation observed=[value] length_before=[before]]
;

scores ~{String = String} = {"Priya" = "ten"}
score = scores.get("Priya") catch:
    then ""
;
length_before = scores.length
-- The alias reaches final use inside the call, which returns a fresh owned String.
part_before = observe(score, length_before)
-- Mutation succeeds because the shared alias is no longer live.
~scores.set("Linus", "seven") catch:
;
length_after = scores.length
[:[part_before] length_after=[length_after]]
