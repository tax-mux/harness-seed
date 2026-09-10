# Development principles

Principles when changing HarnessSeed core, adapters, or agent-facing features.

English companion to [../ja/development-principles.md](../ja/development-principles.md).

## Prefer clarity in every explanation

**Overviews are where writers most often skip.**

Write the opening as normal prose, not a checklist of section titles. A reader should still be able to tell:

- what the thing is (product-facing definition before type or file names)
- why it exists in this repository (the pain without it, and what it deliberately does not do)

Do not substitute a config table for motivation. Motivation must be answerable from the overview alone.

**Treat figures and tables as part of the prose.** Do not stop at “the following diagram shows X” and leave decoding to the reader. After a figure, continue in normal sentences: what happens, where it branches, what stays the same and what differs. Do the same for tables.

“As the diagram/table shows” is fine as a bridge, but **do not merely recite box names, type names, and function names in one breath.** Explain the stages in plain language first; introduce implementation names only when needed. Do not compress a whole figure into a single paragraph.

Avoid meta instructions about how to read the figure (“read left to right”, “the takeaway is…”).

## Prefer general solutions

**Logic that only fixes one case has low value.**

- User utterances used for reproduction (e.g. “explain this project”) are **validation scenarios**, not strings or domain-specific branches to hardcode.
- Ask whether the same shape helps mail, web, and coding alike. If not, lift it into a **general gate or contract** on the layer (memory / plan / exec), not a one-off recipe.
- Hardcode only domain-agnostic mechanical constraints (e.g. reject `skip_execution` without sufficient evidence; prefer knowledge when both channels are on). Do not bake specific paths, task ids, or product names into defaults.
- Write prompts and rules in domain-agnostic wording. Prefer “gather missing evidence with available tools” over “read README”.

## Do not mix layer responsibilities

| Layer | Does | Does not |
|-------|------|----------|
| Memory RAG | Branch work log vs knowledge; fetch candidates | Decide domain procedures |
| Planning | Judge whether evidence is enough; list steps | Fixed recipes for one use case |
| Execution | Choose and run tools from the catalog | Reinvent memory routing |

When fixing a failure, first ask which **general layer contract** broke. Do not ship patches that only make one log utterance pass.

## Separate validation from implementation

- Log validation may use concrete scenarios.
- Implementation must not keep scenario names in code (function names, comments, default goals with a specific domain).
