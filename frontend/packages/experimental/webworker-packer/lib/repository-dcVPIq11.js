import { DEFAULT_ROOT, DEFAULT_ROOT as DEFAULT_ROOT$1, IMAGE_EMPTY_DIRECTORIES, IMAGE_HOME_DIRECTORY, IMAGE_LAZY_DIRECTORY, IMAGE_MANIFEST_PATH, IMAGE_OVERLAY_DIRECTORIES, LOWERING_VERSION, MODULE_PROXIES, MODULE_PROXY_PREFIXES, MemoryVfs, PLATFORM_ENTRY_CONFIG, REPLACED_EXTERNAL_PACKAGES, WorkerModuleLoader, lowerModuleSource, packTar } from "@deepseek-ai/dsh-experimental-webworker-runtime";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, posix, relative } from "node:path";
import { gzipSync } from "node:zlib";
import picomatch from "picomatch";
import yaml from "js-yaml";
import { entryListSchema } from "@deepseek-ai/cordis-plugin-include";
import { PROFILE_PATCH_FILENAME, bundlePatchPaths, composeEntries, loadOverlayPatches, resolveBundleDir } from "@deepseek-ai/dsh-app-boot";
import { OPTIONAL_BUNDLES, PROFILE_TEMPLATES } from "@deepseek-ai/dsh-official-bundles";
import { tmpdir } from "node:os";
import { isSessionFormatJsonObject, parseSessionFormatLogFilename, sessionFormatLogFilename } from "@deepseek-ai/dsh-session-format";
import { createSessionFormatCatalogWithChildren, historicalSessionFormatCatalog, sessionFormatCatalog } from "@deepseek-ai/dsh-session-format-catalog";
import { historicalChildCatalogSource } from "@deepseek-ai/dsh-session-format-v3-to-v4";
// lib/types/transform-image.js
/**
* The wrapper contract packed bodies are emitted against, and the image-entry
* types the pack pass consumes.
*
* One transform serves both sides — the pack pass lowers with the runtime's
* own `lowerModuleSource`, never a reimplementation — and the image records
* the contract version it was lowered against. Bodies emitted against a
* different wrapper contract are refused at mount time rather than
* half-working at run time.
* @module @deepseek-ai/dsh-experimental-webworker-packer/src/transform-image
*/
/** Wrapper contract the packed bodies are emitted against. */
const WRAPPER_CONTRACT = LOWERING_VERSION;
// lib/types/rules.js
/**
* Pack rule tables: the one place the image's include/exclude decisions live.
* Patterns are picomatch globs. Exclude patterns match tree-root-relative
* paths (so `src/**` drops only a root-level source tree), page-asset
* patterns match image paths. Traversal mechanics — nested `node_modules`
* flattening and dot-directory pruning — stay in the collector; these tables
* hold the judgement calls.
*/
/**
* Paths dropped from every collected tree. Test trees, sourcemaps,
* declarations, and archives never resolve at runtime while dominating the
* byte count. Third-party `src/` directories remain eligible because package
* entrypoints may resolve to JavaScript there.
*/
const EXCLUDE = [
	"tests/**",
	"test/**",
	"__tests__/**",
	"coverage/**",
	"**/*.map",
	"**/*.tsbuildinfo",
	"**/*.tgz",
	"**/*.tar",
	"**/*.tar.gz",
	"**/*.d.ts",
	"**/*.d.mts",
	"**/*.d.cts"
];
/**
* Additional paths dropped from workspace and vendored packages only. Their
* runtime plane is built `lib/`; a workspace `dist/` is a page-asset tree the
* static deployment serves itself. External packages may place runtime code
* under either directory.
*/
const EXCLUDE_WORKSPACE = ["src/**", "dist/**"];
/**
* Image paths that belong to the PAGE, not to the worker's loader.
*
* A package's `lib/client.js` entry and `lib/client.*.js` sibling chunks are
* browser bundles behind the `./client` export: the page's own module system evaluates them with its own wrapper,
* which has no ambient-store parameter. Transforming those bodies would
* inject calls the page cannot resolve, so they ship untransformed — their
* only change is the trailing debugger-name line every JavaScript entry
* gains — and the manifest's all-or-nothing claim stays true, because the
* worker loader never evaluates them (the tunnel serves them as bytes).
*/
const PAGE_ASSETS = [
	"node_modules/*/lib/client.js",
	"node_modules/*/lib/client.*.js",
	"node_modules/@*/*/lib/client.js",
	"node_modules/@*/*/lib/client.*.js"
];
/**
* Image specifiers the worker assembly requires directly, beyond the
* roster: they are requested by worker-bundle code, so no image file
* references them and the reachability sweep must seed them as roots. Keep in
* step with the literal `require`/`resolve` calls in the runtime's
* `worker-host.ts`.
*/
const IMAGE_ENTRY_SEEDS = [
	"@deepseek-ai/dsh-app-boot",
	"@deepseek-ai/dsh-cmdline",
	"@deepseek-ai/dsh-launch-environment",
	"@deepseek-ai/cordis"
];
// lib/types/pack.js
/**
* VFS image packer: turns a package roster plus a package index into the single
* gzip-compressed tar the browser runtime inflates and mounts as its filesystem,
* and the lazy archives beside it.
*
* Nothing is compiled here. The image carries the repository's real build products,
* so a preview deployment debugs exactly what the served deployment ships. What the
* pass does add is the pack-time module transform and the manifest that records the
* wrapper contract it was transformed against. No composed configuration is packed:
* the worker host composes the profile's bundle layers from the packages the roster
* materializes, as the served launcher does.
*
* This module holds no repository knowledge: paths, globs, and the roster come in as
* parameters, so the same library packs a different tree by being called
* differently. Locating those inputs is the CLI's job.
* @module @deepseek-ai/dsh-experimental-webworker-packer/src/pack
*/
/** Image path of the manifest; the layout contract's name, re-exported for callers. */
const MANIFEST_PATH = IMAGE_MANIFEST_PATH;
/** Image path of the home patch: `$DSH_HOME/cordis.patch.yml` under the Worker home. */
const HOME_PATCH_PATH = `${IMAGE_HOME_DIRECTORY}/${PROFILE_PATCH_FILENAME}`;
/** Exclude matcher over tree-root-relative paths ({@link EXCLUDE}). */
const excluded = picomatch([...EXCLUDE], { dot: true });
/** Workspace exclude matcher: {@link EXCLUDE} plus {@link EXCLUDE_WORKSPACE}. */
const workspaceExcluded = picomatch([...EXCLUDE, ...EXCLUDE_WORKSPACE], { dot: true });
/** Page-asset matcher over image paths ({@link PAGE_ASSETS}). */
const pageAsset = picomatch([...PAGE_ASSETS], { dot: true });
const readJson = (file) => JSON.parse(readFileSync(file, "utf8"));
/**
* Package name of a module specifier.
* @param specifier - Module specifier, possibly with a subpath.
* @returns The package name (`@scope/pkg/sub` → `@scope/pkg`).
*/
function packageNameOf(specifier) {
	const [first = specifier, second = ""] = specifier.split("/");
	return first.startsWith("@") ? `${first}/${second}` : first;
}
/**
* Collect package names from entry rows and patch lists, recursively through
* `insert` lists and nested `config` row lists (groups). Builtin rows
* (`cordis:group`), preset metadata documents, and file-relative entries carry
* names that are not package specifiers; only scoped names and bare names
* with a subpath count.
* @param rows - Parsed YAML value; anything but a row array is ignored.
* @param names - Package names collected so far.
*/
function collectModuleNames(rows, names) {
	if (!Array.isArray(rows)) return;
	for (const row of rows) {
		if (typeof row !== "object" || row === null) continue;
		const { name, config, insert } = row;
		if (typeof name === "string" && !name.startsWith(".") && (name.startsWith("@") || name.includes("/"))) names.add(packageNameOf(name));
		collectModuleNames(config, names);
		collectModuleNames(insert, names);
	}
}
/**
* Package names the compositions under one config tree name.
* @param root - Directory to walk.
* @returns Package names, deduplicated.
*/
function treeRosterOf(root) {
	const names = /* @__PURE__ */ new Set();
	const walk = (directory) => {
		for (const entry of readdirSync(directory, { withFileTypes: true })) {
			const absolute = join(directory, entry.name);
			if (entry.isDirectory()) {
				walk(absolute);
				continue;
			}
			if (!entry.name.endsWith(".yml") && !entry.name.endsWith(".yaml")) continue;
			collectModuleNames(yaml.load(readFileSync(absolute, "utf8"), { schema: entryListSchema }), names);
		}
	};
	walk(root);
	return [...names];
}
/**
* Resolve one dependency the way Node does: walk up from the importer.
* @param fromDirectory - Directory to start at.
* @param name - Package name.
* @returns The real path of the package directory, or undefined.
*/
function resolveDependency(fromDirectory, name) {
	let directory = fromDirectory;
	for (;;) {
		const candidate = join(directory, "node_modules", name);
		if (existsSync(join(candidate, "package.json"))) return realpathSync(candidate);
		const parent = dirname(directory);
		if (parent === directory) return void 0;
		directory = parent;
	}
}
/**
* Collect files under one directory. Traversal mechanics live here — nested
* package/config collection flattens nested `node_modules` and prunes dot
* directories, while seed collection preserves every directory. Every file
* judgement comes in through `keep` (the {@link EXCLUDE} tables and the npm
* publish view, or an unconditional seed predicate).
* @param root - Source directory.
* @param into - Image entries to add to.
* @param prefix - Image path prefix.
* @param keep - Filter over root-relative paths.
* @param preserveDirectories - Whether dot directories and nested `node_modules`
*   are ordinary fixture content rather than package-manager residue.
*/
function collectTree(root, into, prefix, keep, preserveDirectories = false) {
	const walk = (directory) => {
		for (const entry of readdirSync(directory, { withFileTypes: true })) {
			if (entry.isDirectory()) {
				if (!preserveDirectories && (entry.name === "node_modules" || entry.name.startsWith("."))) continue;
				walk(join(directory, entry.name));
				continue;
			}
			if (!entry.isFile()) continue;
			const absolute = join(directory, entry.name);
			const relativePath = relative(root, absolute).replaceAll("\\", "/");
			if (!keep(relativePath)) continue;
			into[`${prefix}/${relativePath}`] = readFileSync(absolute);
		}
	};
	walk(root);
}
/**
* Predicate for npm's `files` allowlist, with standard glob semantics
* (picomatch). A pattern admits the path itself and everything under it, so a
* bare directory name publishes its whole tree; `!` patterns subtract from the
* admitted set; package.json is always published.
* @param patterns - The package.json `files` array.
* @returns Predicate over package-root-relative paths.
*/
function publishedFilter(patterns) {
	const strings = patterns.filter((pattern) => typeof pattern === "string");
	const normalize = (pattern) => pattern.replace(/^\.\//, "").replace(/\/+$/, "");
	const widen = (pattern) => [pattern, `${pattern}/**`];
	const positive = strings.filter((pattern) => !pattern.startsWith("!")).map(normalize).flatMap(widen);
	const negative = strings.filter((pattern) => pattern.startsWith("!")).map((pattern) => normalize(pattern.slice(1))).flatMap(widen);
	const admits = picomatch(positive, { dot: true });
	const denies = negative.length > 0 ? picomatch(negative, { dot: true }) : () => false;
	return (path) => path === "package.json" || admits(path) && !denies(path);
}
/**
* Keep only the JavaScript the worker can reach, transforming it on the way.
*
* Roots are the export faces of every materialized workspace and vendored
* package — the harness addresses them by constructed name at runtime (Loader
* rows, typert faces, delegating providers such as `-auto` pickers), so the
* sweep prunes files only inside third-party packages — plus the worker
* assembly's own image entries. Resolution runs the runtime loader's own
* algorithm over the candidate set, so pack-time reachability and boot-time
* resolution cannot drift, and a request that resolves nowhere — an undeclared
* or missing dependency — fails the pack rather than the boot.
*
* Two entry classes stay out of the walk by rule: page assets
* ({@link PAGE_ASSETS}) are evaluated by the page's module system, and
* non-JavaScript entries always stay because data reads go through fs paths
* this pass cannot see.
* @param files - Candidate entries after the publish-view filter.
* @param options - Pack options carrying the sweep roots.
* @param rootPackages - Roster package names from the workspace.
* @param root - Virtual root the candidates mount under.
* @returns The final entries plus the sweep's counts.
*/
/** Trailing `sourceMappingURL` comment; the image carries no `.map` files. */
const DANGLING_SOURCE_MAP = /\n\/\/# sourceMappingURL=\S+\s*$/;
/**
* Name one JavaScript entry for the debugger: append the `sourceURL` magic
* comment V8 stacks and DevTools read, so the entry shows under its
* repository path instead of as an anonymous VM script (worker `new Function`
* bodies) or blob entry (page bundles). A trailing `sourceMappingURL` comment
* is stripped first — its `.map` never ships, and once the script has a name
* the debugger would resolve the reference against it and report a load
* failure per script. Only the final line is touched, so every other line
* keeps its number; evaluation cost stays at pack time, where the names are
* already deterministic.
* @param bytes - Entry body as the image would otherwise hold it.
* @param name - Debugger name for the entry.
* @param decoder - Shared UTF-8 decoder.
* @param encoder - Shared UTF-8 encoder.
* @returns The named body.
*/
function nameForDebugger(bytes, name, decoder, encoder) {
	const source = decoder.decode(bytes).replace(DANGLING_SOURCE_MAP, "\n");
	return encoder.encode(`${source}\n//# sourceURL=${name}`);
}
/**
* Debugger names for image entries: a workspace or vendored package file is
* named by its repository path (`packages/<group>/<pkg>/lib/index.js`), the
* shape a reader navigates; an external package file keeps its image key —
* it has no repository path, and its pnpm store path would name a hash.
* @param workspaces - Package name → absolute repository directory.
* @param resolveFrom - Repository root the names are relative to.
* @returns Mapper from an image key to the entry's debugger name.
*/
function debuggerNamer(workspaces, resolveFrom) {
	const repoDirs = new Map([...workspaces].map(([name, directory]) => [name, relative(resolveFrom, directory).replaceAll("\\", "/")]));
	return (key) => {
		if (!key.startsWith("node_modules/")) return key;
		const rest = key.slice(13);
		const segments = rest.split("/");
		const packageName = segments[0]?.startsWith("@") === true ? segments.slice(0, 2).join("/") : segments[0] ?? "";
		const directory = repoDirs.get(packageName);
		return directory === void 0 ? key : `${directory}${rest.slice(packageName.length)}`;
	};
}
/** Declaration-only condition trees add no runtime roots; explicit imports still resolve normally. */
function isDeclarationOnlyExport(value) {
	if (typeof value !== "object" || value === null) return false;
	if (Array.isArray(value)) return value.length > 0 && value.every(isDeclarationOnlyExport);
	const conditions = Object.entries(value);
	return conditions.length > 0 && conditions.every(([condition, target]) => condition === "types" || condition.startsWith("types@") || isDeclarationOnlyExport(target));
}
function sweepImage(files, options, rootPackages, root) {
	const decoder = new TextDecoder();
	const encoder = new TextEncoder();
	const vfs = new MemoryVfs();
	for (const [name, bytes] of Object.entries(files)) if (name.endsWith("/")) vfs.seedDirectory(`${root}/${name}`);
	else vfs.seed(`${root}/${name}`, bytes);
	const stub = () => ({});
	const loader = new WorkerModuleLoader({
		vfs,
		root,
		staticModules: Object.fromEntries(Object.keys(MODULE_PROXIES).map((name) => [name, stub])),
		staticModulePrefixes: Object.fromEntries(Object.keys(MODULE_PROXY_PREFIXES).map((name) => [name, stub]))
	});
	const queue = (options.entries ?? IMAGE_ENTRY_SEEDS).map((specifier) => ({
		specifier,
		from: root,
		importer: "worker assembly entry"
	}));
	for (const name of rootPackages) {
		const manifestBytes = files[`node_modules/${name}/package.json`];
		if (manifestBytes === void 0) continue;
		let manifest;
		try {
			manifest = JSON.parse(decoder.decode(manifestBytes));
		} catch {
			continue;
		}
		const subpaths = manifest.exports === void 0 ? ["."] : Object.entries(manifest.exports).filter(([key, target]) => key.startsWith(".") && !key.includes("*") && !isDeclarationOnlyExport(target)).map(([key]) => key);
		for (const subpath of subpaths) queue.push({
			specifier: subpath === "." ? name : `${name}/${subpath.slice(2)}`,
			from: root,
			importer: `workspace face ${name}`
		});
	}
	const reached = /* @__PURE__ */ new Map();
	const seen = /* @__PURE__ */ new Set();
	const failures = [];
	const tolerated = /* @__PURE__ */ new Set();
	let visited = 0;
	let rewritten = 0;
	for (let entry = queue.shift(); entry !== void 0; entry = queue.shift()) {
		const { specifier, from, importer } = entry;
		let resolution;
		try {
			resolution = loader.resolve(specifier, from);
		} catch (reason) {
			if (importer.startsWith("node_modules/") && !importer.startsWith("node_modules/@deepseek-ai/") || entry.meta === true) tolerated.add(`${importer}: "${specifier}"`);
			else failures.push(`${importer}: "${specifier}" — ${reason.message}`);
			continue;
		}
		if (resolution.kind === "static") continue;
		const path = resolution.path;
		if (seen.has(path)) continue;
		seen.add(path);
		const key = path.slice(root.length + 1);
		const bytes = files[key];
		if (bytes === void 0) continue;
		if (!/\.[cm]?js$/.test(key) || pageAsset(key)) {
			reached.set(key, bytes);
			continue;
		}
		visited += 1;
		const { code, lowered, moduleRequests, metaResolveRequests } = lowerModuleSource({
			filename: `/${key}`,
			source: decoder.decode(bytes)
		});
		if (lowered) rewritten += 1;
		reached.set(key, lowered ? encoder.encode(code) : bytes);
		const directory = path.slice(0, path.lastIndexOf("/"));
		for (const request of moduleRequests) queue.push({
			specifier: request,
			from: directory,
			importer: key
		});
		for (const request of metaResolveRequests) queue.push({
			specifier: request,
			from: directory,
			importer: key,
			meta: true
		});
	}
	if (failures.length > 0) throw new Error(`vfs image: ${String(failures.length)} unresolvable module request(s); an undeclared or missing dependency fails the pack rather than the boot:
  ` + failures.join("\n  "));
	const swept = {};
	const debuggerName = debuggerNamer(options.workspaces, options.resolveFrom);
	let javascriptEntries = 0;
	let dropped = 0;
	for (const [name, bytes] of Object.entries(files)) {
		const isJs = /\.[cm]?js$/.test(name);
		if (!isJs || pageAsset(name)) {
			swept[name] = isJs ? nameForDebugger(bytes, debuggerName(name), decoder, encoder) : bytes;
			if (isJs) javascriptEntries += 1;
			continue;
		}
		const kept = reached.get(name);
		if (kept === void 0) {
			dropped += 1;
			continue;
		}
		swept[name] = nameForDebugger(kept, debuggerName(name), decoder, encoder);
		javascriptEntries += 1;
	}
	return {
		swept,
		transform: {
			visited,
			rewritten
		},
		javascriptEntries,
		droppedJavascriptEntries: dropped,
		unresolvedExternalRequests: [...tolerated]
	};
}
/**
* Drop executable scripts from the image.
*
* A shebang says "program", not "module": nothing in a browser can spawn one and no
* consumer reads their bytes (the packages that expose a launcher path are replaced
* by stubs that answer with a string). They are also the one place top-level `await`
* appears in the closure, which a CommonJS body cannot express.
* @param files - Image entries, mutated.
* @returns The dropped entry names.
*/
function dropExecutables(files) {
	const decoder = new TextDecoder();
	const dropped = [];
	for (const [name, bytes] of Object.entries(files)) {
		if (!/\.[cm]?js$/.test(name)) continue;
		if (decoder.decode(bytes.subarray(0, 2)) !== "#!") continue;
		dropped.push(name);
		delete files[name];
	}
	return dropped;
}
/**
* Materialize the dependency closure of every roster package into the image.
* @param roster - Package names to start from.
* @param options - Pack options carrying the workspace index and resolution root.
* @returns Image entries, per-package file counts, and unresolved dependencies.
*/
function materialize(roster, options) {
	const files = {};
	const packages = /* @__PURE__ */ new Map();
	const missing = [];
	const replaced = new Set(REPLACED_EXTERNAL_PACKAGES);
	const queue = roster.map((name) => ({
		name,
		from: options.resolveFrom
	}));
	for (let entry = queue.shift(); entry !== void 0; entry = queue.shift()) {
		const { name, from } = entry;
		if (packages.has(name) || replaced.has(name)) continue;
		const directory = options.workspaces.get(name) ?? resolveDependency(from, name);
		if (directory === void 0) {
			missing.push(`${name} (from ${relative(options.resolveFrom, from) || "."})`);
			continue;
		}
		const manifest = readJson(join(directory, "package.json"));
		const prefix = `node_modules/${name}`;
		const before = Object.keys(files).length;
		if (options.workspaces.has(name)) {
			const published = Array.isArray(manifest.files) ? publishedFilter(manifest.files) : void 0;
			collectTree(directory, files, prefix, (relativePath) => !workspaceExcluded(relativePath) && (published === void 0 || published(relativePath)));
		} else collectTree(directory, files, prefix, (relativePath) => !excluded(relativePath));
		packages.set(name, Object.keys(files).length - before);
		for (const field of ["dependencies", "peerDependencies"]) {
			if (field === "peerDependencies" && !options.workspaces.has(name)) continue;
			const dependencies = manifest[field];
			if (typeof dependencies !== "object" || dependencies === null) continue;
			for (const dependency of Object.keys(dependencies)) queue.push({
				name: dependency,
				from: directory
			});
		}
	}
	return {
		files,
		packages,
		missing
	};
}
/**
* Request form the dynamic Client build emits for a package-local chunk: a
* literal `require.async("./client.<name>.js")`, served by the Host on demand.
* The minifier may print the literal with any quote, including a template without substitutions.
*/
const CHUNK_REQUEST = /\brequire\.async\(\s*(["'`])(\.\/client\.[A-Za-z0-9][A-Za-z0-9._-]*\.js)\1\s*\)/g;
/**
* Image paths every materialized package's `./client` export names, under any
* condition: those load eagerly, so a chunk request never makes them lazy.
* @param files - Image entries.
* @param packages - Materialized package names.
* @returns Eager Client entry paths.
*/
function clientEntriesOf(files, packages) {
	const decoder = new TextDecoder();
	const entries = /* @__PURE__ */ new Set();
	const collect = (name, target) => {
		if (typeof target === "string") entries.add(`node_modules/${name}/${target.replace(/^\.\//, "")}`);
		else if (typeof target === "object" && target !== null) for (const value of Object.values(target)) collect(name, value);
	};
	for (const name of packages) {
		const bytes = files[`node_modules/${name}/package.json`];
		if (bytes === void 0) continue;
		collect(name, JSON.parse(decoder.decode(bytes)).exports?.["./client"]);
	}
	return entries;
}
/**
* Page chunks requested only through `require.async`, in path order.
* @param files - Image entries after the sweep.
* @param eager - Eager Client entry paths, never lazy.
* @returns Image paths to move into lazy archives.
* @throws When a page asset requests a chunk its package does not publish.
*/
function lazyChunksOf(files, eager) {
	const decoder = new TextDecoder();
	const lazy = /* @__PURE__ */ new Set();
	for (const [name, bytes] of Object.entries(files)) {
		if (!pageAsset(name)) continue;
		const directory = name.slice(0, name.lastIndexOf("/"));
		for (const match of decoder.decode(bytes).matchAll(CHUNK_REQUEST)) {
			const request = match[2];
			const target = `${directory}/${request.slice(2)}`;
			if (files[target] === void 0) throw new Error(`vfs image: ${name} requests chunk "${request}", which the package does not publish`);
			if (!eager.has(target)) lazy.add(target);
		}
	}
	return [...lazy].sort();
}
/** Gzip header byte that records the packing platform; RFC 1952 §2.3.1 spells 255 "unknown". */
const GZIP_OS_UNKNOWN = 255;
/** Offset of that byte in the gzip member header. */
const GZIP_OS_OFFSET = 9;
/**
* Compress the archive into one gzip member the same tree always produces
* byte for byte.
*
* Two header fields would otherwise carry build facts: zlib writes no
* modification time and no original file name for a buffer (`gzipSync` is handed
* neither), and it fills the operating-system byte from the platform it was built
* for, which would make the same tree pack differently on Linux and macOS. That
* byte is overwritten with "unknown" — every gzip reader ignores it, and the
* artifact stops depending on where it was packed.
* @param archive - the ustar archive.
* @returns the compressed image bytes.
*/
function compressImage(archive) {
	const compressed = gzipSync(archive, { level: 9 });
	compressed[GZIP_OS_OFFSET] = GZIP_OS_UNKNOWN;
	return compressed;
}
/**
* Pack one VFS image.
*
* The manifest's claim is all-or-nothing: it names the one contract every packed body
* was emitted against. A module the transform cannot express therefore fails the pack
* rather than downgrading the image, because a mostly-transformed image boots into
* errors far from their cause. Page chunks requested through `require.async` leave
* the base image for deterministic archives named by their content digest, which the manifest registers.
* @param options - Roster, installation, package index, and paths.
* @returns The compressed image, its lazy archives, and what went into them.
* @throws When a config tree or workspace directory named in the options is missing,
* because a silently thinner image fails much later and much less clearly; when the
* installation is also a materialized package; or when a page asset requests a chunk
* its package does not publish.
*/
function packVfsImage(options) {
	const root = options.root ?? DEFAULT_ROOT;
	const encoder = new TextEncoder();
	const configTrees = options.configTrees ?? [];
	for (const tree of configTrees) if (!existsSync(tree.directory)) throw new Error(`vfs image: config tree ${tree.mount} is missing at ${tree.directory}`);
	const roster = [.../* @__PURE__ */ new Set([...options.roster, ...configTrees.filter((tree) => tree.scanRoster === true).flatMap((tree) => treeRosterOf(tree.directory))])];
	const { files, packages, missing } = materialize(roster, options);
	const installation = options.installation;
	if (installation !== void 0) {
		if (packages.has(installation.name)) throw new Error(`vfs image: installation ${installation.name} is also materialized; its manifest must enter the image alone`);
		files[`node_modules/${installation.name}/package.json`] = readFileSync(installation.manifest);
	}
	if (options.homePatch !== void 0 && options.homePatch.length > 0) files[HOME_PATCH_PATH] = encoder.encode(yaml.dump(options.homePatch, {
		schema: entryListSchema,
		noRefs: true
	}));
	for (const tree of configTrees) collectTree(tree.directory, files, tree.mount, (relativePath) => !excluded(relativePath));
	const executables = dropExecutables(files);
	const { swept, transform, javascriptEntries, droppedJavascriptEntries, unresolvedExternalRequests } = sweepImage(files, options, [...packages.keys()].filter((name) => options.workspaces.has(name)), root);
	const lazyArchives = [];
	const lazy = [];
	for (const path of lazyChunksOf(swept, clientEntriesOf(swept, packages.keys()))) {
		const bytes = swept[path];
		delete swept[path];
		const image = compressImage(packTar({ [path]: bytes }));
		const name = `${IMAGE_LAZY_DIRECTORY}/${createHash("sha256").update(image).digest("hex")}.tar.gz`;
		lazyArchives.push({
			name,
			image,
			files: [path]
		});
		lazy.push({
			archive: name,
			files: [{
				path,
				size: bytes.byteLength
			}]
		});
	}
	const manifest = {
		root,
		profile: options.profile,
		...installation === void 0 ? {} : { installation: installation.name },
		lowered: WRAPPER_CONTRACT,
		javascriptEntries: javascriptEntries - lazy.length,
		visitedEntries: transform.visited,
		rewrittenEntries: transform.rewritten,
		lazy
	};
	swept[MANIFEST_PATH] = encoder.encode(`${JSON.stringify(manifest, null, 2)}\n`);
	for (const directory of options.emptyDirectories ?? IMAGE_EMPTY_DIRECTORIES) swept[directory] = /* @__PURE__ */ new Uint8Array(0);
	return {
		image: compressImage(packTar(swept)),
		files: swept,
		lazyArchives,
		packages,
		workspacePackages: [...packages.keys()].filter((name) => options.workspaces.has(name)).length,
		roster,
		missing,
		executables,
		pageBundles: Object.keys(swept).filter((name) => pageAsset(name)),
		javascriptEntries: manifest.javascriptEntries,
		droppedJavascriptEntries,
		unresolvedExternalRequests,
		transform,
		contract: WRAPPER_CONTRACT
	};
}
/**
* Pack opaque data trees into one ordered VFS overlay.
*
* Overlay mounts are restricted to the runtime-owned data directories, so an
* overlay cannot replace configuration, the lowering manifest, or modules.
* Files bypass package excludes and module reachability processing; later
* trees replace earlier files at the same path.
* @param trees - Absolute source directories and their data-directory mounts.
* @returns Deterministic compressed archive plus its uncompressed entries.
*/
function packVfsOverlay(trees) {
	const files = {};
	for (const tree of trees) {
		if (!existsSync(tree.directory)) throw new Error(`vfs overlay: tree ${tree.mount} is missing at ${tree.directory}`);
		const mount = tree.mount.replace(/^\.\//, "").replace(/\/$/, "");
		const first = mount.split("/")[0];
		if (mount === "" || first === void 0 || !IMAGE_OVERLAY_DIRECTORIES.includes(first) || mount.split("/").some((segment) => segment === "" || segment === "." || segment === "..")) throw new Error(`vfs overlay: mount ${JSON.stringify(tree.mount)} must stay under ${IMAGE_OVERLAY_DIRECTORIES.join(" or ")}`);
		collectTree(tree.directory, files, mount, () => true, true);
	}
	return {
		image: compressImage(packTar(files)),
		files
	};
}
// lib/types/preview.js
/** Node-side Session preparation for browser Preview data overlays. */
/**
* Pack Preview data with strictly validated current Session successors prepared in Node.
* Committed source bytes remain in the overlay; only the newest canonical raw generation
* of each Session is restored. Future, malformed, and unsupported selected data refuse the pack.
* @param trees - Ordered source trees under the Preview's home and workspace mounts.
* @returns Deterministic overlay containing unchanged inputs and final current successors.
*/
function packPreviewFixture(trees) {
	const original = packVfsOverlay(trees);
	const successors = /* @__PURE__ */ new Map();
	const sources = selectedSessions(original.files).map((source) => {
		const text = new TextDecoder("utf-8", { fatal: true }).decode(source.bytes);
		if (!text.endsWith("\n")) throw new Error(`preview fixture: ${source.path} has a torn physical tail`);
		const [headerLine = "", ...rows] = text.slice(0, -1).split("\n");
		const header = JSON.parse(headerLine);
		if (!isSessionFormatJsonObject(header) || header["version"] !== source.version || header["id"] !== source.id) throw new Error(`preview fixture: ${source.path} disagrees with its Session header`);
		const restore = (source.version <= 3 ? historicalSessionFormatCatalog : sessionFormatCatalog).createRestore(header, {
			recovery: "strict",
			validation: "current"
		});
		const events = rows.map((row) => JSON.parse(row));
		for (const event of events) restore.decodeRow(event);
		return {
			...source,
			header,
			events,
			artifact: restore.finish()
		};
	});
	for (const source of sources) {
		if (source.version === sessionFormatCatalog.currentVersion) continue;
		const children = sources.filter((child) => posix.dirname(child.directory) === posix.dirname(source.directory) && child.artifact.header.parentSession === source.id && child.artifact.header.origin === "subagent").map((child) => ({
			...historicalChildCatalogSource(child.artifact),
			sourcePath: child.path
		}));
		const restore = createSessionFormatCatalogWithChildren(children).createRestore(source.header, {
			recovery: "strict",
			validation: "current"
		});
		for (const event of source.events) restore.decodeRow(event);
		const artifact = restore.finish();
		const encodedHeader = sessionFormatCatalog.encodeCurrentHeader(artifact.header, artifact.inheritedEventCount);
		const encodedEvents = artifact.events.map((event) => sessionFormatCatalog.encodeCurrentEvent(event));
		const verification = sessionFormatCatalog.createRestore(encodedHeader, {
			recovery: "strict",
			validation: "current"
		});
		for (const event of encodedEvents) verification.decodeRow(event);
		verification.finish();
		successors.set(posix.join(source.directory, sessionFormatLogFilename(sessionFormatCatalog.currentVersion)), [encodedHeader, ...encodedEvents].map((row) => JSON.stringify(row) + "\n").join(""));
	}
	if (successors.size === 0) return original;
	const directory = mkdtempSync(join(tmpdir(), "dsh-preview-sessions-"));
	try {
		for (const [imagePath, content] of successors) {
			const path = join(directory, imagePath.slice(14));
			mkdirSync(dirname(path), { recursive: true });
			writeFileSync(path, content, { flag: "wx" });
		}
		return packVfsOverlay([...trees, {
			mount: "home/sessions",
			directory
		}]);
	} finally {
		rmSync(directory, {
			recursive: true,
			force: true
		});
	}
}
function selectedSessions(files) {
	const selected = /* @__PURE__ */ new Map();
	for (const [path, bytes] of Object.entries(files)) {
		const match = /^home\/sessions\/[^/]+\/([^/]+)\/(session(?:\..*)?\.jsonl(?:\.zstd)?)$/u.exec(path);
		if (match === null) continue;
		const version = parseSessionFormatLogFilename(match[2]);
		if (version === void 0) throw new Error(`preview fixture: ${path} must name a canonical raw Session generation`);
		const directory = posix.dirname(path);
		const previous = selected.get(directory);
		if (previous === void 0 || previous.version < version) selected.set(directory, {
			path,
			directory,
			id: match[1],
			version,
			bytes
		});
	}
	return [...selected.values()];
}
// lib/types/repository.js
/**
* Repository knowledge for the packer: where this tree's workspaces, installation
* manifest, profile bundles, and config trees are, and how to report a pack.
*
* The library half takes all of this as parameters. Keeping the lookup here is what
* lets the same library pack a different tree, and what keeps `pack.ts` free of
* assumptions about pnpm workspaces or the `dsh` CLI.
* @module @deepseek-ai/dsh-experimental-webworker-packer/src/repository
*/
/**
* Repository directories scanned for workspace and vendored packages. The
* image only ever materializes runtime packages, which live here. The Landlock
* package family contributes its unchanged JavaScript entry from `native/`;
* examples and python never occur on a roster's dependency chain.
*/
const WORKSPACE_SCAN_ROOTS = [
	"vendor",
	"packages",
	"native/system/packages",
	"apps"
];
/** The `dsh` installation package: its manifest anchors bundle resolution, as the launcher's does. */
const CLI_PACKAGE = "apps/cli";
/** Diagnostic prefix for the app-boot readers this module calls. */
const BIN_NAME = "dsh-pack-vfs-image";
/** Repository-owned deterministic filesystem content offered by the preview. */
const PREVIEW_EXAMPLE_ROOT = "packages/experimental/webworker-runtime/tests/fixtures/vfs-example";
/**
* Index every workspace and vendored package by name.
* @param repoRoot - Absolute repository root.
* @returns Package name to absolute directory.
*/
function indexWorkspacePackages(repoRoot) {
	const index = /* @__PURE__ */ new Map();
	const visit = (directory) => {
		const manifest = join(directory, "package.json");
		if (existsSync(manifest)) {
			const name = JSON.parse(readFileSync(manifest, "utf8")).name;
			if (typeof name === "string") index.set(name, directory);
			return;
		}
		for (const entry of readdirSync(directory, { withFileTypes: true })) {
			if (!entry.isDirectory()) continue;
			if (entry.name === "node_modules" || entry.name.startsWith(".")) continue;
			visit(join(directory, entry.name));
		}
	};
	for (const scanRoot of WORKSPACE_SCAN_ROOTS) {
		const absolute = join(repoRoot, scanRoot);
		if (existsSync(absolute)) visit(absolute);
	}
	return index;
}
/**
* Read what one shipped profile offers from the installation the launcher boots:
* the template's bundles and every optional bundle, each resolved from the CLI
* manifest as the launcher resolves them, and the packages their patch files name.
* Machine state never contributes, so the same tree always packs the same roster.
* @param repoRoot - Absolute repository root.
* @param profile - Shipped profile template name.
* @returns Bundle names and the roster to materialize.
* @throws When the profile has no shipped template, or a bundle does not resolve or declares no patch.
*/
function profileRoster(repoRoot, profile) {
	const template = shippedTemplate(profile);
	const bundles = [.../* @__PURE__ */ new Set([...template.bundles, ...OPTIONAL_BUNDLES])];
	const names = new Set(bundles);
	for (const bundle of bundles) collectModuleNames(bundlePatches(repoRoot, bundle), names);
	return {
		bundles,
		roster: [...names]
	};
}
/**
* The home patch one shipped profile's image carries: every row `PLATFORM_ENTRY_CONFIG` names, with the config the
* template's bundle layers compose for it and the Worker's required fields over that config. A patch replaces a row's
* whole config, so the composed fields stay; the image carries the same bundle files read here.
* @param repoRoot - Absolute repository root.
* @param profile - Shipped profile template name.
* @returns Home patch rows; a row the composition lacks contributes none.
* @throws When the profile has no shipped template, or a bundle does not resolve or declares no patch.
*/
function platformHomePatch(repoRoot, profile) {
	const composed = composeEntries([shippedTemplate(profile).bundles.flatMap((bundle) => bundlePatches(repoRoot, bundle))]);
	return Object.entries(PLATFORM_ENTRY_CONFIG).flatMap(([id, fields]) => {
		const row = findRow(composed, id);
		if (row === void 0) return [];
		return [{
			id,
			config: {
				...typeof row.config === "object" && row.config !== null && !Array.isArray(row.config) ? row.config : {},
				...fields
			}
		}];
	});
}
/** Find one Loader row by id, inside group configs too. */
function findRow(rows, id) {
	if (!Array.isArray(rows)) return void 0;
	for (const row of rows) {
		if (row.id === id) return row;
		const nested = findRow(row.config, id);
		if (nested !== void 0) return nested;
	}
}
/** The shipped template of one profile, or a refusal naming it. */
function shippedTemplate(profile) {
	const template = Object.hasOwn(PROFILE_TEMPLATES, profile) ? PROFILE_TEMPLATES[profile] : void 0;
	if (template === void 0) throw new Error(`vfs image: no shipped profile template named ${JSON.stringify(profile)}`);
	return template;
}
/** One bundle's patch rows, resolved from the CLI manifest as the launcher resolves them. */
function bundlePatches(repoRoot, bundle) {
	const directory = resolveBundleDir(BIN_NAME, bundle, installationManifest(repoRoot).manifest, join(repoRoot, CLI_PACKAGE));
	const declaration = JSON.parse(readFileSync(join(directory, "package.json"), "utf8")).dsh?.bundle;
	if (declaration === void 0) throw new Error(`vfs image: bundle ${bundle} declares no dsh.bundle`);
	return bundlePatchPaths(directory, declaration).flatMap((file) => loadOverlayPatches(BIN_NAME, file));
}
/**
* The installation manifest the image anchors bundle resolution at.
* @param repoRoot - Absolute repository root.
* @returns The CLI package name and its manifest path.
*/
function installationManifest(repoRoot) {
	const manifest = join(repoRoot, CLI_PACKAGE, "package.json");
	const { name } = JSON.parse(readFileSync(manifest, "utf8"));
	return {
		name,
		manifest
	};
}
/**
* Pack this repository's base image for one shipped profile.
* @param repoRoot - Absolute repository root.
* @param profile - Shipped profile template name.
* @param root - Virtual root; the runtime default when omitted.
* @returns The packed image and its lazy archives.
*/
function packRepositoryImage(repoRoot, profile, root) {
	return packVfsImage({
		profile,
		roster: profileRoster(repoRoot, profile).roster,
		installation: installationManifest(repoRoot),
		...root === void 0 ? {} : { root },
		workspaces: indexWorkspacePackages(repoRoot),
		resolveFrom: repoRoot,
		configTrees: configTrees(repoRoot),
		homePatch: platformHomePatch(repoRoot, profile)
	});
}
/**
* Config trees the CLI package declares for deployment images
* (`dsh.configTrees` in its package.json): `path` is relative to the CLI
* package root, `mount` is the image path, `scanRoster` feeds the tree's yml
* plugin rows into the pack roster. The CLI owns its config layout; this
* reader follows the declaration instead of naming directories. A malformed
* declaration refuses the pack.
* @param repoRoot - Absolute repository root.
* @returns Trees with absolute source directories.
*/
function configTrees(repoRoot) {
	const packageDir = join(repoRoot, CLI_PACKAGE);
	const declared = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8")).dsh?.configTrees;
	if (declared === void 0) return [];
	if (!Array.isArray(declared)) throw new Error(`vfs image: ${CLI_PACKAGE} dsh.configTrees must be an array`);
	const mounts = /* @__PURE__ */ new Set();
	return declared.map((entry, index) => {
		const tree = entry;
		const at = `${CLI_PACKAGE} dsh.configTrees[${String(index)}]`;
		if (tree === null || typeof tree !== "object" || typeof tree.mount !== "string" || tree.mount === "" || typeof tree.path !== "string" || tree.path === "" || tree.scanRoster !== void 0 && typeof tree.scanRoster !== "boolean") throw new Error(`vfs image: ${at} must declare a string mount, a string path, and an optional boolean scanRoster`);
		if (mounts.has(tree.mount)) throw new Error(`vfs image: ${at} repeats mount ${JSON.stringify(tree.mount)}`);
		mounts.add(tree.mount);
		return {
			mount: tree.mount,
			directory: join(packageDir, tree.path),
			...tree.scanRoster === void 0 ? {} : { scanRoster: tree.scanRoster }
		};
	});
}
/**
* Built-in filesystem fixtures offered by the repository preview.
* Session and Workspace semantics remain opaque here; the owning runtime tests
* validate those files through their production readers.
* @param repoRoot - Absolute repository root.
* @returns Named chooser entries and their overlay trees.
*/
function previewFixtures(repoRoot) {
	const root = join(repoRoot, PREVIEW_EXAMPLE_ROOT);
	return [{
		id: "vfs-example",
		label: "Built-in showcase",
		description: "Sample workspace, tool cards, subagents, and paged history.",
		trees: ["home", "workspace"].map((mount) => ({
			mount,
			directory: join(root, mount)
		}))
	}];
}
/**
* Render one pack as the lines a build log should carry.
*
* Refusals and unresolved dependencies are the two states a reader must not miss, so
* they are spelled out rather than counted.
* @param result - What the pack produced.
* @param repoRoot - Absolute repository root, for relative paths.
* @param outputFile - Where the image was written.
* @returns Lines to print.
*/
function describePack(result, repoRoot, outputFile) {
	const sizeOf = (prefix) => Object.entries(result.files).filter(([name]) => name.startsWith(prefix)).reduce((sum, [, bytes]) => sum + bytes.byteLength, 0);
	const megabytes = (bytes) => `${(bytes / 1024 / 1024).toFixed(2)} MB`;
	const workspaceCount = result.workspacePackages;
	const heaviest = [...result.packages.entries()].map(([name, count]) => ({
		name,
		count,
		bytes: sizeOf(`node_modules/${name}/`)
	})).sort((left, right) => right.bytes - left.bytes).slice(0, 12);
	return [
		`vfs image: ${relative(repoRoot, outputFile)}`,
		`  roster entries      ${String(result.roster.length)}`,
		`  packages            ${String(result.packages.size)} (${String(workspaceCount)} workspace)`,
		`  files               ${String(Object.keys(result.files).length)}`,
		`  raw                 ${megabytes(Object.values(result.files).reduce((sum, bytes) => sum + bytes.byteLength, 0))}`,
		`  compressed          ${megabytes(result.image.byteLength)}`,
		`  config + presets    ${megabytes(sizeOf("config/"))}`,
		`  lazy archives       ${String(result.lazyArchives.length)} (${megabytes(result.lazyArchives.reduce((sum, archive) => sum + archive.image.byteLength, 0))} compressed)`,
		...result.lazyArchives.map((archive) => `    ${megabytes(archive.image.byteLength).padStart(9)}  ${archive.name}  ${archive.files.join(", ")}`),
		`  javascript entries  ${String(result.javascriptEntries)} (dropped ${String(result.executables.length)} executable scripts, ${String(result.pageBundles.length)} page bundles verbatim)`,
		`  wrapper contract    ${result.contract}`,
		`  transform           ${String(result.transform.rewritten)} of ${String(result.transform.visited)} reached entries rewritten, ${String(result.droppedJavascriptEntries)} unreachable dropped`,
		`  unresolved          ${String(result.unresolvedExternalRequests.length)} third-party request(s) left to fail loud at require time`,
		"  heaviest packages:",
		...heaviest.map((entry) => `    ${entry.bytes.toString().padStart(9)} B  ${entry.name} (${String(entry.count)} files)`),
		...result.missing.length === 0 ? [] : ["  unresolved dependencies:", ...result.missing.map((entry) => `    ${entry}`)],
		""
	];
}
export { packRepositoryImage as a, packPreviewFixture as c, collectModuleNames as d, packVfsImage as f, installationManifest as i, DEFAULT_ROOT$1 as l, WRAPPER_CONTRACT as m, describePack as n, previewFixtures as o, packVfsOverlay as p, indexWorkspacePackages as r, profileRoster as s, configTrees as t, MANIFEST_PATH as u };
