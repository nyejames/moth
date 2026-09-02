-- remove returns an owned non-copy String that remains usable after later set and clear.

observe |value String, before Int| -> String:
    return [:map_remove_returns_owned_value observed=[value] length_before=[before]]
;

scores ~{String = String} = {"Priya" = "ten", "Grace" = "twelve"}
removed = ~scores.remove("Grace") catch:
    then ""
;
length_before = scores.length
-- The removed owned value is captured before any later mutation.
captured = observe(removed, length_before)
-- Later set and clear mutate the map; the removed value stays independent.
~scores.set("Linus", "seven") catch:
;
length_mid = scores.length
~scores.clear()
length_after = scores.length
-- The removed owned value is still usable after set and clear.
[:[captured] length_mid=[length_mid] length_after=[length_after] still=[removed]]
