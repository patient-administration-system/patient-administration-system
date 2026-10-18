// Delivery channels a message template may use, in the PAS's own wire names.
export const CHANNELS = [
  { id: "whatsapp", label: "WhatsApp" },
  { id: "apple_messages", label: "Apple Messages" },
  { id: "sms", label: "SMS" },
  { id: "email", label: "Email" },
  { id: "print", label: "Print" },
] as const;

const KNOWN = new Set<string>(CHANNELS.map((c) => c.id));

/** Keep only known channels, in the order given, without duplicates. */
export function cleanChannels(values: string[]): string[] {
  return [...new Set(values)].filter((v) => KNOWN.has(v));
}
