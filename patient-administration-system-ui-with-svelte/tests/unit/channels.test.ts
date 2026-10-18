import { describe, expect, it } from "vitest";
import { cleanChannels } from "#lib/channels.js";

describe("cleanChannels", () => {
  it("drops unknown channels and duplicates but keeps order", () => {
    expect(cleanChannels(["sms", "fax", "whatsapp", "sms"])).toEqual(["sms", "whatsapp"]);
  });
  it("returns an empty list for nothing", () => expect(cleanChannels([])).toEqual([]));
});
