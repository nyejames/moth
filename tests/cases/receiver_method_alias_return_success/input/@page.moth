Point = |
    x Int,
    y Int,
|

self_ref |this Point| -> this:
    return this
;

p ~= Point(1, 2)
q ~= p.self_ref()
q.x = 99
io.line([:receiver_method_alias_return_success value=[q.x]])
