# Moth Language Overview

Moth is a programming language and build system for modern UI-driven apps and webpages.

Keep this file focused on compiler-facing language facts: syntax shape, semantic invariants, edge cases, and deferred surface. Put expanded examples, tutorials, and user-facing explanations in docs-site source files under `docs/src/docs/**`.

Design principles:
- Powerful string templates for rendering content and describing UI
- Minimal, consistent syntax that is easy to parse and reason about
- Fast compile times for hot-reload development builds
- Memory safety through mandatory borrow and lifetime-region validation, with GC as a legal representation and optional ownership optimisation
- Strict static typing with a small number of concise, opinionated patterns

## Language Design Scope

Moth keeps the source language deliberately small. Compiler complexity is allowed when it
makes the programmer-facing model simpler, safer, and more predictable.

This document separates future work into two categories:

- **Deferred**: fits Moth's language design but is not implemented yet or is not part of the
  current source surface.
- **Outside language design scope**: intentionally not planned because it would make the language
  broader, more implicit, more solver-heavy, or harder to reason about.

A feature listed outside design scope should not be implemented unless the language philosophy is
explicitly changed first.

A concise maintainer-facing summary is published at
`docs/src/docs/codebase/design-scope/overview.mtf`. This document remains the
complete compiler-facing authority for the exact language-scope list.

## Outside the Language Design Scope

The following surfaces are intentionally outside Moth's language design scope.

| Surface | Reason |
|---|---|
| General-purpose macros, procedural macros, derive macros, and AST macros | They create a second compile-time language and make code harder to inspect. Templates, constants, and builder directives are the constrained compile-time surface. |
| General closures, anonymous function values, generic function values, and higher-order polymorphism | They add broad function-value semantics and solver pressure. Reactivity is the constrained UI-oriented mechanism intended to cover many closure-heavy template cases without adding general function values. |
| Dynamic trait values / trait objects | They require erased wrappers, runtime dispatch, dynamic-safety rules, backend-specific runtime support, and a second meaning for trait names. Use choices for runtime heterogeneity and generic trait bounds for static reuse. |
| Trait inheritance, trait aliases, trait composition, default methods, associated types, and associated constants | These turn traits into a type-level programming system rather than simple method contracts. |
| Generic traits and generic trait methods | These make trait solving significantly more complex and create Rust-like abstraction patterns. |
| Blanket, conditional, negative, or specialized conformance | These require coherence and specialization rules outside Moth's simplicity target. |
| Structural conformance | Matching method shapes must not silently imply conformance. `Type must TRAIT` stays explicit. |
| Type-set constraints, union constraints, underlying-type constraints, and user-defined numeric/operator constraints | Generic bounds name traits only. They are not a constraint sublanguage. |
| Operator overloading | Operators remain compiler-owned and predictable. |
| Source-authored receiver methods for builtins, imported types, dependency package types, external opaque types, or types declared in another file | Source-authored receiver methods belong only to the same file as their nominal receiver type. Use free functions for other types. |
| User-defined `HASHABLE`, custom map hashers/comparers, and user-defined keys for builtin maps | Builtin map syntax stays scalar-keyed. More sophisticated maps belong in packages as ordinary structs. |
| First-class public `Result` values and result pattern matching | `Error!`, postfix `!`, and `catch` are the language error path. Users can define ordinary choices when they want explicit result values. |
| Exceptions | Expected failures use `Error!`; invariants use `assert`. The two lanes stay distinct: `Error!` handles recoverable failure, `assert` handles impossible invariant violation. |
| Reflection, runtime type IDs, compile-time type inspection, and type-returning functions | These encourage generic meta-programming and weaken static readability. |
| Higher-kinded types, type functions, partial type application, and parameterized type aliases | These introduce a type-level abstraction language. |
| User const generics beyond fixed collection capacity | Capacity syntax remains a small built-in collection feature, not a general type parameter system. |
| Source-visible lifetime, reference-category, and ownership annotation systems | Moth omits explicit reference types and lifetime syntax, not references themselves. The compiler tracks ownership and lifetimes internally. |
| Source-visible RC, retain/release, weak ownership, finalizers, and unrestricted dynamic shared ownership | Internal backend RC remains allowed for already-legal topology. Diagnostics, common-owner groups, `copy`, and builder lifecycle roots replace source-level RC. |
| Backend-specific observable semantics leaking into source | Source semantics stay backend-neutral. Backends receive validated topology and may not reconsider source legality. |

## Related references

- `docs/src/docs/**` — user-facing docs-site pages and real `.moth` examples
- `docs/compiler-design-overview.md`: compiler stage ownership and cross-stage architecture
- `docs/src/docs/codebase/memory-management/overview.mtf` — reference semantics, borrow validation, lifetime regions, ownership and backend lowering
- `docs/src/docs/codebase/design-scope/overview.mtf` — accepted mechanisms, constraints and outside-scope families
- `docs/src/docs/progress/@page.moth` — current implementation status
- `docs/roadmap/roadmap.md` — planned work

## Syntax Summary

| Feature | Rule |
|---|---|
| Blocks | `:` opens a scope; `;` closes it. Semicolons do not terminate statements. |
| Braced literals/templates | `{}` are collections or hashmaps depending on the type/literal shape. `[]` are string templates only. |
| Comments | In ordinary code, `--` starts a single-line comment. Template and Moth template bodies treat it as content. |
| Operators | Logical/equality forms use words such as `is` and `not`; symbolic equality and logical-not forms are not operators. |
| Mutability | `~` marks mutable bindings/access. In declarations it appears before the type: `name ~Type = value`. |
| References | Shared immutable reference semantics are the default. Moth omits explicit reference types and lifetime syntax, not references themselves. |
| Constants | `#` marks compile-time constants: `name #= value`. |
| Module exports | `export:` is a strict block valid only in a module root file. It contains public declarations and grouped re-exports. |
| Copies | Copies must be explicit. Constructing a fresh value from shared inputs does not implicitly copy those inputs. |
| Parameters/fields | Function parameters and struct/choice fields use `|...|`. Defaults use `=`. |
| Results/options | Error returns use `Error!`; options use `T?`. |
| Generics | Declaration-site generics use `type`: `Box type A = | value A |`. Concrete instances use `of`: `Box of String`. |
| Traits | Trait declarations and conformances use `must`; generic bounds use `is`. Trait names are static contracts, not value types. |
| Explicit casts | `cast` / `cast!` convert to an explicit builtin target. Scalar conversion constructors are removed. |
| Renaming | `as` is used for type aliases, namespace import aliases, and grouped import aliases. |
| Shadowing | No visible name may be redeclared while still in scope. |

### Names and shadowing

- Types, structs, choices, generic parameters, and type aliases: `PascalCase`
- Variables and functions: `regular_snake_case`
- Shadowing is invalid: a visible name may not be redeclared in the same visible scope.

Mutable bindings are reassigned with `=`:

```moth
value ~= 1
value = 2 -- reassigns the existing mutable binding
```

## Core Syntax Patterns

```moth
count ~= 0
ratio Float = 1.5
text_slice = "text"
letter = '🦋'

message = [:
    Templates create owned strings.
]

values ~{Int} = {}
names {String} = {"Priya", "Gollum"}

Person = |
    name String,
    age Int,
|

Status ::
    Ready,
    Failed | message String |,
;

increment |value Int| -> Int:
    return value + 1
;
 
value = 10
reference = value
copied = copy value

left = "Hello "
right = "world"
joined = [left, right]
```

Raw backtick string slices aren't implemented in the current source surface. Backticks are inline-code delimiters inside `$md` content.

Quoted string slices support exactly five escapes:

| Escape | Value |
|---|---|
| `\\` | backslash |
| `\"` | double quote |
| `\n` | newline |
| `\r` | carriage return |
| `\t` | tab |

Every other escape is invalid. A backslash cannot continue a quoted string across a physical
newline. Raw backtick tokenization does not decode escapes and preserves backslashes and logical
newlines, but that tokenizer behaviour does not make raw backticks valid source expressions.

`copy` accepts a visible binding, field projection or parenthesised place. Literals, templates, calls and computed expressions aren't copy places and are rejected.

## Memory semantics

**Moth is reference-semantic by default, copy-explicit and move-inferred. It omits explicit reference types and lifetime syntax, not references themselves.**

Canonical detail lives under `docs/src/docs/codebase/memory-management/`. This section records the compiler-facing source rules.

### Shared aliases and non-lexical activity

- Reading, binding, passing, returning or storing an existing value uses shared reference semantics by default.
- Shared aliases are read-only and do not imply ownership or implicit copy.
- Alias activity is non-lexical and control-flow-sensitive. A shared alias blocks overlapping mutation only until its last potential use on the relevant path.
- An unused shared alias does not block later mutation.

### Mutable aliases versus fresh mutable slots

- `alias ~= source`, or an equivalent typed mutable declaration from an existing place, creates an exclusive write-through mutable alias.
- Assignment through a mutable alias writes through to the referent and must not silently detach or rebind the alias.
- A mutable declaration initialized from a fresh value creates an independent mutable slot.
- `~place` requests exclusive access to an existing mutable place. It is not a move marker or type constructor.
- Mutable receivers require an existing mutable place. Fresh-rvalue materialisation applies only to ordinary mutable parameters.

### Aggregate storage

Existing values stored in structs, choices, collections, maps, tuples, templates or other aggregates retain shared reference semantics by default. `copy` creates independent storage. At a proven final use, the compiler may realise the same source operation as an ownership transfer without changing source meaning.

Maps own their entry structure while keys and values follow the same shared/copy/inferred-transfer rules. Map lookup keys are borrowed. `get` returns a shared alias. `remove` returns the removed value under normal lifetime and ownership rules.

### Explicit `copy`

`copy place` performs a semantic deep copy of the complete copyable runtime value graph reachable from the place. Internal alias topology is preserved, same-region cycles are preserved, the copied graph shares no mutable allocation with the source, reactive sources are copied as current values, and non-copyable external resources produce diagnostics. Full contract: `docs/src/docs/codebase/memory-management/access-and-aliasing/access-and-aliasing.mtf`.

### Optional inferred transfer

There is no move syntax and no ordinary mandatory-consuming value operation. Inferred transfer is optional. When safe transfer is not proven on every relevant path, the operation remains a borrow. Immutable and mutable parameters may both receive inferred destruction responsibility at a proven final-use call site. Parameter access mode remains separate from optional ownership effect.

### Result freshness and independence

- A fresh result root has a new root allocation but may retain legal references.
- An alias result reuses an existing root or projection.
- An independent result graph has no retained Moth reference to pre-existing storage.

Canonical detail: `docs/src/docs/codebase/memory-management/lifetime-regions-and-escape-validation/lifetime-regions-and-escape-validation.mtf`.

### Accepted but deferred: declared memory groups

The following is accepted end-state syntax. Implementation is deferred and must not be treated as current source support:

- `group name:` creates a declared hard lifetime region
- `name [access/type] into group_name = expression` places a declaration into the current or ancestor group only
- destination-group lexical ownership and visibility follow the declaration point through the remainder of the destination group
- a declaration targeting an ancestor group is legal only from a straight-line nested `block:` or nested `group:` that executes at most once; it is invalid directly inside `if` branches, match arms, `catch` branches, loops or any construct that may execute zero or multiple times
- V1 has no expression-site placement, no `into group` syntax on reassignment, no extraction, and no unrestricted group-to-group adoption. A mutable binding already owned by a group may be reassigned only with a fresh result root valid for that group, an independent copy allocated into that group, or a same-group value transferred at a proven final use.
- group identity is not part of types or signatures

Design authority: `docs/src/docs/codebase/memory-management/declared-memory-groups/`. Implementation sequencing: `docs/roadmap/plans/grouped-memory-design.md`.

### External platform imports

Ordinary values in restricted host bindings cross by value and cannot be retained as references into Moth storage. Mutable host access applies only to opaque foreign handles. General Wasm Components use the separate WIT value-only profile described by the memory authority.

### Outside memory-surface scope

Source-visible RC, retain/release, weak ownership, finalizers and unrestricted dynamic shared ownership are outside language design scope. Internal backend RC remains allowed for already-legal topology.

### Function Calls, Named Arguments, and Mutable Access

Named arguments use `parameter = value`. Access mode is chosen at the call site.

```moth
sum(values)          -- positional shared
sum(~values)         -- positional mutable/exclusive
sum(items = values)  -- named shared
sum(items = ~values) -- named mutable/exclusive
```

Rules:
- Positional arguments must precede named arguments; no positional argument is allowed after the first named argument.
- Each parameter may be supplied once.
- Host calls and builtin member calls are currently positional-only.
- Defaulted parameters may be omitted; named arguments can skip earlier defaults.
- Parameter defaults fully fold in the declaration-time compile-time context. Reactive parameters don't support defaults.
- A `~T` parameter accepts `~place` for an existing mutable place, or a plain fresh rvalue such as a literal, template, constructor call, or computed value.
- Passing an existing place to a `~T` parameter without `~` is an error.
- `~` is place-only syntax and is invalid on immutable bindings, literals, templates, constructor calls, and computed expressions.
- Mutable bindings and mutable call access are separate: `value ~= ...` declares or reassigns a mutable binding; `fn(~value)` requests exclusive access for one argument.
- Collections and mutable receiver calls follow the same explicit call-site mutability rule.

Function parameters and struct fields share default-value syntax:

```moth
describe |prefix String = "item", subject String| -> String:
    return [prefix, ":", subject]
;

Config = |
    height Int = 100,
    width Int,
|

label = describe(subject = "apple")
default_height = Config(width = 80)
```

Choice payload fields do not currently support defaults.

### Numeric Semantics

`Int` is a signed 32-bit integer with the range `-2147483648` through `2147483647`.
`Float` is a finite `f64`; non-finite values are rejected at source, cast, arithmetic, and external
boundary points that can introduce them.

| Form | Result |
|---|---|
| Whole-number literal | `Int` |
| Decimal-point literal | `Float` |
| `Int + Int`, `Int - Int`, `Int * Int`, `Int % Int` | `Int` |
| `/` | Real division; `Int / Int -> Float` |
| `//` | Integer division; `Int // Int -> Int`, truncating toward zero |
| Mixed `Int`/`Float` arithmetic | `Float` |

Source numeric literals support lowercase exponent syntax such as `1e6`, `1e-6`, and `1.0e+21`.
Uppercase `E` is rejected. A leading `-` is part of a signed numeric literal only in prefix position,
for example `-1` or `-1e6`; unary negation for non-literals must also be attached, for example
`-count`. Spaced unary negation such as `- count` and unary `+` are rejected.

Symbolic binary operators must have spaces around them, such as `a + b`. Assignment `=` follows
the same outer-spacing rule but is not a binary operator. Forms like `a+b`, `a-1` and `count=1`
are syntax errors.

Compound symbolic assignments (`+=`, `-=`, `*=`, `/=`, `//=`, `%=`, `^=`) follow the same
spacing rule: `count += 1` is valid, while `count+=1`, `count +=1`, and `count+= 1` are
syntax errors. The mutable declaration spelling `~=` remains adjacent, with whitespace before `~`
and after `=`. Write `count ~= 1`, not `count~= 1`, `count ~=1` or `count ~ = 1`.

Runtime numeric operations are checked. Integer overflow, divide/modulo by zero, invalid integer
exponents, and non-finite Float results are failures. When the enclosing function has builtin
`Error!` as its error return slot, numeric failures recover through that builtin `Error` channel.
Custom fallible channels, non-fallible functions, and active-root top-level runtime code use trap
mode. Statically known numeric failures are compile-time diagnostics, even inside builtin
`Error!` functions.

There is no implicit `Float -> Int` coercion. Use `cast!` at an explicit `Int` target when a
fallible conversion is intended.

### Explicit Casts

`cast` is the explicit conversion marker for converting one value to a compiler-supported builtin
target type. Supported targets are:

```text
Bool
Int
String
Char
Float
Error
```

Forms:

```moth
value Target = cast expression
value Target = cast! expression
value Target = cast expression catch:
    then fallback
;
value Target = cast expression catch |err|:
    then fallback
;
```

Rules:
- The target must come from the immediate typed boundary: an annotated declaration or assignment, an explicit return slot, a concrete function parameter, a struct or choice field, a default value, a typed collection or map entry, or a `then` arm whose enclosing value-producing block has an explicit receiver.
- Generic parameter slots are not cast targets. Generic inference does not look through `cast`.
- Conditions, loop conditions, assertions, template interpolation, operator operands, expression statements, and untyped declarations are not cast targets.
- `T?` receiving contexts cast to the inner builtin `T`, then use ordinary contextual `T -> T?` wrapping. Optional source values are not auto-unwrapped.
- In `target T? = cast source catch:`, recovery handlers also produce the inner `T`; the compiler wraps the successful or recovered `T` into `T?` after the cast. `then none` is invalid in that handler because `none` is not an inner builtin cast result.
- `String -> Int` and `Float -> Int` casts materialize signed 32-bit integer values from `-2147483648` through `2147483647`. Values outside that range fail the cast so compile-time folding and runtime lowering agree.
- `String -> Int` and `String -> Float` casts use Moth numeric text grammar over the whole string. Surrounding whitespace, uppercase exponent markers, unary plus, malformed separators/exponents, `NaN`, and infinity spellings are rejected.
- `String -> Float` additionally rejects grammar-valid text that materializes to a non-finite `Float`.
- `Float -> String` uses Moth's formatter, not backend-native display. It normalizes `-0.0` to `0` and uses stable lowercase exponent formatting where exponential notation is required.
- Same-type casts are invalid. `Int -> Float` contextual promotion remains separate from explicit casts, but `cast` is allowed when the source is naturally `Int` and the target is `Float`.
- `cast expression` requires infallible evidence. `cast! expression` requires fallible evidence and propagates through the current function's `Error!` return slot. `cast expression catch:` requires fallible evidence and recovers locally.
- `cast! expression catch:` is invalid. Propagation and local recovery are mutually exclusive.
- `cast!` and `cast ... catch:` handle only cast failures. Operand `Error!` values must be handled before the cast.

Scalar constructor-style conversions are removed:

```moth
Int(value)
Float(value)
Bool(value)
String(value)
Char(value)
```

`Error(...)` remains a real builtin constructor:

```moth
err = Error("Missing number", 200)
```

Use `cast` for conversion to `Error`:

```moth
err Error = cast "Missing number"
```

Core cast evidence is exposed through compiler-owned static traits such as
`CASTABLE_TO_STRING` and `TRY_CASTABLE_TO_INT`. Same-file nominal structs, choices, and aligned
generic nominal constructors can declare conformance to these traits when they provide the
required receiver method, for example `to_string |This| -> String` or
`try_to_int |This| -> Int, Error!`.

These traits prove source evidence only. User-defined cast targets, generic cast targets, external
opaque cast targets, generic cast traits, and broad return-type-directed conversion are outside
Moth's cast design scope.

### Options, Results, `then`, and Assertions

#### Options

Optional types use `T?`. `none` requires an optional type context.

```moth
name String? = none

find_name |id String| -> String?:
    if id.is_empty():
        return none
    ;

    return "Alice"
;
```

Rules:
- A `T` value can be used where `T?` is expected.
- `none` is the only special option value.
- Canonical recovery is explicit present-value inspection: `if maybe is |value| then value else fallback`.
- Statement-only absence inspection remains valid: `if maybe is none: ... ;`.
- Full option matches support `none`, literal/relational present-value patterns, `|value|` capture, guards, and `else`.
- Options support equality against `none`, the same option type when its inner type supports equality and a compatible inner value. Options don't support ordering.
- A full option match may omit `else =>` when it has both an unguarded `none` arm and an unguarded present-value capture arm.
- Direct fallback syntax such as `maybe else then fallback` is rejected.
- Postfix `?` unwraps a present value or returns `none` from the current function.

```moth
display_name = if maybe_name is |name| then name else "guest"

get_display_name |id String| -> String?:
    user = find_user(id)?
    return user.name
;

name = if maybe_name is |name|:
    then name
else
    then "guest"
;

label = if maybe_name is:
    "Priya" => then "Hi Priya"
    |name| => then name
    none => then "guest"
;
```

#### Error returns

Error-returning functions mark one return slot with `!`. `Error` is builtin and reserved. Its public fields are:
- `message String`
- `code Int = 0`

`ErrorKind`, `ErrorLocation`, and `StackFrame` are ordinary user-available names.

Use `return!` to produce the error path, postfix `!` to bubble it, and `catch:` / `catch |err|:` to recover.

```moth
parse_number |text String| -> Int, Error!:
    if text.is_empty():
        return! Error("Missing number", 200)
    ;

    return 42
;

load |text String| -> Int, Error!:
    return parse_number(text)!
;

fallback = parse_number("42") catch:
    then 0
;

inline_fallback = parse_number("42") catch |err| then err.code
```

The inline catch binding is local to its fallback expression. First-class public `Result` values are outside language design scope. The special `!` return is only for the error path. Success values use the normal return list.

An error-only function uses `-> Error!` and may fall through normally. A direct top-level `return!` produces its error. Nested-block `return!` in an error-only function remains a current implementation gap.

Automatic checked-numeric recovery is compiler-owned and applies only when the current function's
fallible return slot is builtin `Error!`. It does not make numeric operators source-visible
`Error!` expressions, and it does not adapt failures into custom fallible channels.

#### Value-producing blocks and multi-returns

`then` returns one or more values from the nearest active value-producing block to the receiving site.

Supported producers:
- value-producing `if`
- full match
- block-form `catch:` / `catch |err|:` recovery

Valid receiving sites:
- declarations and assignments
- multi-bind
- returns
- nested `then`

Value-producing blocks are not general expressions and are rejected in function arguments, operator operands, constructor arguments, collection literals, template interpolation, and expression statements.

```moth
value = if condition then 1 else 0
state = if status is Ready then "ready" else "waiting"
name = if maybe_name is |name| then name else "guest"
fallback = parse_number("42") catch then 0

name, score = load_user(id) catch |err|:
    io.line(err.message)
    then "guest", 0.0
;
```

Multi-value blocks must produce the receiver arity on every producing path. Multi-bind accepts explicit multi-return function calls and value-producing blocks at closed RHS receiving sites. Regular declarations remain single-target, and user-visible tuple values are not supported.

Inline `if` supports Bool conditions and choice predicates. The block `if ...: then ...` form has a known implementation gap and should not be treated as current-valid syntax.

```moth
pair || -> String, Int:
    return "Ana", 2
;

name, count = pair()
```

#### Assertions

`assert` is a statement-only language intrinsic for invariants.

```moth
assert(index < items.length())
assert(index < items.length(), "index must be in bounds")
assert(false, "unimplemented backend path")
```

Rules:
- Assertions are always checked.
- Failure is unrecoverable, does not return `Error!`, and cannot be caught with `catch`.
- `assert` cannot be assigned, passed, imported, aliased, or used in expression position.
- Expected failures should use typed error propagation with `Error!` and `catch`.
- `assert(false)` and `assert(false, "message")` are statically terminal and may end a non-`Void` function or value-required `catch` handler.
- Dynamic `assert(condition)` is not statically terminal.
- The second literal message is optional. Assertion messages are currently string literals only.

### Collections

Collections are ordered, zero-indexed, homogeneous groups.

```moth
values ~= {'a', 'b', 'c'}  -- {Char}
empty_values ~{Int} = {}   -- explicit type required
fixed_values {3 Int} = {10, 20}

capacity #Int = 4
scratch ~{capacity String} = {}

larger_capacity #Int = capacity + 2
larger_scratch ~{larger_capacity String} = {}
labels {capacity} = {"alpha", "beta"} -- declaration-target shorthand

items ~= {10, 20, 30}
~items.push(40) catch:
;
~items.set(0, 99) catch:
;
removed = ~items.remove(1) catch:
    then 0
;
```

Rules:
- `{T}` is a growable collection type.
- `{N T}` is a fixed collection type with exact maximum length `N`.
- Fixed capacity is semantic type identity, not an allocation hint: `{Int}`, `{4 Int}`, and `{8 Int}` are distinct incompatible types.
- Fixed capacity in type position must be either a positive `Int` literal or a bare visible `#Int`
  constant name. Capacity position is not general expression position: arithmetic, function calls,
  field access, const-record projection, conditionals, and nested expressions are invalid there.
  Put any calculation in a named compile-time constant first.
- `~` is binding/access mode. It is not part of the collection type shape: `~{4 Int}` is mutable access to a `{4 Int}`, not a separate type.
- Non-empty literals infer their element type from items.
- Empty literals require an explicit collection type at the declaration site.
- In a fixed collection receiving context, a literal constructs that fixed collection directly and must not exceed the fixed capacity.
- Capacity-only shorthand such as `{capacity}` is valid only on a binding declaration with an immediate non-empty collection literal initializer that can infer the element type.
- Capacity-only shorthand is invalid for empty literals, non-literal initializers, signatures, aliases, fields, and returns.
- Immutable value bindings cannot be initialized with an empty fixed collection literal. Mutable fixed empty bindings and fixed collection field defaults are valid.
- Element type is not inferred from later `push`, assignment, loop, function-argument, HIR, or borrow-analysis use.
- Mutating operations require explicit mutable/exclusive receiver access: `~items.push(...)`, `~items.set(...)`, `~items.remove(...)`.
- `get`, `set`, `push`, and `remove` are fallible; `length` is infallible.
- `collection.get(index)` returns `Elem, Error!`.
- `~collection.push(value)` returns no success value and must still be handled with `!` or `catch`.
- `~collection.set(index, value)` replaces an existing element only; it does not fill unused fixed capacity.
- `~collection.push(value)` appends after the current last element and fails when a fixed collection is already full.
- `~collection.remove(index)` removes the element at that index, shifts later elements down, and frees one slot in a fixed collection.
- `collection.length()` returns the current logical length, not fixed capacity.
- Indexed writes use `~items.set(index, value)`; assignment through `get` is removed.
- The compiler may lower collection methods directly without a runtime call.

### Hash Maps

Hash maps are insertion-ordered key/value groups. The current source surface targets the HTML JavaScript backend; HTML-Wasm rejects reachable map use before backend lowering.

```moth
scores ~= {"Priya" = 10, "Grace" = 12} -- {String = Int}
empty_scores ~{String = Int} = {}

score = scores.get("Priya") catch:
    then 0
;

~scores.set("Linus", 7) catch:
;

removed = ~scores.remove("Grace") catch:
    then 0
;

if scores.contains("Priya"):
    io.line("found")
;

count = scores.length
~scores.clear()
```

Rules:
- `{Key = Value}` is a hashmap type. The key and value sides are ordinary type annotations.
- `{key = value}` is a hashmap literal. Any top-level `=` entry inside a non-empty `{...}` value literal makes the whole literal a hashmap literal.
- Every hashmap literal entry must be a key/value pair. Mixing collection items and hashmap entries is invalid.
- Empty map literals require an explicit or contextual hashmap type.
- Inline map type annotations are limited to two nested map levels in Alpha. Use a named type alias when deeper nesting is needed.
- Bare identifiers in key position are variable references, not string-key shorthand.
- Hashmaps are insertion ordered. First insertion determines entry position; replacing an existing key updates the value without moving the entry or replacing the stored key.
- Removing a key removes that entry from the order. Re-inserting the key appends a new entry.
- Builtin hashmap key types are permanently limited to `String`, `Int`, `Bool`, and `Char`.
  This is a language-owned map surface, not a general hashing abstraction.
- `Float`, structs, choices, collections, hashmaps, traits, functions, external opaque types, and generic parameters are invalid keys.
- Values follow the same runtime-storable rules as collection elements.
- Maps own their entry structure. Existing keys and values stored in entries follow the ordinary shared-reference, explicit-copy and inferred-transfer rules.
- `get`, `contains`, and `remove` borrow the lookup key.
- `get` returns shared access to the stored value. Mutating the same map while that shared value is live is invalid.
- `remove` removes the entry and returns the removed value under the normal lifetime and ownership rules.
- `set` inserts or replaces a value and does not return the old value.
- `get`, `set`, and `remove` are fallible and must be handled with postfix `!` or `catch:`.
- `contains`, `length`, and `clear` are infallible.
- `map.length` is a read-only property, not a method call.

Outside the builtin hashmap design scope: hashsets as language syntax, user-defined hashers or
comparers, `Float` keys, user-defined key types, generic key maps through `HASHABLE`, map equality,
mutable entry APIs, indexing syntax, const hashmaps, fixed/capacity maps, and specialized map
variants. More sophisticated maps should be ordinary Standard package or user-defined structs.

Wasm hashmap runtime/lowering remains deferred backend work for the existing scalar-keyed builtin map
surface.

### Core IO

`io` is a preluded namespace alias to `@core/io`.
Console output functions accept string-compatible content only: escaped string slices, owned
strings, and templates. They reject non-string values directly; wrap values in a template when you
want debug-style interpolation.

```moth
io.line("Hello, World!")
io.print("loading...")
io.debug("checking state")
io.warn("slow path")
io.error("failed path")

name = "Alice"
io.line([: Hello, [name]])
```

`import @core/io` binds the same namespace explicitly, and `import @core/io as output` creates a
file-local namespace alias such as `output.line("Hello")`.

Input polling is available under `io.input.*` on HTML-JS. The handle type
`io.input.Input` is an opaque external type: Moth code can pass it to the core IO functions
but cannot construct it with struct syntax or inspect fields.

```moth
input ~= io.input.new() catch:
    assert(false, "input not available")
;

io.input.update(~input)

if io.input.key_pressed(input, "d"):
    io.line("pressed d")
;

if io.input.pointer_down(input, "left"):
    x = io.input.pointer_x(input)
    y = io.input.pointer_y(input)

    io.line([: pointer [x], [y]])
;

last_key = io.input.last_key_pressed(input)

if last_key is |key|:
    io.line([: last key: [key]])
;

io.input.close(~input)
```

`io.input.new() -> io.input.Input, Error!` is fallible because the active backend/runtime may not
provide input support. `io.input.update(~input)` drains pending browser events into edge state, and
`io.input.close(~input)` releases the listeners and resets the handle. `update` and `close`
require mutable access; polling reads use shared access.

Keyboard polling supports `key_down`, `key_pressed`, `key_released`, `last_key_pressed`, and
`last_key_released`. Single alphabetic key strings normalize to lowercase, space is `"Space"`, and
special keys use browser logical names such as `"ArrowLeft"`, `"Enter"`, `"Escape"`,
`"Backspace"`, and `"Shift"`. Pointer polling supports `"left"`, `"middle"`, and `"right"`
buttons, current `pointer_x` / `pointer_y` coordinates as `Float`, edge-state helpers, and
`last_pointer_pressed` / `last_pointer_released`. Unknown key or button strings return `false`;
missing `last_*` values return `none`; closed handles return neutral values.

HTML-JS maps console helpers to the browser console and input helpers to window/document-level
keyboard and pointer polling. HTML-Wasm and other targets reject reachable Core IO calls before
backend lowering until they implement equivalent lowerings. Browser event delivery still depends
on returning to the host event loop, so a synchronous infinite Moth loop can prevent new
input events from being observed. The current source surface does not add callbacks, promises, async tasks, frame/tick
APIs, targeted DOM/canvas input sources, event queues, filesystem/path IO, fetch/network IO,
timers, text entry/IME, touch gestures, gamepads, drag/drop, clipboard, wheel scrolling, or file
picker APIs.

## String Template System

Templates use `[]`; collections use `{}`. `""` creates escaped string slices. Expression-position raw backtick slices aren't implemented. Templates create owned strings and may fold at compile time or lower to runtime string construction.

Template head/body shape:

```moth
[$md, $children([:<li>[$slot]</li>]):
    # Title
    [: child]
]
```

Core rules:
- The head and body are separated by `:`.
- Authored `.moth` templates must close with `]`; truncated heads, bodies, nested child templates, and directive-argument templates produce syntax diagnostics.
- Template bodies capture variables from the surrounding scope.
- Backticks and backslashes inside template bodies are ordinary body text and are preserved for formatters such as `$md`. Regular quoted string literals decode only `\\`, `\"`, `\n`, `\r` and `\t`.
- Literal template delimiters in output use ordinary string insertion, such as `[: ["[literal]"]]`.
- Only direct top-level template expressions in an active HTML module root contribute page fragments.
- Top-level runtime templates run in active-root `start()` order.
- Top-level const templates fold at compile time and are merged separately.
- Templates assigned to variables or returned from functions do not contribute page fragments by themselves.
- Runtime Float interpolation and `Float -> String` casts use the same Moth formatter as
  compile-time folding, so template output does not depend on host-native Float stringification.

### Template Directives

`$` introduces compiler-handled template directives. Directive availability is registry-based:
- Frontend directives are available by default.
- Builders may register project-specific directives with the same `$name` syntax.
- Unknown directives are syntax/rule errors.

| Directive | Meaning |
|---|---|
| `$slot` / `$insert(...)` | Slot declaration/contribution |
| `$fresh` | Opt out of the immediate parent’s `$children(...)` wrapper |
| `$md` | Parse body as Moth Markdown |
| `$raw` | Preserve authored body whitespace |
| `$note` / `$todo` | Ignored comments |
| `$doc` | Documentation comment template |
| `$children(...)` | Apply a direct-child wrapper template or string slice |
| `$html` | HTML-builder raw HTML |
| `$css` | HTML-builder CSS checks |
| `$escape_html` | HTML-builder HTML escaping |
| `$code` | HTML-builder code highlighting |

`$children(...)` applies only to direct children. `$fresh` is per-child and only affects wrapper application from the immediate parent. Formatting directives do not flow into nested child templates; redeclare them where needed. For `$children(...)` template arguments, the child template must close with `]` before the directive closes with `)`.

```moth
list #= [$children([:<li>[$slot]</li>]):
    <ul>
        [$slot]
    </ul>
]

[list:
    [: one]
    [$fresh: [: two]]
]
```

### Template Slots

Slots let one template receive content from another.

| Slot form | Meaning |
|---|---|
| `[$slot]` | Default slot |
| `[$slot("name")]` | Named slot |
| `[$slot(1)]`, `[$slot(2)]` | Positional slots |
| `[$insert("name"): ...]` | Contribution to a named slot |

Routing rules:
- Loose contributions fill positional slots first in ascending numeric order.
- Remaining loose contributions go to the default slot if it exists.
- Loose content that cannot be assigned is an error when no default slot exists.
- Missing slots render as empty strings.
- Repeated slots replay the same contribution.
- Runtime-capable templates support default, named, positional, and loose contributions from template `if` and `loop` bodies.
- Runtime slot applications are routed during AST preparation and lower through ordinary runtime string accumulators.

```moth
card = [:
    <h1>[$slot("title")]</h1>
    <section>[$slot]</section>
]

[card:
    [$insert("title"): Hello]
    Body
]

img = [:
    <img src="[$slot(1)]" alt="[$slot]">
]

[img, "logo.png": Site logo]

title = [:
    <h1 style="[$slot("style")]">
        [$slot]
    </h1>
]

[title:
    [$insert("style"): color: blue;]
    Hello world
]
```

Named inserts may be authored directly in an application. Storing a named insert in a binding and passing it later is a current implementation gap.

Nested `$children(...)` wrappers remain scoped to direct children, so row/cell-style helpers can be layered without wrapper leakage.

### Reactivity

Reactivity is a constrained template/UI mechanism, not a general closure or
function-value support.

Current reactivity syntax:
- reactive declarations: `name $Type = value` and `name $= value`
- reactive parameters: `param $Type`
- template subscriptions: `$(source)` in template head/capture positions only

Rules:
- `$Type` is reactive access syntax, not a first-class wrapper type.
- Reactive identity is binding/source metadata, not semantic `TypeId` identity.
- A reactive declaration owns stable mutation-capable runtime storage in its declaring scope.
- A reactive parameter is a read/subscription handle to an existing source; it does not grant mutation permission.
- `$(source)` accepts exactly one bare reactive source identifier in the current source surface.
- `$(source)` captures stable source identity and read-only subscription metadata; it never captures
  a mutable borrow, copied value, or computed expression.
- Top-level runtime HTML fragments in the HTML-JS builder are the first live sink. The backend
  mounts reactive template strings and rerenders the whole mount slot when any dependency source is
  invalidated.
- Field/path subscriptions, expression dependency tracking, reactive IO sinks, fine-grained DOM updates, template-owned event/action/effect syntax, `$bind(...)`, typed component messages, and HTML-Wasm runtime support are deferred follow-ups.
- `[source]` remains a snapshot template read.
- General closures, anonymous function values, generic function values, and higher-order polymorphism are outside the current language design scope.

### Template Head Suffix Control Flow

Templates support `if` and `loop` as final head suffixes before the body colon. If the head already has a value, helper, or directive, put a comma before the suffix.

```moth
[if show:
    Visible
]

[card, if show:
    Visible inside card
]

[if maybe_name is |name|:
    Hello [name]
[else if use_fallback]
    Hello fallback
[else]
    Hello guest
]

[loop items |item, index|:
    [if item.skip:
        [continue]
    ]

    [index]: [item]

    [if item.done:
        [break]
    ]
]
```

Template `if`:
- supports `Bool` conditions and option-present capture;
- supports standalone `[else if ...]` with the same selectors;
- supports optional standalone `[else]`;
- does not support full pattern-match branch chains in the template head.

Template `loop`:
- supports conditional loops, collection iteration, and numeric ranges using normal loop-header syntax;
- allows collection/range loops to bind item/counter and optional zero-based index;
- concatenates iterations directly with no implicit separator;
- supports structural `[break]` and `[continue]` targeting the nearest active template loop.

Output/wrapper rules:
- `[head, loop ...:]` wraps the whole aggregate once; put per-iteration wrappers inside the loop body.
- False/no-else branches and zero-iteration loops produce structural no-output and skip shared wrappers.
- Output before `[break]` or `[continue]` is preserved; iterations with no output before control flow do not count as emitted.

Const rules:
- Top-level `#[if ...:]` and `#[loop ...:]` must fully fold at compile time.
- Const-required `if` validates every branch body.
- Const conditional loops fold to no-output only when the condition is compile-time `false`; compile-time `true` and runtime/unknown conditions are rejected.
- Const range/collection loops use `template_const_loop_iteration_limit`, default `10_000`, capped at `1_000_000`.
- Runtime template control flow is lazy.

Runtime slot applications are valid inside template control flow after normal slot routing. Escaped unresolved `[$slot]` or `$insert(...)` artifacts inside runtime control-flow bodies are invalid template structure. Runtime slot applications appended inside template loops follow the same nearest-loop `[break]` / `[continue]` rules as direct loop body content.

### Markdown Formatting

`$md` is Moth's small markdown flavour, not a full CommonMark implementation.

Inline code uses paired isolated single backticks on the same markdown line and renders as `<code>...</code>`.
Markdown emphasis and link parsing do not run inside inline code. Empty spans, repeated backtick runs, unmatched backticks, multiline spans, variable-length delimiters, fenced code blocks, and markdown-level backtick escaping are not part of Moth's markdown flavour.

Dynamic expression anchors may appear inside a parent-authored code span, but `$md` does not inspect their rendered output. Child templates are opaque barriers to the parent formatter and cannot be inside a parent-authored code span or pair delimiters across that child boundary.

### Moth Template `.mtf` Content Files

Moth template files are HTML-builder content helpers. A `.mtf` file is authored as the body of an implicit compile-time markdown template:

```moth
content #String = [$md:
    ...entire .mtf file body...
]
```

The compiler builds that structure directly; it does not prepend wrapper source text.

Import `.mtf` files with normal extensionless source import syntax:

```moth
import @docs/intro
import @docs/intro {
    content as intro_content,
}

[:[intro.content] [intro_content]]
```

Rules:
- A `.mtf` file exposes exactly one generated constant, `content #String`.
- Direct extension imports such as `import @docs/intro.mtf` are invalid; use `import @docs/intro`.
- `.mtf` files are never page entries, module roots, config files, or standalone project types.
- `.mtf` files have no imports, declarations, frontmatter, metadata, or raw-source preservation.
- A `.mtf` body must fully fold at compile time. Runtime functions, runtime bindings, and types are not visible.
- The implicit markdown template means `--` is body text, not a Moth comment.
- `.mtf` bodies follow normal template-body and `$md` semantics. Literal template delimiters use string insertion, such as `["[literal]"]`; nested authored templates still use normal `[...]` syntax and must close explicitly.
- Nested `.mtf` templates with no explicit directive also default to `$md`. Any explicit directive on that nested template, such as `$raw`, `$fresh`, `$html`, `$code`, `$css`, or `$escape_html`, overrides the Moth template Markdown default for that template.

Inside compiler-integrated HTML project builds, a `.mtf` body sees a restricted flat compile-time scope:
- exported compile-time constants and const records from `@html`, such as `[p: body]`;
- exported compile-time constants and const records from the same-directory module root, when one exists.

Same-directory module-root constants and `@html` constants don't shadow each other. If both surfaces expose the same visible name, compilation fails with a visible-name collision diagnostic. Authors resolve collisions by renaming the module-root export. Functions, structs, choices, type aliases, traits, methods, external/runtime APIs, and the generated self `content` constant are not visible in the `.mtf` body.

Module roots can re-export Moth template content explicitly:

```moth
-- src/@docs.moth
export:
    import @docs/intro {
        content as intro_content,
    }
;
```

Use `.moth` files for pages, composition, imports, functions, and richer compile-time setup. Use `.mtf` for small markdown-first content fragments consumed by `.moth`.

### Plain Markdown `.md` Content Files

Plain Markdown files are HTML-builder content helpers for raw CommonMark-style Markdown. A `.md`
file renders to HTML at compile time and exposes exactly one generated constant:

```moth
content #String = "<rendered html>"
```

Import `.md` files with the same extensionless source import syntax as Moth and Moth template
source assets:

```moth
import @docs/intro
import @docs/intro {
    content as intro_content,
}

[:[intro.content] [intro_content]]
```

Rules:
- A `.md` file exposes exactly one generated constant, `content #String`.
- Direct extension imports such as `import @docs/intro.md` are invalid; use `import @docs/intro`.
- `.md` files are never page entries, module roots, config files, or standalone project types.
- `.md` files have no Moth imports, declarations, interpolation, templates, frontmatter, or metadata.
- `.md` files do not see same-directory module-root constants or `@html` constants. They have no Moth scope.
- Raw HTML is preserved in the current source surface. This feature does not add a sanitizer or raw-HTML policy key.
- Markdown links and images render literal `href` and `src` values in the current source surface. They are not tracked assets and are not rewritten.
- `.mtf` remains the Moth-aware content format. Use `.mtf` when content needs `$md`,
  nested templates, or the restricted same-directory module-root constant scope.

### If Statements and Pattern Matching

Statement `if` is non-exhaustive. It has no statement-level `else if`; use nested `if` or full match.

```moth
if value is true:
    io.line("then")
else
    io.line("else")
;
```

Full pattern matching uses `if <value> is:` and is exhaustive.

```moth
if value is:
    < 0 => io.line("negative")
    0 => io.line("zero")
    <= 10 => io.line("small")
    else => io.line("large")
;
```

Rules:
- Arms are delimited by the next line-initial arm, `else =>`, or the final `;`.
- Per-arm semicolons are invalid.
- Arm headers must start at the beginning of a logical line.
- Arm forms are `<pattern> => <body>`, `<pattern> if <Bool> => <body>`, `else => <body>`, and bodyless `else =>`.
- In statement matches, bodyless `else =>` catches all remaining cases and executes no statements.
- Non-choice scrutinees require `else =>`, except a full option match with unguarded `none` and present-value capture arms.
- Choice scrutinees require either `else =>` or coverage of every variant.
- Any guarded choice arm requires `else =>`.
- The same choice variant cannot be matched more than once.
- The catch-all default arm is only `else =>`. `_ => ...` is not a wildcard pattern.
- `else => _` is invalid; use bodyless `else =>` for an explicit no-op fallback.

A statement match can use an empty fallback arm to explicitly ignore all remaining cases:

```moth
if value is:
    0 => io.line("zero")
    else =>
;
```

Empty `else =>` is for statement matches. Value-producing matches must still produce the required `then` values on every selected path.

Supported patterns:
- literals: `1`, `"ok"`, `true`
- choice variants: `Ready`, `Status::Ready`
- choice payload captures: `Err(message)`, `Pending(retry_count, message)`
- renamed payload captures: `Err(message as error_text)`
- relational scalar patterns: `< 0`, `<= 10`, `> 0`, `>= 100`

Payload capture names must match declared field names unless renamed with `as`. Payload exhaustiveness is tag-level. Relational patterns support ordered scalar scrutinees: `Int`, `Float`, and `Char`. Nested choice payload patterns are deferred.

## Loops

Moth has one loop keyword: `loop`.

```moth
loop condition:
    ...
;

loop items |item, index|:
    ...
;

loop 0 to 10 by 2 |i|:
    ...
;
```

Forms:
- conditional loop: repeats while a `Bool` condition is true;
- collection loop: yields item and optional zero-based index;
- range loop: yields counter and optional zero-based index.

Range rules:
- `to` is exclusive; `to &` is inclusive.
- `loop to n` is sugar for `loop 0 to n`.
- Direction is inferred from bounds.
- Without `by`, ascending ranges use `+1`; descending ranges use `-1`.
- With descending bounds, `by` is treated as a magnitude and the compiler applies the sign.
- `by 0` is invalid.
- Float ranges are supported, but `by` should be treated as required to avoid ambiguous or non-terminating loops.
- Runtime range counter updates are checked numeric operations. Their failures follow the enclosing
  numeric failure mode. Const range loops diagnose statically known counter failures during AST
  folding.
- Bindings use `|...|` after the loop source and may be omitted when unused.

```moth
loop 0.0 to 1.0 by 0.1 |t|:
    io.line([t])
;
```

## Structs and Receiver Methods

Structs are nominal runtime types. Matching field shapes do not make two structs interchangeable. Type aliases to structs are transparent and do not create new struct identity.

```moth
Vector2 = |
    x Float,
    y Float,
|

reset |this ~Vector2|:
    this.x = 0
    this.y = 0
;

vec ~= Vector2(x = 12, y = 87)
~vec.reset()
```

Receiver method rules:
- A receiver method is a top-level function whose first parameter is named `this`.
- `this` is reserved and may appear only as the first receiver parameter and inside that method body.
- There may be exactly one `this` parameter.
- Supported source-authored receiver types are user-defined structs and choices declared in the same source file as the receiver method.
- Aligned declaration-site generic nominal receivers are valid when the method belongs to the same generic type declaration.
- Source-authored receiver methods for built-in scalars, imported source types, dependency package types, external opaque types, and types declared in another file are rejected.
- Compiler-owned collection operations and compiler/builder-owned builtin operations are not user extension methods.
- `this T` is immutable; `this ~T` is mutable.
- Mutable receiver calls require explicit mutable/exclusive receiver syntax: `~value.method(...)`.
- Receiver methods are called only with receiver syntax; `method(value, ...)` is invalid.
- Mutable receiver methods require a mutable place receiver; temporaries and rvalues cannot be mutated.
- Field writes follow the same mutable-place rule.

Receiver method visibility is tied to receiver type visibility.

- A source-authored receiver method is visible wherever its receiver type is visible.
- Receiver methods are not imported, aliased, or re-exported independently.
- Namespace imports may make the receiver type visible, but receiver methods are not namespace fields.
- A module root exposes a type's same-file receiver methods across the boundary when its `export:` block exposes the type.
- Use free functions for private helpers or operations on types owned by other files or packages.

## Traits

Traits are explicit nominal method contracts.

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

Rules:
- Trait names use all-caps identifiers.
- `TRAIT must:` declares a top-level trait contract.
- Trait requirements are method signatures only; marker traits with no requirements are valid.
- Requirement receivers use `This` or `~This`, not lowercase `this`.
- Bare `This` and `~This` are receiver-only and valid only as the first requirement parameter.
- Direct non-receiver `This` parameters must be named, for example `other This`.
- Direct return `This` is supported.
- Composed `This` forms such as `This?`, `{This}`, and `Box of This` are rejected in the current source surface.
- `This` is trait-local syntax and is rejected outside trait declarations.
- `Type must TRAIT` declares explicit conformance. It is bodyless and newline-terminated; do not add a semicolon.
- A matching method without `Type must TRAIT` is not conformance.
- Conformance validates exact receiver mutability, non-receiver parameter modes/types, return types, and return channels. Parameter names do not matter.
- Canonical conformance evidence for same-file structs, choices, and generic type constructors is reusable wherever both the type and trait are visible.
- User-authored conformance for builtins, imported types, dependency package types, external opaque types, and types declared in another file is rejected.
- `TRAIT must not TRAIT, OTHER_TRAIT` declares narrow trait-incompatibility metadata. No concrete type may explicitly conform to both traits. The relation is symmetric, affects only conformance validation, and does not create negative conformance or trait composition.
- Trait bounds may enable calls on generic parameters inside the bounded generic declaration.
  Concrete values use ordinary visible receiver methods. A conformance declaration proves that a
  type satisfies a trait; it does not independently import trait methods as concrete receiver
  methods.

Traits are not value types. A trait name may appear in a trait declaration, an explicit
conformance declaration, or a generic bound. It is invalid as an ordinary variable, parameter,
field, return, collection element, choice payload, or alias target type.

Core cast traits such as `CASTABLE_TO_STRING` and `TRY_CASTABLE_TO_STRING` are compiler-owned
static traits. They resolve without imports, cannot be redeclared, imported, exported, aliased, or
shadowed, and cannot be used as ordinary values. The compiler registers builtin evidence for the
supported cast table, and same-file nominal source types, including aligned generic constructors,
can add user-authored evidence by declaring conformance to the relevant core cast trait. Infallible
and fallible cast trait pairs for the same target are incompatible through compiler-owned
`must not` metadata.

Use a generic bound for static reuse:

```moth
render type Item is DISPLAY_TEXT |value Item| -> String:
    return value.display()
;

show_text type Item is CASTABLE_TO_STRING |item Item| -> String:
    text String = cast item
    return text
;
```

Use a choice for runtime heterogeneity:

```moth
Renderable ::
    LabelItem | value Label |,
    ButtonItem | value Button |,
;
```

Static trait declarations, conformances, and generic bounds are frontend semantics and are backend-independent.

Deferred trait surfaces include static non-method requirements, compiler-owned builtin conformance
facts, and broader standard trait taxonomy.

Outside trait design scope: default methods, associated types/constants, inheritance,
aliases/composition, generic traits/methods, blanket/conditional/negative/specialized conformance,
structural conformance, dynamic trait values / trait objects, downcasting/reflection, output
coercion, operator integration, automatic primitive conformances, and derive/hash/order/format/
iteration/serialization trait families.

## Choices

Choices are nominal tagged unions. Variants are either unit variants or record-payload variants.

```moth
Result ::
    Ok,
    Err | message String, code Int |,
;

success = Result::Ok
failure = Result::Err(message = "bad request", code = 400)
```

Rules:
- Unit variants are constructed as `Choice::Variant`.
- Payload variants are constructed as `Choice::Variant(...)` with positional or named arguments.
- Payload fields are immutable after construction.
- Direct payload field access is deferred and should use pattern matching.
- Payload field mutation is rejected.

Structural equality is supported only when every payload field across every variant supports equality.

Supported equality payloads:
- `Int`, `Float`, `Bool`, `Char`, `String`
- choices whose payloads all support equality
- options whose inner type supports equality

Unsupported equality payloads:
- structs
- collections
- functions
- fallible result carriers
- external opaque types

## Generics

Generics are declaration-site type abstractions for top-level structs, choices, and free functions. Generic parameters are introduced with `type` after the declaration name.

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

Generic parameter rules:
- Names use type-name style.
- Parameters are scoped to the declaration.
- Parameters are compile-time placeholders, not runtime values.
- Every declared generic parameter must be used by the declaration.
- A parameter cannot collide with another parameter in the same declaration or with visible concrete types, type aliases, external package types, builtins, or other type-position names.

Generic function calls use normal call syntax only. Type arguments are inferred from immediate call arguments and, at closed receiving sites, the immediate expected result type.

```moth
value = identity(42)
typed_value Int = identity(42)

empty type A || -> {A}:
    return {}
;

items {Int} = empty()
```

Inference does not use later mutation, later use, whole-program analysis, HIR, borrow validation, or expected parameter context from an outer function call into a nested generic call. Use ordinary annotations to make nested calls explicit.

Unconstrained generic code can pass values through, return them, store them in generic structs/choices, forward them to other generic functions when immediate call evidence solves the parameters, and use generic parameters in local annotations.

Declaration-site trait bounds use `is`:

```moth
render type Item is DISPLAY_TEXT |item Item| -> String:
    return item.display()
;

render_pair type A is DISPLAY_TEXT, B is DISPLAY_TEXT |left A, right B| -> String:
    return [left.display(), right.display()]
;
```

Use `and` for multiple bounds on one parameter. Commas still separate generic parameters. `where` syntax remains rejected. Concrete generic calls and generic struct/choice instantiations require visible reusable evidence for each concrete type argument. Trait names cannot appear as value types, so there is no trait value that can satisfy or fail a static generic bound.

Operations that require behavior from an unconstrained generic type are rejected. Trait bounds currently enable unique bound-provided receiver calls. Arithmetic, equality/comparison, field access, template interpolation requiring string-like behavior, and external/IO behavior still require concrete type support or a future dedicated trait integration.

Concrete generic aliases are supported:

```moth
StringBox as Box of String
```

Name intermediate concrete aliases instead of writing nested `of` applications:

```moth
Pair type A, B = |
    first A,
    second B,
|

StringIntPair as Pair of String, Int
value Box of StringIntPair = Box(Pair("count", 3))
```

Rejected or deferred in the current source surface:
- explicit call-site syntax such as `identity of Int(42)`, `identity<Int>(42)`, `identity[Int](42)`, or `identity(42 Int)`
- inline generic sugar such as `|value type A|`
- receiver methods on concrete generic instances
- generic external package functions and generic external package types
- recursive generic types
- nested `of` applications except through concrete alias workarounds

Outside generic design scope:
- generic function values and higher-order polymorphism
- type values, type-returning functions, type-level `#if`, and compile-time type inspection
- `where` clauses and file-local evidence-backed generic-bound dispatch
- parameterized generic aliases and partial type application
- higher-kinded types, const generics beyond fixed capacity, lifetime parameters, specialization, and associated types

## Type Aliases

Type aliases give another compile-time name to an existing type. They are transparent and do not create nominal identity.

```moth
UserId as Int
Names as {String}
MaybeName as String?
StringBox as Box of String

import @types { UserId as ExternalId }
LocalId as ExternalId

id UserId = 42
raw Int = id -- valid
```

Aliases can target builtins, structs, choices, options, collections, fully concrete generic instances, imported types, and external package types.

Aliases are transparent type spellings, not constructors. Construct a nominal struct, choice or generic instance through its canonical nominal name.

## Module System, Config, and Imports

A module is a directory-scoped set of Moth source files compiled together. A directory becomes a module root when it contains one `@*.moth` or `+*.moth` file. The suffix after either marker is cosmetic. More than one root file in the same directory is rejected. A project contains normal modules, scoped support packages, an optional project package facade and other builder inputs.

### Project config

`config.moth`:
- lives at the project root;
- uses normal declaration syntax;
- is one self-contained compile-time source file with no source imports or package resolution;
- accepts one required open `project` const record, private helper constants declared before use, and top-level builder and tooling section records;
- may contain folded scalar values, optionals, nested anonymous const records, collections and folded template strings as project fields;
- requires `project.name` as a valid package-style identifier for stable project identity;
- rejects every source import including relative, project, Core, Builder, dependency and binding-backed imports;
- rejects runtime declarations, mutable bindings, functions, named support types, traits, conformances, standalone templates, `#[...]` page fragments and `export:`.

The command selects the builder before config schema validation. `config.moth` does not select the builder.

Only active sections are schema-validated. Inactive sections are parsed and folded but discarded. The active builder project section is required, even when empty.

Builder sections use backend-neutral folded values and cannot declare `#Import`. Output settings are builder-owned.

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

Direct project `#Import` fields use the accepted primitive and optional domain: `String`, `Int`, `Float`, `Bool`, `Char` and optional forms. Nested project fields cannot declare imports.

See `docs/build-system-design.md` for the full config, `@project` and source `#Import` contract.

### Entry-local config blocks

A normal module root may contain one optional `config:` block for root-local builder metadata. It is not an embedded `config.moth` file.

```moth
config:
    html #= |
        title = "Docs",
        description = "Documentation pages",
        head = default_head,
    |
;
```

Rules:
- valid only at the top level of a normal module root
- at most one block per normal root
- invalid in normal non-root files, support roots, the project package facade, inside `export:`, inside executable bodies and in `config.moth`
- contains section records only: no imports, aliases, helper constants, support types or `#Import` declarations inside the block
- these live outside the block in the normal root file
- uses the root file's ordinary compile-time visibility: imported constants, `@project`, same-file constants declared before the block and resolved source `#Import` constants are available through normal visibility
- same-file forward references remain invalid
- creates no ordinary module symbol, HIR or project-global value
- cannot contain `project` or change project-level builder behaviour
- active entry sections are schema-validated; inactive sections are folded but not validated
- every normal module selected into the command's semantic graph has its block validated whether or not an entry activates it
- imported modules never apply their entry metadata to an importer

See `docs/build-system-design.md` "Entry-local config: blocks" for the full contract.

### Import syntax and rules

```moth
import @path/to/file
import @core/math as math
import @vendor/drawing.js as drawing

import @path/to/file {symbol}
import @path/to/file {symbol as local_name}

import @components {
    render as render_component,
    Button as UiButton,
    Card,
}

import @docs {
    pages/home {render as render_home},
    pages/about {render as render_about},
}
```

Rules:
- Project source imports resolve from the importing file's owning module root, not the file's directory.
- `@./...` has no supported meaning and paths containing `..` are rejected.
- Imports target exported symbols, not file-level start functions.
- Source namespace imports create shallow, field-access-only import records.
- External package namespace imports can expose nested package-local symbol paths such as `io.input.*`.
- Import records are not first-class values.
- Import source child paths directly. Do not traverse source path segments through namespace fields.
- Ordinary import aliases are file-local, cannot be independently re-exported, and must not collide with visible names.
- An alias authored inside a grouped re-export in `export:` defines the public API name for that exported symbol.
- Alias leading-case mismatches warn.

```moth
import @some/path { Symbol as LocalName }

export:
    import @some/path { Symbol as PublicName }
;
```

`LocalName` is file-local. `PublicName` is the exported API name created by the grouped re-export.
- Grouped imports cannot use a trailing group-level alias.
- Direct symbol-path imports such as `import @core/math/sin` are invalid.
- `.moth` source imports are extensionless.
- Direct project/local JavaScript imports require `.js` and a builder `.js` external import provider.
- Invalid namespace path stems require explicit aliases.
- Normal module-root files have no direct import form. Import the directory's public module surface.
- `@@name` is invalid. The leading `@` in `import @path` starts an import path and is not part of the module name. A second `@` in any path component is rejected.
- Direct imports of `+*.moth` support root files and `config.moth` are invalid.

### Module roots, runtime, public APIs and packages

| File/root | Role |
|---|---|
| one `@*.moth` file per normal module directory | Normal module root with a cosmetic filename |
| one `+*.moth` file per support directory | API-only scoped package root named by its directory |
| optional project-root `+*.moth` beside `config.moth` | API-only external package facade named by project config |
| entry-selected normal module | Owns active top-level runtime/start code and direct page fragments |
| imported normal module | Provides only its `export:` public surface and never executes root runtime |
| implicit `start` | Contains active-root top-level runtime code; build-system-only; not importable |
| normal `.moth` files | Declarations only; no top-level executable statements |
| `config.moth` | Not a module and cannot be imported directly; Stage 0 derives the synthetic `@project` interface from its folded `project` record |

Execution and visibility:
- An entry assembly activates one normal module's top-level runtime code and page fragments exactly once.
- Other files contribute declarations that must be imported explicitly.
- An imported normal module never replays its top-level runtime code or page fragments in the importer.
- Support modules and the project package facade have no implicit `start`, top-level runtime work, page fragments, routes or builder artifacts.
- A normal module root may contain private imports, private declarations, one strict `export:` block, runtime start code and direct page fragments.
- A support root may contain private imports, private declarations and one strict `export:` block. Runtime work and fragments are rejected.
- Declarations outside `export:` stay private to the module. A root with no `export:` exports nothing.
- Grouped imports inside `export:` re-export symbols, and grouped aliases define the public API name.
- Public APIs must not expose private root-only types in signatures, fields, aliases, generic bounds, or exported constant types.
- Receiver methods are visible across a module boundary when `export:` exposes the receiver type's source surface. Type aliases do not automatically re-export private implementation methods.
- A support package is visible only within its nearest ancestor normal module's subtree. It is visible to that owner, normal sibling modules and their descendants, but not above the owner, outside its subtree or from another support package in the same scope.
- A support package is unavailable inside its own private implementation subtree.
- A project package facade is not visible to the project's internal modules. It may assemble public descendant surfaces below `entry_root` for external consumers.
- API-only roots produce no HTML artifact.

Import resolution:
- Every source import starts from the importing file's owning module root.
- A normal module may import files and unrooted directories it owns, direct child normal modules, support packages visible in its lexical scope and registered packages.
- A normal module may not import its parent, an ancestor, a normal sibling, a grandchild directly, a sibling's descendant, an unrelated branch or another module's private file path.
- Reaching a child module or support package ends filesystem traversal and exposes only its `export:` facade. Imports cannot bypass that facade with paths such as `@child/internal`.
- A child module re-exports anything its parent should see from deeper descendants.
- A support root may import files it owns, descendant modules in its private subtree, support packages from a strictly outer scope and registered packages. It may not import its parent, normal sibling consumers or same-scope support siblings.
- The project package facade is the only assembly exception. It resolves project paths from `entry_root` and receives only descendant public interfaces.
- `@./...`, parent components and importing-file-relative resolution are unsupported.
- Extensionless `.moth`, `.mtf`, `.md` and directory identities share one strict namespace. Same-name files, directories, modules and visible packages are rejected instead of resolved by precedence.
- Explicit-extension provider files such as `.js` may coexist with a same-stem directory only when syntax remains unambiguous.
- Grouped imports expand into individual symbol imports.
- Same-module file cycles are accepted when declarations resolve through the dependency graph. Circular compile-time constant dependencies are rejected. Cross-module and package visibility still applies.

Package metadata has two orthogonal axes:

| Package | Origin | Backing |
|---|---|---|
| `@html` | Builder | MothSource |
| `@core/collections`, `@core/io`, `@core/math`, `@core/text`, `@core/random`, `@core/time` | Core | ExternalBinding |
| `@web/canvas` | Builder | ExternalBinding |
| scoped `+*.moth` package | ProjectLocal | MothSource |
| project-root package facade | ProjectLocal | MothSource |
| annotated project-local `.js` import | ProjectLocal | ExternalBinding |

`Standard` and `Dependency` are valid origins even when no current package uses them. The prelude is implicit-import policy, not a package origin or backing. It exposes the bare `io` namespace as an alias to `@core/io`.

Core packages require explicit imports unless they are part of the prelude. Unsupported builder packages are rejected with an unsupported-by-builder diagnostic. Source-backed packages expose compiled immutable interfaces backed by support roots, the project package facade or builder-supplied source.

The HTML builder's `@html` source-backed package exposes authored HTML helpers, including `canvas`, `get_canvas_context`, `Canvas`, and `get_canvas`. Its cosmetic root filename is currently `packages/html/@mod.moth`, but its public API comes from the root's `export:` block. `Canvas` is a source-owned wrapper around the raw external context, so method-style calls such as `~drawing.fill_rect(...)` come from ordinary Moth receiver methods rather than external package metadata. The raw `@web/canvas` symbols themselves are not re-exported through `@html`. Import raw drawing APIs directly from `@web/canvas` when needed.

### External platform package imports

Project builders may provide binding-backed packages such as `@core/io`, `@core/math` or `@web/canvas`. These aren't Moth source files. They expose opaque external types, compile-time constants and external free functions only.

```moth
import @core/math
import @core/math { sin as sine }

value = math.sin(1.0)
aliased = sine(1.0)
```

Rules:
- The progress matrix is the source of truth for supported external packages and backend targets.
- For normal builds, the `io` namespace alias and compiler-owned `Error` are available without explicit imports.
- Prelude external symbols do not override source declarations or explicit imports.
- Explicit external imports must not collide with visible source symbols.
- External aliases follow normal file-local collision and case-convention rules.
- External opaque types can be passed, returned, and used by external functions, but cannot be constructed with struct syntax or field-accessed.
- External packages do not expose receiver methods. Use free functions for raw external APIs, or a source-owned wrapper type when method-style ergonomics are wanted.
- Existing Core, Builder and JavaScript-backed bindings use a restricted host-binding profile: ordinary Moth values cross by value, host code may not retain references into ordinary Moth storage, and opaque handles represent foreign identities rather than Moth reference types.
- Observable external resources require explicit close or teardown. Host finalization timing must not define language behaviour.
- Future general Wasm library imports use a value-only WIT component profile. Supported arguments lower from shared reads into independent component values and results lift into independent Moth result graphs. No Moth alias, lifetime owner, ownership state or destruction responsibility crosses the component boundary.
- WIT resources, callbacks, async operations, futures, streams, shared-memory views, raw pointers, returned aliases and retained Moth references are deferred beyond the V1 value-only profile.

Initial optional core packages:
- `@core/math`: `PI`, `TAU`, `E`, and `Float` math helpers.
- `@core/text`: `length`, `is_empty`, `contains`, `starts_with`, `ends_with`. `text.length` counts Unicode scalar values, matching `Char`, which represents one Unicode scalar. The current JS lowering uses UTF-16 code units and does not yet match the accepted contract.
- `@core/random`: `random_float`, `random_int`; `random_int(min, max)` is inclusive at both ends and swaps bounds when `min > max`; seeded random is deferred.
- `@core/time`: opaque `Duration`, `TimeMark`, and `Timestamp` types; monotonic `mark_now`, `elapsed_since`, and `duration_between`; duration construction/conversion helpers; Unix timestamp construction/conversion helpers; and fallible ISO timestamp parsing/formatting.

Time package split:
- Use `TimeMark` for elapsed time and frame deltas.
- Use `Timestamp` for real-world UTC instants.
- Use `Duration` for elapsed amounts.
- `timestamp_from_iso_string` is fallible and must be handled with postfix `!` or `catch`.

The HTML builder supports annotated single-file `.js` imports through `@moth.opaque` and `@moth.sig`. `@moth.sig` declares each exposed function. `@moth.opaque` is required only when signatures use foreign opaque handle types. A function-only module may use `@moth.sig` without `@moth.opaque`. JavaScript export names are runtime implementation details. Moth names come from annotations. Supported JS export forms are `export function name(...) { ... }` and block-bodied arrow exports. `@moth.sig` annotations expose free functions. `this` receiver-style signatures are rejected during registration. Runtime imports from builder-registered modules must be named static imports. The exact signature subset: parameters are positional and typed, mutable parameters use `name ~Type`, signatures have zero or one success return, an optional final `Error!` channel is allowed, `Error!` must be final, generic signatures are rejected, option and collection types are rejected, callbacks are rejected, multi-success returns are rejected, `this` receiver-shaped signatures are rejected for external package registration, opaque types must be declared through `@moth.opaque` before use where required, JavaScript export names are runtime implementation details, and Moth names come from annotations. Unsupported JS features include arbitrary dependency graphs, default exports, re-exports, CommonJS, classes, JS constants, property accessors, callbacks, async functions, collections/options in JS signatures, generic external types, receiver methods, and multi-success JS returns.

Deferred package-system features:
- package manager, versions, remote fetching, lockfiles, and override/shadowing rules
- persistent canonical module artifacts and dependency-package caches
- user-authored external binding files
- wildcard imports/exports and namespace exports
- automatic docs/API extraction from module-root `export:` blocks
- seeded random, full date/time/time-zone/calendar APIs, Temporal-backed calendar implementation, locale-aware formatting/parsing, local time-zone lookup, async timers/sleep/intervals, browser animation scheduling packages, and non-JS lowerings for JS-backed core packages

Moth `$md` links use `@path (label)`. Plain Markdown `.md` files use ordinary `[label](path)` links. Plain Markdown href and src values are rendered literally rather than rewritten as tracked assets.

### Binding Modes, Top-Level Declarations, and Constants

Binding forms:

```moth
name = value
name Type = value

name ~= value
name ~Type = value

name #= value
name #Type = value
names #{String} = {"Ana", "Bo"}
maybe_name #String? = none
```

Rules:
- `#` marks compile-time constants; it does not control visibility.
- Top-level functions, structs, choices, type aliases, and `#` constants in ordinary files are importable inside the same module by default.
- Cross-module visibility is controlled by the nearest module root's `export:` block.
- Runtime top-level bindings and expressions are start-body code, not importable declarations.
- `#[...]` is active-module-root-only top-level const-template syntax. It must fully fold and may contribute compile-time page fragments.

Top-level dependency ordering includes constants, type aliases, structs, choices, function signatures, and type annotations. Executable body statements do not affect top-level declaration order.

Constant rules:
- Must be initialized.
- Cannot be mutable.
- May reference only constants.
- Must fully fold at compile time.
- Same-file constant evaluation follows source order.
- Cross-file constant dependencies are resolved in dependency order.

```moth
site_name #String = "Moth"
major_version #Int = 1
full_name #= [: [site_name] v[major_version]]
```

Struct instances assigned to constants can become data-only const records when all constructor arguments fold. Const records are field-access-only compile-time member groups. They cannot be assigned to runtime values, passed, returned, placed in collections, or used through runtime methods.

```moth
Basic = | defaults String |
values #= Basic("Only allowed const values here")

label = values.defaults -- valid
io.line(values.defaults)     -- valid: use a field
```
