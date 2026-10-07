# Quick note of noticed bad diagnostics to fix

## Non const in const template

```
result = 0.1 + 0.2

#[result]
```

Produces "Unknown value name 'result'." which is unhelpful and should be a diagnostic about being unable to use a non-constant value in a const template.


## Compile time overflow (possible Dec bug)

Using a Dec still says: Compile-time integer overflow while evaluating '^'. Numeric profile: Int32/Float64.

for:
```
result #Dec = 100 ^ 2 ^ 2 ^ 2 ^ 2 ^ 2

#[result]
```

Either this is a Dec bug since Dec should be arbitary size and exponentiation should allow this (and Int32/Float64 is actually whats being used under the hood when it should't be) or this is valid (and needs justification) but gives wrong types in the error message

## mtf unhelpful unknown variable error

```
Test case 1: [left, right] in .mtf prose (FAILS).
```

Error should be more helpful and describe that undeclared variables are being used here.

Currently reports a generic "Compile-time evaluation error --> test_brackets.mtf:1:1" error.

## No diagnostic for using a keyword for a variable name

```
taxicab |left Cell, right Cell| -> Int:
    ax, ay = to_coords(left)
    bx, by = to_coords(right)
    dx = if ax > bx then ax - bx else bx - ax
    dy = if ay > by then ay - by else by - ay
    return dx + dy
;
```

Error given:
Unary negation must be attached to its operand with no intervening whitespace.

Hint: Suggestion: Write unary negation as `-value` or `-1`

--> test.moth:247:42
247 |     dy = if ay > by then ay - by else by - ay
    |                                          ^

but 

