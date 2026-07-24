User = |
    name String,
|

Status :: Ready, Busy;

status_label |status Status| -> String:
    if status is:
        Ready => return "ready"
        Busy => return "busy"
    ;
;

user $User = User("Priya")
maybe_user $User? = user
status $Status = Status::Busy

[:
    reactive_declaration_composite_success
    user=[user.name]
    maybe=[if maybe_user is |present|:
        [present.name]
    [else]
        none
    ]
    status=[status_label(status)]
]
