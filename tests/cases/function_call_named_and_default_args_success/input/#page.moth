-- Ordinary function-call named/default argument binding. One primary owner
-- covers `language.functions.named_and_default_arguments`: mixed positional-
-- then-named arguments, all-named arguments independent of authored parameter
-- order, and named omission of an earlier defaulted parameter. Each routing
-- dimension is named and independently observable through one top-level runtime
-- output event.

format |prefix String = "<<", value String, suffix String| -> String:
    return prefix + value + suffix
;

mixed_positional_then_named = format("P", value = "V", suffix = "1")
all_named_reverse_order = format(suffix = "2", value = "W", prefix = "Q")
named_skips_earlier_default = format(value = "X", suffix = "3")

[:function_call_named_and_default_args_success mixed_positional_then_named=[mixed_positional_then_named] all_named_reverse_order=[all_named_reverse_order] named_skips_earlier_default=[named_skips_earlier_default]]
