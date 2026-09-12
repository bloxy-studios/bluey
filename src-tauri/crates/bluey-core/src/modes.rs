//! The 10 built-in modes as **data** (spec: modes are data, not code). Each
//! mode carries its system instructions, response schema, latency preference,
//! context requirements and sidebar grouping. The app seeds storage from
//! [`built_in_modes`]; users may edit instructions but built-ins are never
//! deleted.

use crate::types::{
    BlueyMode, ContextRequirement, ModelRole, PreferredLatency, ResponseSchemaId, ResponseStyle,
    BUILT_IN_MODE_IDS,
};

/// Sidebar group for job seekers.
pub const GROUP_LOOKING_FOR_WORK: &str = "Looking for work";
/// Sidebar group for professional/at-work modes.
pub const GROUP_WORK: &str = "Work";
/// Sidebar group for learning modes.
pub const GROUP_LEARNING: &str = "Learning";

struct ModeSpec {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    icon: &'static str,
    group: Option<&'static str>,
    schema: ResponseSchemaId,
    latency: PreferredLatency,
    requirements: &'static [ContextRequirement],
    preferred_model_role: Option<ModelRole>,
    instructions: &'static str,
}

/// All built-in modes, with `created_at`/`updated_at` set to `now` (an RFC 3339
/// timestamp). Order and ids match [`BUILT_IN_MODE_IDS`].
pub fn built_in_modes(now: &str) -> Vec<BlueyMode> {
    SPECS
        .iter()
        .map(|spec| BlueyMode {
            id: spec.id.to_string(),
            name: spec.name.to_string(),
            description: spec.description.to_string(),
            icon: spec.icon.to_string(),
            system_instructions: spec.instructions.trim().to_string(),
            response_schema: spec.schema,
            preferred_latency: spec.latency,
            context_requirements: spec.requirements.to_vec(),
            built_in: true,
            group: spec.group.map(String::from),
            response_style: None,
            preferred_model_role: spec.preferred_model_role,
            attached_document_ids: Vec::new(),
            created_at: now.to_string(),
            updated_at: now.to_string(),
        })
        .collect()
}

/// A built-in mode by id (timestamps set to now), or `None` for unknown ids.
pub fn mode_by_id(id: &str) -> Option<BlueyMode> {
    if !is_built_in(id) {
        return None;
    }
    let now = crate::now_iso();
    built_in_modes(&now).into_iter().find(|m| m.id == id)
}

/// Whether `id` is one of the 10 built-in mode ids.
pub fn is_built_in(id: &str) -> bool {
    BUILT_IN_MODE_IDS.contains(&id)
}

/// The response style in effect for a mode: the global style with the mode's
/// partial override applied on top.
pub fn effective_style(mode: &BlueyMode, global: &ResponseStyle) -> ResponseStyle {
    let mut style = global.clone();
    if let Some(patch) = &mode.response_style {
        if let Some(length) = patch.length {
            style.length = length;
        }
        if let Some(tone) = patch.tone {
            style.tone = tone;
        }
    }
    style
}

const SPECS: [ModeSpec; 10] = [
    ModeSpec {
        id: "general",
        name: "General",
        description: "Everyday copilot for whatever is on your screen or in the conversation.",
        icon: "file-text",
        group: None,
        schema: ResponseSchemaId::Answer,
        latency: PreferredLatency::Fast,
        requirements: &[
            ContextRequirement::Screen,
            ContextRequirement::Accessibility,
            ContextRequirement::Transcript,
            ContextRequirement::SessionMemory,
        ],
        preferred_model_role: None,
        instructions: GENERAL_INSTRUCTIONS,
    },
    ModeSpec {
        id: "interview",
        name: "Interview",
        description: "Real-time suggested answers in live job interviews, grounded in your resume.",
        icon: "graduation-cap",
        group: Some(GROUP_LOOKING_FOR_WORK),
        schema: ResponseSchemaId::SuggestedResponse,
        latency: PreferredLatency::UltraFast,
        requirements: &[
            ContextRequirement::Transcript,
            ContextRequirement::Resume,
            ContextRequirement::JobDescription,
            ContextRequirement::SessionMemory,
        ],
        preferred_model_role: None,
        instructions: INTERVIEW_INSTRUCTIONS,
    },
    ModeSpec {
        id: "behavioral-interview",
        name: "Behavioral Interview",
        description: "STAR-shaped answers to behavioral questions, told naturally from your resume.",
        icon: "message-square-quote",
        group: Some(GROUP_LOOKING_FOR_WORK),
        schema: ResponseSchemaId::Behavioral,
        latency: PreferredLatency::Fast,
        requirements: &[
            ContextRequirement::Transcript,
            ContextRequirement::Resume,
            ContextRequirement::SessionMemory,
        ],
        preferred_model_role: None,
        instructions: BEHAVIORAL_INSTRUCTIONS,
    },
    ModeSpec {
        id: "coding-interview",
        name: "Coding Interview",
        description: "Restates the problem, explains the approach and writes complete solutions with complexity.",
        icon: "code-xml",
        group: Some(GROUP_LOOKING_FOR_WORK),
        schema: ResponseSchemaId::Coding,
        latency: PreferredLatency::Balanced,
        requirements: &[
            ContextRequirement::Screen,
            ContextRequirement::Accessibility,
            ContextRequirement::Transcript,
            ContextRequirement::SessionMemory,
        ],
        preferred_model_role: Some(ModelRole::Default),
        instructions: CODING_INSTRUCTIONS,
    },
    ModeSpec {
        id: "system-design",
        name: "System Design",
        description: "Structured design walkthroughs from requirements to trade-offs, with optional diagrams.",
        icon: "boxes",
        group: Some(GROUP_LOOKING_FOR_WORK),
        schema: ResponseSchemaId::SystemDesign,
        latency: PreferredLatency::Deep,
        requirements: &[
            ContextRequirement::Screen,
            ContextRequirement::Accessibility,
            ContextRequirement::Transcript,
            ContextRequirement::SessionMemory,
        ],
        preferred_model_role: Some(ModelRole::Reasoning),
        instructions: SYSTEM_DESIGN_INSTRUCTIONS,
    },
    ModeSpec {
        id: "case-interview",
        name: "Case Interview",
        description: "Framework-first guidance through consulting cases, with clean calculations.",
        icon: "presentation",
        group: Some(GROUP_LOOKING_FOR_WORK),
        schema: ResponseSchemaId::Case,
        latency: PreferredLatency::Balanced,
        requirements: &[
            ContextRequirement::Screen,
            ContextRequirement::Transcript,
            ContextRequirement::Documents,
            ContextRequirement::SessionMemory,
        ],
        preferred_model_role: None,
        instructions: CASE_INSTRUCTIONS,
    },
    ModeSpec {
        id: "sales",
        name: "Sales",
        description: "Objection handling and suggested responses on live sales calls.",
        icon: "store",
        group: Some(GROUP_WORK),
        schema: ResponseSchemaId::Sales,
        latency: PreferredLatency::UltraFast,
        requirements: &[
            ContextRequirement::Transcript,
            ContextRequirement::Documents,
            ContextRequirement::SessionMemory,
        ],
        preferred_model_role: None,
        instructions: SALES_INSTRUCTIONS,
    },
    ModeSpec {
        id: "recruiting",
        name: "Recruiting",
        description: "Screening questions, candidate answers and role pitches for recruiters.",
        icon: "briefcase",
        group: Some(GROUP_WORK),
        schema: ResponseSchemaId::Recruiting,
        latency: PreferredLatency::Fast,
        requirements: &[
            ContextRequirement::Screen,
            ContextRequirement::Transcript,
            ContextRequirement::JobDescription,
            ContextRequirement::Documents,
            ContextRequirement::SessionMemory,
        ],
        preferred_model_role: None,
        instructions: RECRUITING_INSTRUCTIONS,
    },
    ModeSpec {
        id: "team-meeting",
        name: "Team Meeting",
        description: "Live decisions, action items and open questions, plus a clean recap.",
        icon: "video",
        group: Some(GROUP_WORK),
        schema: ResponseSchemaId::Meeting,
        latency: PreferredLatency::Fast,
        requirements: &[ContextRequirement::Transcript, ContextRequirement::SessionMemory],
        preferred_model_role: None,
        instructions: MEETING_INSTRUCTIONS,
    },
    ModeSpec {
        id: "lecture",
        name: "Lecture",
        description: "Live notes, explanations, study guides and practice questions from lectures.",
        icon: "book-open",
        group: Some(GROUP_LEARNING),
        schema: ResponseSchemaId::Lecture,
        latency: PreferredLatency::Fast,
        requirements: &[
            ContextRequirement::Screen,
            ContextRequirement::Transcript,
            ContextRequirement::SessionMemory,
        ],
        preferred_model_role: None,
        instructions: LECTURE_INSTRUCTIONS,
    },
];

// ── System instructions ──────────────────────────────────────────────────────
//
// Judgment only. Voice (first person, answer first), length ceilings and the
// output fields are owned by the response contract, the style block and the
// schema fragment the WebView sends with every ask (`src/ai/prompts`,
// `src/modes/prompts`; docs/MODE_SYSTEM.md). A mode says what a strong answer
// in this situation gets right — never "lead with the answer" again.

const GENERAL_INSTRUCTIONS: &str = r#"
You are Bluey in General mode: whatever is on the screen or in the conversation, the user gets the one thing they need right now.

Judgment calls:
- Resolve "this", "that" and "it" from the screen or the last transcript lines before answering; never ask what was meant when the screen shows it.
- When the visible thing is a question or a problem, the answer is the deliverable; when it is content with no question, the most useful fact, fix or next step about it is.
- Transform tasks (rewrite, translate, convert, extract) return the finished result ready to paste, working from the exact visible text — quote, do not paraphrase.
- Code on screen: name the concrete identifiers, files and lines you can see.
- Missing context (wrong window, nothing heard): one sentence saying what to show or repeat, no apology.
- Keep numbers, units and names exactly as the source has them.
"#;

const INTERVIEW_INSTRUCTIONS: &str = r#"
The user is a candidate in a live job interview; every answer is words they can say out loud, in a natural spoken register — contractions, plain words, varied sentence length. A person talking, never an essay.

Judgment calls:
- Ground truth is the resume and the job description when provided: only the employers, projects, technologies, dates and outcomes that appear there. When the resume lacks what the question needs, stay honestly generic ("in a previous role…") or leave a bracketed slot like [your example] to fill while speaking.
- One direct sentence answers the question; two to four sentences of real experience support it; an optional closing line ties it to this role. Under ~120 spoken words unless the question demands more.
- An ambiguous question gets the most likely reading answered, plus one trailing line for the other reading.
- Half-heard question or no resume: answer instantly with a skeleton and bracketed slots rather than pausing.
- Mirror the interviewer's terminology; confident, never arrogant.
"#;

const BEHAVIORAL_INSTRUCTIONS: &str = r#"
The user is answering behavioral questions — "Tell me about a time…", "Describe a situation…", "How do you handle…" — and speaks the answer as natural prose.

Judgment calls:
- Shape every answer as STAR inside — situation, task, action, result — without labels and without visible seams: a sentence or two of scene, the action told as a story with the user as the actor ("so I did X"), a concrete result with a number when the resume has one, one closing line on what they learned.
- Pick the single strongest, most relevant experience for the competency probed — leadership, conflict, failure, ambiguity, deadline pressure, influence; prefer recent, senior and quantified. Never blend jobs into one anecdote; where a detail is missing, leave a neutral bracketed placeholder.
- Around 60–90 seconds of speech.
- A question that turns out not to be behavioral gets a brief plain interview answer, not a forced story.
"#;

const CODING_INSTRUCTIONS: &str = r##"
The user is in a live coding interview; the working solution is the deliverable and everything else supports it.

Judgment calls:
- Infer the language from the visible editor, file extension or judge UI; Python only when nothing indicates otherwise. Conform exactly to any visible signature or harness: same function name, parameter order and return type.
- Code is complete and runnable — exact, consistent indentation, never truncated or elided ("# rest omitted", "…"), meaningful names, comments only where the logic is genuinely non-obvious.
- Name the pattern behind the approach (two pointers, sliding window, BFS, dynamic programming…) and why it beats the naive solution, in a few lines.
- Complexity: time and space with a one-line reason each. Edge cases: the inputs that break naive solutions (empty, single element, duplicates, negatives, overflow, ties) and how the code handles them.
- A partially visible statement gets the most reasonable reading, with the assumption stated in one line — never stall.
- Follow-ups ("optimize it", "what if it's sorted?") change only what changes: the delta and the updated code, not a re-derivation.
"##;

const SYSTEM_DESIGN_INSTRUCTIONS: &str = r#"
The user is in a system design interview; the design and its trade-offs, stated the way a strong senior engineer would at a whiteboard, are the deliverable.

Judgment calls:
- State assumptions where the prompt is silent so the user can say them aloud; tailor every number to the stated scale, and when none is given assume large and say so.
- Quantify: users, QPS, storage, bandwidth, with the arithmetic shown briefly.
- Cover what this system actually turns on — requirements (functional and non-functional), the request flow through the components, the core APIs, the data model and where each entity lives, storage choices and why, caching and invalidation, async work, consistency, partitioning and replication, failure modes and rate limits, security — and skip what genuinely does not apply rather than padding.
- Trade-offs are the substance: two to four decisions, each with the road not taken and why.
- A Mermaid diagram of the architecture is welcome once the design has more than a handful of components.
"#;

const CASE_INSTRUCTIONS: &str = r#"
The user is in a consulting case interview — market sizing, profitability, market entry, M&A, pricing, operations. Cases are scored on process, so the deliverable is the next thing to say at this stage of the case.

Judgment calls:
- Early: the two or three clarifying questions worth asking (objective, timeframe, constraints, definitions) and a MECE structure tailored to this exact case — three or four branches, one line each; never recite a canned framework name as the answer.
- Mid-case: which branch to attack first and what data to request. When numbers appear, do the arithmetic in a clearly formatted block — assumptions, then steps, then the result rounded to speakable figures.
- At the end: a one-minute synthesis — recommendation, two or three supporting reasons, key risks — and the concrete next step to propose.
- New data updates the affected branch; never restart. Flag any assumption the user should state aloud before relying on it.
- Skimmable at a glance while the user keeps talking.
"#;

const SALES_INSTRUCTIONS: &str = r#"
The user is the seller on a live call; the deliverable is the exact next thing to say — natural, conversational, one to three sentences. Acknowledge first, then reframe; never argue with the prospect, never sound scripted.

Judgment calls by moment:
- Objection: validate it, isolate it ("is that the main thing holding you back?"), then address it.
- Pricing: anchor on value and outcomes before numbers; never offer a discount unprompted.
- Competitor mention: stay respectful, differentiate on the prospect's own stated needs, no trash talk.
- Buying signal: name it and move to a concrete next step — demo, pilot, proposal date.
- Product question: answer plainly from the product facts in the provided documents; when the fact is not there, commit to a follow-up rather than guess.
- Negotiation: trade, don't cave — every concession gets something in return.

Reuse the prospect's own words. The whole output must be readable in under five seconds.
"#;

const RECRUITING_INSTRUCTIONS: &str = r#"
The user is the recruiter on a live candidate call — screen, intake or offer conversation; you are on their side of the table. The deliverable is what they say next or the next question to ask.

Judgment calls:
- Lines about the role, team, process and timeline come only from the job description and company notes provided; never invent benefits, compensation figures or promises.
- The next screening question probes for specifics — scope, technologies, team size, outcomes, dates — never a yes/no confirmation. A vague answer gets a natural probe ("you mentioned leading the migration — what was your part specifically?").
- Compensation: share the posted range when the context has one; otherwise ask the candidate's expectations first. Flag anything that looks internal-only as not to be said aloud.
- Track coverage in one short line as the call goes: motivation, experience match, compensation, logistics and notice period, next steps.

Warm, professional, efficient; the candidate leaves with a clear picture and a concrete next step.
"#;

const MEETING_INSTRUCTIONS: &str = r#"
The user is in a live meeting. Silence beats noise: surface only the genuinely significant moments, each as a single line the user can act on.

Judgment calls:
- Important: a metric, commitment, risk or date worth remembering. Decision: what was decided in one sentence, plus who decided when that is clear. Action item: task — owner — deadline, the owner a speaker label from the transcript or "unassigned", the deadline only if one was actually said. Question: an open question aimed at the user or left hanging.
- Attribute strictly with the speaker labels present in the transcript; never guess a name that was not spoken. Use the participants' own words for the substance of decisions and action items.
- On a recap request or when the meeting ends: three to six bullets of what was discussed and concluded, decisions with their stated rationale, action items merged and de-duplicated, open questions.
- Unclear attribution, deadline or number: write "(unclear)" instead of inventing one.
"#;

const LECTURE_INSTRUCTIONS: &str = r#"
The user is following a lecture, talk or course video. Track topics as they change, definitions verbatim where precision matters, worked examples, and formulas, dates and names exactly as given.

Judgment calls:
- Ground everything strictly in this lecture's transcript and slides, and always separate what the lecturer said from what you add as explanation.
- Explaining the last concept: simpler words, then one new example the lecturer did not use, clearly marked as yours. A recap of the last few minutes: three to five bullets.
- Notes are hierarchical — topic headings, sub-points, definitions in bold. A study guide is what is testable: key terms with definitions, main results and theorems, the pitfalls the lecturer flagged. Likely exam questions mix recall and application, each with a brief model answer.
- When the lecturer corrects an earlier statement, prefer the correction and note the change.
- Unclear audio at a key moment: mark it "[unclear]" so the user reviews that part.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn modes() -> Vec<BlueyMode> {
        built_in_modes("2026-09-07T00:00:00.000Z")
    }

    #[test]
    fn ten_modes_with_unique_ids_matching_the_constant() {
        let modes = modes();
        assert_eq!(modes.len(), 10);
        let ids: Vec<&str> = modes.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            BUILT_IN_MODE_IDS.to_vec(),
            "ids and order match BUILT_IN_MODE_IDS"
        );
        let unique: std::collections::HashSet<&&str> = ids.iter().collect();
        assert_eq!(unique.len(), 10);
        assert!(modes.iter().all(|m| m.built_in));
        assert!(modes
            .iter()
            .all(|m| m.created_at == "2026-09-07T00:00:00.000Z"));
    }

    #[test]
    fn instructions_are_substantial_and_specific() {
        for m in modes() {
            let words = m.system_instructions.split_whitespace().count();
            // Judgment only — voice, length and fields come from the WebView's
            // response contract, style block and schema fragment (docs/MODE_SYSTEM.md).
            assert!(
                words >= 80,
                "{} instructions too short: {words} words",
                m.id
            );
            assert!(
                words <= 220,
                "{} instructions too long: {words} words",
                m.id
            );
            assert!(!m.description.is_empty());
        }
    }

    #[test]
    fn schema_latency_group_icon_and_role_mapping() {
        let by_id = |id: &str| mode_by_id(id).expect("built-in mode");

        let m = by_id("general");
        assert_eq!(m.response_schema, ResponseSchemaId::Answer);
        assert_eq!(m.preferred_latency, PreferredLatency::Fast);
        assert_eq!(m.group, None);
        assert_eq!(m.icon, "file-text");
        assert_eq!(m.preferred_model_role, None);

        let m = by_id("interview");
        assert_eq!(m.response_schema, ResponseSchemaId::SuggestedResponse);
        assert_eq!(m.preferred_latency, PreferredLatency::UltraFast);
        assert_eq!(m.group.as_deref(), Some(GROUP_LOOKING_FOR_WORK));
        assert_eq!(m.icon, "graduation-cap");

        let m = by_id("behavioral-interview");
        assert_eq!(m.response_schema, ResponseSchemaId::Behavioral);
        assert_eq!(m.preferred_latency, PreferredLatency::Fast);
        assert_eq!(m.icon, "message-square-quote");

        let m = by_id("coding-interview");
        assert_eq!(m.response_schema, ResponseSchemaId::Coding);
        assert_eq!(m.preferred_latency, PreferredLatency::Balanced);
        assert_eq!(m.preferred_model_role, Some(ModelRole::Default));
        assert_eq!(m.icon, "code-xml");

        let m = by_id("system-design");
        assert_eq!(m.response_schema, ResponseSchemaId::SystemDesign);
        assert_eq!(m.preferred_latency, PreferredLatency::Deep);
        assert_eq!(m.preferred_model_role, Some(ModelRole::Reasoning));
        assert_eq!(m.icon, "boxes");

        let m = by_id("case-interview");
        assert_eq!(m.response_schema, ResponseSchemaId::Case);
        assert_eq!(m.preferred_latency, PreferredLatency::Balanced);
        assert_eq!(m.group.as_deref(), Some(GROUP_LOOKING_FOR_WORK));
        assert_eq!(m.icon, "presentation");

        let m = by_id("sales");
        assert_eq!(m.response_schema, ResponseSchemaId::Sales);
        assert_eq!(m.preferred_latency, PreferredLatency::UltraFast);
        assert_eq!(m.group.as_deref(), Some(GROUP_WORK));
        assert_eq!(m.icon, "store");

        let m = by_id("recruiting");
        assert_eq!(m.response_schema, ResponseSchemaId::Recruiting);
        assert_eq!(
            m.group.as_deref(),
            Some(GROUP_WORK),
            "recruiting is for recruiters at work"
        );
        assert_eq!(m.icon, "briefcase");

        let m = by_id("team-meeting");
        assert_eq!(m.response_schema, ResponseSchemaId::Meeting);
        assert_eq!(m.group.as_deref(), Some(GROUP_WORK));
        assert_eq!(m.icon, "video");

        let m = by_id("lecture");
        assert_eq!(m.response_schema, ResponseSchemaId::Lecture);
        assert_eq!(m.group.as_deref(), Some(GROUP_LEARNING));
        assert_eq!(m.icon, "book-open");
    }

    #[test]
    fn context_requirements_are_sensible() {
        let by_id = |id: &str| mode_by_id(id).expect("built-in mode");
        assert!(by_id("interview")
            .context_requirements
            .contains(&ContextRequirement::Resume));
        assert!(by_id("interview")
            .context_requirements
            .contains(&ContextRequirement::JobDescription));
        assert!(by_id("coding-interview")
            .context_requirements
            .contains(&ContextRequirement::Screen));
        assert!(by_id("team-meeting")
            .context_requirements
            .contains(&ContextRequirement::Transcript));
        assert!(by_id("lecture")
            .context_requirements
            .contains(&ContextRequirement::Transcript));
    }

    #[test]
    fn mode_by_id_and_is_built_in() {
        assert!(is_built_in("general"));
        assert!(is_built_in("team-meeting"));
        assert!(!is_built_in("my-custom-mode"));
        assert_eq!(
            mode_by_id("sales").map(|m| m.name),
            Some("Sales".to_string())
        );
        assert_eq!(mode_by_id("nope"), None);
    }

    #[test]
    fn effective_style_applies_mode_overrides() {
        use crate::types::{ResponseLength, ResponseStylePatch, ResponseTone};
        let mut mode = mode_by_id("general").expect("mode");
        let global = ResponseStyle {
            length: ResponseLength::Concise,
            tone: ResponseTone::Natural,
        };
        assert_eq!(effective_style(&mode, &global), global);

        mode.response_style = Some(ResponseStylePatch {
            length: Some(ResponseLength::Detailed),
            tone: None,
        });
        let effective = effective_style(&mode, &global);
        assert_eq!(effective.length, ResponseLength::Detailed);
        assert_eq!(
            effective.tone,
            ResponseTone::Natural,
            "unset patch fields keep the global"
        );
    }

    #[test]
    fn modes_serialize_camel_case() {
        let m = mode_by_id("general").expect("mode");
        let v = serde_json::to_value(&m).expect("serialize");
        assert!(v.get("systemInstructions").is_some());
        assert!(v.get("responseSchema").is_some());
        assert_eq!(v["preferredLatency"], "fast");
        assert_eq!(v["builtIn"], true);
    }
}
