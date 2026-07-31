-- Test that pure acyclic if/else chains use structured JS, not the dispatcher.
-- Regression test: ensures the backend does not accidentally fall back to the
-- switch-based dispatcher for simple branching.

x = 10
y = 20

result ~= "default"
label ~= "default"

if x > 5:
    result = "x is big"
else
    result = "x is small"
;

if y > 15:
    label = "y is big"
else
    label = "y is small"
;

io.line([: [result]])
io.line([: [label]])
