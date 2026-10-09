# Writing tone — blog posts & user docs

Applies to everything in `docs/` (the MkDocs site: blog posts, guides,
reference, philosophy) and other reader-facing prose we ship — release
notes, changelogs, README copy.

Code comments and `.context/` docs are exempt; they're for ourselves
and follow different rules (see `AGENTS.md`'s "default to writing no
comments" guidance).

Blog posts skew personal and casual; docs skew terse and reference-
style. Both share the voice rules and the AI-tropes section.

## Voice

- **First person singular ("I")** for personal work, opinions,
  decisions. **"We"** for project-level statements.
- **Developer-to-developer.** Write like a technical forum post, not
  a press release.
- **Plainspoken.** No buzzwords, no hype adjectives ("revolutionary",
  "game-changing", "excited to announce"). Say what the thing does.
- **Comfortable with imperfection.** Admit missed milestones, known
  issues, half-baked ideas. "Not yet production ready" is fine.
- **Honest about uncertainty.** "I'm not sure if this is the right
  approach" beats false confidence.

## Structure by post type

### Release announcements
- One- or two-sentence intro: what version, what's notable.
- Bulleted list of changes — no paragraph-per-feature bloat.
- Bug-fix-only releases get 2–3 sentences total. Don't pad them.

### Roadmap / plans
- State the goal plainly.
- Numbered or bulleted plan.
- Flag what's uncertain or might change.

### Technical deep-dives
- Get to the point in the first sentence.
- Code examples over prose explanations.
- Headers and lists, not long paragraphs.

### General announcements
- Short. Say the thing, link if relevant, done.

### Guides & reference docs
- Lead with the task or concept, not setup throat-clearing.
- Show the command or screenshot before explaining it.
- Cross-link to related pages instead of restating.

## Do

- Get to the point immediately. One-sentence intros, not three
  paragraphs of context.
- Use bulleted lists for features and changes.
- Thank contributors by name.
- Admit when something isn't done: "I was hoping to add X but wanted
  to get this out first."
- Casual phrasing where it fits: "worst case…", "hope to see you
  there".
- Self-deprecating humor when it lands: "The major improvement is
  that it actually runs now."

## Don't

- Marketing or sales-pitch tone.
- Superlatives: "amazing", "incredible", "excited to announce".
- Pad short announcements into long posts.
- Corporate speak: "We are pleased to inform you", "leveraging
  synergies".
- Over-polish. Should read like it was written quickly and honestly,
  not workshopped by a comms team.
- "Stay tuned" or other empty filler closings.
- Sections added just to make a post look longer.

## Closing conventions

End with one of:
- A casual call to action: "Let me know if you run into issues."
- A specific ask: "Try the new filter syntax and tell me if it makes
  sense."
- Nothing — short posts don't need a closing.

## Avoiding AI tropes

LLM-assisted prose has a recognisable shape: false drama, hedged
grandiosity, recycled scaffolding. One trope occasionally is fine;
clusters, or any one pattern repeated, give the game away.
**Vary, be specific, accept some imperfection.** If a sentence could
open any blog post on any product, rewrite it.

Source: <https://tropes.fyi/tropes-md>.

### Word choice
- No magic adverbs: "quietly", "deeply", "fundamentally",
  "remarkably", "arguably".
- Banned vocabulary: "delve", "utilize", "leverage" (verb), "robust",
  "streamline", "harness", "seamless", "certainly".
- No grand nouns as decoration: "tapestry", "landscape", "paradigm",
  "synergy", "ecosystem", "framework". Name the thing.
- Plain copulas. "X is Y", not "X serves as Y" / "stands as" /
  "represents".

### Sentence structure
- No negative parallelism: "It's not X — it's Y", "not because X but
  because Y", "The question isn't X. The question is Y."
- No dramatic countdowns: "Not X. Not Y. Just Z."
- No self-answered rhetorical questions: "The result? Devastating."
- Don't open multiple sentences identically ("They assume… They
  assume…").
- Rule of three sparingly. Stacked tricolons read as AI.
- Cut filler transitions: "It's worth noting", "Importantly",
  "Notably", "Interestingly".
- Cut hollow `-ing` tails: "highlighting its importance",
  "reflecting broader trends".
- Avoid fake "from X to Y" ranges when X and Y aren't on a real
  spectrum.

### Paragraph & list structure
- Don't manufacture emphasis with one-sentence paragraphs.
  "He published this. Openly. In a book." is theatrical.
- If it's a list, format it as a list. Don't disguise enumeration as
  prose with "The first… The second… The third…".

### Tone
- No false suspense: "Here's the kicker", "Here's the thing", "Here's
  where it gets interesting".
- No patronising analogies: "Think of it as…", "It's like a…" —
  unless the analogy is actually load-bearing.
- No "imagine a world where…" futurism.
- No performative vulnerability ("And honestly, I'll admit…"). Real
  vulnerability is specific and uncomfortable.
- Don't assert something is "clear", "simple", or "obvious" without
  showing it.
- Don't inflate stakes. Most features are not world-historical.
- Cut "Let's break this down" / "Let's unpack" / "Let's explore".
- Name sources. Not "experts argue" or "observers note".
- Don't coin compound labels ("the supervision paradox") without
  defining and earning them.

### Formatting
- Em dashes: a few per piece, not twenty. When in doubt use `--` or
  a comma.
- Don't bold-prefix every bullet (`**Thing**: description` on every
  line). Mix it up; bold when one item genuinely needs to stand out.
- Straight quotes and ASCII arrows (`->`), not smart quotes or `→`.

### Composition
- Don't announce structure ("In this section we'll explore…") and
  don't recap it ("As we've seen…").
- Introduce a metaphor once. Don't run "ecosystem" or "engine"
  through every paragraph.
- Don't rapid-fire historical analogies ("Apple didn't build Uber.
  Facebook didn't build Spotify…").
- Don't restate one idea ten different ways. A 4000-word piece that
  says one 800-word thing five different ways is worse than the 800
  words.
- Never duplicate paragraphs. Reread long pieces; AI-assisted drafts
  often repeat sections verbatim.
- No signposted conclusions: "In conclusion", "To sum up",
  "As we've seen".
- No "despite its challenges, X remains promising" closer.

## Editing pass

After drafting, search for: `delve`, `tapestry`, `landscape`,
`leverage`, `serves as`, `worth noting`, `Imagine`, `In conclusion`,
`Despite`, `seamless`, `excited`, `—`, `→`. Each hit is a question,
not necessarily a fix — but each one earns its place or gets cut.
