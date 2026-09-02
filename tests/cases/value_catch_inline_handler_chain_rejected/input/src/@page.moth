can_error || -> Int, Error!:
    return 1
;

recover || -> Int:
    value = can_error() catch then can_error() catch then 0
    return value
;
