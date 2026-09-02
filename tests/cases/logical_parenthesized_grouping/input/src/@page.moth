a = true
b = false
c = false

grouped = (a or b) and c
ungrouped = a or b and c

grouped_taken ~= 0
if grouped:
    grouped_taken = 1
;

ungrouped_taken ~= 0
if ungrouped:
    ungrouped_taken = 1
;

[:logical_grouping grouped=[grouped] ungrouped=[ungrouped] taken=[grouped_taken]-[ungrouped_taken]]
