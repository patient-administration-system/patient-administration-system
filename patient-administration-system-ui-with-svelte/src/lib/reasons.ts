// The reasons a cancelling patient can pick on the reply-link page (CAS-2). Each is a coded barrier in the
// PAS (`barrier_code`), and each has a label (`link.why.<code>`) in every catalogue.
import type { components } from "./server/api-types";

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

// Each reason must be a barrier code the PAS accepts: a code renamed or dropped there fails `pnpm run check` here.
true satisfies Reason extends components["schemas"]["BarrierCode"] ? true : false;

export const isReason = (v: unknown): v is Reason =>
  typeof v === "string" && (REASONS as readonly string[]).includes(v);
