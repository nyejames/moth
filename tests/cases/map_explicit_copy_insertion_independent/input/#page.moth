-- Explicit copies of a non-copy key and value remain usable after `set` inserts the copies.

scores ~{String = String} = {}
key String = "Priya"
value String = "ten"
key_copy = copy key
value_copy = copy value
~scores.set(key_copy, value_copy) catch:
;
length = scores.length
stored = scores.get(key) catch:
    then ""
;
[:map_explicit_copy_insertion_independent key=[key] value=[value] stored=[stored] length=[length]]
