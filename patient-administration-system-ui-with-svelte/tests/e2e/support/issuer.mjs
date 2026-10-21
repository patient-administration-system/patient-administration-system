// Starts the real token issuer (`pas-token-issuer`, the Rust binary) for the browser suite: generates
// a throwaway key, then serves its public keys and keeps a service-token file fresh. The UI reads the
// token file (PAS_API_TOKEN_FILE) and the mock API verifies tokens against the served keys, so the
// suite exercises the same path as a deployment: issuer -> token file -> UI -> API.
import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../../../..");
const dir = process.env.ISSUER_DIR ?? path.resolve(here, "../../../test-results/issuer");
const port = process.env.ISSUER_PORT ?? "4195";
const bin = process.env.PAS_TOKEN_ISSUER ?? null;

const run = (args) =>
  bin
    ? [bin, args]
    : [
        "cargo",
        [
          "run",
          "--quiet",
          "--manifest-path",
          path.join(root, "Cargo.toml"),
          "--bin",
          "pas-token-issuer",
          "--",
          ...args,
        ],
      ];

rmSync(dir, { recursive: true, force: true });
mkdirSync(dir, { recursive: true });

const [cmd0, args0] = run(["keygen", "--dir", dir, "--kid", "e2e-1"]);
const gen = spawnSync(cmd0, args0, { stdio: "inherit" });
if (gen.status !== 0) process.exit(gen.status ?? 1);

const [cmd, args] = run([
  "serve",
  "--key",
  path.join(dir, "private.key"),
  "--kid",
  "e2e-1",
  "--sub",
  "pas-ui",
  "--ttl",
  "60",
  "--token-file",
  path.join(dir, "service.token"),
  "--bind",
  `127.0.0.1:${port}`,
]);
const child = spawn(cmd, args, { stdio: "inherit" });
for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => child.kill(sig));
child.on("exit", (code) => process.exit(code ?? 0));
