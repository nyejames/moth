identity type T |value T| -> T:
    return value
;

-- The previous semantic ID bridge reused TypeParameterId(0) for every generic
-- declaration, so this diagnostic could render the earlier function's `T`
-- instead of this function's `U`.
bad_add type U |left U, right U| -> U:
    return left + right
;
