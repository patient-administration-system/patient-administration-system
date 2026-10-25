// Delivery channels a message template may use, in the PAS's own wire names. The visible name of
// each is the catalogue entry `channel.<id>`.
export const CHANNELS = [
  { id: "whatsapp" },
  { id: "apple_messages" },
  { id: "sms" },
  { id: "email" },
  { id: "portal" },
  { id: "print" },
] as const;

export type Channel = (typeof CHANNELS)[number]["id"];

const KNOWN = new Set<string>(CHANNELS.map((c) => c.id));

/** Keep only known channels, in the order given, without duplicates. */
export function cleanChannels(values: string[]): Channel[] {
  return [...new Set(values)].filter((v): v is Channel => KNOWN.has(v));
}
