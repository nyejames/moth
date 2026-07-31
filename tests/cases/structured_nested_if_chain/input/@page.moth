-- Acyclic match with many literal arms that must stay structured.

pick_label |n Int| -> String:
    if n is:
        0 => return "branch_a"
        1 => return "branch_b"
        2 => return "branch_c"
        3 => return "branch_d"
        4 => return "branch_e"
        else => return "branch_f"
    ;
;

result = pick_label(7)
io.line([: [result]])
