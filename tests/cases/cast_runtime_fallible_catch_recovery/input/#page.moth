Holder = |
    code ~Int,
|

try_float |text String| -> Float, Error!:
    return cast! text
;

code_for_float |text String, holder ~Holder|:
    ignored Float = cast text catch |err|:
        holder.code = err.code
        then 0.0
    ;
;

code_for_float_int |value Float| -> Int:
    result Int = cast value catch |err|:
        then err.code
    ;
    return result
;

code_for_char |text String, holder ~Holder|:
    ignored Char = cast text catch |err|:
        holder.code = err.code
        then ' '
    ;
;

code_for_int_char |value Int, holder ~Holder|:
    ignored Char = cast value catch |err|:
        holder.code = err.code
        then ' '
    ;
;

float_holder ~Holder = Holder(code = 0)
code_for_float("not a number", ~float_holder)
bad_float_fmt = float_holder.code

big Float = try_float("1e100") catch |err|:
    then 0.0
;
oor_float = code_for_float_int(big)

char_holder ~Holder = Holder(code = 0)
code_for_char("ab", ~char_holder)
bad_char = char_holder.code

cp_holder ~Holder = Holder(code = 0)
code_for_int_char(1114112, ~cp_holder)
bad_cp = cp_holder.code

[: runtime catch codes: [bad_float_fmt]|[oor_float]|[bad_char]|[bad_cp]]
