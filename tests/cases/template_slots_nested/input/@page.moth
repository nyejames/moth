complex_nested_slots #= [: before [: another scene inside [$slot("inner")]] after]
even_more_complex #= [: BEFORE [: another scene inside [$slot("first")] [$slot] after the nested slot [: another nested scene [$slot("second")]]] AFTER]

using_slots = [even_more_complex:[$insert("first"): first] in between [$insert("second"): second]]

io.line([: [using_slots]])
