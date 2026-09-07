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
| General (default) | answer | fast | screen, accessibility, transcript, documents | direct, concise, contextual answer |
| Interview | suggested-response | ultra-fast | transcript, resume, job_description, screen | what a human interviewee would naturally say |
| Behavioral Interview | behavioral | fast | transcript, resume | Suggested answer · Story used · Key point (STAR internally) |
| Coding Interview | coding | balanced | screen, accessibility, transcript | Approach · Solution (exact code) · Complexity · Edge cases |
| System Design | system-design | deep | screen, transcript | requirements → … → trade-offs, optional Mermaid diagram |
| Case Interview | case | balanced | transcript, screen | Clarify · Framework · Analyze · Calculate · Synthesize · Recommend |
| Sales | sales | ultra-fast | transcript, documents | Suggested response · Why it works · Optional follow-up |
| Recruiting | recruiting | fast | transcript, documents, job_description | candidate responses, qualification questions, follow-ups |
| Team Meeting | meeting | fast | transcript, session_memory | live: Important / Decision / Action item / Question; after: Summary … |
| Lecture | lecture | fast | transcript, screen, session_memory | concepts, definitions, notes, study guide, questions |

Definitions live in `bluey_core::modes::built_in_modes` and are seeded into SQLite on first
run; users can edit instructions and attach files. *Reset to default* restores the original.

## Custom modes
Settings → Modes → **New Mode**: name, description, instructions ("Meeting context"), response
style, preferred output schema, attached context files, preferred model role, latency
preference. Persisted in the `modes` table. Actions: duplicate, edit, delete, set default,
set active. Validation: `validateModeDraft` (name 1–48 chars, instructions ≤ 8k chars).

## How a mode shapes a response
1. **Prompt** — `mode.systemInstructions` + the schema's `ModePrompt` fragment + style.
2. **Context** — `contextRequirements` decide which sources are gathered and how retrieval is
   scoped (candidate modes pull resume/CV chunks; recruiting/sales pull JD/company notes).
3. **Routing** — `preferredLatency` and `preferredModelRole` feed the model router.
4. **Output** — `responseSchema` selects the structured schema and the HUD renderer (sections,
   code block with copy/expand, diagram view, calculations separated).
5. **Classifier** — detection rules adapt to the mode (objections in Sales, decisions/actions in
   Team Meeting, behavioral cues in interviews).
6. **Summary** — the post-session summary is structured per mode (e.g. lecture study guide,
   meeting decisions/action items).

## Context priority
current explicit user input > session context > mode context > global "My Context" >
general knowledge. Only relevant chunks are retrieved and only high-value context is sent —
see `AI_ARCHITECTURE.md` (token budget).
