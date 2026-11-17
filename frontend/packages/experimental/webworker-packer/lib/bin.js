#!/usr/bin/env node
import { a as packRepositoryImage, c as packPreviewFixture, n as describePack, o as previewFixtures } from "./repository-dcVPIq11.js";
import { PREVIEW_FIXTURE_MANIFEST_FILE, PREVIEW_FIXTURE_MANIFEST_VERSION } from "@deepseek-ai/dsh-experimental-webworker-runtime";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
// lib/types/bin.js
/**
* Pack a Preview deployment from this repository: materialize and lower the
* base image for one shipped profile, write its lazy chunk archives beside it,
* then write each named fixture overlay and their manifest.
*
* Usage: dsh-pack-vfs-image --out <file> [--profile web] [--root /dsh]
*        node --import tsx/esm src/bin.ts --out ../../apps/web/dist/preview/vfs-image.tar.gz
* @module @deepseek-ai/dsh-experimental-webworker-packer/src/bin
*/
/**
* Read one `--flag value` pair.
* @param name - Flag name without dashes.
* @param fallback - Value when the flag is absent.
* @returns The value.
* @throws When the flag is present with no value, because silently packing the
* default profile is worse than stopping.
*/
function flag(name, fallback) {
	const index = process.argv.indexOf(`--${name}`);
	if (index === -1) {
		if (fallback !== void 0) return fallback;
		throw new Error(`dsh-pack-vfs-image: --${name} is required`);
	}
	const value = process.argv[index + 1];
	if (value === void 0 || value.startsWith("--")) throw new Error(`dsh-pack-vfs-image: --${name} needs a value`);
	return value;
}
const repoRoot = fileURLToPath(new URL("../../../../", import.meta.url));
const profile = flag("profile", "web");
const out = flag("out");
const outputFile = isAbsolute(out) ? out : resolve(process.cwd(), out);
const result = packRepositoryImage(repoRoot, profile, flag("root", "/dsh"));
if (result.missing.length > 0) throw new Error(`vfs image: ${String(result.missing.length)} dependencies did not resolve; the image would be incomplete`);
mkdirSync(dirname(outputFile), { recursive: true });
writeFileSync(outputFile, result.image);
for (const archive of result.lazyArchives) {
	const path = join(dirname(outputFile), archive.name);
	mkdirSync(dirname(path), { recursive: true });
	writeFileSync(path, archive.image);
}
const fixtureDefinitions = previewFixtures(repoRoot);
const fixtureDirectory = join(dirname(outputFile), "fixtures");
mkdirSync(fixtureDirectory, { recursive: true });
const fixtureLines = [];
const fixtures = fixtureDefinitions.map((fixture) => {
	const packed = packPreviewFixture(fixture.trees);
	const file = `fixtures/${fixture.id}.tar.gz`;
	writeFileSync(join(dirname(outputFile), file), packed.image);
	fixtureLines.push(`  fixture overlay     ${fixture.id} (${String(packed.image.byteLength)} B compressed)`);
	return {
		id: fixture.id,
		label: fixture.label,
		description: fixture.description,
		overlays: [file]
	};
});
const manifest = {
	version: PREVIEW_FIXTURE_MANIFEST_VERSION,
	defaultFixture: fixtures[0]?.id ?? null,
	fixtures
};
writeFileSync(join(dirname(outputFile), PREVIEW_FIXTURE_MANIFEST_FILE), `${JSON.stringify(manifest, null, 2)}\n`);
process.stdout.write([
	...describePack(result, repoRoot, outputFile),
	...fixtureLines,
	""
].join("\n"));
export {};
