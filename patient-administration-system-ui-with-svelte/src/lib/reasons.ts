// The reasons a cancelling patient can pick on the reply-link page (CAS-2). Each is a coded barrier in the
// PAS (`barrier_code`), and each has a label (`link.why.<code>`) in every catalogue.
export const REASONS = [
  "cannot_travel",
  "cost",
  "caring",
  "work",
  "unwell",
  "anxiety",
  "access_need",
  "other",
] as const;

export type Reason = (typeof REASONS)[number];

export const isReason = (v: unknown): v is Reason =>
  typeof v === "string" && (REASONS as readonly string[]).includes(v);
