Accumulator = |
    total Int,
|

add |this ~Accumulator, amount Int|:
    this.total = this.total + amount
;

acc ~= Accumulator(0)
acc.total = 3
~acc.add(4)
io.line([:receiver_method_field_write_then_mutable_call value=[acc.total]])
