import { describe, expect, it } from "vitest";

import { describeImport, formatOffset, speakerLabel } from "@/features/settings/session-import";
import type { TranscribeFileResult } from "@/lib/types";

function result(partial: Partial<TranscribeFileResult> = {}): TranscribeFileResult {
  return {
    session: {
      id: "ses-1",
      modeId: "general",
      startedAt: "2026-09-08T10:00:00.000Z",
      status: "completed",
      title: "Imported · standup.wav",
    },
    segments: [],
    speakers: 0,
    durationMs: 0,
    stored: true,
    ...partial,
  };
}

const segment = { id: "seg", source: "system" as const, text: "", startTime: 0, endTime: 0, finalized: true, createdAt: "" };

describe("session-import helpers", () => {
  it("describes an import with counts, speakers and storage state", () => {
    expect(describeImport(result({ segments: [{ ...segment }], speakers: 0 }))).toBe(
      "Imported 1 segment into “Imported · standup.wav”",
    );
    expect(
      describeImport(result({ segments: [{ ...segment }, { ...segment, id: "seg-2" }], speakers: 2, stored: false })),
    ).toBe("Imported 2 segments from 2 speakers into “Imported · standup.wav” — not stored (transcript storage is off)");
    expect(describeImport(result({ session: { ...result().session, title: undefined } }))).toContain(
      "“Untitled session”",
    );
  });

  it("labels diarized speakers and falls back to the audio source", () => {
    expect(speakerLabel({ speaker: "spk_1", source: "system" })).toBe("Speaker 1");
    expect(speakerLabel({ speaker: "spk_12", source: "system" })).toBe("Speaker 12");
    expect(speakerLabel({ speaker: "Interviewer", source: "system" })).toBe("Interviewer");
    expect(speakerLabel({ source: "microphone" })).toBe("You");
    expect(speakerLabel({ source: "system" })).toBe("Them");
  });

  it("formats offsets as mm:ss and h:mm:ss", () => {
    expect(formatOffset(0)).toBe("00:00");
    expect(formatOffset(4_000)).toBe("00:04");
    expect(formatOffset(250_000)).toBe("04:10");
    expect(formatOffset(3_725_000)).toBe("1:02:05");
    expect(formatOffset(-5)).toBe("00:00");
  });
});
