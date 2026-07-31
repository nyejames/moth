-- Prove mutable param reassignment: the caller-visible result after mutation is
-- rendered into the page, proving caller-observable semantics.

increment |value ~Int| -> Int:
    value = value + 1
    return value
;

x ~= 10
result = increment(~x)
io.line([: [result]])

[:mutable_param_reassignment result=[result]]
