inactive_branch || -> String, Error!:
    return! Error("inactive branch evaluated", 500)
;

render_role || -> String, Error!:
    is_admin = false
    is_editor = true

    return [if is_admin:
        [inactive_branch()!]
    [else if is_editor]
        selected editor branch
    [else if true]
        [inactive_branch()!]
    [else]
        fallback branch
    ]
;

result = render_role() catch:
    then "caught inactive branch"
;

[:template_else_if_runtime_chain [result]]
