/** Typed loader for the JSON fixture sets in tests/fixtures/<name>/. */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import type {
  AITask,
  BlueyMode,
  ContextSnapshot,
  DetectedEventType,
  OCRContext,
  ResponseSchemaId,
  ResponseType,
  TranscriptSegment,
} from "@/lib/types";

export const FIXTURE_NAMES = [
  "interview",
  "behavioral",
  "coding",
  "system-design",
  "sales",
  "meeting",
  "lecture",
] as const;

export type FixtureName = (typeof FIXTURE_NAMES)[number];

export interface FixtureExpected {
  /** Explicit typed instruction for the ask, when the fixture uses one. */
  instruction?: string;
  task: AITask;
  schemaId: ResponseSchemaId;
  responseType: ResponseType;
  visionRequired: boolean;
  detectedEventType: DetectedEventType;
  requiresResponse: boolean;
}

export interface Fixture {
  name: FixtureName;
  transcript: TranscriptSegment[];
  ocr: OCRContext;
  mode: BlueyMode;
  snapshot: ContextSnapshot;
  expected: FixtureExpected;
}

function readJson<T>(name: FixtureName, file: string): T {
  // vitest cwd is the project root; import.meta.url is not reliable across
  // its transform pipeline, so resolve from cwd.
  const path = join(process.cwd(), "tests", "fixtures", name, file);
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

export function loadFixture(name: FixtureName): Fixture {
  return {
    name,
    transcript: readJson<TranscriptSegment[]>(name, "transcript.json"),
    ocr: readJson<OCRContext>(name, "ocr.json"),
    mode: readJson<BlueyMode>(name, "mode.json"),
    snapshot: readJson<ContextSnapshot>(name, "snapshot.json"),
    expected: readJson<FixtureExpected>(name, "expected.json"),
  };
}

export function loadAllFixtures(): Fixture[] {
  return FIXTURE_NAMES.map((name) => loadFixture(name));
}
