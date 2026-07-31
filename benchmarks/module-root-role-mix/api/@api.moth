import @./detail {detail_tag}

private_prefix #= "api"

export:
    format_status |label String| -> String:
        return [: [private_prefix]: [detail_tag(label)]]
    ;
;
