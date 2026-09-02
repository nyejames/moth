inactive_branch || -> String, Error!:
    return! Error("inactive branch evaluated", 500)
;

render_selected || -> String, Error!:
    return [if true:
        selected lazy branch
    [else]
        [inactive_branch()!]
    ]
;

result = render_selected() catch:
    then "caught inactive branch"
;

[:template_control_flow_runtime_laziness [result]]
