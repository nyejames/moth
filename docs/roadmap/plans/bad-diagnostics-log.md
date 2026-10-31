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

## HTML template warnings highlight the template opener instead of the offending markup

Reproduction in an HTML docs component:

```moth
sidebar #= [$html:
    <script>
        console.log("hello")
    </script>
]
```

The `MOTH-SYNTAX-0005` warning says `Potentially unsafe '<script' tag found in '$html' template`, but the diagnostic snippet points at `[$html:` on the declaration line instead of highlighting the authored `<script` token. The warning itself is expected for this heuristic. Its source attribution is not.

Likely cause (inspected on main): `src/projects/html_project/styles/html.rs` calculates literal-match start/end character offsets. `src/projects/html_project/styles/validation.rs` then selects the containing `FormatterInputPiece::Text` source span in `map_warning_span_to_source_span` without narrowing that span to the matched substring. The test `html_validation_warnings_keep_authored_spans` currently expects a whole-body span, so it also encodes the overly broad result.

Fix the source mapping where formatter spans are still available, preserving the original authored location through whitespace formatting, nested pieces and interpolations. Convert character offsets to source-byte spans correctly (including non-ASCII text) rather than treating flattened-character indexes as byte offsets. Preserve multiple distinct warning sites within one template. Update the existing test and add multiline, multiple-violation, non-ASCII and composed-template coverage. Expected primary span: the exact offending `<script` tag, not the template opener.

## `catch` after an assertion explains safety-catch eligibility

```moth
assert(a + b > 0) catch:
    then false
;
```

`reject_assert_fallible_suffix` in `src/compiler_frontend/ast/statements/asserts.rs` reuses `CatchOnNonFallible` and `BangOnNonFallible`. The message then describes the assertion as an expression that is not an authored safety-catch site such as arithmetic, which reads as self-contradictory when the condition contains arithmetic. Assertions sit outside `catch` and `!` entirely, so the diagnostic should say the assertion statement cannot take a handler or propagation suffix.

## Match arm `->` and value-match colons reach the wrong diagnostic

```moth
if value is:
    1 -> io.line("one")
    else => io.line("other")
;
```

`1 -> ...` reports `invalid_match_arm.expected_arm_header` instead of `InvalidArrow`, which is only reachable through `else ->` or a pattern suffix on a line that also has `=>`. The `diagnostic_match_arrow_separator` case asserts only `MOTH-SYNTAX-0028`, so it hides the wrong reason. Inside a value match such as `result = if value is:`, an arm written `1:` or `else:` reports "Unexpected end of file, expected ';'" rather than the ordinary arm-header or expected-`=>` diagnostic.

## Namespace receiver-method statement calls leak a path id

A bare statement call such as `tick(value)`, where `tick` is a receiver method declared in a file bound as a source namespace, renders `'tick' is a receiver method for 'PathId(7)'`. The receiver-method branch in `src/compiler_frontend/ast/statements/body_symbol.rs` formats an internal path id instead of the source-visible receiver type name.
