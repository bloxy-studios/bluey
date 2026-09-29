/**
 * Answer languages offered in Settings → General. Settings store the code (Rust default `"en"`);
 * builds before UX-042 stored the name ("Spanish"), which still reads as its code.
 */
export const OUTPUT_LANGUAGES = [
  { code: "en", name: "English" },
  { code: "es", name: "Spanish" },
  { code: "fr", name: "French" },
  { code: "de", name: "German" },
  { code: "pt", name: "Portuguese" },
  { code: "ja", name: "Japanese" },
  { code: "ko", name: "Korean" },
  { code: "zh", name: "Chinese" },
] as const;

function find(value: string) {
  const wanted = value.trim().toLowerCase();
  return OUTPUT_LANGUAGES.find((l) => l.code === wanted || l.name.toLowerCase() === wanted);
}

/** A stored output language (`"es"` or a legacy `"Spanish"`) as its code; unknown values pass through. */
export function outputLanguageCode(value: string): string {
  return find(value)?.code ?? value;
}

/** The name a prompt asks for (`"es"` → `"Spanish"`); unknown values pass through. */
export function outputLanguageName(value: string): string {
  return find(value)?.name ?? value;
}
