# Moth language cheatsheet

Moth is a small, statically typed language with first-class string templates, mandatory borrow and lifetime validation and a backend-neutral compiler frontend. This document is a compact end-state reference for writing Moth code. It describes source syntax and project structure, not compiler internals or implementation status.

Detailed explanations and tutorials live on the [Moth documentation site](https://nyejames.github.io/moth/docs/).

## Rules to internalise first

- `:` opens a block. `;` closes it. Semicolons do not end statements.
- Bindings are immutable unless the declaration uses `~`.
- Existing values use shared read-only reference semantics by default. Binding a value to another name does not copy it.
- `~place` requests exclusive mutable access. It is not a type, reference operator or move marker.
- `copy place` makes an independent deep copy. Moth has no explicit move syntax or lifetime annotations.
- `[]` creates strings and templates. `{}` creates collections or maps.
- `!` is the typed error path. `?` is the option path. `assert(...)` is the only panic-like invariant failure.
- `@path` binds a dependency surface. An explicit non-source extension in expression position creates a compile-time `Path` value.
- Names never shadow another visible name.
- Moth has no general closures, function values, macros, exceptions, trait objects or operator overloading.

## Common invalid translations

| Do not write | Write |
|---|---|
| `import @core/math` | `@core/math` |
| `left == right` | `left is right` |
| `left != right` | `left is not right` |
| `!ready`, `a && b`, `a || b` | `not ready`, `a and b`, `a or b` |
| `{ ... }` for a code block | `: ... ;` |
| `statement;` | `statement` |
| `left + right` for strings | `[left, right]` |
| `&value`, `&mut value`, `move value` | shared access, `~value`, or `copy value` |
| `_ => fallback` | `else => fallback` |
| `items[index]` | `items.get(index)!` or `~items.set(index, value)!` |
| `Box<String>` | `Box of String` |
| an inline closure | a named function, trait pattern or reactive source/subscription |

## Blocks, comments and names

```moth
if ready:
    io.line("ready")
else
    io.line("waiting")
;
```

`--` starts a single-line comment in ordinary Moth code. Inside template and `.mtf` bodies, `--` is ordinary output text.

Naming conventions:

- Types, structs, choices, aliases and generic parameters: `PascalCase`
- Variables and functions: `regular_snake_case`
- Traits: `ALL_CAPS`
- No visible name may be redeclared while it remains in scope.

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

Quoted strings cannot continue across a physical newline and support only these escapes: `\\`, `\"`, `\n`, `\r` and `\t`. Backticks are not raw source strings. They delimit inline code inside `$md` content.

`String + String` is invalid. Concatenate with a template:

```moth
joined = [left, right]
```

String equality compares content with `is` and `is not`. Strings do not support ordering operators.

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

Mutability belongs to a binding or access operation, not to type identity. `~{String}` means a mutable binding whose type is `{String}`. It is not a separate mutable collection type.

A constant:

- must have an initialiser
- cannot be mutable
- may depend only on other compile-time values
- may reference only earlier constants in the same file
- must fully fold during compilation

`#` controls compile-time evaluation, not public visibility. Cross-module visibility comes only from `export:`.

## Reference semantics, copying and ownership

Existing values are shared by reference by default:

```moth
items ~= {"Priya", "Rob"}
shared_items = items

count = shared_items.length()
~items.push("Emmy") -- valid because shared_items is not used again
```

A shared alias is read-only. It blocks overlapping mutation only while it may still be used on that control-flow path. The compiler tracks this non-lexically.

A mutable declaration made from an existing place creates a write-through exclusive alias. A mutable declaration made from a fresh expression creates an independent mutable slot:

```moth
items ~= {"Priya"}
writer ~= items
~writer.push("Rob") -- writes the same collection

fresh ~= {"Emmy"}  -- independent fresh collection
```

Putting an existing value into a struct, choice, collection, map or another aggregate keeps the same shared-reference rule unless the source is copied or transferred at a proven final use.

Use `copy` for independent storage:

```moth
independent = copy items
```

`copy` accepts a visible binding, field projection or parenthesised place. It deep-copies the complete copyable value graph, preserves internal alias topology and shares no mutable allocation with the source. Non-copyable external resources are rejected.

Moth has no source move operator. At a proven final use, the compiler may transfer destruction responsibility instead of borrowing. This is an optimisation and never changes source meaning. Failure to prove a transfer does not reject otherwise valid code.

The compiler always validates:

- shared versus exclusive access
- retained aliases and escapes
- lifetime-region topology
- result aliasing and freshness
- destruction responsibility where ownership optimisation applies

These checks apply even when a backend uses garbage collection. GC changes representation, not source legality. A function may return an alias or projection only when the referenced storage remains valid for every caller use. Otherwise return fresh storage or `copy` it. There are no source `&`, `&mut`, lifetime parameters, retain/release calls or weak references.

### Declared memory groups

Use a declared group when a set of allocations should share an explicit hard lifetime:

```moth
group request:
    parsed ParsedPost into request = parse_post(post)
    html String into request = render_post(parsed)
;
```

`into group_name` appears on a declaration after access or type syntax and before `=`:

```moth
rows ~{Row} into scratch = {}
```

A group is not a value, type, allocator object or signature parameter. Values and aliases cannot outlive it. A child may retain a parent value, but a parent or sibling cannot retain a child value. Placement into an ancestor group is limited to straight-line nested `block:` or `group:` code that runs at most once. There is no expression-site placement, extraction or unrestricted group-to-group adoption.


## Numbers and operators

### Numeric types

- `Int`: signed 32-bit integer
- `Float`: finite IEEE-754 `f64`. `NaN` and infinities are never valid Moth values
- `Number` and `Number0`: arbitrary-precision integer with scale 0
- `Number1` through `Number256`: fixed-scale arbitrary-precision decimals
- `Byte`: unsigned `0..255` scalar for byte-oriented APIs. Ordinary arithmetic is not defined on `Byte`

```moth
count Int = 42
ratio Float = 0.5
large Number = 1000000000000000000000
price Number2 = 12.50
byte Byte = 255
```

Whole literals naturally infer `Int`. Decimal literals naturally infer `Float`. A receiving `NumberN` context materialises an exact fixed-scale value. Moth never silently rounds a literal, so `price Number2 = 1.239` is invalid.

Exponents use lowercase `e`: `1e6`, `1e-6`, `1.0e+21`. Uppercase `E` and unary `+` are invalid. Negation must be attached: `-1`, `-count`, not `- count`.

### Arithmetic

- `Int / Int -> Float`
- `Int // Int -> Int`, truncating toward zero
- Mixed `Int` and `Float` arithmetic produces `Float`
- `NumberN` combines with the same `NumberN` scale or with `Int`
- Different `Number` scales require an explicit cast
- `NumberN` and `Float` do not mix implicitly
- Positive-scale `NumberN` multiplication, division and exponentiation round half to even at the operation boundary
- Scale-zero `Number` uses `//` and `%` for integer division and remainder. `/` is invalid at scale 0
- Positive-scale `NumberN` uses `/` and `%`. `//` is invalid
- `NumberN ^ Int` requires a non-negative exponent
- `^` is right-associative

All numeric operations are checked. Overflow, divide-by-zero, invalid exponents, non-finite Float results and invalid fixed-scale operations fail. A statically known failure is a compile-time diagnostic. In a function whose final error slot is builtin `Error!`, supported checked failures enter that channel. Otherwise they trap as unrecoverable runtime failures.

Operator precedence, highest first: unary `not` and `-`, `^`, `* / // %`, `+ -`, comparisons, `and`, `or`.

Equality and logic use words:

```moth
same = left is right
different = left is not right
ready = has_input and is_valid
retry = timed_out or disconnected
blocked = not ready
```

## Explicit casts

`cast` converts to the builtin target supplied by the immediate typed receiving boundary:

```moth
count Int = cast! text
fallback Int = cast text catch then 0
label String = cast value
```

Forms:

```moth
value Target = cast expression          -- proven infallible
value Target = cast! expression         -- propagate cast failure
value Target = cast expression catch:   -- recover locally
    then fallback
;
```

Rules:

- Cast targets are compiler-owned builtins such as `Bool`, `Int`, `Float`, `NumberN`, `Char`, `String` and `Error`.
- `cast!` requires a compatible `Error!` channel in the current function.
- `cast ... catch` handles only cast failure.
- `cast! ... catch` is invalid.
- Same-type casts are invalid.
- Generic inference does not look through `cast`.
- Scalar conversion constructors such as `Int(value)` and `String(value)` are invalid.
- `NumberN` scale widening is exact and infallible. Scale narrowing is exact-or-fail and never rounds.
- `Float` and `NumberN` lossy conversion uses named library helpers, not `cast`.
- Numeric text casts consume the whole string. They reject surrounding whitespace, uppercase `E`, `NaN` and infinity spellings.
- `NumberN -> String` uses canonical decimal text with no exponent, negative zero or redundant trailing fractional zeroes.
- Same-file nominal types may provide compiler-owned cast evidence for supported builtin targets. For example, `CASTABLE_TO_STRING` expects `to_string |this Type| -> String`, while a fallible cast trait expects an `Error!` method. Users cannot create new cast target families.


## Functions and calls

```moth
greet |name String, punctuation String = "!"| -> String:
    return [: Hello [name][punctuation]]
;

message = greet("Priya")
custom = greet(name = "Rob", punctuation = "?")
```

- Parameters use `|...|`. A no-parameter function uses `||`.
- Omit `->` when there is no success return value.
- Defaults must fold at compile time.
- Positional arguments must come before named arguments.
- Each parameter may be supplied once.
- Named arguments may skip earlier defaulted parameters.
- Binding-backed and compiler builtin calls are positional unless their package contract says otherwise.

### Mutable parameters and calls

```moth
append |items ~{String}, value String|:
    ~items.push(value)
;

names ~{String} = {}
append(~names, "Priya")
append({"Rob"}, "Emmy") -- fresh rvalue needs no ~
```

An existing mutable place passed to a mutable parameter requires `~place`. A fresh literal, template, constructor call or computed result may fill the mutable slot without `~`. `~` is invalid on immutable places and fresh rvalues.

### Multiple returns

```moth
pair || -> String, Int:
    return "Priya", 2
;

name, count = pair()
```

Multiple returns are not tuple values. They may be received only by a matching multi-bind, return or value-producing block.

Moth functions are named declarations, not first-class values. There is no closure literal, callback value or generic higher-order function surface.


## Options

`T?` is an optional value. `none` needs an optional receiving context.

```moth
find_name |id String| -> String?:
    if id is "":
        return none
    ;

    return "Priya"
;
```

A `T` value may be used where `T?` is expected. Postfix `?` unwraps a present value or returns `none` from the current optional function:

```moth
load_label |id String| -> String?:
    name = find_name(id)?
    return [: User: [name]]
;
```

Inspect an option explicitly:

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

A complete option match needs an unguarded `none` arm and an unguarded present-value capture, or an `else =>` arm. Options support equality when their inner type does.

## Errors and recovery

A fallible function marks one final return slot with `!`:

```moth
MissingName #Error = Error("Missing name", 404)

load_name |id String| -> String, Error!:
    if id is "":
        return! MissingName
    ;

    return "Priya"
;
```

`Error` is a reserved compiler-owned structured value. Its physical representation is opaque. Source code observes only:

- `message String`
- `code Int`, defaulting to `0`

A constant `Error` construction declares a reusable static error descriptor. Runtime construction may include dynamic message content:

```moth
return! Error([: Unknown user [id]], 404)
```

Use normal success `return`, `return!` for the error path, postfix `!` to propagate and `catch` to recover:

```moth
load_page |id String| -> String, Error!:
    name = load_name(id)!
    return [: Hello [name]]
;

name = load_name("Priya") catch then "guest"

name = load_name(id) catch |err|:
    io.warn(err.message)
    then "guest"
;
```

An error-only function may fall through successfully:

```moth
validate |ready Bool| -> Error!:
    if not ready:
        return! Error("not ready")
    ;
;
```

Custom error channels use ordinary nominal types:

```moth
ValidationFailure = |
    message String,
|

validate_name |name String| -> ValidationFailure!:
    if name is "":
        return! ValidationFailure("empty name")
    ;
;
```

Postfix `!` requires the caller's error type to match exactly. Convert between error types explicitly with `catch` and `return!`.

`Error!` is not a first-class `Result` value. It cannot be stored or pattern-matched as a result carrier. Define a normal choice when the domain needs an explicit result-like value.

### Assertions and panic behaviour

Moth has no general `panic` or exception syntax. `assert` is the only invariant-failure surface:

```moth
assert(index < items.length())
assert(index < items.length(), "index must be in bounds")
assert(false, "unimplemented path")
```

`assert` is statement-only. Assertions are always checked in development and release builds. The optional message must be one quoted string literal. Failure is unrecoverable and cannot be caught or propagated as `Error!`. `assert(false)` is statically terminal. Expected failures belong in a typed error channel.


## `if`, matching and value-producing blocks

### Statement `if`

```moth
if ready:
    io.line("ready")
else
    io.line("waiting")
;
```

Statement `if` is not required to be exhaustive. It has no statement-level `else if`. Nest another `if` when needed.

### Full pattern match

```moth
if value is:
    < 0 => io.line("negative")
    0 => io.line("zero")
    <= 10 => io.line("small")
    else => io.line("large")
;
```

Patterns include literals, relational scalar patterns, choice variants, choice payload captures and option captures. Add a guard with `pattern if condition => body`.

Match arms have no trailing semicolons. `else =>` is the only catch-all. `_ =>` is invalid. A statement match may use a bodyless `else =>` as an explicit no-op. Choice matches must cover every variant or include `else =>`. Any guarded choice match needs `else =>`.

### Value-producing blocks

Compact form:

```moth
label = if ready then "ready" else "waiting"
name = if maybe_name is |value| then value else "guest"
```

Block form:

```moth
label = if ready:
    then "ready"
else
    then "waiting"
;
```

Full match:

```moth
label = if status is:
    Ready => then "ready"
    Failed(message) => then message
;
```

`then` sends values to the nearest active receiving declaration, assignment, multi-bind, return or enclosing `then`. Value-producing `if`, match and block-form `catch` are closed receiving constructs, not general expressions. Do not place them directly inside function arguments, operators, constructors, collection items or template interpolation.

Every producing path must provide the receiver's value count and types.


## Loops

Moth uses one loop keyword:

```moth
loop condition:
    work()
;

loop items |item, index|:
    io.line([: [index]: [item]])
;

loop 0 to 10 by 2 |value|:
    io.line([value])
;
```

- `loop condition` repeats while a `Bool` is true.
- `loop collection |item, index|` iterates values with an optional zero-based index.
- `loop start to end by step |value, index|` iterates a range.
- `to` excludes the end. `to &` includes it.
- `loop to n` means `loop 0 to n`.
- Direction follows the bounds. Descending ranges apply a negative direction automatically.
- `by 0` is invalid. Float ranges should state `by` explicitly.
- `break` and `continue` target the nearest ordinary loop.


## Structs and receiver methods

Structs are nominal runtime types:

```moth
Person = |
    name String,
    age Int = 0,
|

person ~= Person(name = "Priya", age = 30)
person.age += 1
```

Matching field shapes do not make two structs the same type, and structs do not gain automatic structural equality. Fields may have compile-time defaults. Constructors accept positional or named arguments under normal call rules.

A receiver method is a top-level function whose first parameter is named `this`:

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

- `this T` is shared read-only access.
- `this ~T` is mutable access.
- A mutable receiver call requires `~place.method(...)`.
- Source-authored receiver methods must live in the same file as their nominal struct or choice.
- Methods cannot extend builtins, imports, dependency types, opaque host types or types from another file.
- Methods remain attached to the receiver type. They are not imported or re-exported as separate functions.

### Runtime anonymous records

```moth
point = |
    x = 10,
    y = 20,
|

x = point.x
```

Each literal site creates a distinct hidden nominal type. Identical field shapes at two sites do not unify. Anonymous runtime records support local binding, field reads, shared access, mutation through a mutable place and `copy` through ordinary struct rules.

They have no source-visible type name, constructor, methods or conformance. They cannot appear in signatures, returns, aliases, exported surfaces, struct or choice fields, collections, maps or generic arguments.


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

Unit variants use `Choice::Variant`. Payload variants use constructor arguments. Payload fields are immutable and are read through pattern matching:

```moth
if status is:
    Ready => io.line("ready")
    Loading(progress) => io.line([: [progress]%])
    Failed(message, code) => io.error([: [code]: [message]])
;
```

Choice payload fields have no defaults and cannot be mutated. Payload capture names match declared field names unless renamed with `as`. Choice equality is available only when every payload type supports equality.


## Collections

Collections are ordered, zero-indexed and homogeneous.

```moth
names ~= {"Priya", "Rob"}  -- inferred {String}
empty ~{Int} = {}            -- empty literal needs a type
fixed ~{3 Int} = {10, 20}    -- exact capacity 3

capacity #Int = 4
scratch ~{capacity String} = {}
labels {capacity} = {"a", "b"} -- declaration-only capacity shorthand
```

- `{T}` is a growable collection type.
- `{N T}` is a fixed-capacity collection type. Capacity is part of type identity.
- A fixed capacity is a positive literal or a bare visible `#Int` constant.
- An empty fixed collection binding must be mutable. An immutable empty fixed binding is invalid.
- A non-empty literal infers its element type. An empty literal needs an explicit receiving type.
- `{T}`, `{4 T}` and `{8 T}` are incompatible types.
- There is no indexing syntax or builtin collection equality.

Operations:

```moth
~names.push("Emmy") -- growable push is infallible and returns no value

first = names.get(0) catch then "guest"

~names.set(0, "Huw") catch:
    io.error("invalid index")
;

removed = ~names.remove(1) catch then "guest"
count = names.length()
```

For a fixed collection, `push` is fallible only because capacity may be full:

```moth
~fixed.push(30) catch:
    io.warn("fixed collection is full")
;
```

`get` returns shared access to the item. `get`, `set`, fixed `push` and `remove` are fallible. `set` replaces an existing item, `push` appends and `remove` shifts later items down. `length()` reports the current logical length. Growable `push` and `length` are infallible. Growable allocation exhaustion is unrecoverable rather than an `Error!` result.

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
- Empty map literals need an explicit map type.
- Maps preserve insertion order. Replacing a value does not move its entry.
- Builtin keys are limited to `String`, `Int`, `Bool` and `Char`.
- `get`, `set` and `remove` are fallible.
- `contains`, read-only `length` and `clear` are infallible.
- `get` returns shared access to the stored value. The map cannot be mutated while that alias remains live.
- There are no map indexes, mutable entry APIs, fixed maps, map equality, builtin sets or user-defined hashers and key types.


## Compile-time records and const templates

An anonymous record in a compile-time receiving context is a field-access-only const record:

```moth
metadata #= |
    channel = "alpha",
    nested = |
        enabled = true,
    |,
|

channel #= metadata.channel
```

Every field must fully fold. A fully folded named struct constant may also act as a const record. Const records can be nested and exported when their full value is public and representable. The complete record is not a runtime value: it cannot be passed, returned, stored in runtime data or used through methods.

A direct top-level const template uses `#[...]` and must fully fold:

```moth
#[$md:
    # Compile-time page fragment
]
```

## Type aliases

Aliases are transparent compile-time names, not new nominal types:

```moth
UserId as Int
Names as {String}
MaybeName as String?
StringBox as Box of String
```

`UserId` and `Int` remain interchangeable. Construct a struct, choice or generic instance through its canonical nominal constructor, not through the alias name.

## Generics

Generics are declared on top-level functions, structs and choices:

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

Concrete types use `of`:

```moth
box Box of String = Box("Moth")
```

Function type arguments are inferred from immediate arguments and, at a closed receiving site, the expected result type:

```moth
empty type A || -> {A}:
    return {}
;

value = identity(42)
items {Int} = empty()
```

There is no explicit call-site type argument syntax. Generic inference does not use later mutation, later uses or distant outer call context. Add an ordinary type annotation when evidence is insufficient.

Use a concrete alias to avoid nested `of` applications:

```moth
Pair type A, B = |
    first A,
    second B,
|

StringIntPair as Pair of String, Int
value Box of StringIntPair = Box(Pair("count", 3))
```

Moth has no `where` clauses, generic receiver methods, parameterised aliases, partial type application, higher-kinded types, lifetime parameters or user const generics beyond fixed collection capacity.

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

- Trait requirement receivers use direct `This` or `~This`. Composed forms such as `This?` and `{This}` are invalid.
- Concrete receiver methods use lowercase `this`.
- Conformance is explicit. Matching method shapes alone do not conform.
- User conformance belongs in the same file as the nominal target type.
- A conformance declaration is bodyless and has no trailing semicolon.
- Traits are not runtime value types.

Generic bounds use `is`:

```moth
NAMED must:
    name |This| -> String
;

render type Item is DISPLAY_TEXT and NAMED |item Item| -> String:
    return [item.name(), ": ", item.display()]
;
```

Commas separate generic parameters. `and` adds bounds to one parameter. Bound calls resolve statically to concrete methods.

Trait incompatibility metadata uses:

```moth
READABLE must not WRITABLE
```

Use a choice for runtime heterogeneity. Moth has no trait objects, dynamic dispatch through trait values, default methods, associated items, inheritance, trait aliases, generic traits, blanket conformance or specialisation.


## Templates

A template is a first-class `String` value:

```moth
name = "Priya"
message = [: Hello, [name].]
```

The head comes before `:`. The body continues until `]`:

```moth
[$md:
    # Hello [name]
]
```

Templates capture surrounding values. Fully static templates fold to strings at compile time. Runtime templates lower to only the string construction and control flow they need.

A direct top-level template in an entry-selected normal module root contributes a page fragment. A template assigned to a binding or returned from a function does not become a page fragment by itself.

Literal `[` or `]` output can be inserted through a normal quoted string:

```moth
[: ["[literal]"]]
```

### Directives

| Directive | Use |
|---|---|
| `$slot` | Declare a default, named or positional content slot |
| `$insert(...)` | Contribute to a named slot |
| `$children(...)` | Wrap each direct child |
| `$fresh` | Skip the immediate parent's child wrapper |
| `$md` | Apply Moth's small Markdown flavour |
| `$raw` | Preserve authored whitespace |
| `$literal` | Treat the body as literal text with no nested template syntax |
| `$note`, `$todo` | Discard comment content |
| `$doc` | Documentation template |
| `$html` | HTML-builder raw HTML |
| `$css` | HTML-builder CSS checking |
| `$escape_html` | HTML escaping |
| `$code("language")` | Code highlighting |

Frontend directives are always available. A selected builder may register more `$name` directives.

### Slots and child wrappers

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

Slots may be default, named or positional. Missing slots render as empty strings. Repeated slots replay the same content.

```moth
list #= [$children([:<li>[$slot]</li>]):
    <ul>[$slot]</ul>
]

[list:
    [: one]
    [$fresh: [: two]]
]
```

`$children(...)` affects direct children only. `$fresh` opts one child out of its immediate wrapper.

### Template control flow

`if` or `loop` may be the final head suffix:

```moth
[if show:
    Visible
]

[card, if show:
    Visible inside card
]

[if maybe_name is |name|:
    Hello [name]
[else]
    Hello guest
]

[loop items |item, index|:
    [index]: [item]
]
```

Template loops support structural `[break]` and `[continue]`. A shared helper or wrapper around a loop applies once to the complete aggregate. Put per-item wrappers inside the loop body.

### Moth Markdown

`$md` supports headings, paragraphs, ordered and unordered lists, emphasis, links and paired single-backtick inline code. It is intentionally smaller than CommonMark and does not provide fenced code blocks or pipe tables.

Moth Markdown links use:

```text
@./relative-page (label)
@https://example.com (external label)
```

Use `$code("moth")` for highlighted code blocks.


## Reactivity

Reactivity is a constrained source-and-subscription system for templates and UI sinks. It is not a closure or general function-value system.

Declare stable reactive storage with `$Type` or `$=`:

```moth
count $Int = 0
ready $= false
names ${String} = {"Priya"}
```

Assignment updates the same source and invalidates subscribers:

```moth
count = count + 1
~names.push("Rob")
```

A plain template read is a snapshot. `$(source)` is a live read-only subscription:

```moth
snapshot = [: Count: [count]]
live = [: Count: [$(count)]]
```

A subscription accepts exactly one bare reactive source identifier in a template head or capture position, not a field path, call or computed expression. It does not capture a mutable borrow or copy the value.

Reactive parameters receive a read/subscription handle to an existing source. They do not grant mutation and cannot have defaults:

```moth
counter |count $Int| -> String:
    return [: Count: [$(count)]]
;
```

Reactive syntax is storage and access metadata. `$Int` is not a wrapper type and semantic type identity remains `Int`.


## Resource paths and `Path`

An explicit non-source extension in expression position creates a compile-time `Path`:

```moth
logo #= @assets/logo.svg
font #Path = @assets/fonts/site.woff2
icons #{Path} = {@assets/add.svg, @assets/remove.svg}
```

A resource path:

- resolves from the owning module root
- names an existing regular file inside that module or package's owned tree
- has an explicit non-source extension
- cannot use `@/`, `@./`, `..` or `@@`
- is not an external URL

`Path` is compile-time-only. It can appear in constants, const records, compile-time collections and templates. It cannot appear in runtime bindings, function signatures, runtime aggregates, options, maps, generic applications, casts, comparisons or project config values. In a normal module's entry config, pass a resource-bearing template `String` when a builder field accepts a resource URL.

Insert a path directly into a template. The builder keeps it structural until it knows the final output location:

```moth
logo #Path = @assets/logo.svg

image #= [$html:
    <img src="[logo]" alt="Moth">
]
```

External, protocol-relative and site-root URLs are ordinary untracked strings:

```moth
external #= "https://example.com/logo.svg"
cdn #= "//cdn.example.com/app.js"
site_root #= "/favicon.svg"
```

## `.mtf` and `.md` content files

A `.mtf` file is the body of an implicit compile-time `$md` template. It exposes exactly one constant, `content #String`.

Bind it with an extensionless dependency clause:

```moth
@docs/intro content as intro_content

page = [: [intro_content]]
```

A `.mtf` file has no declarations, dependency clauses, frontmatter or runtime scope. It may use nested templates and a restricted compile-time scope supplied by its same-directory module root and the HTML builder.

A plain `.md` file also exposes `content #String`, but it has no Moth scope, interpolation or templates. Its Markdown links and raw HTML remain literal.

Use:

- `.moth` for code, composition, declarations and pages
- `.mtf` for Moth-aware Markdown-first content
- `.md` for plain Markdown content

## Dependencies and aliases

Moth uses dependency clauses directly. There is no `import` keyword.

### Direct selections

```moth
@core/math sin, cos, PI
@components render as render_component, Button as UiButton, Card
```

Each selected declaration becomes a file-local name. `as` creates a local alias.

The path and first selected name must share a physical line. A comma explicitly continues onto the next line, and a trailing comma is invalid:

```moth
@core/math sin,
    cos
```

### Namespace dependency

```moth
@core/math
@vendor/drawing.js as drawing

value = math.sin(math.PI)
drawing.draw()
```

A source namespace is a shallow field-access-only namespace, not a first-class value. Binding-backed packages may expose nested package-local namespaces such as `io.input`.

A clause-level `as` aliases the whole namespace and cannot be combined with direct selections.

### Path rules

- Source dependencies resolve from the declaring file's owning module root, not the file's physical directory.
- `.moth`, `.mtf` and `.md` source paths omit the extension.
- Provider files such as annotated `.js` keep their extension.
- `@./`, `@/`, parent traversal and `@@` are invalid.
- A path may traverse ordinary directories owned by the same module.
- Reaching a child module or support package ends traversal and exposes only its `export:` surface.
- Direct symbol paths such as `@core/math/sin` are invalid. Select `sin` after `@core/math`.
- Source clauses bind already registered package roots. They never acquire an undeclared external package.

Dependency aliases are file-local and cannot shadow another visible name.


## Modules, exports and project-local packages

A module is a directory-scoped compilation and visibility unit rooted by one `@*.moth` or `+*.moth` file. The suffix after the marker is cosmetic.

```text
project/
├── config.moth
├── +package.moth              optional external project package facade
└── src/
    ├── @site.moth             normal root for src/
    ├── helpers.moth           ordinary file in the same module
    ├── shared/
    │   └── +package.moth      scoped @shared support package
    └── pages/
        ├── @page.moth         child normal module
        └── article.moth
```

### Normal modules

- One `@*.moth` file marks a normal module root.
- Ordinary files contain declarations only. Their declarations enter the module through dependency clauses.
- A normal root may contain declarations, one `export:` block, entry-local `config:`, top-level runtime work and direct page fragments.
- Top-level runtime work compiles into a dormant implicit `start`.
- A builder-selected entry activates that `start` exactly once. The HTML route follows the module directory, not the cosmetic root filename.
- The implicit `start` has no `Error!` channel, so top-level fallible work must recover with `catch`.
- Depending on a module exposes its public interface and never runs its top-level work. A normal root with no builder activity remains API-only.

### Public API

`export:` is the only cross-module public marker and is valid only in a module root:

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

The block may contain public functions, structs, choices, aliases, traits, constants and explicit direct-selection re-exports. Items outside stay private. Public signatures, fields, aliases and bounds cannot expose private types or traits. Receiver methods become visible with their exported receiver type and are not exported separately.

### Scoped support packages

A `+*.moth` root inside the source tree creates an API-only package named by its directory, such as `@shared`. It is visible within the nearest ancestor normal module's subtree under the scoped package rules. It has no top-level runtime work, route or page fragments.

Use a support package for declarations shared by normal sibling modules. Direct normal-sibling module dependencies remain invalid.

### Project package facade

An optional project-root `+*.moth` beside `config.moth` creates the project's external package facade. It is API-only, can assemble public descendant surfaces and is not visible to the project's internal modules. Its public API and reachable implementation cannot depend on the project's private `@project` values.

External project dependencies are registered by project dependency metadata before source graph construction. Only direct dependencies become source-visible. Transitive dependencies remain private to their package. A project-local root alias changes source spelling but not canonical package identity. The project-level declaration spelling belongs to the package manager and manifest contract. File-local code always binds the registered facade through normal `@package` clauses.

## Project configuration

`config.moth` is one self-contained compile-time file at the project root. It is not a module, cannot be depended on and produces no runtime code. `entry_root` is a relative source directory strictly below the project root.

```moth
default_channel #= "alpha"

project #= |
    name = "moth_docs",
    version #Import of String = "0.1.0",
    entry_root = "src",
    metadata = |
        channel = default_channel,
    |,
|

html #= |
    dev_output = "dev",
    release_output = "release",
|
```

Rules:

- The command selects one artefact builder before config validation. `config.moth` does not select the builder.
- One open `project` const record is required.
- `project.name` is required and provides stable project identity.
- Earlier private helper constants may be reused by later fields.
- Other top-level const records are builder or tooling sections.
- The active builder's project section is required, even when empty.
- Project values may contain folded scalars, options, collections, templates-as-strings and nested anonymous const records.
- Builder sections use backend-neutral folded values. Output settings belong to the builder section. Inactive sections still fold but are not schema-validated or retained.
- Config has no runtime declarations, mutable bindings, functions, named support types, source dependency clauses, page fragments or `export:`.

### Build inputs and `@project`

A direct project field or top-level source declaration may define a typed build-input contract:

```moth
-- direct field inside project
version #Import of String = "0.1.0"

-- module-wide source declaration
api_url #Import of String = "http://localhost:8080"
optional_label #Import of String? = none
```

Accepted types are `String`, `Int`, `Float`, `Bool`, `Char` and their optional forms. A source default is one primitive literal or `none`, not a call, template, cast or constant reference.

Pass values on the command line:

```bash
moth build . --input api_url=https://example.com
moth check . --input api_url=https://example.com
moth dev . --input api_url=https://example.com
```

Project fields are exposed to source through the explicit immutable `@project` dependency:

```moth
@project version

label = [: Version [version]]
```

`@project` is never injected implicitly and cannot be directly re-exported. Every dependency package has its own config, build inputs and private `@project`. Consuming-project inputs do not flow into it.

For source contracts, a compatible fixed project field wins, followed by a resolved project `#Import`, explicit command input, builder global and the shared default. Same-name source contracts must agree on type, optionality and default.

### Entry-local config

A normal module root may contain one root-local `config:` block:

```moth
@project version

favicon_url #= [@assets/favicon.svg]

extra_head #= [$html:
    <meta name="generator" content="Moth [version]">
]

config:
    html #= |
        title = "Moth docs",
        description = "Moth language documentation",
        lang = "en",
        favicon = favicon_url,
        head = extra_head,
    |
;
```

The block:

- is valid only at the top level of a normal root
- contains section records only
- uses ordinary compile-time visibility from dependencies, `@project`, earlier constants and resolved `#Import` values
- cannot contain dependencies, helpers, types, `#Import` declarations or a `project` section
- creates no source-visible symbol or runtime value
- stores metadata only for that root's possible entry

Project and entry schemas are separate. Entry settings never override or merge with project settings implicitly.


## Builders, backends and outputs

One command selects one project builder and any tooling overlays. The compiler frontend remains backend-neutral. Builders define:

- project and entry config schemas
- builder packages and template directives
- supported external binding providers
- entry and artefact policy
- target capabilities
- output names and layout

The HTML builder turns selected normal module roots into routes and documents. It supplies the source-backed `@html` package plus browser-oriented binding packages such as `@web/canvas`.

A mixed HTML build assigns reachable functions to JavaScript or Wasm automatically. `start`, DOM use and JavaScript-backed dependencies require JavaScript. Other supported functions may lower to Wasm. JavaScript may call generated Wasm wrappers. A Wasm-owned Moth function never calls a JavaScript-owned Moth function, so JavaScript requirements propagate back to callers. Moth source has no backend-selection annotations and source semantics do not change by target. `moth check` applies the same reachable target validation without writing artefacts.

Common commands:

```bash
moth new html my-site
moth dev .
moth check .
moth build . --release
```

Builders return output records. The build system validates output roots, writes files, tracks manifests and removes stale files owned by that builder and profile.

## Core and external packages

The prelude exposes `io` as the `@core/io` namespace:

```moth
io.line("Hello")
io.warn([: User [name] is missing])
```

Console functions accept string-compatible content. Wrap non-string values in a template. `io.set_title(text)` changes a browser document title on builders that expose that capability and is rejected on unsupported targets.

Core and builder packages use the same dependency syntax as source modules:

```moth
@core/math sin, PI
@core/text as text
@html
@web/canvas as canvas
```

Common Core roots include `@core/io`, `@core/math`, `@core/text`, `@core/random`, `@core/time` and `@core/collections`.

### Annotated JavaScript bindings

The HTML builder can expose typed free functions from an annotated project-local `.js` file:

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

Use `@moth.opaque` only when signatures mention a foreign opaque handle type. Binding signatures are positional and non-generic, with zero or one success return plus an optional final `Error!` slot. Callbacks, async functions, options, collections, receiver forms and multiple success returns are outside this binding profile. External APIs expose free functions, constants and opaque types, not source receiver methods. Ordinary Moth values cross restricted host bindings by value. Host code cannot retain references into Moth storage. Observable opaque resources require an explicit close or teardown API.

General Wasm component dependencies use a value-only WIT profile: arguments cross as independent values and results return as independent Moth graphs. References, callbacks, resources, futures, streams, raw pointers and shared-memory views do not cross this boundary.

## Deliberate language limits

These are design rules, not missing syntax to invent:

- No name shadowing
- No general macros or AST metaprogramming
- No closures, anonymous callable values, function values or higher-order polymorphism
- No exceptions, catchable panic or first-class public `Result` values
- No general `panic`. Use `Error!` for expected failure and `assert` for impossible invariants
- No operator overloading or user-defined literals
- No trait objects, runtime trait dispatch, inheritance, default methods, associated items or generic traits
- No structural conformance, blanket conformance, specialisation or negative conformance
- No reflection, runtime type IDs or compile-time type inspection
- No explicit reference types, lifetime syntax, move syntax, source RC, weak references or finalisers
- No parameterised type aliases, partial type application or higher-kinded types
- No user const generics beyond fixed collection capacity
- No receiver extensions for builtins, imports, dependency types or types from another file
- No wildcard dependencies or exports
- No user-defined builtin map keys, hashers, sets, indexing or mutable entry APIs
- No string `+`
- No `_` wildcard match arm

When a pattern needs dynamic heterogeneous values, use a choice. When it needs static reusable behaviour, use a generic trait bound. When UI code would usually capture a closure, use reactive sources, reactive parameters and template subscriptions.

Further detail: [language](https://nyejames.github.io/moth/docs/language-overview/), [memory](https://nyejames.github.io/moth/docs/memory/), [templates](https://nyejames.github.io/moth/docs/templates/), [projects](https://nyejames.github.io/moth/docs/project-structure/), [packages](https://nyejames.github.io/moth/docs/packages/) and [design scope](https://nyejames.github.io/moth/docs/design-scope/).
