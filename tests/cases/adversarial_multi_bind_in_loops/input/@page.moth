pair |value Int| -> Int, Int:
    return value, value + 1
;

idx ~= 0
total ~= 0

loop idx < 4:
    left, right = pair(idx)
    total = total + left + right
    idx = idx + 1
;

io.line([: adversarial_multi_bind_in_loops final-total=[total]])
