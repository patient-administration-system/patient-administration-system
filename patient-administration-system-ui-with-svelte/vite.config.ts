import adapter from "@sveltejs/adapter-node";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";
import { sveltekit } from "@sveltejs/kit/vite";
import { svelteTesting } from "@testing-library/svelte/vite";
import { defineConfig } from "vitest/config";

export default defineConfig({
    // `svelteTesting()` selects Svelte's browser/client resolve condition so
    // components mount client-side under jsdom (rather than SSR), and wires
    // testing-library auto-cleanup. It only affects the vitest run.
    plugins: [
        sveltekit({
            preprocess: vitePreprocess(),
            adapter: adapter(),
            csp: {
                // Nonces for dynamically rendered pages, hashes for prerendered
                // ones; SvelteKit adds them to script-src and style-src.
                mode: "auto",
                directives: {
                    "default-src": ["self"],
                    "script-src": ["self"],
                    "style-src": ["self"],
                    // Style ATTRIBUTES: only the one value SvelteKit's own screen-reader
                    // announcer (`#svelte-announcer`) sets, allowed by its hash
                    // (`unsafe-hashes` is what lets a hash cover an attribute). Found on
                    // 2026-10-18 by removing the old blanket 'unsafe-inline': every
                    // route reported exactly this one violation and nothing else (no Lily
                    // widget or page sets a style attribute). If SvelteKit changes that
                    // string, tests/e2e/security-headers.spec.ts fails on every route;
                    // recompute with: printf '%s' '<style value>' | openssl dgst -sha256 -binary | base64
                    "style-src-attr": ["unsafe-hashes", "sha256-S8qMpvofolR8Mpjy4kQvEm7m1q8clzU4dfDH0AmvZjo="],
                    "img-src": ["self", "data:"],
                    "font-src": ["self"],
                    "connect-src": ["self"],
                    "object-src": ["none"],
                    "base-uri": ["self"],
                    "form-action": ["self"],
                    "frame-ancestors": ["none"]
                }
            }
        }),
        svelteTesting()
    ],
    server: { port: 5173, strictPort: false },
    test: {
        include: ["tests/unit/**/*.{test,spec}.{ts,js}"],
        environment: "jsdom",
        globals: true,
    },
});
