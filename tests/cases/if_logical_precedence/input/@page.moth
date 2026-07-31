a = true
b = false
c = false

not_and = not a and b
or_after_and = a or b and c
grouped = (a or b) and c

not_and_taken ~= 0
if not_and:
    not_and_taken = 1
;

or_after_and_taken ~= 0
if or_after_and:
    or_after_and_taken = 1
;

grouped_taken ~= 0
if grouped:
    grouped_taken = 1
;

[:if_logical_precedence not_and=[not_and] or_after_and=[or_after_and] grouped=[grouped] taken=[not_and_taken]-[or_after_and_taken]-[grouped_taken]]
