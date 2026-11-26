# Unified Bracket Construction for Moth

**Status:** Design proposal for review. The decisions in this document reflect the design discussion, but the redesign as a whole is not yet adopted or scheduled.

**Date:** 26 November 2026. The bracket review is dated 25 November. The agreed surface syntax below records the subsequent design decisions.

**Repository evidence:** The review independently verified `main` at `d6b323082777e73663c8eacd6e03852f97d7e769` and `docs/const-foundation-profile-independent-semantics` at `e235a04c5d282e6cd41c0ec86b77a31c60055101`. These are review anchors, not a future implementation baseline. The branches have different const-evaluation and roadmap contracts (Part II). Reconfirm both on activation.

**Scope:** Collection and map syntax, String templates, typed sequence construction, slots and wrappers, constant evaluation, semantic interfaces, memory semantics, backend lowering, MON and the affected language and package documentation.

## Purpose

Unify String templates, collection literals and map literals around square brackets. Ordered construction gains typed splicing, structural control flow and slots without creating a new runtime value category. Maps keep their separate keyed semantics.

The motivating uses are binary data, ordinary sequence construction and package-defined UI trees. A package can choose any HTML component representation and explicitly render it to String. The compiler must not privilege a particular component system.

Square brackets replace the collection and map delimiters in Moth and MON. Moth uses the freed braces for blocks and choice declarations. Braces in raw template text, quoted Strings, comments and embedded languages retain their literal meaning.

The following decision record fixes the surrounding source syntax. The companion document Moth Surface Syntax After Unified Bracket Construction (`moth-surface-syntax-design.md`) gives the full control-flow, constructor and channel examples. Parts I to III cover bracket construction and its cutover. Section 34 identifies the remaining bracket questions. Implementation remains unscheduled.

## Agreed surface syntax (26 November 2026)

Moth reuses a small set of forms across the language. A field list describes what a constructor accepts. Constructor arguments supply those fields. A pair of braces encloses a definition or an executable block, which can contain several lexical scopes. Square brackets construct text, sequences and maps under the rules in Part I.

### Named structs and choices

A named struct has one constructor named after its type. A choice declares its constructors inside braces. Each payload constructor uses the same typed field-list syntax as a struct.

```moth
Size = | width I32, height I32 = 720 |

WindowEvent = {
    Opened,
    Resized | width I32, height I32 = 720 |,
}

size = Size(width = 1280)
event = WindowEvent::Resized(width = 1280)
```

Both values in this example have a height of 720. `Size(...)` produces a `Size`. `WindowEvent::Resized(...)` produces a `WindowEvent`. An explicitly declared choice retains its variant names even when it has just one variant. A unit variant such as `WindowEvent::Opened` takes no parentheses. A variant that declares fields uses parentheses even when every field has a default and its argument list is empty.

Each field has a unique name within its constructor, an explicit type and an optional default that must fully evaluate at compile time against that type. Positional arguments fill fields in declaration order. Named arguments follow and may skip a defaulted field. Each field can be supplied once. An explicit argument supplies its value, an omitted defaulted field receives its default and an omitted required field is an error. The same rules apply to struct fields and choice payload fields.

Patterns inspect the completed value. For example, `case Resized(width, height):` binds both fields even if the constructor call omitted `height`. Sharing field declarations does not introduce structural conversions, writable choice payloads or new equality rules. Ordinary record values and calls retain their parenthesised forms.

MON follows the same omission rule for literal data. With the corresponding prepared schema, `Size(width = 1280)` and `WindowEvent::Resized(width = 1280)` both materialise the default height. The codec uses the validated default value in its schema. It never executes Moth code. Optionality alone does not supply a default.

The existing `Route` contract determines when input arrives. Ordinary construction and a zero-input route use complete-constructor argument routing. For a one-input route, the captured arguments fill the required contiguous leading prefix and the future input supplies the trailing field, even if that field declares a default. For example, at a `Route of I32 -> WindowEvent` parameter, `WindowEvent::Resized(1280)` captures the width and leaves the height for the event. [Route construction](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/wiring/wiring.mtf#L151-L215)

### Blocks, matches and regions

A function or control-flow block uses one brace pair. A two-branch `if` keeps bare `else` inside that pair. A full match uses `if value is {`, line-initial `case pattern:` arms and `else:` for the fallback. The existing exhaustiveness rules determine when a fallback is required. A case pattern's captures are visible in its guard and body. The next arm starts a sibling scope.

```moth
describe |event WindowEvent| -> String {
    return if event is {
        case Opened: then "opened"
        case Resized(width, height): then [: [width] x [height]]
    }
}
```

A declared region uses its name followed by a braced body, such as `scratch { ... }`. A declaration with `into scratch` places eligible storage there. Every exit from the region ends that lifetime, including an early return. No `drop` keyword is introduced. Braces replace the ordinary semicolon closer. A colon still separates a bracket head from its construction body.

### Collections and channels

A fixed collection type always states both its capacity and its element type, as in `[8 I32]`. Section 8 gives the capacity rules and section 27 shows the migration from capacity-only declarations.

Channels V1 retains prefix `>> worker(writer)` for child-task dispatch, infix `message >> writer` for send and prefix `<< reader` for receive. Ordinary binding gives `message = << reader`. Expressionless `if { ... }` uses `case operation:` for readiness arms and `else:` for its optional immediate fallback. This replaces the arrow-labelled readiness arms in the earlier Channels V1 design. This reshuffle retains `to` and inclusive `to &` for ranges. [Channels V1 design](https://github.com/nyejames/moth/blob/f8e1f585573c9d3939b48e89b6644a75784c98c0/docs/docs/async/channels.mtf)

## Part I - Language rules and worked examples

### 1. One construction surface

Square brackets describe sequence and map types in type position. In value position they construct ordered output or keyed data. A template is the compositional form of bracket construction, not a separate runtime value category.

| Existing form | Proposed form | Meaning |
|---|---|---|
| `{I32}` | `[I32]` | Growable collection of I32 |
| `{8 I32}` | `[8 I32]` | Fixed-capacity collection of at most eight I32 values |
| `{{I32}}` | `[[I32]]` | Collection of collections |
| `{String = I32}` | `[String = I32]` | Map from String to I32 |
| `{1, 2, 3}` | `[1, 2, 3]` | Construct a collection of three integers |
| `{=}` | `[=]` | Empty map, with key/value types supplied by context |
| `[: Hello]` | `[: Hello]` | Construct a String from text |

The spelling changes, not the identity of the existing semantic collection or map shapes. Fixed capacity remains part of type identity, not an allocation hint or an exact length. String remains its own semantic type, not an alias for `[Byte]` or `[Char]`.

```moth
numbers [I32] = [1, 2, 3]
names [String] = ["Priya", "Rob"]
bytes [Byte] = [137, 80, 78, 71]
fixed [8 I32] = [1, 2, 3]
message String = ["Count: ", 42]
```

Mutable declarations, access markers, collection operations, aliasing and generic type contracts retain their established meanings. Migrate their type spellings rather than inventing new operations.

```moth
numbers ~[I32] = [1, 2]
~numbers.push(3)
```

Examples use proposed syntax unless marked as old source. Each snippet has its own scope; repeated names in comparison examples are independent declarations. They are design examples, not runnable fixtures against the baseline compiler.

### 2. Type evidence and receiving contexts

The compiler determines the construction type from known semantic evidence. It must not try several interpretations until one type-checks, infer a type from a desired conversion or choose a type based on the selected backend or profile.

An eligible immediate receiver (binding, assignment, parameter, return, field/default or collection entry) can guide bracket construction. It cannot reinterpret an existing value.

```moth
set_title(["Moth", " Patterns"]) -- String parameter
set_tags(["Moth", "Patterns"])  -- [String] parameter

numbers = [1, 2]
set_title(numbers)                -- invalid: numbers is [I32]
```

An inferred binding fixes its construction kind and nesting shape at declaration. Later uses cannot change them. Existing body-local numeric-only inference may still resolve raw numeric leaves under its own rules.

A type-specific directive may supply evidence directly. For example, `$md`, `$html` and `$css` establish a String construction context. Conflicting evidence is a diagnostic rather than a conversion. A known wrapper supplies the receiving contract for its contributions and establishes its output type.

In the absence of a stronger receiving contract:

- A non-empty raw textual body establishes String.
- A head consisting of String values constructs String by concatenation.
- Homogeneous non-String values establish a collection of their element type.
- An existing sequence supplies sequence evidence and its element type. Compatible scalar contributions can extend that sequence. For example, a `[String]` collection plus a String constructs `[String]`, rather than concatenating into text.
- Explicit nested head constructions establish collection nesting, including when their leaves are Strings.
- A direct map entry establishes map construction.
- Incompatible or insufficient evidence requires a diagnostic. There is no default that converts a heterogeneous head into String.

```moth
numbers = [0, 1, 2]             -- [I32]
flags = [true, false]           -- [Bool]
text = ["Hello", " world"]     -- String
another = [: Hello world]       -- String
mixed = [1, "hello"]           -- diagnostic

names [String] = ["Priya", "Rob"]
more = [names, "Emmy"]          -- [String]
```

Automatic value-to-text coercion is permitted only once a String context is established. Existing formatting and conversion eligibility still applies. A String receiver does not imply arbitrary collection-to-text conversion or a new cast contract.

```moth
number = [42]                   -- [I32]
label String = [42]             -- "42"
article = [$md: There are [42] items.]
```

A non-String head combined with raw body text is not a licence to infer String and retroactively stringify the head. The author must establish a String context explicitly (through a receiver or appropriate directive). Thus `something = [0, 1, 2: 3, 4, 5]` is ambiguous and rejected. An explicitly String-typed form still has to satisfy the normal head/body and wrapper grammar. Do not silently redefine the colon as a general concatenation operator.

Raw numeric literals receive exact element types (including Byte and U64) through existing numeric rules. Already typed values are not widened to manufacture homogeneity.

### 3. Empty constructions

All empty bracket constructions require type evidence. A colon on its own supplies no evidence. This deliberately rejects the tempting default of interpreting an untyped empty bracket as an empty String.

```moth
value = []                     -- diagnostic
text = [:]                     -- diagnostic
text String = []               -- empty String
other String = [:]             -- empty String
text_literal = ""              -- inferred empty String
labels [String] = []           -- empty collection
matrix [[I32]] = []            -- zero rows
one_row [[I32]] = [[]]          -- one empty row
unknown = [[]]                 -- diagnostic: unknown element type
rows = [[], [1, 2]]             -- [[I32]], first row is empty
```

A concrete parameter, return or field receiver can supply evidence, as can a compatible directive, the entry sink or a wrapper's slot schema. A known optional receiver may supply its underlying construction type only through the established presence-wrapping rule: empty output is not `none`. An unfilled slot and an open wrapper without fill are structural cases, not untyped empty literals. In a nested construction, a non-empty sibling row can supply the element type for an empty row regardless of source order. This is local construction resolution, not a general permission to infer an earlier declaration from later uses.

An empty map uses `[=]`. The delimiter establishes the kind but not the key or value types. Preserve the distinction between `[]` and `[=]`, including at a typed map receiver; a collection is not silently converted to a map. A typed empty collection must remain distinct from an empty String and from an empty map. Empty values are also distinct from structurally absent output in template composition.

### 4. Heads and bodies

Heads contain ordinary expressions, directives and, under this proposal, explicit nested sequence constructions. Bodies retain their template role. Raw body content is textual and is only permitted when constructing String. In a non-String construction, a body may contain nested constructions and structural control flow, with formatting whitespace ignored. It is not a second expression parser.

```moth
invalid [I32] = [: 0 1 2 3 4]  -- diagnostic at raw body text
invalid [String] = [: Home Docs] -- diagnostic: this is a collection receiver
labels [String] = ["Home", "Docs"]

numbers [I32] = [:
    [0]
    [1]
    [2]
]
```

A textual child can deliberately construct a String to contribute as one element of `[String]`. Its own text context is distinct from that of the outer collection.

```moth
greetings [String] = [loop names |name|:
    [: Hello, [name]!]
]
```

The enclosing body collects the String result of each child. A raw word outside a child is still invalid. Formatting-only whitespace is discarded in a typed sequence body, not in a String body. A whitespace-only body with no other evidence needs a precise inference rule (section 34); do not strip whitespace from a known String construction. A textual child in a sequence body is one String contribution, whereas direct head nesting establishes a collection dimension. The same punctuation in different grammar positions is not a global nesting heuristic. No raw-body token sequence should be reinterpreted as numbers, field expressions or application-specific markup because of its receiver type. The lexer must not need semantic type information to decide whether raw body characters are code.

Directives and child constructs keep their grammatical roles. A bracket expression used as a directive operand or a normal function argument is not an explicit nested row merely because it is lexically inside another pair of brackets. The parser owns these boundaries.

### 5. Splicing, nesting and collection identity

Concatenation is fundamental to ordered construction. A compatible existing sequence contributes its elements in order, while a compatible scalar contributes one element. Splicing is exactly one level, not recursive flattening.

```moth
first = [1, 2]
second = [3, 4]

flat = [first, second]         -- [1, 2, 3, 4]
extended = [first, 3, 4]        -- [1, 2, 3, 4]
rows = [[first], [second]]      -- [[1, 2], [3, 4]]
one_row = [[first, second]]     -- [[1, 2, 3, 4]]
```

A direct nested bracket construction in a sequence head preserves one additional collection boundary. This applies at any supported depth. It is not a special runtime marker or an implicit deep copy. The nested head syntax replaces the suggested `$item` directive, which is unnecessary.

A bracketed child in a body continues to contribute its constructed output. A child loop can therefore emit many items into its parent's sequence. Head nesting and body composition must remain distinguishable in the parser and the semantic representation.

```moth
groups [[String]] = [["Home", "Docs"], ["API"]]
```

The nested row establishes a collection context for its contents, so the Strings stay separate. The implementation must not first default the row to String and subsequently convert it into a collection.

For concrete code, an annotation does not turn an ordinary splice into a singleton stored collection. `matrix [[I32]] = [first, second]` is therefore invalid for `[I32]` operands; write the nested constructions or use ordinary insertion. `matrix [[I32]] = [another_matrix]` is valid and retains the existing row references, with fresh outer membership. A direct nested head row is never reinterpreted as text just because all of its leaves are Strings. To construct a fresh row, use `[[row]]`. To retain an existing row value as one element, use ordinary collection insertion (or the generic single-element contract described below). A splice of an existing matrix into another matrix splices its rows, not their elements.

```moth
row ~[I32] = [1, 2]
constructed [[I32]] = [[row]]
stored ~[[I32]] = []
~stored.push(row)
```

`constructed` owns a fresh inner row. `stored` retains the existing row under ordinary stored-reference semantics. The fresh row has independent membership and storage, while its reference-valued elements retain their normal sharing relationships. Splicing a sequence is shallow construction, not `copy`. Deep independent duplication still requires the existing explicit copy operation where legal. Optimisation may coalesce allocations only when the difference is unobservable and the validated memory contract permits it.

This also applies to fixed-to-growable splices and sequences containing handles or resource-bearing Strings. Preserve source element order, reference provenance and lifetime obligations. Do not copy opaque browser-owned objects or resolve structural URLs as a side effect of construction.

### 6. Evaluation order and failure

Evaluate contributions in authored order. A scalar or sequence expression is evaluated exactly once at its written position. A splice consumes the source membership and element values at that position under ordinary collection access and retention rules. A later contribution must not change the earlier contribution by mutating its source before the earlier read has happened.

For example, assume `update_and_return` mutates its collection argument and returns an I32:

```moth
items ~[I32] = [1, 2]
result [I32] = [items, update_and_return(~items)]
```

If the accesses are legal, the earlier splice contributes the original membership before the later call. Retained aliases and overlapping mutations remain subject to borrow and lifetime validation. This is not permission to bypass access checks by making a hidden copy.

The const-evaluation branch and its BC-06 HIR capture investigation establish a relevant order requirement: an explicit `copy` used as a call argument reads its source at its written position, before a later argument can mutate it. Bracket construction must preserve the same rule for `copy` and for ordinary contributions. A length prepass, reservation strategy, expression staging or bulk append must not defer that read, re-evaluate an operand or change its failure order.

A splice's length must be observed at the same semantic capture boundary as its membership. It is not sufficient to save a reference and its length, then reread its elements after later effects. Prefer direct append into the destination as each contribution is consumed. Distinguish storing shared element references from snapshotting the source's membership; a later legal mutation to a shared referent still follows ordinary alias semantics. This is not a universal deep-snapshot rule.

Capacity estimates may use already known static bounds or safely available evaluated values. Dynamic length discovery may capture the result of an operand once and reuse it. It may not call an operand twice, even when that operand is eligible for constant evaluation. Preallocating by evaluating all operands before copying their elements is invalid when later operands can affect earlier sources.

Preserve ordinary source errors, `Error!` propagation, assertion behaviour and allocation/resource-failure contracts. A failed construction must not publish an incomplete new value or partially write its receiving binding. Earlier explicit effects are not rolled back. Do not manufacture a new failure channel for bracket syntax. Template conditional bodies remain lazy at execution even where all authored branches require static validation.

### 7. Generic construction

A generic body decides whether a contribution stores one element or splices a sequence using its declared types and receiving contracts before specialisation. Specialising a generic must not change the operation's cardinality or introduce a String interpretation.

| Generic parameter types | Receiver | Meaning of `[left, right]` |
|---|---|---|
| `left T`, `right T` | `[T]` | Store two elements |
| `left [T]`, `right [T]` | `[T]` | Splice two sequences |

When `T` is `[I32]`, the first form stores two existing rows under ordinary sharing rules. The second receives two matrices and concatenates their rows. When `T` is String, the first form still stores two Strings because `[T]` is a declared collection contract. A generic parameter is not automatically spliced simply because its eventual concrete type is a collection.

This is the deliberate generic exception to the concrete nesting rule: declared `T` storage can retain a row without reconstructing it.

A generic body that lacks enough declared evidence needs an annotation or diagnostic. Existing generic inference, evidence and materialisation rules otherwise stay intact (section 22).

### 8. Fixed capacities

A fixed-capacity receiver requires proof during mandatory semantic compilation that every possible selected contribution fits. Failure to prove a bound is a source diagnostic, not a speculative optimisation failure and not an implicit runtime overflow path.

Assume `prefix` is a runtime parameter of type `[3 I32]` and `items` is a runtime `[I32]` parameter:

```moth
a [4 I32] = [prefix, 9]         -- valid, at most four elements
b [4 I32] = [items, 9]          -- diagnostic: no proven upper bound
```

The valid construction contains the actual elements of `prefix`, followed by 9. It does not pad to capacity. A known const sequence length can provide a tighter bound than the source capacity. A nested row contributes one element to the outer capacity and has its own independent inner capacity contract.

Use deterministic, conservative upper bounds: one for an ordinary scalar contribution, the proven length or capacity of a splice, checked sums for sequential contributions and a sound maximum over mutually exclusive branches. A loop or wrapper may contribute a bound only if the mandatory semantic owner can establish its maximum output. In particular, a loop with unknown iteration count is not assumed to fit. Do not add a general theorem prover, release-only interval analysis or repeated interpretation of effectful operands. More advanced proof classes require evidence of need and separate validation.

A fixed collection type always specifies both its maximum capacity and its element type, written `[capacity T]`. The capacity must be a positive `I32` literal or a visible `#I32` constant name. A literal at a known fixed-capacity receiver takes its type from that receiver.

The former capacity-only annotation `{capacity}` has no replacement shorthand. Migrate each use by writing the element type inferred in the original program. For example, `numbers {8} = {1, 2}` becomes `numbers [8 I32] = [1, 2]`. In type position `[8]` is invalid. In value position `[8]` remains a valid one-element construction. The existing rules for empty immutable fixed bindings, mutable empty fixed bindings and fixed field defaults still apply.

Repeated slot uses and per-child wrappers can increase length. Account for each actual emission, not just each authored contribution; checked sums/products and upper bounds must not overflow. Do not count a nested row's elements against the outer capacity. A known zero-iteration path has no emitted elements, but authored validation still follows the existing rules.

The exact initial set of loop/wrapper bound proofs is an implementation feasibility decision. The accepted invariant is that acceptance never depends on profile, backend, optional proof tables or allocation heuristics. Where no proof exists, the caller can use existing checked fixed-capacity insertion and explicit error handling. Keep the existing count domain and checked physical allocation rules.

### 9. Maps

Maps use square brackets in both type and value position, but retain their separate keyed semantics. A direct key/value entry distinguishes a map from ordered construction.

```moth
scores [String = I32] = ["Priya" = 10, "Rob" = 8]
chapters = ["intro" = 1, "install" = 2]
drafts [String = I32] = [=]
```

A direct entry's `=` establishes the map kind. An equals sign belonging to a call argument, record field, slot default or nested construction does not. Use existing map key eligibility, value typing, duplicate-key rules, order and error contracts. Preserve authored key-then-value evaluation and existing duplicate/error timing rather than batching or reordering keys for a faster builder. Map types retain the existing limitations on inline nesting and aliases unless independently revised.

Maps accept keyed entries in the head only. This proposal does not add map splicing, template-body map construction, automatic map merging, new iteration behaviour or a new policy for overwriting duplicate keys. Reject mixtures of direct keyed and sequence entries. The map parser and ordinary map semantic owner may retain a distinct path after bracket classification.

### 10. Typed slots

A slot collects ordered contributions. It does not require exactly one value. Use an ordinary parameter or struct field to express an exactly-one requirement.

The receiving wrapper establishes one stable receiving type per named, positional or default slot key. All uses of the same key must agree. A String consumer and a `[String]` consumer cannot implicitly reinterpret the same inserted values in different ways. Report the conflict at the wrapper definition with both receiving witnesses. A package that needs both representations can accept `[String]` and explicitly join it for text output.

Slot defaults are completed compile-time values of the established type, never unresolved wrappers. Preserve bare slot names, positive positional indices, the head-only slot declaration rule and the named-only `$in` target grammar. Loose head/body contributions follow the existing positional-then-default routing contract. Do not invent an insert target expression or count scalar elements of a splice as separate positional arguments without a separate design decision. Slot routing works on contributions, not flattened output elements. Repeated occurrences can retain distinct compatible defaults, as today. Preserve the difference between an absent insert and a supplied empty value. An absent insert may select a default. An explicitly empty insert counts as supplied and suppresses the default. Repeated slot occurrences replay the same routed contribution. Preserve existing lazy/evaluation rules and sharing, including runtime slot source plans. Repeated consumption must not accidentally evaluate an effectful contribution again merely because the result is typed. A supplied conditional that produces no output still counts as supplied; a default never counts as presence. An unfilled occurrence without a default emits the established empty result of its slot type. Head-only wrapper references with no fill can remain open; an application with routed content closes its own slots. Distinguish that declaration/reference state from a fully resolved empty application.

An insert is routed only to its immediate receiver. There is no ancestor search, ambient output effect or implicit resource forwarding across bindings, function calls or module boundaries. Unknown keys remain diagnostics. A typed mismatch is diagnosed at the contribution with the receiver's declaration as useful related context.

`$supplied` tests routed presence and contributes no type evidence of its own. If a slot is used elsewhere, that use establishes its type. If the guard is its only use, the slot receives the enclosing construction type. Preserve the existing rule that guard-only payloads are validated but need not be evaluated.

```moth
nav_link #= [$html:<a href="[$slot 1]"[$supplied current: aria-current="page"]>[$slot]</a>]
[nav_link, "#templates":[$in current:]Templates]
```

Here `current` receives String from the enclosing wrapper, giving the empty insert its required type evidence. A guard-only slot does not create a new unit type or a special Boolean insert syntax.

Typed sequence wrappers can use the same structural slot forms as String wrappers. For example, with ordinary package-defined `HtmlNode` values:

```moth
navigation_items #[HtmlNode] = [:
    [home_link]
    [$slot]
    [contact_link]
]

navigation [HtmlNode] = [navigation_items:
    [functions_link]
    [templates_link]
]
```

The wrapper collects nodes, and the completed result is `[HtmlNode]`. It is not a first-class wrapper object. The exact grammar, if any, for declaring a slot's type independently of its enclosing receiving context was not settled in the interview. Infer from established receiving uses where possible. Do not invent a public annotation spelling or silently support context-dependent slot polymorphism. A feasibility prototype must show how a wrapper's single stable slot schema is represented and validated.

### 11. Wrappers and direct-child boundaries

Allow one receiving wrapper per bracket. Remove implicit multi-wrapper head chains. A receiver is an open template with a slot schema, not an ordinary completed value, a formatter or an insert helper. A head may still contain ordinary positional contributions, multiple named insert helpers and compatible directives around its one receiver. Preserve the existing meaning of ordinary output preceding the receiver versus content routed after it. Removing multi-receiver layers is not permission to move prefix content into the receiver. Explicitly retain the current placement restrictions on inserts before a receiver. The exact ordering case must be tested against the existing `template_head_insert_before_head_receiver` fixture, migrated to explicit nesting. The compiler should diagnose a second receiving wrapper at that wrapper, label the first and suggest explicit nesting.

```moth
-- Old implicit chain:
[bold, italic:Experimental]

-- New explicit composition:
[bold:[italic:Experimental]]
```

A wrapper's type and slot schema are declaration-owned and available to callers across module boundaries. Explicit child nesting establishes where contributions go. A wrapper that constructs `[T]` consumes typed child output under its schema and emits ordinary `[T]` output. It need not manufacture an arbitrary nominal value. To turn children into one nominal value, call an ordinary function with a collection parameter.

```moth
sidebar = aside([:
    [heading("Documentation")]
    [loop pages |page|:
        [link(page.title, page.url)]
    ]
])
```

Here `aside` takes `[HtmlNode]` and returns `HtmlNode`. Its ordinary call supplies the receiving type and explicitly constructs the parent node. `[aside: ...]` is reserved for applying a declared template wrapper. It is not an implicit call to a function with a collection argument. There is no new callable trait or implicit constructor invocation.

A child wrapper applies once per direct child construction, not once per emitted element. If one child loop emits `intro` and `install` and another child emits `reference`, the enclosing child wrapper receives two groups: `[intro, install]` and `[reference]`. Moving a wrapper inside a loop makes it apply per iteration. Preserve the established output-conditioned wrapper rules, including the distinction between no output and a selected empty sequence. `$fresh` skips one parent child-wrapper operation, not all ancestors.

For `$before` and `$after`, retain the existing one-operand and named-slot-only restrictions. Their typed counterparts prepend or append compatible output around direct child constructions without stringification. Do not assume a typed prefix/suffix turns a sequence into a nominal object. Such transformations belong to explicit calls. The exact compatible operand shapes and fixed-capacity proof rules must be verified against existing wrapper semantics before implementation is finalised. Slot and wrapper compatibility must be resolved before formatting or flattening; typed values cannot be coerced to text to conceal incompatible slots.

A named open wrapper or insert helper remains a statically known template declaration. Open wrappers may be exported and imported with their stable schema. Const functions return completed values, not open wrappers, unresolved insert helpers or dynamically selected slot contracts. Wrapper factories and arbitrary first-class open-template values are outside this design.

### 12. Control flow and directives

Template branches and loops apply to compatible String or sequence constructions. They preserve output order, lazy runtime selection and existing condition/loop semantics. A loop can contribute zero, one or multiple elements per iteration through its body. No output is distinct from an explicitly selected empty value.

```moth
links [HtmlNode] = [loop pages |page|:
    [if page.visible:[nav_link(page)]]
]
```

Ordinary imperative loops remain available for local bindings, complex updates, early termination and checked collection insertion. Do not extend template loops into full statement blocks just to replace a deferred feature.

Directive applicability follows the operation's concrete output contract. Structural directives such as `$slot`, `$in`, `$supplied`, `$note`, `$before`, `$after` and `$fresh` can be generalised where their existing semantics have a typed equivalent. Text formatters such as `$md`, `$html`, `$css`, `$escape_html`, `$code` and `$raw` remain String-specific. The current directive registry and canonical directive references must determine the exhaustive list and any additional context restrictions. `$note` retains its lexical child-only rule and produces no value or type evidence. Do not widen source directives or their attachment rules by association.

Type-specific directives supply type evidence and report conflicts with known receivers. They do not become user-defined conversion or evaluator hooks. All directive names and semantics remain compiler-owned. Builders and packages do not extend the grammar or install arbitrary parsing or TIR callbacks.

### 13. HTML style composition and reusable components

The one-receiver rule deliberately retains the original goal of fast inline, Tailwind-style utility composition. A utility that contributes CSS to the receiver's `style` slot is not an element wrapper. Multiple utilities can target that slot in source order.

```moth
heading #= [$html:<h2 style="[$slot style]">[$slot]</h2>]
heavy #= [$in style:font-weight: 700;]
slanted #= [$in style:font-style: italic;]
red #= [$in style:color: red;]

[heading, heavy, slanted, red:Heading]
[heading, heavy:Build status: [span, slanted, red:experimental]]
```

The first heading has all three style declarations. The second uses an explicit nested element to style only part of its content. Core HTML element wrappers such as `bold` and `italic` remain different from utilities that set CSS properties on one element. Do not introduce a special exemption for a chain of formatting wrappers.

A recurring group can be a named insert that targets the same slot:

```moth
style_text #= [:[$slot style]]
warning #= [$in style:[style_text, heavy, slanted, red]]

[heading, warning:Breaking change]
[span, warning:Deprecated]
```

This is a representative design pattern based on existing insert routing. The exact example should become a regression fixture as part of the implementation, not be assumed to pass the baseline compiler without testing. A function can also combine a `[String]` of CSS declarations into String where styles are data. The caller then targets `style` explicitly. A function returning String does not secretly carry a destination.

Reusable stylesheets stay separate tracked CSS resources. Inline `$css` remains a String formatter for inline declarations. Shared styles, selectors and responsive rules belong in CSS files. Component resource dependencies are explicit data and root contributions. No new `$stylesheet` effect, automatic ancestor routing or implicit component registration is introduced.

Package authors can define their own node types, constructors and renderers. A structured HTML tree can provide stronger diagnostics than a String without requiring the compiler to understand the DOM. A node description is not a live browser-owned `Element` or `Node` handle. Ordinary runtime functions can convert descriptions into browser operations if an appropriate DOM package exists. No subscription, lifecycle or automatic rerender semantics follow from bracket construction.

```moth
-- Illustrative user-defined component API, not a required Core API.
navigation [HtmlNode] = [nav_link(intro), nav_link(install)]
[render_html(navigation)]
```

HTML entry output remains String. The explicit call to `render_html` crosses the library's representation boundary. The builder owns document assembly, structural resource resolution and existing stylesheet deduplication. A package may hide the rendering call within a page helper, but the compiler and HTML builder do not recognise a privileged `HtmlNode`, `HtmlPart` or `HtmlPage` type. Core packages may provide useful conventional abstractions without excluding alternative component libraries.

### 14. Entry and content-source contracts

Bare direct entry templates remain const-required. Explicit `~[...]` remains the runtime fragment spelling. Both use the HTML builder's known String receiving contract. Consequently, a head-only numeric entry template is interpreted under String evidence, not inferred as an unconsumed collection. A typed sequence must be explicitly rendered before becoming an HTML fragment. Direct builder `$in` destinations use their registered output contract and remain root-only, const-required contributions. Ordinary slots do not acquire builder privileges.

Keep the current one-CSS-origin `stylesheets` insert and complete-rendered-contribution `head` deduplication. Multi-resource or typed collection handoff remains separate design; concatenating head snippets loses their individual deduplication boundaries.

Other builders may eventually declare their own concrete destination types, such as a byte sequence, under an explicit builder contract. This proposal does not add a binary builder, arbitrary sink conversion or new output-purpose directive. Existing `.mtf` content remains a compiler-supported String source kind with its fixed Markdown/prelude contract. `.md` remains its separate existing source adapter. Neither is reinterpreted as a typed collection based on its importer.

## Part II - Compiler impact and simplification

### 15. Architectural approach and current branch contracts

Unify source interpretation, not all construction implementations:

```text
Retained bracket syntax + receiving context
  -> resolve kind, exact type and contribution roles in AST
  -> ordinary literal/collection/map path OR structural TIR composition
  -> existing typed expressions, constant values and neutral runtime handoff
  -> validated HIR, borrow/lifetime analysis and target/physical planning
```

The completed expression has one authoritative result type and operation. TIR remains AST-local. HIR, public interfaces and builders never reconstruct bracket interpretation from source. Preserve stage-owned dense storage and the existing canonical value/type owners. The compiler and build design documents remain authoritative outside this proposal's explicit changes.

**Branch reconciliation (verified during review).** On `main`, the roadmap first queues moving optional release source-call folding after semantic verification. Its compiler reference still describes AST speculation. On `docs/const-foundation-profile-independent-semantics`, the active const foundation plan instead removes that speculation, removes the frontend build profile and establishes a debug/release HIR parity gate. Its checkpoints C1-C3 are delivered; C4 publication, C5 integration and later work remain open. The production borrow companion records the BC-06 evidence and paused production rewrite. The current branch tip is a documentation/evidence commit, not proof that a production borrow replacement or const publication has landed. These different states must not be merged into one fictitious implemented contract.

For this redesign, require the settled profile-independent semantic foundation and correct const value publication/access contracts before expanding typed construction. Required const evaluation stays in Stage 4 over typed expressions and existing value semantics. Do not reintroduce early optional whole-function evaluation, candidate notes or budgets removed by the active plan, or build a HIR interpreter to recover the old direction. A future optimisation may fold already validated operations only under its own accepted contract; it cannot change type selection, capacity proof, borrow legality, constraint evidence or authored static selection.

The actual implementation baseline must be selected after the active foundation and its blocking provenance work are reconciled. This proposal neither activates that work nor assumes its experimental proof is production-ready.

### 16. Tokenization and source preparation

Relevant owners include `src/compiler_frontend/tokenizer/lexer.rs`, the shared `moth-lexical` crate, header syntax preparation, retained path references and the compiler's directive registry.

The lexer already distinguishes bracket heads from bodies. Keep that lexical transition independent of types. `utilities/token_scan.rs` currently has brace-specific counters in `NestingDepth`, `ExpressionBoundaryDepth` and body-local binding scanning, alongside `TemplateBalance`, `TemplateNestingTracker` and open-construct tracking. Audit these together. Track braced Moth blocks and declarations at their grammatical boundaries. Remove collection-specific brace tracking after its consumers move to brackets. Preserve the separate requirements for expression termination, source-scope recovery and template skipping.

- Replace brace collection/map grammar with bracket grammar in type and value positions. Keep the brace tokens used by blocks and declarations. Retire modes specific to brace-delimited collections and maps. Keep literal characters in text, strings, comments and formatter payloads intact.
- Recognise explicit nested head constructions while preserving nested brackets in directive operands, calls, generic/type syntax and child bodies. Distinguish empty `[]`, body-opening `[:]` and empty map `[=]` without guessing a type.
- Retain raw body text and its source spans until the semantic construction type is known. A typed sequence rejects non-layout text in its own body. Do not run the text through an expression lexer or infer its type from token spelling.
- Disambiguate direct map entries at the bracket's own syntactic depth. An equals sign inside an argument, record field, slot default or nested expression is not a map discriminator. Reuse the existing delimiter and argument-routing owners.
- Update grammar consumers that skip or capture balanced ranges, including declaration shells, signature/default parsing, generic bodies, config, source directives, orphan checking and entry capture. An entry's direct `[...]` and `~[...]` rules remain unchanged.
- Preserve structural provider and file-reference discovery for all authored references, including inactive branches. The new bracket spelling must not cause resource paths or qualified occurrences to disappear from Stage 0 discovery or create accidental dependency clauses.
- Preserve source identity and exact diagnostic ranges, including UTF-8 text and malformed nesting. Do not add a second lexical scanner in header preparation, a builder or the MON service.

Audit the shared lexical word/token inventory, tokenizer snapshots, editor-facing token classifications, syntax documentation and every place that assumes a brace token starts or terminates a collection. Internal Rust braces, data serialisation used only by tooling and embedded host-language text are outside this syntax removal.

### 17. Type syntax and one bracket interpretation owner

Reuse canonical String, growable/fixed sequence and map types. No new type family or layout follows from the spelling. The high-value refactor is the current split between `ast/expressions/parse_expression_dispatch.rs` (separate curly-literal and template dispatch), `builtins/expression_parsing.rs` (`parse_curly_literal_expression`, `CurlyLiteral::Ready/Retained`) and `ast/expressions/parse_expression_templates.rs` (String template construction). `type_coercion/parse_context.rs` contains `ExpectedType`, `ExpectedCollectionContext`, `ExpectedMapContext` and `ExpectedCurlyLiteralContext`. Its `parse_expectation_for_type_id` keeps collection/map/optional and numeric context, but otherwise returns `Infer`, including for plain String. The latter must change for unified brackets.

**Consolidate at the boundary:** replace delimiter-specific dispatch with one bracket classification/receiving operation. Reuse `ast/statements/collections.rs`: `new_curly_literal`, `parse_inferred_curly_literal`, entry parsing, `LiteralSide`, `CollectionEntries`/`MapEntries`, `complete_deferred_collection`/`complete_deferred_map`, and the final `build_collection_expression`/`build_map_expression` owners. These already perform local shape joining, pending numeric completion and keyed validation. Preserve their useful typed builders rather than implement a second bracket literal parser in TIR. Audit the inferred and explicitly typed branches for shared entry handling before adding splice roles. Replace delimiter-specific entry endings and context adapters; retire `CurlyLiteral` naming and obsolete paths once callers migrate. Keep separate cast-target ownership. Avoid passing a new redundant "template expected type" beside an already available receiving type; preserve concrete construction evidence without making unrelated expressions generally context-directed. A map-specific or collection-specific shape may still be a useful borrowed view of the canonical type; do not replace all three with one sprawling state object.

Resolve in this order:

1. Recognise grammar-only shape (direct map entries, direct nested rows, body/structural constructs, directives) at the appropriate depth. The existing expression parser remains the sole parser for operands.
2. Reconcile that shape with the immediate receiving type, known wrapper/slot contract and applicable directive evidence. A receiver can select String formatting but cannot erase a conflicting explicit map or row shape.
3. If needed, gather local homogeneous/sibling evidence before defaulting all-String heads or numeric leaves. Retain unresolved nested rows only within their construction owner. A later sibling may type an earlier empty row; a later unrelated use cannot change the declaration's construction kind.
4. Resolve each contribution as store-one, splice-one-level, nested-row construction, text append/format or map entry. These are semantic roles, not necessarily a new retained enum. Use existing operation shapes where adequate. Preserve generic roles before substitution and do not reroute an already typed operand by its concrete specialised shape.
5. Materialise exact types, validate capacity/access and publish normal typed AST operations. Use typed diagnostics for conflicting evidence, missing evidence and illegal contributions, with both source/receiver witnesses where useful.

`NumericConstructionState`, `numeric_known_shape.rs`, `statements/numeric_body_completion.rs` and `templates/tir/numeric_intake.rs` own unresolved numeric leaves and completion. Reuse that owner; do not introduce general unknown `TypeId`s, a second constraint solver or permanent inference recipes. Resume affected construction work without reparse or repeated subtree materialisation. Keep this local evidence separate from numeric-only equalities that can already extend across a body.

`declaration_syntax/type_syntax/parse.rs` currently owns `parse_collection_type`, `collect_collection_inner_range`, `scan_top_level_assigns`, capacity classification and map-side parsing. Migrate that type grammar directly, including `token_can_start_type_specification` and generic argument termination. Reuse the bounded type token window instead of routing types through the template body lexer/parser. Audit aliases, generics, map nesting, optional forms, hidden anonymous records and type displays. Type parsing and value parsing share delimiters, not semantic inference. Preserve Stage 3 capacity/default ordering and canonical identity. Remove the obsolete brace-delimited collection and map type grammar and its diagnostics. Changed semantics need normal compatibility invalidation, not a new fingerprint family.

### 18. Ordinary collections and maps keep a cheap path

A head-only `[1, 2, 3]` should become the same ordinary collection construction as today's literal. A map should use its existing keyed path. Neither needs TIR nodes, slot schemas, wrapper plans or formatting. Classify once, using retained delimiter context and the ordinary operand parse; do not speculatively parse every bracket as a template and then discard it when inferred as a collection.

Use the existing collection literal lowering (`hir/hir_expression.rs`) and collection traversal (`hir/hir_statement/loop_lowering.rs`) for ordinary output. Known scalar lists can construct directly. For dynamic splices, prefer ordinary typed accumulation and iteration over a new collection runtime. Preserve section 6's authored-position capture, including `copy`, and section 8's capacity proof. Reuse existing checked counts and allocation policy. Reserve from sound static bounds or from the already evaluated source at the correct position, never an eager length prepass across effects.

A bulk append is a possible optimisation, not a new source abstraction. Add one only if measured benefit warrants the explicit access, retained-edge, capacity, failure and lowering contracts. Avoid allocating one-element intermediate collections per scalar or one temporary TIR per ordinary operand. If ordinary iteration cannot express the required capture or memory semantics, stop to specify the missing primitive with the existing memory owners rather than hide a copy or an unchecked push. Retain String-specific direct rendering and structural resource handling.

### 19. TIR: generalise composition, not every value operation

Current architecture: parser emission writes text, expressions, child templates, slots, inserts, wrappers and control flow into a module-local TIR store. An exact effective `TirView` owns structural reads across overlays. Preparation validates/classifies the view. Folding or runtime handoff then consumes that same view. TIR is dropped before completed AST crosses the phase boundary.

The typed extension must preserve this ownership. Investigate the smallest construction-output contract sufficient to let existing structural composition work with String and typed sequences. A String-specialised accumulation path is still appropriate, including formatters and structural resource pieces. A sequence-specialised path can consume typed element/splice contributions. Maps do not need TIR body semantics in this proposal.

Relevant inspection routes (verify paths against the implementation branch):

- `src/compiler_frontend/ast/templates/template_head_parser/` and template node creation: head parsing, declaration lookup, directive compatibility and expression intake.
- `src/compiler_frontend/ast/templates/tir/` including `node.rs`, `store.rs`, `view.rs`, `preparation.rs` and the numeric intake owner: structural identity and exact-view classification.
- `tir/slot_composition/`, `tir/slot_layout.rs` and `tir/fold/wrappers.rs`: receiving schema, contributions, child wrapping and overlays.
- `tir/fold/` and the ordinary template folding owner: selected branch/loop reduction, no-output state and constant projection.
- Runtime handoff/materialisation owners: neutral owned payloads for lazy runtime output.
- Public const template rebuilding and projection: imported wrapper contracts and stable type identities.

Avoid turning every TIR node into a tagged generic value container or adding a second traversal that just computes a type already known during AST resolution. A type or contribution role belongs with the smallest owner whose consumers require it. In particular, inspect `tir/contribution_shape.rs`, `slot_plan.rs`, `slot_layout.rs` and `template_render_units.rs` before adding another shape or output classification. `ContributionShape` already distinguishes child contributions and parent-wrapper skipping; `TirSlotSchema` tracks keys and value consumers. Extend those actual owners where their contracts fit. Do not add a competing structural child/cardinality classifier or presence table. Contribution shape (a direct child versus a scalar) and resolved element/splice role are different facts; do not conflate them. Prefer one slot-schema owner with a stable output/slot type contract and specialised String/sequence sinks. If sequence structural lowering can reuse the same resolved composition plan while emitting ordinary typed collection operations earlier, do that rather than retain parallel String-like handoff trees indefinitely. Measure the effect of any widened hot record, new side store or retained wrapper metadata. Use compact typed IDs/ranges and existing view transitions where they suffice.

Existing String-only nodes for Text, Resource and SiteRoot remain meaningful. Do not reinterpret them as Byte or arbitrary collection elements. In a typed sequence, a structural String can be stored as one String element through an explicit compatible contribution. The String's resource and site-root pieces remain intact until the final output context supplies characters. A typed collection carrying a resource-bearing String still uses ordinary value provenance and resource reachability.

Preserve exact-view validation even for runtime classifications and inactive required bodies. Preserve lazy branch evaluation, loop ordering, slot presence and wrapper activation. Selected empty sequence output and structural no-output need a clear distinction at the structural layer; neither needs a new ordinary language value variant. Do not collapse the distinction simply because both may ultimately produce a collection of length zero.

### 20. Remove multi-receiver chain machinery and consolidate slot ownership

This is the clearest deletion opportunity. At both reviewed revisions, `ast/templates/tir/slot_composition/head_chain.rs` exists. The active branch's `is_tir_receiver` obtains a contextual slot layout, `build_tir_chain_graph` accumulates `TirChainLayer`/`TirChainItem` vectors and switches the active layer, `push_tir_chain_item` routes items and `resolve_tir_chain_items`/`resolve_tir_chain_layer` perform recursive bottom-up resolution. A one-receiver application no longer needs that graph of implicit layers.

**Recommended cut:** identify the optional receiver once, collect its fills in authored order, route once with its slot schema and emit the existing composed view. Keep any legitimate root-prefix output and current head/body boundary. Delete chain-only layer/item types, switching, recursive layer-reference resolution and obsolete conflict/recovery logic after checking callers. Explicit nesting uses existing child-template transitions. Do not merely constrain a multi-layer graph to length one and leave the redundant mechanism behind.

**Receiver identity:** `template_head_parser/head_parser.rs` has `InlinableHeadTemplate`, `should_inline_template_head_reference`, head compatibility state and local/imported template lookup. `parse_expression_templates.rs` distinguishes template kinds and folded/open results. Use the established declaration/value distinction at binding and import as the semantic owner of whether a reference is a completed value, open receiver or stored insert helper. Where the public value shape already establishes it, consume that fact rather than rediscover it from initializer structure or repeatedly walking a slot schema to classify a value. Keep receiver classification separate from the actual contextual layout needed during composition. Avoid a parallel global registry or cached copy of every template schema. Verify that the first/second receiver diagnostic comes from the single head owner, not overlapping checks in parsing and composition.

**Preserve necessary complexity:** `slot_composition/schema.rs` still expands placeholders, fills named child-wrapper contexts and resolves guards. `contributions.rs`, `slot_layout.rs`, wrapper-set owners and exact-view overlays are not all chain machinery. Keep them where required by explicit nested wrappers, repeated slots, `$fresh`, `$before`/`$after` and presence guards. Audit the remaining call graph before deleting modules or merging functions. A smaller direct routing function is preferable to a universal abstraction over String and collection stores.

Measure reduction in graph construction, slot-layout walks, copies, retained payloads and source LOC. No deletion is claimed by the proposal itself. The named functions were inspected, not profiled; this is a concrete refactor hypothesis to validate against real Core HTML table and style-group usage.

### 21. Const evaluation and completed value publication

Use `ConstValueStore` for completed String, collection and map values. Open wrappers stay structural compile-time contracts. Required const calls operate over canonical typed expressions in Stage 4. Reuse the active frame machine's value/operation owners and bounded execution rather than invent an interpreter-only collection domain or a second TIR evaluator. The active branch's const execution uses live storage identities and projection-aware mutation generations; immutable folded publication is a separate boundary. Do not apply runtime HIR borrow validation to live const bodies or treat an immutable value graph as their mutable storage.

The important integration prerequisite is correct publication of owned graphs preserving observable sharing and read-only retained referents. A fresh inner row must publish fresh membership; a splice or generic stored element must preserve the correct shared identities and permissions. Equal payloads are not proof of aliasing or disjointness. Required const evaluation, ordinary eager literal/template folding and runtime execution must agree on values, order and errors under their respective existing access contracts. Do not claim const and runtime access validation are identical mechanisms.

Preserve candidate isolation and commit only complete reachable results. Reuse structural String pieces rather than resolve URLs. `parse_expression_templates.rs` currently constructs a const-required template then explicitly prepares a `Value` view again in that path. The code comments identify different semantic preparation modes. Audit whether the unified completed/open boundary can eliminate repeated work by returning the existing authoritative preparation outcome to the correct consumer. Delete a traversal only if required validation, exact-view identity and lazy runtime classification are preserved. Do not weaken validation or add a cache to compensate for accidental repetition. Prior TIR fold caching was measured and rejected.

The review baseline's active const plan removes early speculative ordinary-function evaluation. Keep mandatory semantics profile-independent (section 15). Any separately approved late optimiser must consume the final semantic contracts and preserve runtime checks on refusal; this redesign neither implements nor requires it.

### 22. Public interfaces, generics and dependency ordering

`src/compiler_frontend/folded_value.rs` already distinguishes ordinary folded values from public open const templates. The inspected revision exposed the relevant public payload near line 195. Use that separation as the model for exported typed wrappers.

A completed collection exports an ordinary folded value with canonical types and stable provenance. A public open template publishes the stable output type, slot schema, defaults and the minimal structural data necessary to reproduce the existing composition contract. It never exports a donor-local TIR ID, AST expression ID, local type ID, runtime slot plan or a live semantic store. Reconstruction, if required, creates consumer-local data through the existing compiler-owned import boundary.

Preserve one canonical slot contract through imports, aliases and re-exports. Avoid publishing both a slot schema and a second independently authoritative inferred schema. If the existing portable template data must retain occurrences and defaults as well as the stable receiving schema, document which facts own the contract and validate reconstruction against it. Include slot-type closure dependencies in private-type checks even when the final output type does not mention them. Repeated uses, guard-only slots and defaults must be validated at the declaration owner before publication. A public interface cannot expose a private nominal type or bypass package/project provenance rules merely because that type appears in a slot. A semantic change to an exported slot schema or output contract must invalidate relevant consumers using the existing public-interface fingerprint. An implementation-only change retains the ordinary fingerprint distinctions where applicable.

The current generic materialisation architecture retains source-owned syntax and a stable semantic closure separately from executable AST/HIR. Extend its declared-type evidence and retained construction roles so concrete specialisation cannot reinterpret a generic `T` contribution as a splice or a String template. Reuse the canonical generic request and materialisation path. Do not copy provider HIR into consumers or add a second generic cache. Test across source packages and facades, not only local functions.

Header preparation and Stage 3 declaration ordering must continue to account for constant initialisers, capacity expressions, aliases, slot defaults, const functions and structurally visible template controls. Preserve the accepted const-function dependency and cycle policy in the active plan. Ordinary function bodies do not become dependency graph nodes simply because they contain a bracket expression. Graph-active references remain distinct from selected output liveness.

### 23. Borrowing, lifetime topology and memory planning

A sequence construction creates an ordinary collection allocation family. A nested head row creates a separate fresh inner family. A splice reads and stores existing element values under the ordinary reference and retention semantics. An explicit `copy` remains a deep independent copy with the existing eligibility and internal-sharing rules. The compiler may choose a cheaper representation only when the validated memory plan allows it.

The change must not introduce a special template ownership mode or sidestep existing collection mutation checks. Reuse canonical AST/HIR operations so borrow validation, result provenance and lifetime topology see the actual reads, writes and retained edges. In particular:

- A splice must not retain a borrow of the source's membership longer than required just to support preallocation. It must also not shorten a real lifetime needed by stored reference elements.
- A new collection containing existing references must satisfy ordinary retained-edge outlives and alias constraints. Fresh membership does not prove unique element storage or permission to mutate the stored referents. Copying an immutable reference into a fresh collection does not promote it to a writable reference. Preserve correlations across branches and call results instead of flattening provenance to an unqualified union of roots. The current BC-06 production correction is still paused; use its accepted eventual ordinary provenance contract, not its research model as a new production authority. The same section 6 capture regression must be tested at HIR, since a correct AST expression order alone does not establish correct projected operand capture.
- Nested rows, repeated slots and generic singleton storage must preserve their actual provenance and retention cardinality.
- A later operand that mutates the source cannot change the earlier copied membership. If its mutation conflicts with live retained aliases or accesses, the ordinary checker rejects it rather than silently deep-copying.
- Fixed capacity proof does not prove disjointness, ownership transfer or freedom from lifetime failures.
- Allocation coalescing, affine cleanup, region placement, REC and host-GC choices remain physical planning decisions. No source acceptance depends on optimiser success or backend collector policy.

Read the canonical memory-management access/aliasing, lifetime/escape and runtime/backend lowering references. The earlier inspection used `docs/developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf` near line 253 for stored-value semantics. The accepted memory authority, not a convenience lowering, owns the result.

### 24. Runtime handoff, HIR and backend lowering

Keep TIR AST-local. After typed composition and finalisation, hand off neutral owned data or normal typed construction operations. Runtime sequence construction may require an accumulation destination with known exact element type and lazy control flow. Prefer existing collection literals, ordinary iteration, append and placement operations over a second structural IR at HIR level.

The HIR validator should receive exact resolved types and ordinary construction operations, with capacity proof evidence only where needed to establish their contract. Prefer a proven-total construction over modelling it as an unchecked variant of the public checked `push` API. If a new internal operation is necessary, identify its validator and memory-plan owner and reuse it for ordinary and structural sequence construction. Avoid synthesising public `Error!` handlers that cannot occur in accepted fixed construction. It must not infer template output type, inspect source syntax, reconstruct slot schemas or rerun const evaluation. Retain normal success/error slot contracts and explicit operation order. HIR lowering must evaluate each contribution once, with any necessary argument/element prelude in authored order. The authored-position copy rule is a regression gate for required const execution, HIR operand capture and generated runtime code.

Backend implications:

- JavaScript: reuse the existing ordinary collection/map representation and String runtime helpers. Preserve exact element types, source order and resource URL resolution. Do not use an unbounded JS spread argument list as the only implementation of a dynamic splice. Respect existing host-boundary validation.
- Wasm: reuse supported collection layouts, validated memory plans and capability gates. The inspected `src/backends/backend_feature_validation.rs` distinguishes supported text operations from more limited aggregate operations. Do not claim general collection or nested-node support solely because syntax is accepted. Preserve explicit target diagnostics for unsupported reachable operations. Keep the existing text fast path.
- Mixed HTML-JS/Wasm: target assignment, reachability, runtime helper imports, structural resource maps and final output remain existing link/planning responsibilities. A bracket expression does not select a backend or force JavaScript because it happens to build a user-defined HTML type.
- Native and SDK/Lappet paths: audit any public semantic type/formatting APIs, target validation and collection layout assumptions that consume compiler results. No new foreign ABI representation follows from a delimiter change. Extend physical lowering only where an accepted operation cannot be expressed using existing validated primitives.

Audit backend debug printing, generated code snapshots, HIR display and error messages for old brace spelling where they intentionally describe Moth types. Do not change Rust, WAT or JavaScript delimiters. The compiler emits ordinary values, while downstream builders continue to consume the existing String output contract for HTML.

### 25. Proposed refactor gates

The following are feasibility and engineering gates for a subsequent implementation plan, not authorisation to begin work before adoption.

**Gate A - grammar and evidence.** Prove head-only brackets can select String, sequence or map from one receiving-context owner. Demonstrate numeric literal materialisation, empty/nested rows, map disambiguation and declaration-time shape stability. Show a simple collection takes a cheap path without allocating a TIR tree or requiring slot composition.

**Gate B - contribution semantics.** Prove concatenation, explicit nesting, generic singleton/splice roles, shallow membership identity and authored evaluation order. Include a dynamic source that mutates later, an effectful operand counted once and a `copy` read before a later argument/contribution mutates its source. Validate fixed-capacity acceptance independently of profile.

**Gate C - structural composition.** Prove String and typed sequence wrappers share the same local routing and child-group boundaries without retaining a universal template evaluator. Demonstrate a typed exported slot contract, conflicting repeated slot uses, empty vs absent output and guard-only payload handling. Remove the implicit multi-receiver path and migrate real wrapper chains.

**Gate D - evaluation and lowering.** Prove required const, eager ordinary folding and runtime results using the settled Stage 4 evaluator and validated runtime capture/retention contracts. Validate transaction rollback, resource-bearing values, imported wrappers, generated generic bodies and ordinary borrow/lifetime checks. Check actual backend capabilities without promising unimplemented aggregate lowering.

**Gate E - syntax cutover and deletion.** Complete Moth and MON migration together. Delete old grammar and chain-only support. Confirm that collection and map consumers use brackets in both languages and that braced Moth blocks and choice declarations retain their own grammar. Check for duplicate collection interpretation. Validate the full docs build, standard packages, test fixtures, target checks, benchmark suite and architecture boundary checks.

Choose the smallest complete implementation at each gate. A temporary internal experiment is acceptable on a dedicated branch. Do not expose half a public grammar cutover, ship two permanent syntaxes or add compatibility layers that later become the authoritative implementation.

### 26. Stop conditions and performance evidence

Stop if the gates require semantic type information in the lexer, repeated parsing/evaluation, profile-dependent acceptance, a second value/evaluation domain, donor-local IDs in interfaces, a permanent dual grammar, or implicit component effects. Escalate missing capture, retention or publication semantics rather than weakening access checks. Sections 15-24 name the relevant owners and permissible changes.

Benchmark ordinary collection-heavy code, text-heavy docs, mixed structural constructions, repeated const calls and imported/generic wrappers against the actual pre-change revision. Include deep nesting, dynamic splices, refused candidates and complete allocation/destruction cost. Record parse/type/fold throughput, expression/TIR retained and temporary memory, preparation/layout walks, allocations, lowering time and emitted size. Show the ordinary literal fast path avoids TIR allocation and existing String output does not gain a common-path cost without justified evidence. Use the current project benchmark/profiling owners and measure code deletion rather than equating fewer Rust types with lower cost. Reject speculative caches or universal abstraction layers absent evidence.

## Part III - Migration and roadmap consequences

### 27. Coordinated language cutover

On adoption, replace brace collection and map syntax with square brackets everywhere in Moth source. No legacy aliases or transitional language mode are proposed. Classify brace use in context so that Moth blocks, choice declarations and embedded text keep their intended meaning.

The migration must be syntax-aware. A global substitution of `{` and `}` is unsafe because source contains HTML, CSS, JavaScript, embedded code, literal text, Rust examples and comments. It is also semantically incomplete because `[a, b]` may now concatenate Strings or splice existing collections.

| Previous intent | Proposed source | Migration concern |
|---|---|---|
| Ordinary integer collection | `[1, 2, 3]` | Usually mechanical |
| Collection of Strings | `labels [String] = ["a", "b"]` | An untyped all-String head now creates String |
| Inferred empty value | `text String = []` | Both `[]` and `[:]` require evidence |
| Nested literal matrix | `[[1, 2], [3, 4]]` | Explicit nested rows preserve shape |
| Store an existing row | `~rows.push(row)` | `[[row]]` constructs fresh membership instead |
| Concatenate collections | `[left, right]` | Splice exactly one level |
| Numeric String head | `label String = [42]` | Add explicit String evidence where intent was text |
| Typed fixed collection | `[8 I32]` | Prove contributions fit; capacity is not exact length |
| Capacity-only binding `numbers {8} = {1, 2}` | `numbers [8 I32] = [1, 2]` | Supply the original element type explicitly |
| Map literal | `["a" = 1]` | Preserve keyed semantics |
| Empty map | `map [String = I32] = [=]` | Key/value evidence required |
| Old implicit wrapper chain | `[outer:[inner:Body]]` | Preserve routing and conditional/loop scope |

An old collection literal that stored a collection-valued binding as an element cannot be migrated by replacing braces with brackets. That would change storage identity and cardinality. Use explicit ordinary insertion or a generic helper with a single-element receiving contract when preservation is necessary. Use explicit nesting only when a fresh inner collection is intended.

An old String template containing only numeric or Boolean head values may become a typed sequence unless its receiver or directive establishes String. A raw body continues to establish text where consistent with its head. Inspect ambiguous cases rather than insert an annotation indiscriminately. Empty templates that rely on the old default must receive an exact type, a suitable existing directive or a known receiving boundary.

Existing ordinary value assignment, `.length`, collection methods, maps, range and collection loops, access markers and explicit `copy` keep their contracts. Migrate type and value spellings in traits, generics, choices, signatures, returns, defaults, casts where applicable, configuration, resource-related records and code examples. Do not introduce new collection operations simply to make the migration mechanical.

### 28. Migrate implicit wrapper chains carefully

Every head with more than one actual receiving wrapper requires explicit nesting. Do not mistake a formatter, named insert helper or positional value for a receiver. The common Tailwind-style usage remains unchanged:

```moth
[title, center:PROGRESS MATRIX]
[heading, heavy, slanted, red:Heading]
[section, $md:Content]
```

A flat chain with two elements must be split at the receiver boundary, retaining the inserts and positional values that originally belonged to each receiver.

```moth
-- Previous implicit chain:
[nav_group, guide_title, nav_link, "#templates", smaller, bold:Templates]

-- Explicit receiver structure:
[nav_group, guide_title:[nav_link, "#templates", smaller:[bold:Templates]]]
```

A final condition or loop in an old head can govern the entire receiving chain. Moving it into the innermost body can change whether an outer wrapper exists or how often it is emitted. Preserve the original selected-output boundary.

```moth
-- Previous:
[panel, bold, if visible:Notice]

-- Equivalent explicit selection:
[if visible:[panel:[bold:Notice]]]
```

`[panel:[bold, if visible:Notice]]` is not a general equivalent: it can leave the panel present when the inner condition produces no output. Apply the same care to aggregate loops, direct-child wrappers and `$fresh`. Preserve authored ordering and whitespace semantics where visible in output. Verify real rendered output, not just successful parsing.

The inspected compatibility anchors include `packages/core/html/@package.moth`, `packages/core/css/@package.moth`, `docs/docs/progress/@entry.moth`, `docs/docs_styles/@package.moth` and `tests/cases/template_head_insert_before_head_receiver/`. Review `template_slot_defaults_multiple_named_inserts` and `template_stored_named_insert_success` for ordered and stored insert behaviour. Use the current branch's actual paths if they have moved.

Document flat utility application, reusable named style bundles and explicit element nesting in the HTML package guide and Moth Patterns. Do not replace short, valid utility heads with verbose calls simply to satisfy the one-receiver rule.

### 29. MON format cutover

MON is a literal-data format with a separate codec, not Moth's executable expression grammar. It must adopt the new sequence and map delimiters in the same coordinated change.

```mon
chapters = ["Introduction", "Installation"]
positions = ["intro" = 1, "install" = 2]
drafts = [=]
rows = [[1, 2], [3, 4]]
```

In MON, `chapters` is a sequence of two strings. There is no all-String concatenation inference, splicing, template head/body, directives, const calls or wrapper interpretation. The receiving schema supplies exact element/key/value types where necessary. The parity target is ordinary typed Moth literal data, not every executable bracket expression. An empty data collection remains an empty collection under its schema and is not inferred as String.

In `crates/moth-mon/src/reader.rs`, migrate `parse_container_value`, `parse_collection_tail`, `parse_map_tail`, value-start classification and `is_number_delimiter`. In `writer.rs`, update `write_collection` and `write_map`, including empty-map output. Audit all delimiter errors, schema/default examples and tests. Keep the reader's existing container dispatch and writer's separate collection/map operations; a shared delimiter does not justify a general format engine. Preserve its standalone dependency boundary, exact numeric materialisation, finite limits, span handling, duplicate-key policy, deterministic writing and ordered data semantics. Reuse `moth-lexical` only for genuinely shared lexical facts. Do not import the compiler parser or TIR into the codec.

The reader and writer switch together. Existing brace-form MON files require a deliberate one-time data migration, not permanent dual grammar acceptance. The format remains ordinary literal data. If a migration tool is justified, make it syntax-aware and test quotes, escapes, braces within String values and malformed source. Do not conflate MON migration with arbitrary embedded-language rewriting. Update language/codec compatibility metadata according to the existing version policy. No version number or public migration command is fixed by this proposal.

Moth-native MON encoding/decoding and static MON builders retain their separate roadmap status. The syntax cutover must not imply those integrations are implemented, nor extend a MON schema to encode unresolved structural resource Strings or first-class open wrappers.

### 30. Repository-wide impact inventory

Parts II and III identify the primary owners. During implementation, inventory the following additional consumers using `AGENTS.md`, `index.md`, token/operation references and actual imports. Record why an inspected owner needs no change rather than forcing a refactor.

- **Lexical and source infrastructure:** shared `moth-lexical`, token schemas/storage, newline and delimiter scanning, retained ranges, header/default parsing, orphan units, diagnostic families, editor token classifications and escaped/raw payloads.
- **Semantic surfaces:** declaration/receiver/generic/trait/choice/record types, API indexes, type pretty-printers, capacity/default ordering, const/generic projection and remapping, config/input validation and content-source adapters.
- **Executable consumers:** expression visitors and freezing/compaction, HIR remapping/validation, borrow and lifetime summaries, target capabilities, runtime imports, JS/Wasm/native debug displays and memory/layout contracts. No physical layout change follows merely from syntax.
- **Packages and tooling:** all shipped Core/Builder sources, annotated JS/WAT binding signatures where Moth syntax is used, external API examples, CLI scaffolds, SDK/Lappet consumers and any editor/formatter tooling actually present. Do not invent an integration for an absent tool.
- **Data and tests:** MON code and schema examples, crate tests, generated fixture sources, expected diagnostics/outputs, samples, benchmarks and profiling workloads. Preserve host-language braces.
- **Documentation and roadmap:** section 32's canonical owners, both matrices, Moth Patterns and all affected queued/deferred syntax examples.

Use token/category and typed-operation searches, not punctuation replacement alone. Check fixture-generating Rust strings and embedded examples as well as ordinary `.moth` and `.mtf` files.

### 31. Roadmap changes conditional on adoption

This is a proposal, not an accepted implementation plan. Do not mark the language surface as implemented or activate roadmap work merely by publishing it.

If approved:

1. Add one coordinated redesign implementation plan to `docs/roadmap/plans/` and its sequenced entry to `docs/roadmap/roadmap.md`. The exact filename, branch and activation status follow the project convention at adoption time. Include the complete cutover and deletion gates.
2. Reconcile the plan with the active const-evaluation work. The supplied branch revision is `e235a04c`, and the current permanent compiler authority describes the AST-owned evaluator. Implement against the settled foundation after its merge and correctness validation, including const graph publication and the prerequisite borrow-provenance correction. Reconcile the distinct main/active-branch optimisation roadmap described in section 15. Do not revive early ordinary-function speculation or incompatible HIR-executor assumptions. Recheck any concurrent template, numeric, diagnostic, resource or memory refactor before assigning work.
3. Remove collection-producing `loop ... then` from accepted design and the implementation roadmap. Typed bracket loops supersede most of its use. Preserve a concise, explicitly unaccepted future design question describing immutable collection results from full statement loops and the conditions for reopening it. Remove accepted-language claims, queued progress promises and obsolete implementation instructions for that form.
4. Preserve the independent repeated option-capture loop and its current roadmap/status contract. The combined plan `docs/roadmap/plans/collection-producing-and-repeated-option-loops-plan.md` was identified in the previous inspection. Split or narrow that plan so removing the producing-loop portion does not delete the option-capture work or its unique accepted rules. Recheck its current name and status first.
5. Update component and browser package proposals to use the new spelling and explicit String output boundary. DOM handle types, typed events, Channels, Wiring, lifetime ownership and host ingress retain their separate decisions and implementation prerequisites. Typed bracket construction is not a reason to make a particular Core HTML component type mandatory.
6. Keep multi-resource stylesheet contribution design, per-head-snippet deduplication, arbitrary typed builder output, wrapper factories, general typed literal grammars and first-class open templates outside this work unless separately approved. Record genuine dependencies or blockers, rather than silently adding them to the accepted redesign.
7. Review the roadmap for syntax examples and assumptions about collections, map types, byte sequences, MON and typed package APIs. Update accepted future plans only within the superseded scope. Do not alter unrelated feature commitments or suggest that backend support follows automatically from the new frontend syntax.

At implementation completion, retire the finished plan in accordance with the roadmap policy after all unique accepted contracts have been transferred to their permanent owners. Keep historical proposals separate from accepted implementation instructions.

### 32. Canonical documentation and progress matrices

The user-facing language references own exact source semantics. The developer compiler and build references own stage boundaries, identity, storage and output contracts. Moth Patterns explains how those features compose, rather than defining a competing grammar.

Update the following as part of the coordinated work:

- `docs/docs/collections/`: square-bracket type/literal syntax, homogeneous inference, String distinction, nesting/splicing, explicit membership identity, fixed-capacity proof and map syntax. Retain access/error semantics.
- `docs/docs/templates/`: heads/bodies, contextual construction, empty values, typed slots/defaults, guard-only slots, explicit wrapper nesting, control-flow and child-wrapper grouping. Clearly teach raw text as String-only.
- `docs/docs/constants/`, function and generic references: constant values, const function returns, declared generic contribution roles and receiving-type behaviour. Follow the current const-function implementation and syntax.
- Numeric, types, structs, choices, access/aliasing and memory references: new type spelling and exact receiving/retention semantics wherever affected. Keep numeric-only inference policy distinct from construction-kind inference.
- `docs/docs/directives/`: output-type applicability and evidence from formatters. Preserve source attachment, child-only rules and compiler ownership.
- `docs/docs/project-structure/` and resource references: top-level const-by-default `[ ... ]`, runtime `~[...]`, direct builder `$in` receiving contracts and structural String resource semantics. Do not revive `#[...]` as a direct entry spelling.
- Core HTML/CSS package references and Moth Patterns: ordinary component types/functions, explicit node rendering to String, flat style utilities, reusable named style bundles and explicit element nesting. Update package paths to the current `@core/...` and `@builder/...` namespace policy. Leave proposed API names labelled as such.
- `docs/docs/mon/mon-format.mtf` and its Basic companion: bracket literal data, map spelling and schema-driven distinction from executable Moth templates.
- `docs/docs/cheatsheet/` and the actual cheatsheet owner: complete syntax migration and representative examples. Search all documentation sources rather than only the main reference pages.
- `docs/compiler-design-overview.md`, `docs/build-system-design.md` and `docs/compiler-data-layout-design.md` where affected: canonical type interpretation, TIR/constant/public interface boundaries, lowering handoff, source delimiter representation and compatibility. Update memory/backend authorities only for real changed contracts.
- `docs/docs/progress/@entry.moth` and the packages/builders matrix: distinguish accepted/planned support from implementation. Record simple collection syntax, typed structural construction and typed slots/wrappers accurately if delivery is staged. Preserve target-specific unsupported operations and the separate status of const functions, DOM, events and MON-native integration. Remove collection-producing loop commitments only after adoption; retain repeated option-capture status.

Maintain the docs' Basic and Advanced division. Basic should teach a straightforward collection and String construction before nested splicing and typed wrapper examples. Advanced should explain shape evidence, sharing, generic stability and why builders accept concrete output values. Avoid dense language that teaches implementation details as syntax requirements. Run the actual docs compiler and render representative pages to catch accidental parsing of examples and wrong wrapper migration.

### 33. Regression and validation matrix

Use the current testing guide. Extend existing tests instead of duplicating equivalent cases across stages. Unit tests own grammar and typed failure reasons; integration fixtures prove actual outputs, publication and runtime behaviour.

| Gate | Required coverage |
|---|---|
| A: grammar and evidence | Exact type and output for scalar/all-String heads, typed `[String]`, Byte/numeric receivers, mixed heads, directive conflicts, empty forms (`[]`, `[:]`, `[[]]`, `[=]`), both sibling-row orders, optional presence, raw-text rejection and layout whitespace. Direct map entries vs call arguments/slot defaults, nested heads vs bodies/operands, unchanged type suffixes. Accept fixed types with a literal or named capacity and an explicit element type. Reject capacity-only annotations even with a non-empty literal. Preserve `[T]` as a growable type and `[8]` as a value expression. Diagnostics retain original spans. Reject brace-delimited collections and maps in type and value positions. Recognise the agreed braced blocks and declarations in their own positions. |
| B: contribution semantics | One-level splice, fixed/growable sources, fresh nested membership vs stored rows, generic store/splice specialised to scalars/String/collections, shared read-only referents, internal sharing after explicit graph copy, correlated branch/call provenance and illegal topology rejection. Effectful operand once, dynamic lengths, early capture before later mutation (including `copy`), failures before/after other effects. Fixed proof from length/capacity, alternatives, repeated slots, loops and nesting. Insufficient bounds rejected across profiles. Preserve empty-fixed rules. |
| C: structural composition | Local/imported typed wrappers, named/positional/default slots, consistent repeated slot types, missing/wrong inserts, defaults vs supplied empty, guard-only validation without evaluation, conditional no-output presence, repeated runtime-slot replay, prefix content and head-only open references vs closed applications. Direct-child loop grouping, `$before`/`$after` and `$fresh`. Multiple style utilities accepted, second receiver diagnosed, reusable warning style and real migrated Core HTML/docs fixtures. |
| D: semantic publication and execution | Completed constant graphs, stable public slot/output type closure, re-export identity, private types and fingerprint changes; generic materialisation keeps declared roles. Const functions cannot return open wrappers/helpers. Required const and ordinary eager/runtime paths agree on output, order and applicable failures. Debug/release parity of validated semantics; no removed speculative-fold notes/budgets. Resource-bearing values and inactive dependency references survive correctly. HIR and borrow validation cover capture and retention; JS/Wasm/native execution or explicit reachable target refusal. No target-specific construction type. |
| E: full cutover | HTML entry String boundary, explicit runtime `~[...]`, direct builder slots, existing per-origin CSS and head dedup contracts, user-defined node rendering. `.mtf`/`.md`, config and source packages. MON round trips, map order/duplicates, nested values, exact numeric schemas, `[=]`, empty data sequences, malformed delimiters, budgets and quoted braces. Source/parity fixtures, code examples, CLI scaffolds, package installation, docs build and rendered output. Verify removal of the old brace-delimited collection and map grammar, the old chain layer and the compatibility routes. |

Run the current `AGENTS.md`-routed format, lint, architecture, test, documentation, backend and benchmark gates. Verify actual definitions of `just bench-frontend-check`, `just bench-check`, `just validate` and `just validate-full` before use. Record omitted platforms/features and reasons. Section 26 owns the performance gate.

### 34. Open design/implementation edges

The major semantic decisions above were agreed in the interview. The following narrow edges are intentionally not disguised as settled source rules:

- Whitespace-only bodies with no type evidence: distinguish `[:]` (always needs evidence), raw non-layout text (String evidence) and a body containing only formatting whitespace. Review recommendation: require evidence for layout-only bodies and preserve actual whitespace under an explicit String receiver. Confirm before adoption rather than silently defaulting.
- Positional slot and nested-head contribution boundaries: preserve the current one-contribution routing unit before splicing; verify how a direct nested row and a dynamic child loop reach a positional slot. If existing routing cannot express the typed contract without new cardinality rules, resolve that explicitly rather than count elements as positions.
- Exact syntax for explicitly typing an individual slot independently of the enclosing construction, if ordinary receiving evidence cannot express a useful wrapper contract. The invariant is one stable type per key, not a particular new annotation.
- The exact grammar for a head containing ordinary nested row expressions alongside wrapper/directive operands and text formatting. The required distinctions are specified. A parser prototype must demonstrate unambiguous classification and useful diagnostics, including mixed nested and scalar contributions. Reject unclear cases instead of silently choosing another interpretation.
- The initial proof vocabulary for fixed-capacity loop and wrapper contributions. Conservative refusal is required whenever the compiler cannot establish a sound upper bound. More sophisticated proofs do not belong in release-only optimisation.
- The minimal internal representation and reuse strategy for typed structural construction. Benchmark and inspect actual owners before widening TIR, introducing HIR operations or extracting shared components. The cheap literal path and deletion of obsolete chain handling are required outcomes, not a prescribed class hierarchy.
- Individual typed applicability of specialised wrapper/formatter directives beyond the common structural cases. Preserve accepted String semantics, validate compatible typed operations and document explicit exclusions rather than generalise by analogy without evidence.

The following are excluded, not missing implementation details: user-extensible template semantics, arbitrary nominal construction by implicit bracket function invocation, runtime open-template values, automatic ancestor-slot routing, arbitrary map splicing, transparent String/Byte equivalence, automatic HTML node rendering by the builder and a new component framework. The agreed uses of braces for blocks and choice declarations are recorded above.

Any further semantic choice that materially changes a worked example, public contract, evaluation order or source acceptance requires design review. Do not hide such choices in a compiler implementation or silently broaden the language to pass migrated tests.

### 35. Adoption and completion

Adoption requires approval of Part I and resolution of the source-visible questions in section 34. Activation requires the settled foundation in section 15. Implementation completes only when gates A-E and section 26 pass, all old grammar/chain-only paths are deleted, permanent docs carry the accepted semantics and the conditional roadmap/matrix changes in sections 31-32 are complete. Review the resulting diff using the code-review guide, separating deletion, consolidation, justified specialisation and deferred opportunities. No new runtime template system, implicit component effect or general parser is permitted as a shortcut.

## Reference baseline and source anchors

The references below record the review's observed repository state, not a pin for future activation. Both branch heads were independently checked. The active branch plan, production borrow companion, current roadmap, receiving context, expression dispatch, template expression parser, head-chain implementation and fixed-collection/slot reference were reopened during review. The earlier source inspection at main remains useful for the remaining routes. No full source checkout, compiled prototype, benchmark or test execution was performed. The exact authored-position `copy` correction is retained as a design requirement; the reviewed BC-06 notes provide related evidence rather than proof that every production path implements it.

- [Current main roadmap](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/roadmap/roadmap.md), [active-branch roadmap](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/docs/roadmap/roadmap.md), [active const foundation plan](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/docs/roadmap/plans/const-functions-evaluation-foundation-and-semantics-plan.md) and [production borrow companion](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/docs/roadmap/plans/production-borrow-checker.md) - verified divergence, remaining publication and provenance prerequisites.
- [Curly literal dispatch](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/builtins/expression_parsing.rs), [expression dispatch](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/ast/expressions/parse_expression_dispatch.rs), [template expression parsing](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/ast/expressions/parse_expression_templates.rs) and [receiving context on the active branch](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/type_coercion/parse_context.rs) - concrete consolidation seams.
- [Active head-chain implementation](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/ast/templates/tir/slot_composition/head_chain.rs), [slot expansion owner](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/ast/templates/tir/slot_composition/schema.rs) and [fixed collection reference](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/docs/docs/collections/fixed-collections.mtf) - verified deletion targets and retained constraints.
- [Compiler design overview at d6b32308](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/compiler-design-overview.md) - semantic stages, TIR, const evaluation, exact numeric receiving rules, public interfaces, memory and HIR.
- [Build system design](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/build-system-design.md) - entry/root destinations, String resource placement, package boundaries and linking.
- [Tokenizer](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/src/compiler_frontend/tokenizer/lexer.rs#L987) and [receiving context](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/src/compiler_frontend/type_coercion/parse_context.rs#L88).
- [Template head lookup](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/src/compiler_frontend/ast/templates/template_head_parser/head_parser.rs#L629), [implicit wrapper-chain composition](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/src/compiler_frontend/ast/templates/tir/slot_composition/head_chain.rs#L259) and [public folded values](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/src/compiler_frontend/folded_value.rs#L195).
- [Collection literal lowering](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/src/compiler_frontend/hir/hir_expression.rs#L692), [loop lowering](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/src/compiler_frontend/hir/hir_statement/loop_lowering.rs#L1595) and [target validation](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/src/backends/backend_feature_validation.rs#L1385).
- [Collection/map parser](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/ast/statements/collections.rs), [type grammar](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/declaration_syntax/type_syntax/parse.rs), [token scanning](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/utilities/token_scan.rs), [contribution shape](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/ast/templates/tir/contribution_shape.rs), [slot layout](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/src/compiler_frontend/ast/templates/tir/slot_layout.rs), [MON reader](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/crates/moth-mon/src/reader.rs) and [writer](https://github.com/nyejames/moth/blob/e235a04c5d282e6cd41c0ec86b77a31c60055101/crates/moth-mon/src/writer.rs) - additional verified implementation owners.
- [Template slots](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/templates/template-slots.mtf), [template control flow](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/templates/template-control-flow.mtf) and [fixed collections](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/collections/fixed-collections.mtf).
- [Core HTML](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/packages/core/html/%40package.moth) and [Core CSS](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/packages/core/css/%40package.moth).
- [Access and aliasing](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/developer-docs/memory-management/access-and-aliasing/access-and-aliasing.mtf), [MON format](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/docs/mon/mon-format.mtf) and [combined producing/option loop plan](https://github.com/nyejames/moth/blob/d6b323082777e73663c8eacd6e03852f97d7e769/docs/roadmap/plans/collection-producing-and-repeated-option-loops-plan.md).

The supplied compiler/build design, language cheatsheet and style guide were also checked when preparing this document. The older template-owned resource requirements proposal is historical context only, not an authority to reintroduce hidden stylesheet effects. The examples in this document are illustrative future source. No compiler changes, test executions or repository mutations are claimed by this proposal.
