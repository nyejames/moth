# Moth language cheatsheet

Moth is a small, statically typed language with first-class string templates, explicit mutable access and mandatory borrow and lifetime validation. This is a compact reference for humans and coding models writing Moth source.

This document describes the **accepted end-state language surface**. The current Alpha compiler may not support every accepted feature yet. Use the [progress matrix](../progress/) to check implementation and target status.

Status wording used here:

- **Accepted deferred** means the source contract is accepted but implementation is absent, partial or target-limited.
- **Design pending** means no final syntax or semantic contract exists. These areas are listed without speculative examples.

Detailed explanations live throughout the [Moth documentation](../).

## Rules to internalise first

- `:` opens a block. `;` closes it. Semicolons do not end statements.
- Bindings are immutable unless declared with `~`.
- Existing values use shared read-only access. Rebinding does not copy.
- `~place` requests exclusive access for one operation. It is not a type, reference or move.
- `copy place` creates independent storage. Moth has no explicit move or lifetime syntax.
- `[]` creates strings/templates. `{}` creates collections/maps.
- `!` is the typed error path. `?` is the option path. `assert` is for invariants.
- `@path` starts dependencies and resource paths. Source dependencies omit extensions.
- Names never shadow another visible name.
- `#Config of T` declares an accepted-deferred typed build configuration constant.
- A compile-time-known `Bool` can specialise an ordinary `if`; there is no `#Config if`.
- Source cannot inspect OS, architecture, backend or target identity.
- General closures, anonymous callables, generic function values, macros, exceptions, trait objects and operator overloading are outside scope.
- Named monomorphic non-capturing function references remain design pending.

## Common invalid translations

| Do not write | Write |
|---|---|
| `let name = value` | `name = value` |
| `let mut count = 0` | `count ~= 0` |
| `fn greet(...)` | `greet \|...\|` |
| `import @core/math` | `@core/math` |
| `pub name` | put the declaration inside the module root's `export:` block |
| `left == right` | `left is right` |
| `left != right` | `left is not right` |
| `!ready`, `a && b`, `a || b` | `not ready`, `a and b`, `a or b` |
| `{ ... }` for a code block | `: ... ;` |
| `statement;` | `statement` |
| `left + right` for strings | `[left, right]` |
| `&value` | `value` |
| `&mut value` | `~value` at an exclusive-access site |
| `move value` | no source form, final-use transfer is inferred |
| `value.clone()` | `copy value` when independent storage is required |
| `match value` | `if value is:` |
| `for item in items` | `loop items \|item\|:` |
| `while ready` | `loop ready:` |
| `_ => fallback` | `else => fallback` |
| `items[index]` | `items.get(index)!` or `~items.set(index, value)!` |
| `Option<T>` | `T?` |
| `Result<T, E>` | success return slots plus a final `E!` slot |
| tuple return values | multiple returns and a matching multi-bind |
| `Box<String>` | `Box of String` |
| an inline closure | a named function, static trait pattern or accepted reactive source/subscription pattern |

## Blocks, comments, scope and names

```moth
if ready:
    io.line("ready")
else
    io.line("waiting")
;
```

`--` starts a single-line comment in ordinary Moth code. Inside template and `.mtf` bodies, `--` is output text. Use `$note` or `$todo` to discard template-authored content.

A plain lexical block uses `block:`:

```moth
name = "Priya"

block:
    label = [: User: [name]]
    io.line(label)
;
```

`block:` creates a child lexical and control-flow scope. Bare labelled blocks are invalid.

Naming conventions:

- Types, structs, choices, aliases and generic parameters use `PascalCase`.
- Variables and functions use `regular_snake_case`.
- Traits use `ALL_CAPS`.
- A visible name cannot be redeclared while it remains in scope.

`Config` and its case-insensitive, leading-underscore keyword-shadow variants are reserved. `Import`
is an ordinary valid user identifier; there is no `import` keyword.

Symbolic binary operators and assignment require spaces on both sides:

```moth
count = left + right
count += 1
count //= 2
count ~= 0 -- ~= stays adjacent
```

`count=1`, `left+right`, `count+=1` and `count ~ = 0` are invalid.

## Core values and strings

```moth
ready = true
count = 42
ratio = 1.5
letter = '🦋'
text = "Moth"
message = [: Hello, [text].]
```

Quoted text creates a read-only string slice. A template creates an owned string. Both use the semantic `String` type at typed boundaries. `Char` stores one Unicode scalar value.

Quoted strings cannot continue across a physical newline. They support only `\\`, `\"`, `\n`, `\r` and `\t`. Backticks are not raw source strings. In `$md` content, paired single backticks delimit inline code.

`String + String` is invalid. Concatenate and interpolate through templates:

```moth
joined = [left, right]
greeting = [: Hello [name]]
```

String equality compares content with `is` and `is not`. Strings do not support ordering operators or in-place character mutation.

## Bindings, mutability and constants

```moth
name = "Priya"             -- inferred immutable binding
age Int = 30               -- typed immutable binding

count ~= 0                 -- inferred mutable binding
names ~{String} = {}       -- typed mutable binding
count = 1                  -- reassignment

site_name #= "Moth"        -- inferred compile-time constant
version #Int = 1           -- typed compile-time constant
```

Mutability belongs to a binding or access operation, not to type identity. `names ~{String}` declares a mutable binding whose semantic type is `{String}`.

A constant:

- has an initializer
- is immutable
- may depend only on visible compile-time values
- must fully fold
- cannot use a same-file forward reference

Visible dependency-bound constants are already folded and may be used. Cross-file constants in the same module follow the module's declaration dependency ordering. `#` controls compile-time evaluation, not visibility.

## Reference semantics, copying and ownership

Existing values use shared read-only access:

```moth
items ~= {"Priya", "Rob"}
shared_items = items

count = shared_items.length()
~items.push("Emmy") -- valid: shared_items has no later use
```

A later use of `shared_items` would keep the alias live and make the mutation invalid. Borrow lifetimes follow control flow.

A mutable declaration from an existing place creates a write-through alias. One from a fresh expression creates an independent slot:

```moth
writer ~= items
~writer.push("Rob")

fresh ~= {"Emmy"}
```

Only `copy` creates independent storage from an existing place:

```moth
independent ~= copy items
```

`copy` accepts a binding, field projection or parenthesised place. It deep-copies the copyable graph, preserves internal alias topology and shares no mutable allocation with the source. Fresh literals, templates, calls and computations are invalid `copy` operands.

Moth has no move operator. The compiler may transfer cleanup responsibility at proven final use without changing aliasing or source meaning. Each allocation still has one semantic lifetime owner, and retained references must point to the same or a longer-lived region.

Memory safety comes from static proof, not from a collector. Borrow validation and lifetime-topology validation are mandatory on every backend and every build profile, so every target accepts and rejects exactly the same programs. Garbage collection is one permitted way to represent a topology the compiler has already proven legal; it can never make an illegal program legal.

Backends that advertise full memory control lower release builds without a tracing collector. That is a property of the backend and the emitted artefact, not a language mode: there is no source or project setting that turns GC on or off, and nothing about it appears in your code.

The compiler may use internal Retained Edge Counting when a runtime-dependent number of persistent stored aliases disappear independently. Ordinary local aliases and `get()` borrows are never counted, and no REC mechanism appears in source.

### Declared memory groups - accepted deferred

```moth
group request:
    parsed ParsedPost into request = parse_post(post)
    html String into request = render_post(parsed)
;
```

`into group_name` appears after access/type syntax and before `=`. A group is a hard local lifetime owner, not a value or type. Placement targets the current group or an ancestor. Parent/sibling storage cannot retain child-group values, and group-owned values cannot escape. Everything in a group is released together when the group exits, never individually. A group is also the only place a program may build a reference cycle. V1 has no expression placement, extraction or unrestricted group transfer.

## Numbers and operators

Current numeric types:

- `Int`: signed 32-bit integer
- `Float`: finite IEEE-754 `f64`. `NaN` and infinities are invalid

```moth
count Int = 42
ratio Float = 0.5
```

Whole literals naturally infer `Int`. Decimal literals infer `Float`. Exponents use lowercase `e`. Uppercase `E`, unary `+` and spaced negation such as `- count` are invalid.

### Number and Byte - accepted deferred

- `Number` and `Number0`: arbitrary-precision scale-zero integers
- `Number1` through `Number256`: fixed-scale arbitrary-precision decimals
- `Byte`: unsigned `0..255` scaffold. Broader runtime semantics remain unsettled

```moth
large Number = 1000000000000000000000
price Number2 = 12.50
byte Byte = 255
```

A receiving `NumberN` context requires an exact literal. `price Number2 = 1.239` is invalid rather than rounded.

Arithmetic rules:

- `Int / Int -> Float`, while `Int // Int -> Int`
- mixed `Int` and `Float` arithmetic produces `Float`
- `NumberN` combines with the same scale or `Int`. Different scales need `cast`
- `NumberN` and `Float` do not mix
- positive-scale `NumberN` uses `/` and `%`. Scale zero uses `//` and `%`
- `NumberN ^ Int` requires a non-negative exponent
- positive-scale multiply, divide and exponentiation round half to even per operation
- `^` is right-associative

Numeric operations are checked. Statically known failure is a diagnostic. Supported runtime failure enters builtin `Error!` only when that is the function's final error slot. Otherwise it traps.

Precedence: unary `not`/`-`, `^`, `* / // %`, `+ -`, comparisons, `and`, `or`.

```moth
same = left is right
ready = has_input and is_valid
blocked = not ready
```

## Explicit casts

`cast` takes its target from the immediate typed receiving boundary:

```moth
ratio Float = cast 3
fallback Int = cast text catch then 0
label String = cast value
```

Propagation needs a complete function context:

```moth
parse_count |text String| -> Int, Error!:
    count Int = cast! text
    return count
;
```

Forms:

```moth
value Target = cast expression
value Target = cast! expression

value Target = cast expression catch |err|:
    then fallback
;
```

Rules:

- Plain `cast` requires infallible evidence.
- `cast!` requires the enclosing function's final error slot to be builtin `Error!`.
- `cast ... catch` handles only cast failure.
- `cast! ... catch` is invalid.
- Same-type casts are invalid.
- Generic inference does not look through `cast`.
- Scalar conversion constructors such as `Int(value)` and `String(value)` are invalid.
- `NumberN` scale widening is exact and infallible. Narrowing is exact-or-fail and never rounds.
- `Float <-> NumberN` has no accepted helper name or lossy conversion policy yet. Do not invent one.
- Numeric text casts consume the whole string and reject surrounding whitespace, uppercase `E`, `NaN` and infinity spellings.
- Same-file nominal types may provide compiler-owned cast evidence for supported builtin targets. Users cannot create new cast target families.

## Functions and calls

```moth
greet |name String, punctuation String = "!"| -> String:
    return [: Hello [name][punctuation]]
;

message = greet("Priya")
custom = greet(name = "Rob", punctuation = "?")
```

Parameters use `|...|`. No parameters use `||`. Omit `->` when there are no success values. Defaults fold at compile time. Positional arguments precede named arguments, which may skip earlier defaults. Host functions and compiler-owned builtin members are positional-only.

Mutable parameters require `~place` for existing storage, but accept fresh rvalues plainly:

```moth
increment |value ~Int|:
    value += 1
;

count ~= 1
increment(~count)
increment(1)
```

Mutable receivers always require an existing mutable place.

Multiple returns are not tuples:

```moth
pair || -> String, Int:
    return "Priya", 2
;

name, count = pair()
```

Receive them through matching multi-bind, return or value-producing syntax. General closure/function-value systems are outside scope. Narrow named function references remain design pending.

## Options: `T?`, `none` and postfix `?`

`T?` is an optional value. `none` needs an immediate optional receiving context.

```moth
find_name |id String| -> String?:
    if id is "":
        return none
    ;

    return "Priya"
;
```

A `T` value may be used where `T?` is expected. Optional values do not unwrap implicitly.

Postfix `?` unwraps a present value or immediately returns `none`:

```moth
load_label |id String| -> String?:
    name = find_name(id)?
    return [: User: [name]]
;
```

Postfix `?` requires an enclosing function with exactly one compatible optional success return slot. It cannot be combined with `catch`.

Inspect options explicitly:

```moth
label = if maybe_name is |name| then name else "guest"

if maybe_name is none:
    io.line("missing")
;

if maybe_name is:
    "Priya" => io.line("admin")
    |name| => io.line(name)
    none => io.line("guest")
;
```

An option match is exhaustive without `else =>` only when it contains both an unguarded `none` arm and an unguarded `|name|` present-value capture. Options support equality when their inner type supports equality.

## Errors, propagation and recovery

A fallible function has one final `!` return slot:

```moth
MissingName #Error = Error("Missing name", 404)

load_name |id String| -> String, Error!:
    if id is "":
        return! MissingName
    ;

    return "Priya"
;
```

Builtin `Error` exposes `message String` and `code Int = 0`. A constant may store a reusable value. Runtime construction may use dynamic text.

```moth
load_page |id String| -> String, Error!:
    name = load_name(id)!
    return [: Hello [name]]
;

name = load_name(id) catch |err|:
    io.warn(err.message)
    then "guest"
;
```

`return!` returns failure, postfix `!` propagates and `catch` recovers. A multi-success handler produces matching arity:

```moth
name, score = load_user(id) catch |err|:
    io.warn(err.message)
    then "guest", 0.0
;
```

An error-only function may fall through successfully. Custom error slots use ordinary nominal types. Postfix `!` requires an exactly compatible caller error slot. Convert error types explicitly with `catch` and `return!`.

`Error!` is not a first-class `Result`. Define a choice for explicit result-like domain values.

### Assertions

```moth
assert(index < items.length())
assert(index < items.length(), "index must be in bounds")
assert(false, "unimplemented path")
```

`assert` is statement-only and always checked. Its optional message is a `String?` expression that defaults to `none`; named and positional arguments use ordinary call rules. Failure is unrecoverable. `assert(false)` is statically terminal. A literal-`true` message is still type/generic/evidence checked but publishes no generated request or executable fact. JavaScript and HTML evaluate reachable runtime messages lazily on the failure edge; HTML-Wasm currently rejects reachable runtime construction while accepting default and fully folded messages.

## `if`, matching and value-producing blocks

Statement `if`:

```moth
if ready:
    io.line("ready")
else
    io.line("waiting")
;
```

A compile-time-known Bool can specialise this ordinary `if` after both branches are frontend-valid;
the selected lexical scope remains intact and inactive executable work does not reach HIR. Runtime
conditions remain runtime branches.

```moth
enabled #= false

if enabled:
    perform_optional_work()
;
```

There is no statement-level `else if`. Nest another `if`.

Full match:

```moth
if value is:
    < 0 => io.line("negative")
    0 => io.line("zero")
    else => io.line("positive")
;
```

Patterns include literals, relational scalars, choice variants/payloads and option captures. Guards follow the pattern:

```moth
Response ::
    Pending | retry_count Int, message String |,
    Complete,
;

response = Response::Pending(2, "offline")

if response is:
    Pending(retry_count, message as pending_message) if retry_count > 0 =>
        io.warn(pending_message)
    Complete => io.line("done")
    else =>
;
```

Payload captures list every field in declaration order. `as` renames only the local binding. Arms have no colon or individual semicolon. `else =>` is the only catch-all, `_ =>` is invalid and guarded choice matches need `else =>`.

Value-producing forms send values with `then`:

```moth
label = if ready then "ready" else "waiting"

status = "ready"
label = if status is:
    "ready" => then "ready"
    "failed" => then "failed"
    else => then "other"
;
```

They work only at closed receiving declarations, assignments, multi-binds, returns or nested `then`. They are not general call, operator, constructor, collection or template expressions. Every producing path matches the receiver's arity and types.

## Loops

Moth uses one `loop` keyword.

Conditional loop:

```moth
count ~= 0

loop count < 3:
    io.line([: [count]])
    count += 1
;
```

Collection loops may omit bindings or bind the item and optional zero-based index:

```moth
count ~= 0

loop items:
    count += 1
;

loop items |item, index|:
    io.line([: [index]: [item]])
;
```

Collection loops capture the source and its length once before iteration. They operate on Moth collections, not a general iterable protocol. Maps are not collection-loop sources.

Range loop:

```moth
loop 0 to 10 by 2 |value, index|:
    io.line([: [index]: [value]])
;
```

- `to` excludes the end. `to &` includes it.
- `loop to n` starts at `0`.
- Direction follows the bounds. `by` supplies a positive magnitude.
- A literal zero step is a diagnostic. A dynamic zero step fails before the first iteration.
- Any range using `Float` requires an explicit `by`.
- Start, end and explicit step expressions are evaluated once from left to right.
- `break` and `continue` target the nearest ordinary loop.

## Structs and receiver methods

```moth
Person = |
    name String,
    age Int = 0,
|

person ~= Person(name = "Priya", age = 30)
person.age += 1
```

Structs are nominal. Matching fields do not imply the same type or structural equality. Fields may have compile-time defaults. Constructors use normal argument routing.

A receiver method is a top-level function whose first parameter is `this`:

```moth
birthday |this ~Person|:
    this.age += 1
;

label |this Person| -> String:
    return [: [this.name], age [this.age]]
;

~person.birthday()
text = person.label()
```

`this T` is shared. `this ~T` is mutable. Mutable receiver calls need `~place.method(...)`. Source methods live in the same file as their nominal struct or choice and cannot extend types owned elsewhere. Methods remain attached to the type and are not imported separately.

Aligned generic receiver methods are supported:

```moth
Box type A = |
    value A,
|

get type A |this Box of A| -> A:
    return this.value
;
```

Methods specialised to one concrete instance are invalid.

### Runtime anonymous records - accepted deferred

```moth
point = |
    x = 10,
    y = 20,
|

x = point.x
```

Each literal site creates a distinct hidden nominal type. It supports local binding, fields, ordinary access and `copy`, but has no source-visible type name, constructor, methods or conformance. It cannot enter signatures, returns, aliases, public surfaces, named aggregate fields or trait evidence. The first implementation also rejects collection/map storage and generic arguments. Wider local storage is unsettled.

## Choices

Choices are nominal tagged unions:

```moth
Status ::
    Ready,
    Loading | progress Float |,
    Failed | message String, code Int |,
;

ready = Status::Ready
failed = Status::Failed(message = "offline", code = 503)
```

Unit variants are values without `()`. Payload variants use constructor arguments. Payload fields have no defaults and are immutable.

Pattern matching is the supported payload-access form:

```moth
if status is:
    Ready => io.line("ready")
    Loading(progress) => io.line([: [progress]%])
    Failed(message, code) => io.error([: [code]: [message]])
;
```

Direct payload field access with narrowing, nested payload patterns and recursive choices are accepted deferred work, but their complete source contracts are not yet documented. Do not infer unrestricted `status.message` access or nested pattern syntax.

Choice equality is available only when every possible payload type supports equality.

## Collections

```moth
names ~= {"Priya", "Rob"}
empty ~{Int} = {}
fixed ~{3 Int} = {10, 20}

capacity #Int = 4
scratch ~{capacity String} = {}
labels {capacity} = {"a", "b"}
```

`{T}` is growable. `{N T}` has fixed capacity `N`, which is part of type identity. Capacity is a positive literal or bare visible `#Int` constant. Empty literals need a receiving type, and an empty fixed binding must be mutable. Collections have no indexing or builtin equality.

```moth
~names.push("Emmy")
first = names.get(0) catch then "guest"
~names.set(0, "Huw") catch:
    io.error("invalid index")
;
removed = ~names.remove(1) catch then "guest"
count = names.length()
```

Accepted end state: growable `push` and `length()` are infallible. `get`, `set`, fixed `push` and `remove` are fallible. Growable allocation exhaustion traps. Fixed `push` handles full capacity with `catch` or postfix `!`. The push split is accepted deferred until its plan lands.

## Hash maps

```moth
scores ~{String = Int} = {
    "Priya" = 10,
    "Rob" = 8,
}

score = scores.get("Priya") catch then 0

~scores.set("Emmy", 12) catch:
    io.error("set failed")
;

found = scores.contains("Rob")
count = scores.length
removed = ~scores.remove("Rob") catch then 0
~scores.clear()
```

- `{Key = Value}` is a map type. `{key = value}` is a map literal.
- Empty maps need an explicit map type.
- Maps preserve insertion order. Replacing a value does not move its entry.
- Builtin keys are limited to `String`, `Int`, `Bool` and `Char`.
- `get`, `set` and `remove` are fallible.
- `contains`, read-only `length` and `clear` are infallible.
- `get` returns shared access to the stored value. The map cannot mutate while that alias remains live.
- Maps have no indexing, mutable entry APIs, fixed-capacity form, equality, builtin sets or user-defined key/hash policy.
- Maps are not collection-loop sources. Read-only map iteration remains a possible deferred follow-up with no syntax yet.

## Compile-time records and const templates

### Anonymous const records - accepted deferred

An anonymous record in a compile-time receiving context is a field-access-only const value:

```moth
metadata #= |
    channel = "alpha",
    nested = |
        enabled = true,
    |,
|

channel #= metadata.channel
```

Every field must fully fold. Const records may nest and may be exported when every reachable field is representable in the public folded-value vocabulary. The complete record is not a runtime value: it cannot be passed, returned, stored in runtime data or used through receiver methods.

A fully folded named struct constant may also act as a data-only const record.

### Const templates

A constant may store a folded template string:

```moth
site_name #= "Moth"

heading #= [$md:
    # [site_name]
]
```

A direct top-level const fragment uses `#[...]`:

```moth
#[$md:
    # Compile-time page fragment
]
```

The direct form is valid only as entry-selected top-level fragment syntax and must fully fold. It contributes page content but does not become runtime HIR.

Const control flow keeps the same template shape:

```moth
#[if show_heading:
    Visible
[else]
    Hidden
]

#[loop items |item|:
    [item]
]
```

Every required branch/body is validated. Const loops are subject to the project iteration limit.

## Type aliases

Aliases are transparent compile-time names, not new nominal types:

```moth
Box type A = |
    value A,
|

UserId as Int
Names as {String}
MaybeName as String?
StringBox as Box of String
```

`UserId` and `Int` remain interchangeable. An alias introduces no constructor. Construct a struct, choice or generic instance through its canonical nominal name.

Use a wrapper struct when distinct identity matters:

```moth
UserId = |
    value Int,
|
```

A compact primitive-backed nominal wrapper or newtype syntax remains design pending. Do not invent one.

## Generics

```moth
identity type A |value A| -> A:
    return value
;

Box type A = |
    value A,
|

Maybe type A ::
    Some | value A |,
    None,
;
```

Concrete types use `of`. Calls and constructors infer from immediate arguments and the immediate receiving type:

```moth
empty type A || -> {A}:
    return {}
;

box Box of String = Box("Moth")
value = identity(42)
items {Int} = empty()
```

There is no explicit call-site type syntax. Inference does not use later mutation, later uses or distant outer calls.

Nested inline `of` is invalid. Name the inner type:

```moth
Pair type A, B = |
    first A,
    second B,
|

StringIntPair as Pair of String, Int
value Box of StringIntPair = Box(Pair("count", 3))
```

One inline application may appear as a collection element, such as `{Box of String}`.

Unconstrained generic code may pass, return and store values, but cannot assume arithmetic, equality, fields, interpolation, IO or methods. Use trait bounds. Moth has no `where`, parameterised aliases, partial application, higher-kinded types, lifetime parameters or general const generics.

## Traits and conformance

Traits are static nominal method contracts:

```moth
DISPLAY_TEXT must:
    display |This| -> String
;

Label = |
    text String,
|

display |this Label| -> String:
    return this.text
;

Label must DISPLAY_TEXT
```

Requirement receivers use `This` or `~This`. A non-receiver `This` needs a name, such as `other This`. Concrete methods use lowercase `this`. Conformance is explicit, same-file and bodyless, with no semicolon. Traits are not value types.

Generic bounds use `is`. `and` adds bounds to one parameter:

```moth
NAMED must:
    name |This| -> String
;

render type Item is DISPLAY_TEXT and NAMED |item Item| -> String:
    return [item.name(), ": ", item.display()]
;
```

Bound calls resolve statically to concrete methods.

```moth
READABLE must:
;

WRITABLE must:
;

READABLE must not WRITABLE
```

This declares symmetric trait incompatibility. `Type must not TRAIT` is not negative conformance.

Use choices for runtime heterogeneity. Moth has no trait objects, dynamic trait dispatch, default methods, associated items, inheritance, trait aliases, generic traits, blanket conformance or specialisation. Static non-method requirements and a broader builtin trait taxonomy remain accepted deferred work without final examples here.

## Templates

Templates are `String` values:

```moth
message = [: Hello, [name].]
content = [$md:
    # Hello [name]
]
```

Static templates fold. Runtime templates lower only needed work. A direct top-level template in an entry-selected root is a page fragment. A bound/returned template is not.

| Directive | Purpose |
|---|---|
| `$slot`, `$insert` | receive and contribute content |
| `$children`, `$fresh` | wrap direct children or skip one |
| `$md`, `$raw` | Markdown or preserved body text |
| `$note`, `$todo`, `$doc` | comments/documentation |
| `$html`, `$css`, `$escape_html` | HTML-builder formatting |
| `$code("language")` | highlighted literal code |

`$literal` is accepted deferred by the resource plan but is not yet in the canonical directive page. Formatter directives do not flow into nested children. `.mtf` children default to `$md`.

Slots and wrappers:

```moth
card #= [:
    <h1>[$slot("title")]</h1>
    <section>[$slot]</section>
]

[card:
    [$insert("title"): Welcome]
    Hello, [name].
]
```

Positional slots receive loose head contributions:

```moth
image #= [: <img src="[$slot(1)]" alt="[$slot]">]
[image, "logo.png": Moth logo]
```

Missing slots render empty and repeated slots replay content. Child wrappers are explicit:

```moth
list #= [$children([:<li>[$slot]</li>]): <ul>[$slot]</ul>]
[list: [: one] [$fresh: [: two]]]
```

`$children(...)` wraps direct children only. `$fresh` skips one immediate wrapper.

Template control flow is the final head suffix:

```moth
[if maybe_name is |name|:
    Hello [name]
[else if use_fallback]
    Hello fallback
[else]
    Hello guest
]

[loop items |item, index|:
    [index]: [item]
]
```

Template `else if` is valid even though statement `else if` is not. Loops support structural `[break]` and `[continue]`. A loop-head wrapper wraps the aggregate once.

`$md` supports headings, paragraphs, lists, emphasis, links and single-backtick inline code. Links use `@./path (label)` or `@https://example.com (label)`. It has no fenced code blocks or pipe tables. Use `$code` for blocks.

## Reactivity

Reactivity V1 is a constrained source, subscription and live-template sink model, not a closure system.

```moth
count $Int = 0
ready $= false
names ${String} = {"Priya"}

count = count + 1
~names.push("Rob")
```

A plain capture is a snapshot. `$(source)` records a live read-only dependency:

```moth
snapshot = [: Count: [count]]
live_string = [: Count: [$(count)]]
```

The result remains `String`. Observable updates happen only at supported sinks. V1 supports a direct top-level HTML-JS runtime fragment:

```moth
count $Int = 0
[: Count: [$(count)]]
```

`io.line` and `assert` are not live sinks. HTML-Wasm is deferred.

A subscription accepts one bare reactive source identifier, not a field path, call or expression. It is dependency metadata, not a mutable borrow or copy.

```moth
counter_view |count $Int| -> String:
    return [: Count: [$(count)]]
;
```

Reactive parameters preserve source identity for reads/subscriptions, grant no mutation and have no defaults. Passing a source to ordinary `T` takes a snapshot. `$T` is not a wrapper type.

Field/path and item subscriptions, expression tracking, derived values, events/actions/effects, `$bind(...)`, component messages, IO sinks, fine-grained DOM updates and keyed diffing remain design-incomplete.

## `.mtf` and `.md` content files

A `.mtf` file is the body of an implicit compile-time `$md` template. It exposes one generated constant, `content #String`.

Bind it extensionlessly:

```moth
@docs/intro content as intro_content

page = [: [intro_content]]
```

A `.mtf` file has no declarations, dependency clauses, frontmatter or runtime scope. It may use nested templates and a restricted compile-time scope supplied by its same-directory module root and the HTML builder.

A plain `.md` file also exposes `content #String`, but has no Moth scope, interpolation or templates. It uses the HTML builder's CommonMark-compatible Markdown renderer with GFM extensions. Its links, images and raw HTML remain literal.

Use:

- `.moth` for code, composition, declarations and pages
- `.mtf` for Moth-aware Markdown-first content
- `.md` for plain Markdown content

## Dependencies and aliases

There is no `import` keyword.

```moth
@core/math sin, cos, PI
@components render as render_component, Button as UiButton

@core/text as text
@vendor/drawing.js as drawing
```

Direct selections create file-local names. Entry-level `as` aliases one selection. Clause-level `as` aliases the whole namespace and cannot combine with selections.

The path and first selection share a physical line. A comma continues the clause. A trailing comma is invalid:

```moth
@core/math sin,
    cos
```

Source namespaces are shallow field-access-only compile-time bindings, not runtime values. Binding-backed packages may expose nested namespaces such as `io.input`.

Path rules:

- resolution starts at the declaring file's owning module root, not its directory
- `.moth`, `.mtf` and `.md` dependencies omit the extension
- provider files such as annotated `.js` keep it
- `@./`, `@/`, parent traversal and `@@` are invalid
- child modules and support packages stop traversal and expose only `export:`
- `@core/math/sin` is invalid. Select `sin` after `@core/math`
- one clause resolves one provider. Selections are flat names
- source clauses bind registered roots and never acquire packages

Aliases cannot shadow another visible name.

## Resource paths and `Path` - accepted deferred

An explicit non-source extension in expression position creates a compile-time `Path`:

```moth
logo #= @assets/logo.svg
font #Path = @assets/fonts/site.woff2
icons #{Path} = {@assets/add.svg, @assets/remove.svg}
```

The path resolves from the owning module root to an existing regular file. It cannot use `@/`, `@./`, parent components or `@@`, and cannot be followed by selections. Source extensions such as `.moth`, `.mtf` and `.md` remain dependencies.

V1 `Path` is compile-time-only. It may appear in constants, const records, compile-time collections, exports and templates. It is rejected in runtime bindings, signatures, runtime aggregates, options, choices, maps, generic applications, casts and comparisons.

```moth
logo #Path = @assets/logo.svg

image #= [$html:
    <img src="[logo]" alt="Moth">
]
```

The builder keeps the resource structural until output placement. In `.mtf`, use nested Moth template syntax such as `[@images/ownership.webp]`. Plain Markdown paths stay text.

External, protocol-relative and site-root URLs are ordinary untracked strings. Extensionless resource escapes, resource roots outside `entry_root` and any wider Path-containing type surface remain unsettled.

## Modules, exports and project-local packages

A module is rooted by one directory-scoped `@*.moth` or `+*.moth`. The suffix is cosmetic.

```text
project/
├── config.moth
├── +package.moth
└── src/
    ├── @site.moth
    ├── helpers.moth
    ├── shared/+package.moth
    └── pages/@page.moth
```

A normal `@` root may contain declarations, one `export:`, entry `config:`, top-level runtime work and page fragments. Top-level work becomes dormant `start` code and runs once only when that module is selected as an entry. Depending on a module never runs it. `start` has no `Error!` slot, so top-level fallible work uses `catch`.

`export:` is the only cross-module public marker:

```moth
private_helper || -> String:
    return "private"
;

export:
    public_name #= "Moth"

    render || -> String:
        return private_helper()
    ;

    @components/card CardData as Card, render_card
;
```

Items outside remain private. Public surfaces cannot expose private types, traits or evidence. Receiver methods become visible with their exported type.

A source-tree `+*.moth` creates an API-only scoped support package named by its directory, such as `@shared`. It has no `start`, page fragment or route and is the mechanism for sharing declarations between normal siblings. Direct normal-sibling dependencies remain invalid unless a future design explicitly changes this.

A project-root `+*.moth` beside `config.moth` is the optional external package facade. It is API-only, not visible internally and cannot expose public or reachable dependence on private `@project` values.

Registered external packages use ordinary file clauses:

```moth
@acme/ui Button, theme
@community/markdown as markdown
```

Only direct dependencies are source-visible. Project-level declaration, version and alias syntax is design-gated. The old `import @package` proposal is invalid.

## Project configuration - accepted deferred

`config.moth` is one self-contained compile-time file, not a module:

```moth
project #= |
    name = "moth_docs",
    version #Config of String = "0.1.0",
    entry_root = "src",
|

html #= |
    dev_output = "dev",
    release_output = "release",
|
```

The command selects the builder first. One `project` record is required. `project.name` gives stable identity. Earlier helper constants may feed later fields. Other top-level records are builder/tooling sections. Config has no runtime declarations, functions, named support types, dependencies, fragments or `export:`.

Build configuration values use primitive or optional types:

```moth
api_url #Config of String = "http://localhost:8080"
optional_label #Config of String? = none
```

A source default is one literal or `none`, not a name, template, call, cast, operator, collection or record. Pass explicit values with `--input name=value`.

CLI inference is immediate: `true`/`false` -> `Bool`, a whole number -> `Int`, a decimal or exponent -> `Float`, `'c'` -> `Char`, a quoted literal -> `String`, and anything else -> `String`. Use an explicitly quoted String for `"true"` or `"42"`; omit an optional input to resolve to `none`. Contracts require exact types, except a present `T` may satisfy `T?`; no other coercion occurs.

```bash
moth build . --input analytics=true
moth build . --input retries=4
moth build . --input ratio=0.75
moth build . --input api_url=https://example.com
moth build . --input 'label="true"'
moth build . --input "separator=':'"
```

Project fields enter source through explicit `@project`:

```moth
@project version
label = [: Version [version]]
```

`@project` is never implicit or directly re-exported. Each dependency has its own config and inputs.

One entry `config:` block may appear only at a normal root's top level:

```moth
favicon_url #= [@assets/favicon.svg]

config:
    html #= |
        title = "Moth docs",
        favicon = favicon_url,
    |
;
```

It contains section records, uses ordinary compile-time visibility and creates no symbol/runtime value. Dependencies, helpers, types and `#Config` declarations stay outside. Project and entry schemas do not merge.

## Builders, targets and commands

One command selects one artefact builder, one profile and any tooling overlays. The compiler frontend remains backend-neutral. Builders provide config schemas, packages, directives, source kinds, entry policy, capabilities and output policy.

The HTML builder turns selected normal roots into routes and documents. Mixed HTML output assigns reachable functions to JavaScript or Wasm automatically. Source has no target-selection annotations, platform/backend query values or target-conditioned source; the builder interprets stable semantics and selects physical targets.

`moth check` performs the same reachable target validation as a build without writing artefacts.

```bash
moth new html my-site
moth dev .
moth check .
moth build . --release
```

Final builder-selection syntax and any Moth-native build-script system remain design pending. Stable builder capabilities may be used without revealing target identity.

## Core and external packages

The prelude exposes `io` as `@core/io`. Console helpers accept exactly one `String`:

```moth
io.line("Hello")
io.line([: count = [count]])
```

`io.set_title` is accepted deferred for document-capable JavaScript hosts.

Annotated project-local JavaScript exposes free functions and optional opaque types:

```js
/**
 * @moth.sig emphasize |text String| -> String
 */
export const emphasize = (text) => {
    return `**${text}**`;
};
```

```moth
@vendor/format.js emphasize
label = emphasize("Moth")
```

JavaScript constants, receiver methods, callbacks, async functions, options, collections, generics and multiple success returns are rejected. Host code receives ordinary Moth values by value and cannot retain references into Moth storage.

WIT V1 is also value-only. Resources, callbacks, async, futures, streams, shared-memory views, raw pointers and returned/retained aliases remain deferred V1 gaps.

## No stable syntax yet

Do not infer forms for:

- project dependency declarations, versions, lockfiles, registry/local/Git sources
- named non-capturing function references or primitive-backed newtypes
- async scopes, coroutines, channels or suspension
- reactive paths, derived values, events/effects, `$bind` or component messages
- direct choice payload access/narrowing or nested payload patterns
- lossy `Float <-> NumberN`, Number formatting helpers or broader Byte APIs
- group capacity, expression placement, extraction, adoption or group transfer
- broader anonymous-record storage
- final builder-selection or build-script syntax

Roadmaps and design drafts do not create accepted syntax.

## Deliberate language limits

Do not invent shadowing, macros, general closures/function values, exceptions, catchable panic, first-class public `Result`, operator overloading, trait objects/dynamic dispatch, associated items, broad conformance, reflection/type values, explicit reference/lifetime/move syntax, source RC, parameterised aliases, higher-kinded types, general const generics, cross-owner receiver extensions, wildcard dependencies/exports, extensible builtin maps, string `+` or `_` match arms.

Outside scope also includes target/platform conditional compilation, conditionally present imports,
declarations or exports, builder-provided target identity flags and backend-specific source legality.

Use choices for runtime heterogeneity, trait bounds for static reuse, `copy` for independent storage, typed error channels for expected failure and accepted reactive subscriptions for live template reads.

Further detail: [language](../language-overview/), [memory](../memory/), [templates](../templates/), [projects](../project-structure/), [packages](../packages/), [progress](../progress/) and [design scope](../design-scope/).
