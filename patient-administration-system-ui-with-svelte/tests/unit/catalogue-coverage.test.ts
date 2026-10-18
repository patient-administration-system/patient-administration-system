// NFR-5, second half: no visible English outside the translation function. Every Svelte template
// is parsed; a text node, a string literal in `{…}` or a text-bearing attribute that contains
// English words fails, unless it is on the short, justified allow-lists below.
import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { parse } from "svelte/compiler";

const SRC = join(process.cwd(), "src") + "/";
const WORDS = /[A-Za-z]{2,}/;

// Attributes whose value a person reads or hears.
const READ_ATTRS = new Set(["aria-label", "aria-description", "alt", "title", "placeholder", "label", "caption"]);

// Literal text that is code or a name, not prose: Tera variable examples shown in <code>.
const TEMPLATE_ALLOW = new Set([
  "{{ patient.name.given }}",
  "{{ appointment.start }}",
  "{{ appointment.reason }}",
  "{{ portal_url }}",
]);

function svelteFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    return statSync(p).isDirectory() ? svelteFiles(p) : p.endsWith(".svelte") ? [p] : [];
  });
}

type Node = { type: string; [k: string]: unknown };

function violations(source: string): string[] {
  const out: string[] = [];
  const ast = parse(source, { modern: true }) as unknown as { fragment: Node };
  const visit = (n: unknown): void => {
    if (Array.isArray(n)) return n.forEach(visit);
    if (!n || typeof n !== "object") return;
    const node = n as Node;
    if (node.type === "Text") {
      const data = String(node.data ?? "").trim();
      if (WORDS.test(data)) out.push(`text "${data}"`);
    } else if (node.type === "ExpressionTag") {
      const e = node.expression as { type?: string; value?: unknown } | undefined;
      if (e?.type === "Literal" && typeof e.value === "string" && WORDS.test(e.value) && !TEMPLATE_ALLOW.has(e.value)) {
        out.push(`literal "${e.value}"`);
      }
    } else if (node.type === "Attribute") {
      // An attribute's value is code or a name unless the attribute is one a person reads or hears
      // (`class="error"` and `method="POST"` are not prose). So check those, and do not descend.
      if (READ_ATTRS.has(String(node.name))) {
        const v = node.value;
        const parts = Array.isArray(v) ? v : [];
        for (const p of parts as Node[]) {
          if (p.type === "Text" && WORDS.test(String(p.data))) out.push(`${node.name}="${String(p.data)}"`);
        }
      }
      return;
    }
    for (const [k, v] of Object.entries(node)) if (k !== "expression" || node.type !== "ExpressionTag") visit(v);
  };
  visit(ast.fragment);
  return out;
}

describe("no visible English outside t() (NFR-5)", () => {
  const files = svelteFiles(SRC);
  it("finds the templates", () => expect(files.length).toBeGreaterThan(8));
  for (const file of files) {
    it(file.slice(SRC.length), () => {
      expect(violations(readFileSync(file, "utf8"))).toEqual([]);
    });
  }
  it("the scan itself catches English (self-test)", () => {
    expect(violations("<h1>Hello there</h1>")).toEqual(['text "Hello there"']);
    expect(violations('<img alt="A cat" />')).toEqual(['alt="A cat"']);
    expect(violations('<p>{"Some words"}</p>')).toEqual(['literal "Some words"']);
    expect(violations("<p>{t('x')}</p>")).toEqual([]);
  });
});
