-- A non-copy String lookup key is borrowed by contains, get, and remove.

observe |key String, value String, present Bool| -> String:
    return [:borrowed_lookup_key key=[key] value=[value] present=[present]]
;

scores ~{String = String} = {"Priya" = "ten", "Grace" = "twelve"}
key String = "Priya"
present = scores.contains(key)
value = scores.get(key) catch:
    then ""
;
-- The get alias reaches final use inside the call, which returns a fresh owned String.
captured = observe(key, value, present)
-- remove borrows the same key binding; the get alias is no longer live.
removed = ~scores.remove(key) catch:
    then ""
;
length_after = scores.length
-- The same key binding remains usable after all three operations.
label = key
[:[captured] removed=[removed] length_after=[length_after] label=[label]]
