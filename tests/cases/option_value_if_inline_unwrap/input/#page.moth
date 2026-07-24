display_name |maybe_name String?| -> String:
    name = if maybe_name is |name| then name else "guest"
    return name
;

maybe_missing String? = none

present_name = display_name("Priya")
missing_name = display_name(maybe_missing)

io.line("option_value_if_inline_unwrap")
io.line([:present [present_name]])
io.line([:missing [missing_name]])
