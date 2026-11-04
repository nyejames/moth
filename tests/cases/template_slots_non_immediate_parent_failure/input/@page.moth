outer #= [: OUTER [$slot("outer")] END]
inner #= [: INNER [$slot("inner")] END]
outer_insert #= [$insert("outer"): no-grandparent-matching]
output = [outer, inner, outer_insert]
io.line([: [output]])
