// Explicit transcription choices. Historical language names remain available
// in the backend for summaries of existing notes.
export type Language = {
  value: string;
  label: string;
  native?: string;
};

export const LANGUAGES: Language[] = [
  { value: "en", label: "English" },
  { value: "nl", label: "Dutch", native: "Nederlands" },
];

export function languageOptionLabel(lang: Language): string {
  if (!lang.native || lang.native === lang.label) return lang.label;
  return `${lang.label} (${lang.native})`;
}
