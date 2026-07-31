-- Constant struct constructor with a field whose type is another user nominal.
-- Regression: before the frontend type boundary cleanup, unresolved parsed type
-- references in constructor shells could silently fall back to builtin `none`.

Inner = |
    value Int,
|

Outer = |
    inner Inner,
|

outer #= Outer(Inner(42))

io.line([: [outer.inner.value]])
