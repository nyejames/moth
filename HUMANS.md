<div align="center">

# AI Policy

</div>

This document is about how AI is/should be used in this codebase, if its part of your workflow.

Its also a long, oppinionated rant about LLMs and the limitations of AI tooling.

## How LLMs are used already in this repo

AI tools are used to help with building this language, especially before aggressive optimisation becomes a focus.

All LLM generated output is meticulously planned and structured ahead of time, reviewed, and put through a rigourously tested pipeline with tons of documentation and tests to support that process.

## AI contributions

Contributions that are AI generated can be accepted on their own merit, but might have to adhere to a higher standard of scruitiny before they can be accepted if any smells suggest the AI is making important decisions instead of the developer.

Everything must pass the full validation workflow and follow the compiler documentation strictly. Submissions that show design drift, duplicated implementation paths, weak diagnostics, superficial tests, or overly verbose code will not be accepted.

The [AGENTS](./AGENTS.md) file in this repo is an attempt to keep LLMs on track but is not a complete solution. Human review and clear guidance is still absolutely essential.


</br>
</br>
<div align="center">

# A personal note on using LLM tooling
*or: a constant tidal wave of unfalsifiable opinions*

</div>

This is a solo hobby project so LLMs have become vital for keeping momentum going and allowing me to focus on design, documentation, architecture and planning implementations rather than writing most of the code myself.

The speed up from LLMs in terms of actual features hasn't been that large (I would estimate around 2 - 3x max). This is because good LLM code still requires a lot of auditing and occasional redirecting when it gets confused or it becomes clear that the original brief was not the right approach or not specific enough.

There is plenty of pre-alpha "good enough" scaffold code in this codebase that I've flagged to return to later and improve by hand.
When the time comes for hardening and optimising, I suspect the gains from using agents will decline further.

A lot of the time I would be writing the code myself is now spent staring at diffs and writing plans and documentation.
All the architecture, code organisation and design decisions around this language and compiler are driven by me and always will be. 

<div align="center">
    <img src="./docs/assets/iceland-nostalgia.webp" width="500px" border="3" alt="A road in Iceland streching into the distance"/>
</div>
</br>


---

### "good" agent generated code

Modern agents seem to have a lot of post-training that makes them very "productive" or "proactive". This is good for people who want to leave it alone and come back to find something roughly resembling what they think they wanted on their screen. 

For more serious projects this relentlessness can cause problems. 

Agents given a high-level broad goal are at high risk of introducing design drift or adding unnecessary code. But it's hard to describe intent in a detailed and bounded way.

Good agent written code is produced by giving it a step-by-step strict implementation plan with all the invariants and rules laid out ahead of time. When it starts to drift, or there's something you've missed, then you stop it and steer it in the new direction. Letting them be "proactive" leads to unmaintainable code.

LLMs can allow focus to shift more towards higher level concerns (design / architecture / codebase hygiene) rather than making sure the borrow checker is happy. I want to think about the layout of structs or how to make the constant folding more efficient, not spend time threading a slightly changed function signature through all its call sites.

They're also good at reviewing and helping to structure plans.

If you're writing an implementation plan for an agent to follow, then get an LLM to help you understand what's missing or confusing before you try to execute it.

LLM reviewing, especially when an LLM is loaded up with all the documentation, is often very useful and a vital part of orchestrating other agents. This helps cut down on the final diff staring at least a bit. They can catch things you don't even if they can be wrong. Having an unfeeling statistics machine's report can help bypass our ego. No pleasantries required to decorate the shit-sandwich we have to give another person.

<div align="center">
    <img src="./docs/assets/tng.jpg" width="400px" border="3" alt="Q playing the trumpet in Star Trek"/>
</br>

A brilliant approach or great design insight always comes from you. Not from a looping text prediction engine.

</div>
</br>

---

## A future with AI tooling

I don't think skilled developers, or writers for that matter, are going anywhere.

AI can benefit people who struggle to communicate or execute on their ideas. But LLMs are also a trap for laziness when you could be focusing on developing your own unique voice or thinking for yourself.

Maybe more people than ever will start to understand the difference between a technical demo and ambitious, polished software. Polish and good UX is the remaining 90% of drawing the owl. There are still no shortcuts.

I just hope the keyboard becomes our shrine for carefully considered thought rather than high volume code output.

## Slop in -> slop out

If there isn't sufficient complexity or novelty in your input, the LLM will just parrot the most statistically sensible thing back at you with more pointless noise and overconfidence added.

The paradox is: the higher quality your input is, the better the results you will get back. This isn't a free win.

Your own chaotic and slightly wrong thoughts can be exactly what makes your own written output compelling. If you whack the temperature up on an LLM, the spontaneity fails to come from a place routed in real experience. It fails to draw those invisible connections that map surprising content to meaning. 



<div align="center">

Its not beautiful.

<img src="./docs/assets/scampy_and_radio.jpg" width="500px" border="3" alt="Scampy and old radio"/>
</div>
<br>

I am now much more fond of "obviously human" writing. This is an era where its increasingly apparent *shortcuts* are being taken in producing things with a verisimilitude of art.

Having your own distinct "writing voice" is more valuable than ever. The internet is becoming increasingly saturated with tight, em-dashed sentences loaded with qualifiers, outdated pop culture references and corporate language. Your writing style (bad grammar and all) will stand out more than it ever has against the tidal wave of slop. 

At least I can say I hated semicolons and oxford commas *before* it was cool.

If you think that LLMs are already producing meaningfully creative works with little human guidance, then it might be time to scrutinise your aesthetic taste. When was the last time you consumed something that challenged your taste and made you really think?

Often, the things we connect with the most challenge our perspective or give us something new to process. Our effort is demanded in order to engage.

---

### The fallable confidence of clankers

Anyone who works on complex projects and uses LLMs should be acutely aware of how limited they are when it comes to understanding "big picture" design and architecture.

Even when they seem to make good suggestions about these things, they lack genuine creativity. 

**When Agents are confidently wrong**

I've often gone away to think about a design problem (5 minutes to a few days) after asking an LLM about something and not feeling good about the overly authoritative and confident answer. I'll usually come back with a better solution that fixes or simplifies things in broader or more novel ways. These decisions always become the better long term fit. 

This is when we hear the classic `you're absolutely right` or `that's an improvement`. 

I once wrote a weekend project Rust library called [saying](https://crates.io/crates/saying) (basically a print macro for Rust that adds colour and styiling based keywords in the most concise way possible). I now use this all over the Moth compiler.

I asked an LLM how to write declaritive macros for this, only for it to say something along the lines of: `the formatting you want to do is impossible with declarative macros lol get rekt idiot, that requires a full parser` (*paraphrasing*).

But I knew the solution was possible with layers of conditionally branching macros. I came back to later tell the LLM, in a moment of self aggrandisement, that I got it all working. 

I'm sure I got an understated `You're abolutely right, it was an error to say this was too complex to parse` in reponse.

</br>

---

### Agent Bias

AI also reflects your own biases back at you. While there are biases baked into the powerful models, your own prompts create another issue.

Its very easy to unintentionally steer an LLM towards the solution you prefer rather than the *best* solution. It then proceeds to *proactively* sloppify your codebase with your brilliantly undercooked design choice.

One of the more powerful uses of these tools is sanity checking an idea or reviewing code. They often avoid the tough truth unless you're asking for it explicitly or deliberately requesting actionable feedback. What you ask for and how you ask for it can dramatically change what it focuses on in its reponse. 

Its important to remember you need to be able to let go of ideas. Recognising your bad design is just as important as pushing back on the agent when it gets things wrong or doesn't *see the vision*.

<div align="center">
    <img src="./docs/assets/Here.jpg" width="500px" border="3" alt="A tree in a field in Gloucestershire"/>
</div>
</br>

In this compiler I've made many design calls based on my own experience and intuition that LLMs are consistently confused by. But by sticking to those decisions knowing they are ambitious has got this project a lot further than if GPT was making design calls. One major example is constant folding happening at the AST stage (and in some narrow cases before that), so expressions are flattened before they even get to the HIR. I had to write the detailed compiler documentation because large language models struggle with unusual compiler design choices like this, particularly when it comes to architecture. 

Moth's memory management strategy has been a pain to try and document clearly enough to avoid agents pulling it back towards all the regular boring stuff every other language does. This is what consumes most of my time, spamming all my ideas into an LLM and getting them to organise it into documents that are slightly readable and don't get the technical details wrong.

And even though the tests in this codebase are pretty much all written by AI, I have to constantly stamp by foot down on LLMs adding redundant tests or putting them in the same files as production code (which is a hard no-no in this codebase). But there's no way I'm going back to writing all the tests by hand again.

Adding skills, detailed documentation and a carefully written AGENTS.md doesn't fix everything. Reviewing is essential, and realising they will drift towards a boring mean is important.

<div align="center">

Detailed well scoped tasks make better results.

<img src="./docs/assets/moth-photo.webp" width="350px" border="3" radius="50%" alt="A photograph of a moth"/>
</br>
</div>

## How this applies to Moth

Moth is designed to give detailed, fast feedback for producing reliable code. Good languages for the future should be easy to review and strict about how the code is structured. See [the design principles doc](./docs/src/docs/design-scope/design-principles.mtf) for more info about how this directly related to using LLMs.

If this is an industrial revolution for automated thinking, then its still valuable to remember that the "hand-made" things had to designed by someone in the first place.

The printer didn't replace layering paint onto a canvas by hand and computers didn't replace the need for mathematicians.

The things we got to do and problems we got to solve just became more interesting. Not less important.

I want this language and compiler to be the language for that exact balance. This isn't a language for LLMs, its a language for human creativity with the churn cut out and automated as much as possible.
