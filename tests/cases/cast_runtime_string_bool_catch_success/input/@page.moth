parse |text String| -> Bool:
    value Bool = cast text catch:
        then false
    ;

    return value
;

yes = if parse("true") then "yes" else "no"
no = if parse("TRUE") then "yes" else "no"

[: runtime string bool cast: [yes]/[no]]
