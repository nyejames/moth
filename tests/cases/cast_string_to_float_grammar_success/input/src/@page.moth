parse_float |text String| -> Float, Error!:
    return cast! text
;

one = parse_float("1") catch: then 0.0;
ptfive = parse_float("1.5") catch: then 0.0;
neg = parse_float("-1.5") catch: then 0.0;
exp = parse_float("1e6") catch: then 0.0;
big = parse_float("1e+21") catch: then 0.0;
small = parse_float("1e-6") catch: then 0.0;
under = parse_float("1_000.5e-2") catch: then 0.0;

[: [one]|[ptfive]|[neg]|[exp]|[big]|[small]|[under]]
