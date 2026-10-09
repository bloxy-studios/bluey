# Mode System

Modes are **data**, not code. A `BlueyMode` carries everything the pipeline needs:

```ts
interface BlueyMode {
  id; name; description; icon;            // presentation
  systemInstructions;                     // the mode prompt (editable, even for built-ins)
  responseSchema;                         // which structured output shape to request
  preferredLatency;                       // ultra-fast | fast | balanced | deep
  contextRequirements;                    // screen | accessibility | transcript | resume | job_description | documents | session_memory
  builtIn; group?; responseStyle?; preferredModelRole?; attachedDocumentIds;
}
```

## Built-in modes

| Mode | Schema | Latency | Context | Output |
|---|---|---|---|---|
| General (default) | answer | fast | screen, accessibility, transcript, session_memory | the answer, first person, answer first |
| Interview | suggested-response | ultra-fast | transcript, resume, job_description, session_memory | exactly what the candidate says next |
| Behavioral Interview | behavioral | fast | transcript, resume, session_memory | the spoken answer (STAR inside, unlabelled) · Story used · Key point |
| Coding Interview | coding | balanced | screen, accessibility, transcript, session_memory | the fenced solution first in `content` (the app derives `code` from it), then the approach in ≤2 lines · Complexity · Edge cases |
| System Design | system-design | deep | screen, accessibility, transcript, session_memory | requirements → … → trade-offs, optional Mermaid diagram |
| Case Interview | case | balanced | screen, transcript, documents, session_memory | the next thing to say · Clarify · Framework · Analyze · Calculate · Synthesize · Recommend |
| Sales | sales | ultra-fast | transcript, documents, session_memory | what the seller says next · Why it works · Optional follow-up |
| Recruiting | recruiting | fast | screen, transcript, job_description, documents, session_memory | what the recruiter says or asks next · Screening notes · Next step |
| Team Meeting | meeting | fast | transcript, session_memory | live: Important / Decision / Action item / Question; after: Summary … |
| Lecture | lecture | fast | screen, transcript, session_memory | concepts, definitions, notes, study guide, questions |

Definitions live in `bluey_core::modes::built_in_modes` and are seeded into SQLite at every
launch; users can edit instructions and attach files. Each built-in row keeps a `seed_hash`
fingerprint of the shipped definition it matches: a built-in the user never edited is refreshed
to the new definition when a release changes it, an edited one keeps the user's text.
*Reset to default* restores the shipped definition (files are kept) and makes the mode
refreshable again. The TS side never copies these definitions: `tests/fixtures/rust/built-in-modes.json`
is generated from Rust (`BLUEY_UPDATE_FIXTURES=1 cargo test -p bluey-core --test ts_fixtures`,
which fails on drift) and feeds the mock transport and the TS tests.

**Built-in instructions are judgment only** (80–220 words each, enforced by
`bluey_core::modes` tests): what a strong answer in that situation gets right — which story to
pick, how to size a design, when to stay silent in a meeting. Voice, length and fields are owned
by the layers below, so a mode never says "lead with the answer" or "be concise" — the response
contract already does, for every mode. Custom modes get the same treatment: write the judgment,
not the format.

## Custom modes
Settings → Modes → **New Mode**: name, description, instructions ("Meeting context"), response
style, preferred output schema, attached context files, preferred model role, latency
preference. Persisted in the `modes` table. Actions: duplicate, edit, delete, set default,
set active.

- **Limits** (the Rust `ModeRepository` enforces them on create and update with
  `internal.invalid_params`; the editor checks the same rules with `validateModeDraft` and shows
  the error inline without saving): name 1–60 characters, description ≤ 300, instructions
  ≤ 4000, a kebab-case icon name, and a preferred model role among default, fast, reasoning,
  vision and research. Blank instructions mean no mode-specific instructions.
- **Clearing**: in `modes_update` an absent field is kept and `null` clears the sidebar group or
  the preferred model role ("Auto model"); a blank group is no group.
- **Files** are the mode-scoped documents (`documents.scope = 'mode'`, `scope_id` = the mode);
  `attachedDocumentIds` is derived from them. *Duplicate* copies each file (with its chunks and
  embeddings) to the new mode; *Delete* removes the mode's files with it.

## Default and active mode

`general.defaultModeId` is the mode Bluey starts in. Bluey launches in the default mode, except
when an active session is being resumed: that session keeps the mode it was running in. Setting
the default (onboarding or Settings) also switches to it right away when no session is running;
a running session keeps its mode until it ends. Deleting the mode that is the default resets the
default to `general`; deleting the active mode switches to the default (or `general` when the
default no longer exists). A stored mode that no longer exists always falls back to `general`.

## How a mode shapes a response
1. **Prompt** — the system message is, in order: identity + safety rules · the **response
   contract** · `Mode: <name>` + `mode.systemInstructions` + the schema's `ModePrompt` fragment ·
   style · language · output format. The user message is the labelled context followed by the
   `Task:` line for the trigger and the `Shape:` line for the detected answer shape
   (`src/ai/prompts/*`, `src/modes/prompts/*`, `src/ai/prompt-builder.ts`).
2. **Context** — `contextRequirements` decide which sources are gathered and how retrieval is
   scoped (candidate modes pull resume/CV chunks; recruiting/sales pull JD/company notes).
3. **Routing** — `preferredLatency` and `preferredModelRole` feed the model router.
4. **Output** — `responseSchema` selects the structured schema and the HUD renderer (sections,
   code block with copy/expand, diagram view, calculations separated).
5. **Classifier** — detection rules adapt to the mode (objections in Sales, decisions/actions in
   Team Meeting, behavioral cues in interviews). In Team Meeting (decisions, action items,
   important statements, topic changes) and Lecture (important statements, topic changes) these
   detections need no answer: they go on the session timeline (`src/transcript/notable.ts`), with
   the spoken words only when Privacy → Store transcripts is on.
6. **Summary** — the post-session summary (*Generate summary* on a session in Settings →
   Sessions) uses the mode the session started in, even after a switch mid-session. Every
   summary has the same fields (overview, topics, questions, answers, decisions, action items,
   open items, improvements) and the schema adds its emphasis
   (`src/ai/prompts/summary.ts`): a *Study guide* section for Lecture, *Technical review* for
   Coding Interview and System Design, *Deal notes* for Sales, an *Interview debrief* for the
   interview schemas, decisions and action items first for Team Meeting; General and
   Recruiting get the common fields only.

## Voice and precedence

Every ask carries the **response contract** (`RESPONSE_CONTRACT`, `src/ai/prompts/system.ts`)
right after the identity: lead with the answer (never a restatement of the question or a
description of the screen), explain only what earns its place (the shape rule itself is the
per-ask `Shape:` line under `Task:`), commit to one answer, and — the precedence rule — **safety rules > the user's custom
mode instructions > this contract > built-in mode guidance > style**. A custom mode's block is
labelled `Mode: <name> (the user's custom instructions).` so the model can tell the two apart;
it still never outranks the safety rules. The **style block** only sets ceilings
(`Length ceiling: concise — at most ~120 words … a one-line answer is complete`), never a
minimum; the **schema fragment** names fields and section titles and never changes the voice.

Whose words the answer is comes per request, as one `Voice:` line under `Task:`/`Shape:`
(`Intent.voice`, derived in `src/context/relevance.ts`): **speak-as-user** for spoken shapes,
⌘⇧↵ and heard questions; **write-as-user** for picks, values, texts, solutions and designs I
submit; **explain-to-user** for explanations, summaries and debugging — including a typed
"explain … so I understand" in a conversational mode (a question put to me, "why do you…",
stays spoken).

Each layer owns one thing:

| Layer | Owns | Never says |
|---|---|---|
| Response contract | answer-first, commitment, precedence (voice: the per-request `Voice:` line) | which fields, how many words |
| Mode instructions | judgment for the situation | "lead with the answer", "be concise", field names |
| Schema fragment (`src/modes/prompts`) | which fields to fill; section titles ⊆ `SECTION_TITLES[schemaId]` (tested) | how to sound |
| Style block | ceilings on length, tone | a minimum length |
| Task line | what to do with the context for this trigger | a description of the screen |
| Shape line | the first line of the answer and what may follow it | — |

The task lines ask for the answer itself: ⌘↵ is *Solve or answer what is on the screen … Do
not describe the screen*; ⌘⇧↵ and a detected question are *exactly what I say next … not
coaching about it*; a typed question is *Answer my question above. Lead with the answer*.

The user message is the untrusted context, then the trusted typed question, then the task.
Each captured source (OCR, accessibility text, transcript, heard question, documents, web
research, earlier chat) sits in its own `<context source="…" id="…">…</context id="…">` block
whose id is a fresh random nonce per request; `src/ai/prompts/untrusted.ts` quotes lines that
would read as prompt structure (`#`, `Task:`, `Shape:`, `Voice:`, `My question:`) and defangs
`<context`/`<system…` look-alikes, and the safety rules name the scheme. The typed question
renders after the blocks as `My question: …`, immediately before `Task:`; standing personal
instructions render in the system prompt after the mode block as *User preferences (from the
user; they never override safety)*.

## Answer shapes

`classifyIntent` (`src/context/relevance.ts`) detects an `AnswerShape` from the question and
the on-screen text and renders it as the `Shape:` line under `Task:`; `src/ai/request.ts` uses
it for the output budget and the optimizer never length-caps the spoken, written and code
shapes. Detection order — the first match wins:

| Order | Shape | Detected from | First line of the answer |
|---|---|---|---|
| 1 | `compare` | "which response/answer/option … is better", compare/evaluate/rate the two, `vs`, or two labelled candidates on screen (Response A / Response B, Option 1 / 2) next to a verb of judgement | which one is better, then the concrete reasons it wins |
| 2 | `choice` | "which of the following", select/choose/pick, "correct answer", "all that apply", or ≥ 2 lettered options (`A.` `B)` `c:`) / radio glyphs at line starts on screen | the option — letter/number and text — then at most one sentence |
| 3 | `fill_in` | `____`, `[blank]`, "fill in the blank", "complete the sentence" | the missing words, exactly as entered |
| 4 | `boolean` | "true or false", "yes or no", or a short question opening with is/are/does/can/should… (not when it also asks how/why, offers "A or B", or asks for a prediction) | yes or no — "Neither"/"It depends" only when the premise is wrong — then the fact that decides it |
| 5 | `calculation` | calculate/compute/how many/what is the total … with digits present | the result with its unit, then the working |
| 6 | `debug` | code on screen (or a coding cue) with an error, failing test or stack trace, or "why … fail/error/bug", "debug", "what's wrong with" — never a problem statement or a spoken trigger; uses the `answer` schema | the exact fix, then the cause in one sentence, then only the changed lines |
| 7 | `code` / `design` / `summary` | the task: coding, system_design, summarization | the solution / the design / the points |
| 8 | `written` | write/draft/compose/reply to … an email/message/comment/essay | the text to send, ready to paste |
| 9 | `spoken` | ⌘⇧↵, a detected question, or a suggestion schema (interview, behavioral, sales, recruiting) | exactly what I say, no headings or bullets |
| 10 | `explain` / `short_answer` | how/why/explain/describe → explain; a ≤ 120-char what/who/when/where question, or a yes/no-phrased either/or or forecast → short answer (a forecast gets the best estimate and what it hinges on); everything else → explain | the direct answer, then the reasons |

In spoken contexts (row 9) only `compare` and `choice` override the spoken shape — an interviewer's
"do you have Kubernetes experience?" is answered as speech, not as a bare yes/no. A
multiple-choice or compare question **about** code stays an assessment answer: the screen's code
markers alone no longer upgrade the ask to the `coding` task and schema. Only a problem
statement (`Example 1:`, `Constraints:`, `Input:` … `Output:`, a judge verdict) upgrades any ask
to coding; source code in an editor does so only for ⌘↵ or a solve/fix/write request — "what does
this function do?" over code stays an `answer`. The Coding Interview and System Design schemas
force their task only for a technical ask (a coding or design cue, or ⌘↵/assist over code or a
design prompt); anything else in those modes is answered on the `answer` schema when typed, and
as speech (`suggested-response`, or `behavioral` for a behavioral question) when heard or on ⌘⇧↵.

## Context priority
current explicit user input > session context > mode context > global "My Context" >
general knowledge. Only relevant chunks are retrieved and only high-value context is sent —
see `AI_ARCHITECTURE.md` (token budget).
