-- An immutable non-copy value cannot be borrowed for `set` when it is still needed later.
scores ~{String = String} = {}
value String = "hello"
~scores.set("key", value) catch:
;
io.line([: [value]])
