import @choices { HiddenStatus }

value HiddenStatus = HiddenStatus::Ready

variant ~= "unexpected"
if value is:
    Ready => variant = "Ready"
    Busy => variant = "Busy"
;

[:choice_same_module_private_import_success variant=[variant]]
