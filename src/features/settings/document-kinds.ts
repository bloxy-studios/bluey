import type { DocumentKind } from "@/lib/types";

/** Human labels for `DocumentKind` (Settings → Context, Modes → Files). */
export const DOCUMENT_KIND_LABELS: Record<DocumentKind, string> = {
  resume: "Résumé",
  cv: "CV",
  bio: "Bio",
  portfolio: "Portfolio",
  skills: "Skills",
  experience: "Experience",
  personal_instructions: "Personal instructions",
  job_description: "Job description",
  company_notes: "Company notes",
  role_description: "Role description",
  notes: "Notes",
  other: "Other",
};

/** Kinds offered in the "Add as" picker, most common first. */
export const DOCUMENT_KIND_OPTIONS: readonly DocumentKind[] = [
  "resume",
  "cv",
  "job_description",
  "company_notes",
  "role_description",
  "personal_instructions",
  "bio",
  "portfolio",
  "skills",
  "experience",
  "notes",
  "other",
];
