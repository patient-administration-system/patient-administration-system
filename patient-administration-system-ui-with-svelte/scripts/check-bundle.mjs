// Per-chunk bundle budget (QUAL-24). Run after `pnpm run build`: fails if any JavaScript chunk the browser
// downloads is larger, gzipped, than BUNDLE_CHUNK_KB (default 50), and prints the largest chunks either way.
// The largest chunk was 37 KB gzipped on 2026-10-21; raise the budget only with a reason in the commit.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { gzipSync } from 'node:zlib';

const root = 'build/client';
const budget = Number(process.env.BUNDLE_CHUNK_KB ?? 50) * 1024;

function files(dir) {
	return readdirSync(dir).flatMap((name) => {
		const path = join(dir, name);
		return statSync(path).isDirectory() ? files(path) : [path];
	});
}

let all;
try {
	all = files(root);
} catch {
	console.error(`no ${root}: run \`pnpm run build\` first`);
	process.exit(2);
}
const chunks = all
	.filter((f) => f.endsWith('.js'))
	.map((f) => ({ file: relative(root, f), gz: gzipSync(readFileSync(f), { level: 9 }).length }))
	.sort((a, b) => b.gz - a.gz);
const kb = (n) => `${(n / 1024).toFixed(1)} KB`;
for (const c of chunks.slice(0, 5)) console.log(`${kb(c.gz).padStart(9)}  ${c.file}`);
const over = chunks.filter((c) => c.gz > budget);
if (over.length) {
	console.error(`\n${over.length} chunk(s) over the ${kb(budget)} gzipped budget:`);
	for (const c of over) console.error(`  ${kb(c.gz)}  ${c.file}`);
	process.exit(1);
}
console.log(`\n${chunks.length} chunks, all within ${kb(budget)} gzipped`);
