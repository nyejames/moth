# Repeated option-capture loops implementation plan

## Current state

Queued at low priority. No slice is active. On activation, check the current
statement-loop, option-capture and HIR control-flow owners against the compiler
design and the canonical loop and option references.

## Source contract

A statement loop can inspect a fresh optional result on each attempt:

```moth
loop ~cursor.next() is |item| {
    process(item)
}
```

Any expression of type `T?` can supply the result. An exclusive method call
uses the ordinary `~` access marker. No method name has special meaning.
Evaluate the header once per attempt. A present result binds one immutable
`T` in the body. A `none` result exits without entering it. Reading a
present value grants no extra ownership or mutation rights.

Normal body completion and `continue` reevaluate the header after the
previous body path has finished. A `break` exits without another evaluation.
A return or propagated error follows its ordinary function or recovery
boundary. The binding is out of scope at the next header evaluation and
after the loop. This header has no implicit index or arbitrary choice
pattern. V1 accepts it in statement loops only.

## Implementation and verification

1. Extend the shared loop-header classifier at the top-level `is |name|`
   suffix, before ordinary collection bindings. Reuse the nesting-aware
   cursor and the single-present-capture parser. Parse the operand once,
   check its option type and retain its span and the capture span. A nested
   `is` expression leaves the enclosing loop classification intact. Reject
   malformed captures, multiple names and this header in a bracket loop at
   the source that requests it.
2. Lower the typed operand into the loop header block through the ordinary
   option test and payload extraction. Branch to the exit on `none`. Bind
   the present payload in the body region. Reuse the existing loop target
   stack, error edges and HIR validation. Borrow and lifetime analysis see
   the actual reads, writes and back edges. The backend consumes ordinary
   loop control flow.
3. Exercise an initially empty source and a sequence of present values
   followed by `none`. Count calls across fallthrough, `continue`,
   `break` and early return. Cover nested loop targets, capture scope and
   immutability, non-option and malformed headers, nested `is` expressions,
   ordinary collection headers and bracket-loop rejection. Check a real
   cursor or channel consumer through its supported target once available.
4. Update the canonical conditional-loop and option references and mark
   actual implementation and target coverage in the progress matrix. Run the
   relevant loop and option tests and the repository gates for the checkpoint.
   Perform the Slice review and retire this plan and its roadmap entry in
   the implementing commit.
