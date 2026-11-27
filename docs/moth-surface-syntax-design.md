# Moth Surface Syntax After Unified Bracket Construction

**Design record | 26 November 2026**

This document records the chosen source syntax for Moth after collections, maps and templates move to square brackets. The examples describe the target language. Read each example as a separate scope unless it refers back to the shared `Size` and `WindowEvent` declarations. The checked reference revisions predate this grammar. The syntax migration follows the hard removal of Wire and Route. Closures V1 follows the migration. The Channels V1 runtime has its own delivery requirements.

The bracket proposal defines construction, splicing, wrappers, capacity proofs and MON collection data. This companion defines blocks, branch and readiness arms, declared regions, choice declarations and shared field defaults. Read the examples using the agreed spellings even when the linked baseline references use the earlier grammar. [Unified Bracket Construction for Moth](unified-bracket-construction-design.md)

## 1. Reading the new syntax

```moth
Size = | width I32, height I32 = 720 |

WindowEvent = {
    Opened,
    Resized | width I32, height I32 = 720 |,
    Closed,
}

describe |event WindowEvent| -> String {
    return if event is {
        case Opened: then "opened"
        case Resized(width, height): then [: [width] x [height]]
        case Closed: then "closed"
    }
}

events [WindowEvent] = [WindowEvent::Opened, WindowEvent::Resized(1280)]
descriptions [String] = [loop events |event|:
    [: [describe(event)]]
]
```

The second event carries a width of 1280 and a height of 720. The function reads the completed event through a pattern. The loop constructs two String elements in order. It needs no mutable accumulator or statement that appends a result.

| Form | What an author can read from it |
|---|---|
| `[T]`, `[N T]`, `[K = V]` | A growable collection, a fixed-capacity collection or a map in type position. |
| `[...]` and `[head: body]` | A construction. A colon at the head boundary introduces body content. |
| `Name = \| fields \|` | A named struct with one constructor. Its parameter brackets declare the fields, as they do for a choice payload. |
| `Type = { ... }` | A choice declaration. Its comma-separated members are alternative constructors. |
| `Type(...)`, `Choice::Variant(...)` and `f(...)` | A named constructor, a payload constructor or a function call with the normal argument list. An anonymous record has named entries in parentheses. |
| `header { ... }` | The body of a function, control-flow construct, declaration or named region. Each construct defines visibility inside its body. |
| `case ...:` and `else:` | An arm and its fallback. A subject match tests patterns. An expressionless `if` selects a channel operation. |
| `then` and `return` | `then` supplies a result to an enclosing value-producing construct. `return` exits the function. |

Moth code closes these blocks with `}`. Strings, raw text and embedded languages can still contain literal braces or semicolons. Grouping an expression in parentheses continues to work. Standalone braces introduce neither an anonymous record nor a general-purpose lexical block.

### Parameter brackets

Read `|...|` in its header. A function signature declares named, typed parameters. A trait method requirement uses the same form, with `This` or `~This` for its receiver. A struct or choice variant declares fields. In a loop, an option test or a `catch`, the construct supplies the values and their types. The brackets give those values local names.

Constrained closures follow the syntax migration. A callback parameter declares its own signature inside the enclosing parameter list. At the call site the closure names the incoming arguments and `=>` introduces its result expression.

```moth
row_label |
    title String,
    format_label |text String| -> String,
| -> String {
    return format_label(title)
}

prefix = "Action: "
label = row_label("Save", |caption| => [prefix, caption])
```

The inner `|text String| -> String` belongs to `format_label`. Each invocation supplies a String, which the closure calls `caption`. The closure captures `prefix` from the enclosing scope. A closure can also take zero arguments. In `|| => prefix`, `prefix` is still a capture. [Closures V1](docs/closures/closures.mtf)

## 2. Functions, ordinary blocks and decisions

### One pair of braces around a conditional

```moth
report |ready Bool| {
    if ready {
        io.line("ready")
    else
        io.line("waiting")
    }
}
```

The opening brace belongs to the whole conditional. The bare `else` closes the first branch's visibility and starts the second branch. Declarations in one branch stay local to it. A nested conditional closes its own brace pair before the outer conditional resumes. Indentation shows the structure. The tokens determine it.

A producing conditional can supply several results. Moth already has an inline spelling for a short decision.

```moth
summary |ready Bool| -> String, I32 {
    label, total = if ready {
        then "ready", 1
    else
        io.warn("waiting")
        then "waiting", 0
    }
    return label, total
}

caption |ready Bool| -> String {
    return if ready then "Ready" else "Waiting"
}
```

The multi-bind receives two separate result slots. A declaration, assignment or return can receive a producing block. A value-producing branch must supply its required results or leave that path through recognised control flow.

### Pattern matches

```moth
display_size |event WindowEvent| -> String {
    return if event is {
        case Resized(width, height) if width > 1920:
            then [: Wide display, [width] x [height]]
        else:
            then "standard display"
    }
}

score_band |score I32| -> String {
    return if score is {
        case < 0: then "invalid"
        case 0: then "zero"
        case >= 90: then "excellent"
        else: then "ordinary"
    }
}

greet |name String?| -> String {
    return if name is {
        case |present|: then [: Hello, [present]!]
        case none: then "Hello, guest!"
    }
}
```

A full match evaluates its subject once and considers its arms in source order. Each pattern arm begins with `case` at the start of a logical line. The fallback begins with `else:`. The colon ends the pattern and optional guard. The body can begin on that line or continue across several lines. The next arm at the same match depth or the closing brace ends the body.

Choice patterns capture all payload fields in declaration order. Use `as` to rename an arm-local capture. Those bindings are available to the pattern's guard and body. A guard leaves open the path where the pattern matches but the guard is false. The existing exhaustiveness rules apply. An unguarded choice match can cover all alternatives, and an option match can cover presence and absence. An ordinary numeric match needs a fallback. A bodyless final `else:` in a statement match explicitly ignores the remaining cases.

The `case` marker also identifies a malformed arm early. Moth can continue a comparison across a newline before `is`. A line starting `case false` after `return enabled` begins a new arm even if its colon is missing. Report the missing colon at the arm header. Preserve legitimate multiline comparisons inside statements.

### Recovery and propagation

```moth
read_label |text String| -> String {
    count I32 = cast text catch |err| {
        io.warn(err.message)
        then 0
    }
    return [: Count [count]]
}
```

The `I32` receiver supplies the cast target. A failed conversion logs its error and supplies zero to the binding. For a short recovery, `count I32 = cast text catch then 0` uses the existing inline spelling. An error slot, `return!`, postfix propagation and error mapping keep their established contracts. A closing brace introduces no implicit return.

## 3. Bracket construction in ordinary code

Square brackets do different work in type and value position. A binding on the left can supply a receiving type for the construction on the right.

```moth
title = ["Build", " log"]
labels [String] = ["Build", "Log"]
bytes [8 Byte] = [77, 79, 84, 72]
scores [String = I32] = ["Priya" = 10, "Rob" = 8]
empty_scores [String = I32] = [=]

first = [1, 2]
second = [3, 4]
flat = [first, second]
rows = [[first], [second]]
```

`title` joins two strings. `labels` stores two strings. `bytes` contains four bytes and has room for four more. `flat` has four integers. `rows` has two freshly constructed inner rows. Direct nested brackets preserve a dimension. Splicing a compatible sequence contributes its elements once, in order. A fresh row has its own membership and preserves normal sharing for any reference-valued elements.

Write a fixed collection type as `[capacity T]`. Both parts are required. A positive `I32` literal or visible `#I32` constant supplies the capacity. The old capacity-only annotation goes away. A known field or parameter can still supply the complete fixed type to a literal. A map accepts direct keyed entries and `[=]` for an empty map. A map head gains no sequence-splicing or template-body semantics.

### Statement loops and construction loops

```moth
announce |names [String]| {
    loop names |name, index| {
        io.line([: [index]: [name]])
    }
}

greetings |names [String]| -> [String] {
    return [loop names |name|:
        [: Hello, [name]!]
    ]
}
```

Both headers bind `name` from the same kind of sequence. The braced body executes statements. The bracket body contributes child results to the receiving collection. Formatting whitespace in a sequence body contributes nothing. Raw text belongs inside a child that constructs a String. Ordinary loops remain useful for `break`, `continue` and explicit mutations. Construction loops cover collection production in the bracket design.

### Reusable templates

```moth
strong #= [$html:<strong>[$slot]</strong>]
paragraph #= [$html:<p>[$slot]</p>]

notice = [paragraph:[strong:Build complete]]
```

A bracket can apply at most one receiving wrapper. The inner application constructs `<strong>Build complete</strong>`. The outer one places that output inside a paragraph. Named inserts, defaults and `$supplied` follow the bracket proposal's slot rules. A stylesheet, a quoted string or raw template text containing `{`, `}` or `;` keeps those characters as data.

## 4. Constructors, fields and defaults

A named struct uses its type name as its constructor. A choice declares alternatives in its braced body. At a use site, `::` qualifies the selected variant.

```moth
Size = | width I32, height I32 = 720 |

WindowEvent = {
    Opened,
    Resized | width I32, height I32 = 720 |,
    Closed,
}

size = Size(1280)
wide = Size(height = 900, width = 1440)
opened = WindowEvent::Opened
resized = WindowEvent::Resized(1280)
tall = WindowEvent::Resized(1280, height = 1080)
```

Positional arguments fill fields in declaration order. Named arguments may follow them in any order. Each field can be supplied once. Unknown labels, duplicate arguments and extra positional arguments are errors. Both `size.height` and the `height` payload of `resized` hold `720`. Writing `Size(height = 900)` leaves the required `width` field missing.

An explicitly named alternative keeps its name even when it stands alone. For `DoorEvent = { Opened }`, write `DoorEvent::Opened`. Adding `Closed` later preserves that spelling. A fieldless alternative takes no parentheses. A constructor with fields takes parentheses even when defaults supply every value.

A field default must fully evaluate at compile time against the field's declared type. Apply it only when the caller omits that field. For `Caption = | text String? = "Untitled" |`, `Caption()` uses `"Untitled"` and `Caption(none)` stores `none`. Without an explicit default, an optional field still needs an argument. This proposal extends the existing record rule to fields of a choice alternative. [Record declarations](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/structs/struct-declarations.mtf#L12-L25) and [constructor routing](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/structs/construction-and-fields.mtf#L14-L28).

Anonymous record literals use named entries. In Moth source, `dimensions = (width = 1280, height = 720)` creates a local record with its own identity. Construct a `Size` with `Size(1280, 720)`. Parentheses around a comma-separated pair of positional values do not make a record.

## 5. MON field defaults

MON reads literal data against a receiving schema. A field with the `Size` schema accepts either `(width = 1280, height = 720)` or `Size(1280, 720)`. The anonymous form requires names. An explicit qualifier must match the schema's nominal type. For a choice, `::` identifies the alternative and an optional qualifier checks the choice name.

```mon
size = (width = 1280),
event = ::Resized(width = 1280),
previous = WindowEvent::Opened,
history = [WindowEvent::Opened, ::Resized(1440), ::Closed],
```

The example assumes a root schema with a `Size`, two `WindowEvent` fields and a sequence of `WindowEvent`. Both `size` and `event` receive the declared height of 720. The new rule extends MON choice decoding to apply a prevalidated schema default to an omitted field, just as it does for a named record. At an optional field, an explicit `none` remains `none`. A supplied nested record uses its own field rules. Moth source expressions and constructor calls do not run during decoding. [MON records and choices](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/mon/mon-format.mtf#L204-L252) and [schema defaults](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/mon/mon-format.mtf#L279-L299).

## 6. Channels and readiness

Channels V1 gives each function invocation ownership of the child tasks it starts. Prefix `>>` starts a direct function or receiver-method call as a child task. Dispatch is a statement and its target has zero ordinary success results. Infix `>>` sends a value to a writer. Prefix `<<` receives from a reader. Ordinary calls stay on the current task and can suspend. These are accepted Channels V1 operations awaiting implementation. The examples use the agreed block and arm spellings.

```moth
produce |writer ChannelWriter of String| -> Error! {
    "Priya" >> writer
    "Rob" >> writer
}

run || -> Error! {
    reader, writer Channel of String = Channel()
    >> produce(writer)
    writer.close()

    loop << reader is |name| {
        io.line(name)
    }
}

run()!
```

`Channel()` creates an unbuffered channel with the reader first and the writer second. Each send waits for a matching receive. The child holds its own writer, so closing the parent's writer leaves the child free to send both names. When the child exits, its writer closes. The next receive yields `none` and the loop ends. If the parent kept its writer open while draining, it would wait for another name on a channel it kept open itself. Before `run` returns, it joins its child. A child failure reaches the caller through the owning invocation.

An expressionless `if` chooses a ready channel operation. Each `case` offers one send or receive. Here `handle` accepts the optional result of receiving from `reader`.

```moth
if {
    case incoming = << reader if listening:
        handle(incoming)
    case pending >> writer:
        io.line("sent")
    else:
        io.line("nothing ready")
}
```

The trailing Boolean guard enables the receive when `listening` is true. The binding `incoming` exists only in the winning arm's body. Write `case << reader:` to receive and discard the result. Use an ordinary `if condition` for a Boolean test or a subject match for a value pattern. An expressionless `if` rejects either as a case header.

Among ready arms, the scheduler selects fairly and commits at most one operation. A losing receive consumes nothing and a losing send retains its payload. A final `else:` runs immediately when no enabled operation is ready. Omit it to wait. For one unconditional operation, write `message = << reader` or `message >> writer` directly.

A receive on an open, empty channel waits. Once the last writer closes and the buffer drains, the reader stays ready and returns `none`, so a repeated selection must handle closure or disable that arm. A send with no remaining readers is also ready. If that send wins, it raises a recoverable error and skips its success body. The sender retains a payload that the channel never accepted.

[Channels V1 source reference](https://github.com/nyejames/moth/blob/f8e1f585573c9d3939b48e89b6644a75784c98c0/docs/docs/async/channels.mtf#L5-L146)

## 7. Named regions, methods and other block forms

A declared region has a name and a braced body in executable code. A declaration with `into name` selects the lifetime of eligible fresh storage or an independent copy. This example uses a short-lived copy while preparing a result for an enclosing region.

```moth
print_copy |source [Byte]| {
    request {
        scratch {
            temporary [Byte] into scratch = copy source
            saved [Byte] into request = copy temporary
        }
        io.line([: Saved [saved.length()] bytes])
    }
}
```

`saved` is visible after `scratch` closes because the declaration targets `request` from a straight-line nested region. The copy lets it outlive `temporary`. An ordinary local declaration inside `scratch` keeps its normal visibility. Ancestor placement retains the existing restrictions across a conditional, loop or match arm.

Each region reclaims its storage together on every exit, including an early return or propagated error. Leaving an ordinary branch has no such bulk-placement meaning. A spawned task still belongs to its function invocation, and all task arguments must satisfy the lifetime rules. A region closer cannot join a child task. The spelling introduces no `region` or `drop` keyword. [Declared region contract](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/memory/declared-regions.mtf)

### Trait requirements, methods and generics

The body convention also applies to trait declarations. The requirements are signatures. A receiver method supplies its implementation at the usual top-level definition site.

```moth
DISPLAY_TEXT must {
    display |This| -> String
}

Label = | text String |

display |this Label| -> String {
    return this.text
}

Label must DISPLAY_TEXT

render type Item is DISPLAY_TEXT |item Item| -> String {
    return item.display()
}
```

A named struct keeps ordinary field access and mutation through an eligible mutable place, such as `~size.width = 1600` on a mutable `Size`. Choice patterns bind immutable names. Adding payload defaults does not add direct mutable projection into a choice or change the equality rules. A nominal type with one explicitly listed variant still uses its variant constructor.

Generic functions and const functions use the same braced body. This const function has a typed parameter, a folded call and an ordinary return.

```moth
identity type A |value A| -> A {
    return value
}

twice #|value I32| -> I32 {
    return value * 2
}

width #I32 = twice(640)
screen = Size(width)
```

Ranges keep their existing endpoint and stepping rules in this reshuffle. With a positive step of one, `loop 1 to 3 |n| { ... }` visits 1 and 2. The inclusive `loop 1 to & 3 |n| { ... }` also visits 3. A later range proposal can address that spelling separately.

## 8. Grammar and downstream contracts

### Reading the boundary at its owner

A parser can distinguish the main forms from their headers. `if condition {` starts a Boolean block or single predicate. `if subject is {` introduces a subject for pattern matching. `if {` starts a readiness statement. Within a match or selection, recognise `case` and `else:` at the active arm-list depth. Nesting, the pattern or operation and its optional guard determine where the arm colon belongs. A colon inside an operand's template belongs to that template. An ordinary `else` belongs to its owning Boolean conditional.

The choice declaration `WindowEvent = { ... }` occurs in a type definition. A declared region such as `request { ... }` occurs in an executable body. Neither changes the expression grammar for `WindowEvent::Resized(...)`, `Size(...)` or an anonymous record. A parenthesis after a named type or variant still routes constructor arguments. Data labels stay distinct from local bindings and computed map keys.

### Information the compiled program needs

- Carry the construct, its opener and the boundaries of each branch through parsing. A single brace pair around `if` can enclose sibling scopes. Name resolution must give a match capture to its guard and body, and a readiness receive binding only to its winning body.
- Publish a choice field's typed default with the constructor signature wherever a caller can omit that field. Apply normal positional and named argument routing and preserve evaluation order for supplied arguments. A completed payload contains every field before pattern matching.
- A MON reader obtains validated defaults from its prepared data schema. Apply the same omission and nominal type rules to a qualified variant. Preserve explicit `none` when the data supplies it. Generated Moth code or a call into the Moth evaluator is unnecessary for decoding.
- Preserve region identity, placement and the selected channel operation through later lowering. Changing source punctuation creates no new lifetime, transfer, scheduler or wire-encoding policy.
- Formatters, syntax highlighting and diagnostics need to identify brace and colon roles by context. Embedded CSS, JavaScript, raw template text and MON data each retain their own lexical rules. The old grammar also has inline-closer exceptions. A future implementation needs an explicit account of how those layout rules map to the new closers.

These contracts identify observable information a later plan must preserve. The bracket proposal remains the authority for construction resolution, capacity checks, contribution order and template lowering.

## 9. Migration and observable acceptance criteria

Read the old spellings against the language reference at `d6b32308` and the Channels V1 reference at `f8e1f585`. The examples below describe the expected source and behaviour after the redesign. Use them as source-language acceptance cases for a later implementation.

### Translating existing source

| Existing source | Agreed spelling | What to check |
|---|---|---|
| `if ready:` with bare `else` and one closing `;` | `if ready {` with bare `else` inside the same pair of braces | Exactly one branch runs. Each branch has its own bindings. |
| `if score is:` with `< 0 =>` and `else =>` | `if score is {` with `case < 0:` and `else:` | A value match still uses `then` to supply its results. The existing coverage rules still apply. |
| Channels V1 `if:` with `pending >> writer =>` | `if {` with `case pending >> writer:` | This is a readiness statement. A losing send transfers nothing. |
| `scratch:` closed by `;` | `scratch {` closed by `}` | Preserve explicit `into scratch` placement and cleanup on each exit. |
| `WindowEvent ::` with a variant list closed by `;` | `WindowEvent = {` with the list closed by `}` | Keep `WindowEvent::Opened` and `WindowEvent::Resized(...)` at use sites. |
| A colon-delimited loop over `0 to 2` or `0 to & 2` | A braced loop body with the same range header | With a step of one, the first range visits 0 and 1. The second also visits 2. |

Migrate collection literals according to what they store. Old `names = {"Priya", "Rob"}` becomes `names [String] = ["Priya", "Rob"]` to preserve two elements. An inferred all-String head would concatenate them. Keep braces that belong to text or foreign code. For example, `[:case Ready: {ok}]` produces the String `"case Ready: {ok}"`.

### Distinguishing the new forms

- **Match guard.** Given `Resized | width I32, height I32 = 720 |`, the call `WindowEvent::Resized(width = 1280)` completes both fields. In `case Resized(width, height) if height > 0:`, both names are available to the guard and body. A sibling arm using `height` without a binding fails. Omitting the required `width` at construction also fails. A MON decoder with the corresponding prepared schema supplies the same height without evaluating source code.
- **Readiness guard.** In `case incoming = << reader if listening:`, the guard reads the existing Bool `listening`. The winning arm receives the optional result as `incoming`. Replacing the guard with `if incoming is none` fails when `incoming` is the prospective binding. A fallback runs immediately when no enabled operation is ready. Without a fallback the selection waits. Readiness commits at most one operation.
- **Arm boundaries.** A `case` in a nested match belongs to that match. A line-initial `is` that continues an equality expression after `return value` still belongs to the return. A missing colon in `case Resized(width, height)` should point to that header and allow the next arm at the same brace depth to be recognised. A colon inside a nested template cannot complete the header.
- **Closing the intended construct.** In a Boolean `if`, `else` belongs inside the open brace pair. An `else` after the completed outer `}` with no enclosing conditional is an error. A missing inner `}` should identify its opener. A literal `}` in `[: }]` neither closes the executable body nor repairs the missing brace.
- **Region exit.** A binding placed into an enclosing `request` region from a straight-line nested `scratch` remains usable after `scratch` closes. An ordinary local of `scratch` goes out of scope there. Placing an ancestor-owned declaration across a conditional boundary still fails. Returning early still ends every region exited.

Two compatibility details remain for planning. Determine how `case` affects existing source identifiers and MON schemas with names such as `case` or `Case`. MON has no arm grammar. Also settle any exact formatter rules for inline arms and brace placement. The line-initial arm marker and the distinction between bare `else` and fallback `else:` belong to the agreed grammar. Example layout alone does not establish additional rules.

### Reference revisions

- [Blocks and statements](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/language-overview/blocks-and-statements.mtf) and [pattern matching](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/branching/pattern-matching.mtf) give the pre-reshuffle delimiter and control-flow contracts.
- [Expression newline handling](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/src/compiler_frontend/ast/expressions/parse_expression_dispatch.rs#L1525-L1594) and [arm boundary recognition](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/src/compiler_frontend/ast/statements/match_arm_boundaries.rs) show why a leading arm marker helps diagnose a missing separator.
- [Shared MON identifier rules](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/mon/mon-format.mtf#L101-L114) describe the compatibility surface for any newly reserved word.
- [Channels V1](https://github.com/nyejames/moth/blob/f8e1f585573c9d3939b48e89b6644a75784c98c0/docs/docs/async/channels.mtf) supplies the task, endpoint and selection semantics. This companion replaces its older control-flow punctuation.
