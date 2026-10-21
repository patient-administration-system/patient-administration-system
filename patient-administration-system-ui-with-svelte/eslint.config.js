// Lint rules (QUAL-21), run by `pnpm run lint` in CI after the Prettier check. Formatting is Prettier's job;
// these catch mistakes: the recommended JavaScript, TypeScript and Svelte rules, and, with type information
// for the server code, a promise nobody awaits (`no-floating-promises`): an unawaited PAS call is a request
// that can fail without anyone noticing.
import js from "@eslint/js";
import svelte from "eslint-plugin-svelte";
import globals from "globals";
import ts from "typescript-eslint";

export default ts.config(
  {
    ignores: ["build/", ".svelte-kit/", "node_modules/", "coverage/", "test-results/", "playwright-report/", "static/"],
  },
  js.configs.recommended,
  ...ts.configs.recommended,
  ...svelte.configs.recommended,
  {
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
  },
  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: { parserOptions: { parser: ts.parser } },
  },
  {
    rules: {
      // A leading underscore marks a parameter a signature needs but the body does not.
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
      // Links here are built with `localizePath` (every URL carries its locale), not SvelteKit's `resolve()`;
      // the locale tests and the axe run over every route check them instead.
      "svelte/no-navigation-without-resolve": "off",
    },
  },
  {
    // SvelteKit's own declarations: the empty interfaces are where an app adds its types.
    files: ["src/app.d.ts"],
    rules: { "@typescript-eslint/no-empty-object-type": "off" },
  },
  {
    files: ["src/**/*.ts"],
    ignores: ["src/**/*.svelte.ts"],
    languageOptions: { parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname } },
    rules: { "@typescript-eslint/no-floating-promises": "error" },
  },
);
