// DTAC-12: the parts of a manual accessibility pass that a machine can do, on the pages patients reach from a
// message: the keyboard alone, 320 CSS pixels wide (what 400% zoom of a 1280-pixel window gives, WCAG 1.4.10),
// and WCAG 1.4.12 text spacing. A screen-reader pass and testing with disabled patients still need people.
import { randomBytes } from "node:crypto";
import { expect, test, type Page } from "@playwright/test";
import { MOCK_URL } from "./support/ports";

const token = (prefix = "") => prefix + randomBytes(32).toString("hex").slice(prefix.length);
const acts = async (request: import("@playwright/test").APIRequestContext, t: string) =>
  ((await (await request.get(`${MOCK_URL}/__acts`)).json()) as { token: string; action: string }[])
    .filter((a) => a.token === t)
    .map((a) => a.action);

/** The element has a visible focus indicator: an outline or a shadow, not none. */
async function focusIsVisible(page: Page): Promise<boolean> {
  return page.evaluate(() => {
    const el = document.activeElement as HTMLElement | null;
    if (!el || el === document.body) return false;
    const s = getComputedStyle(el);
    const outline = s.outlineStyle !== "none" && parseFloat(s.outlineWidth) > 0;
    return outline || s.boxShadow !== "none";
  });
}

async function tabTo(page: Page, name: RegExp, max = 12) {
  for (let i = 0; i < max; i++) {
    await page.keyboard.press("Tab");
    const text = await page.evaluate(() => (document.activeElement as HTMLElement | null)?.innerText ?? "");
    if (name.test(text)) return;
  }
  throw new Error(`never reached ${name} by Tab`);
}

test("the skip link is the first stop, shows when focused, and moves to the main content", async ({ page }) => {
  await page.goto(`/en-001/r/${token()}`);
  await page.keyboard.press("Tab");
  const link = page.getByRole("link", { name: "Skip to main content" });
  await expect(link).toBeFocused();
  expect(await focusIsVisible(page)).toBe(true);
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#main$/);
});

test("a patient can confirm from a reply link using only the keyboard, with focus always visible", async ({
  page,
  request,
}) => {
  const t = token();
  await page.goto(`/en-001/r/${t}`);
  await tabTo(page, /Confirm that I am coming/);
  expect(await focusIsVisible(page)).toBe(true);
  await page.keyboard.press("Enter");
  await expect(page.getByText("Your appointment is confirmed.")).toBeVisible();
  expect(await acts(request, t)).toEqual(["confirm"]);
});

test("cancelling with a reason works by keyboard alone: Space on a button, arrows on the reasons, no trap", async ({
  page,
  request,
}) => {
  const t = token();
  await page.goto(`/en-001/r/${t}`);
  await tabTo(page, /Cancel my appointment/);
  await page.keyboard.press("Space");
  await expect(page.getByRole("heading", { name: "Are you sure?" })).toBeVisible();
  // Into the reasons: the first Tab lands on the group, the arrow keys move within it.
  await tabTo(page, /^$/); // a radio has no inner text: the first stop after the heading is the group
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  expect(await focusIsVisible(page)).toBe(true);
  // Tab leaves the group to the confirm button (a keyboard user is never trapped in it).
  await tabTo(page, /Yes, cancel my appointment/);
  await page.keyboard.press("Enter");
  await expect(page.getByText(/Your appointment is cancelled/)).toBeVisible();
  const done = await acts(request, t);
  expect(done).toHaveLength(1);
  expect(done[0]).toMatch(/^cancel(:[a-z_]+)?$/);
});

test("an offer can be accepted from the keyboard", async ({ page, request }) => {
  const t = token();
  await page.goto(`/en-001/o/${t}`);
  await tabTo(page, /Yes, I will take it/);
  expect(await focusIsVisible(page)).toBe(true);
  await page.keyboard.press("Enter");
  await expect(page.getByText(/Your appointment is on/)).toBeVisible();
  expect(await acts(request, t)).toEqual(["offer-accept"]);
});

test("at 320 pixels wide nothing scrolls sideways and every action is still there", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 640 });
  for (const path of [`/en-001/r/${token()}`, `/ar-001/r/${token()}`, `/cy-001/o/${token()}`, "/en-001/signin"]) {
    await page.goto(path);
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow, `${path} scrolls sideways by ${overflow}px`).toBeLessThanOrEqual(1);
  }
  await page.goto(`/en-001/r/${token()}`);
  for (const name of ["Confirm that I am coming", "I need a different time", "Cancel my appointment", "I need help"]) {
    const box = await page.getByRole("button", { name }).boundingBox();
    expect(box, name).not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(321);
  }
});

test("with WCAG 1.4.12 text spacing applied, no text is clipped and nothing scrolls sideways", async ({ page }) => {
  for (const path of [`/en-001/r/${token()}`, `/cy-001/r/${token()}`, `/en-001/o/${token()}`]) {
    await page.goto(path);
    // The page's CSP forbids an inline <style>, so apply the spacing as a constructed stylesheet.
    await page.evaluate(() => {
      const sheet = new CSSStyleSheet();
      sheet.replaceSync(`* { line-height: 1.5 !important; letter-spacing: 0.12em !important; word-spacing: 0.16em !important; }
                p { margin-bottom: 2em !important; }`);
      document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
    });
    const problems = await page.evaluate(() => {
      const bad: string[] = [];
      for (const el of Array.from(document.querySelectorAll<HTMLElement>("button, a, p, h1, h2, label, legend"))) {
        const s = getComputedStyle(el);
        const clipped = (s.overflow !== "visible" || s.overflowX !== "visible") && el.scrollWidth > el.clientWidth + 1;
        if (clipped) bad.push(`${el.tagName} "${(el.innerText || "").slice(0, 30)}" is clipped`);
      }
      const sideways = document.documentElement.scrollWidth - document.documentElement.clientWidth;
      if (sideways > 1) bad.push(`the page scrolls sideways by ${sideways}px`);
      return bad;
    });
    expect(problems, path).toEqual([]);
  }
});
