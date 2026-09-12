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
| General (default) | answer | fast | screen, accessibility, transcript, documents | the answer, first person, answer first |
| Interview | suggested-response | ultra-fast | transcript, resume, job_description, screen | exactly what the candidate says next |
| Behavioral Interview | behavioral | fast | transcript, resume | the spoken answer (STAR inside, unlabelled) · Story used · Key point |
| Coding Interview | coding | balanced | screen, accessibility, transcript | approach + exact code in `content` and `code` · Complexity · Edge cases |
| System Design | system-design | deep | screen, transcript | requirements → … → trade-offs, optional Mermaid diagram |
| Case Interview | case | balanced | transcript, screen | the next thing to say · Clarify · Framework · Analyze · Calculate · Synthesize · Recommend |
| Sales | sales | ultra-fast | transcript, documents | what the seller says next · Why it works · Optional follow-up |
| Recruiting | recruiting | fast | transcript, documents, job_description | what the recruiter says or asks next · Screening notes · Next step |
| Team Meeting | meeting | fast | transcript, session_memory | live: Important / Decision / Action item / Question; after: Summary … |
| Lecture | lecture | fast | transcript, screen, session_memory | concepts, definitions, notes, study guide, questions |

Definitions live in `bluey_core::modes::built_in_modes` and are seeded into SQLite on first
run; users can edit instructions and attach files. *Reset to default* restores the original.

**Built-in instructions are judgment only** (90–180 words each, enforced by
`bluey_core::modes` tests): what a strong answer in that situation gets right — which story to
pick, how to size a design, when to stay silent in a meeting. Voice, length and fields are owned
by the layers below, so a mode never says "lead with the answer" or "be concise" — the response
contract already does, for every mode. Custom modes get the same treatment: write the judgment,
not the format.

## Custom modes
Settings → Modes → **New Mode**: name, description, instructions ("Meeting context"), response
style, preferred output schema, attached context files, preferred model role, latency
preference. Persisted in the `modes` table. Actions: duplicate, edit, delete, set default,
set active. Validation: `validateModeDraft` (name 1–48 chars, instructions ≤ 8k chars).

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
   Team Meeting, behavioral cues in interviews).
6. **Summary** — the post-session summary is structured per mode (e.g. lecture study guide,
   meeting decisions/action items).

## Voice and precedence

Every ask carries the **response contract** (`RESPONSE_CONTRACT`, `src/ai/prompts/system.ts`)
right after the identity: lead with the answer (never a restatement of the question or a
description of the screen), write as the user in the first person, match the shape of the
question, explain only what earns its place, commit to one answer, and — the precedence rule —
the contract and the mode govern voice and content; the **style block** only sets ceilings
(`Length ceiling: concise — at most ~120 words … a one-line answer is complete`), never a
minimum; the **schema fragment** names fields and section titles and never changes the voice.

Each layer owns one thing:

| Layer | Owns | Never says |
|---|---|---|
| Response contract | voice, answer-first, commitment, precedence | which fields, how many words |
| Mode instructions | judgment for the situation | "lead with the answer", "be concise", field names |
| Schema fragment (`src/modes/prompts`) | which fields to fill; section titles ⊆ `SECTION_TITLES[schemaId]` (tested) | how to sound |
| Style block | ceilings on length, tone | a minimum length |
| Task line | what to do with the context for this trigger | a description of the screen |
| Shape line | the first line of the answer and what may follow it | — |

The task lines ask for the answer itself: ⌘↵ is *Solve or answer what is on the screen … Do
not describe the screen*; ⌘⇧↵ and a detected question are *exactly what I say next … not
coaching about it*; a typed question is *Answer my question below. Lead with the answer*.

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
| 4 | `boolean` | "true or false", "yes or no", or a short question opening with is/are/does/can/should… (not when it also asks how/why) | yes or no, then one reason |
| 5 | `calculation` | calculate/compute/how many/what is the total … with digits present | the result with its unit, then the working |
| 6 | `code` / `design` / `summary` | the task: coding, system_design, summarization | the solution / the design / the points |
| 7 | `written` | write/draft/compose/reply to … an email/message/comment/essay | the text to send, ready to paste |
| 8 | `spoken` | ⌘⇧↵, a detected question, or a suggestion schema (interview, behavioral, sales, recruiting) | exactly what I say, no headings or bullets |
| 9 | `explain` / `short_answer` | how/why/explain/describe → explain; a ≤ 120-char what/who/when/where question → short answer; everything else → explain | the direct answer, then the reasons |

In spoken contexts (row 8) only `compare` and `choice` override the spoken shape — an interviewer's
"do you have Kubernetes experience?" is answered as speech, not as a bare yes/no. A
multiple-choice or compare question **about** code stays an assessment answer: the screen's code
markers alone no longer upgrade the ask to the `coding` task and schema.

## Context priority
current explicit user input > session context > mode context > global "My Context" >
general knowledge. Only relevant chunks are retrieved and only high-value context is sent —
see `AI_ARCHITECTURE.md` (token budget).
