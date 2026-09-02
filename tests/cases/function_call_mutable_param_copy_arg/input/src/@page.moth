mutate |value ~Int| -> Int:
    value = value + 1
    return value
;

source = 5
out = mutate(copy source)
[:function_call_mutable_param_copy_arg out=[out]]
