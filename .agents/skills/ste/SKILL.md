---
name: ste
description: Write and edit prose in ASD-STE100 Simplified Technical English (STE). Use whenever you author or rewrite content in this repo — docs/ entries, the index and log prose, AGENTS.md, other skills, and pull-request or commit text. Gives the controlled-language rules that apply to our content: short sentences, active voice, one idea per sentence, consistent terms, simple verbs, and no long noun clusters. Also states what STE does not restrict (code, identifiers, product names, quotations).
---

# Simplified Technical English (ASD-STE100)

STE is a controlled form of English for technical documentation. It keeps
sentences short and clear, so people and machines read them the same way. This
repo adopts STE as its house writing style.

The standard is **ASD-STE100**, Issue 9 (January 2025). It has 53 writing rules
and a dictionary of about 900 approved words. The specification is free to
download from the maintainer after registration. The dictionary is copyrighted,
so this skill gives the principles, not the word list.

- Home: <https://www.asd-ste100.org/>
- Overview: <https://en.wikipedia.org/wiki/Simplified_Technical_English>

## When to apply it

Apply STE to all prose that you author or materially rewrite in this repo:

- `docs/` entries and their `index.md` and `log.md` prose
- `AGENTS.md` and other Markdown guides
- `SKILL.md` files, this one included
- commit messages and pull-request text

STE governs sentences. It does not change the structure rules. Keep using tables,
lists, mermaid diagrams, and definition lists for structure (see `AGENTS.md` →
Authoring conventions).

## The rules that matter for us

Write each sentence to these limits:

- Keep sentences short. Use 20 words or fewer for an instruction. Use 25 words or
  fewer for a description.
- Put one idea in each sentence. Give one instruction per sentence.
- Keep paragraphs short — 6 sentences or fewer. Put one topic in each paragraph.
- Use the active voice. Write "the job writes the record", not "the record is
  written by the job".
- Use the imperative for instructions. Write "run `okf validate`", not "you should
  run `okf validate`".
- Use simple verb tenses. Do not stack auxiliaries (for example, avoid "would have
  been able to").
- Use one term for one thing. Do not change between synonyms for the same concept.
- Prefer simple, common words. Replace a difficult word with a plain word when the
  meaning stays the same.
- Do not write a noun cluster of more than three words. Break "per user license
  state cache" into "cache of per-user license state".
- Keep articles and connecting words (the, a, this, that). Do not write in a
  telegraphic style.
- Be specific. Replace a vague word ("stuff", "handle", "various") with the exact
  term.

## What STE does not restrict

STE allows technical nouns and technical verbs that name real things in the
domain, even when they are not approved words. These stay as they are:

- product, team, and project names (Clio Manage, CBS, Corvum, Grow::Root)
- code identifiers, file paths, commands, and API names
- direct quotations and cited text — copy them exactly, do not change them to STE

## Quick self-check

Before you finish a paragraph, confirm each point:

1. Is a sentence longer than 20–25 words? Split it.
2. Is a sentence passive without a reason? Make it active.
3. Did you use two words for one concept? Choose one.
4. Is there a noun cluster of four or more words? Add a preposition.
