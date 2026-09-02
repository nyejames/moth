Inner = |
    value Int,
|

Outer = |
    inner Inner,
    label String,
|

bump_inner |this ~Outer|:
    this.inner.value = this.inner.value + 1
;

o ~= Outer(Inner(10), "test")
~o.bump_inner()
io.line([:receiver_method_nested_struct_success value=[o.inner.value]])
