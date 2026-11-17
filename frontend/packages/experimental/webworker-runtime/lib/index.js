import { parse } from "acorn";
import { analyze } from "eslint-scope";
import { KEYS } from "eslint-visitor-keys";
// \0rolldown/runtime.js
var __defProp = Object.defineProperty;
var __exportAll = (all, no_symbols) => {
	let target = {};
	for (var name in all) __defProp(target, name, {
		get: all[name],
		enumerable: true
	});
	if (!no_symbols) __defProp(target, Symbol.toStringTag, { value: "Module" });
	return target;
};
// src/polyfill/async-context/als-runtime.ts
/**
* Build the runtime the rewritten code calls.
* @param causality - Snapshot face from the `node:async_hooks` proxy; omitted
*   leaves the rewrite inert (it still hops a microtask, but moves no state).
* @returns Runtime object passed to every module wrapper.
*/
function createAlsRuntime(causality) {
	const snapshot = () => causality?.snapshot();
	const restore = (value) => {
		causality?.restore(value);
	};
	return {
		snapshot,
		pause: (value) => {
			const captured = snapshot();
			return Promise.resolve(value).then((settled) => ({
				ok: true,
				value: settled,
				snapshot: captured
			}), (error) => ({
				ok: false,
				error,
				snapshot: captured
			}));
		},
		resume: (token) => {
			restore(token.snapshot);
			if (token.ok) return token.value;
			throw token.error;
		},
		afterYield: (captured, sent) => {
			restore(captured);
			return sent;
		},
		iterator: (value) => {
			const source = value;
			const asyncFactory = source[Symbol.asyncIterator];
			if (typeof asyncFactory === "function") return asyncFactory.call(source);
			const syncFactory = source[Symbol.iterator];
			if (typeof syncFactory !== "function") throw new TypeError("webworker als: for-await source is neither async nor sync iterable");
			const inner = syncFactory.call(source);
			return {
				next: async (...args) => {
					const step = inner.next(...args);
					return {
						done: step.done ?? false,
						value: await step.value
					};
				},
				return: async (sent) => {
					const step = inner.return?.(sent) ?? {
						done: true,
						value: void 0
					};
					return {
						done: step.done ?? true,
						value: await step.value
					};
				}
			};
		},
		close: async (iterator) => {
			try {
				return await iterator.return?.(void 0);
			} catch {
				return;
			}
		}
	};
}
// src/transport/frames.ts
/**
* Validate a `postMessage` payload as a tunnel frame.
* @param data - Message data received by the worker.
* @returns The frame.
*/
function parseInboundFrame(data) {
	if (typeof data !== "object" || data === null) throw new Error(`webworker tunnel: message is not a frame: ${String(data)}`);
	const frame = data;
	if (frame.t === "init") {
		if (typeof frame.image !== "string") throw new Error("webworker tunnel: init frame needs a string image url");
		if (!Array.isArray(frame.overlays) || frame.overlays.some((overlay) => typeof overlay !== "string")) throw new Error("webworker tunnel: init frame needs an array of string overlay urls");
		return {
			t: "init",
			image: frame.image,
			overlays: frame.overlays
		};
	}
	const id = frame.id;
	if (typeof id !== "string" && typeof id !== "number") throw new Error(`webworker tunnel: frame has no usable id: ${JSON.stringify(frame.id)}`);
	if (frame.t === "abort") return {
		t: "abort",
		id
	};
	if (frame.t === "stream-uplink-end") return {
		t: "stream-uplink-end",
		id
	};
	if (frame.t === "stream-uplink-item") return {
		t: "stream-uplink-item",
		id,
		value: frame.value
	};
	if (frame.t === "stream-open") {
		if (typeof frame.endpoint !== "string" || frame.endpoint.length === 0) throw new Error(`webworker tunnel: stream ${String(id)} needs a non-empty endpoint`);
		return {
			t: "stream-open",
			id,
			endpoint: frame.endpoint,
			payload: frame.payload
		};
	}
	if (frame.t !== "req") throw new Error(`webworker tunnel: unknown frame type ${JSON.stringify(frame.t)}`);
	if (typeof frame.method !== "string" || typeof frame.url !== "string") throw new Error(`webworker tunnel: request ${String(id)} needs string method and url`);
	if (typeof frame.headers !== "object" || frame.headers === null) throw new Error(`webworker tunnel: request ${String(id)} needs a headers object`);
	const headers = {};
	for (const [key, value] of Object.entries(frame.headers)) if (typeof value === "string") headers[key.toLowerCase()] = value;
	const body = frame.body;
	if (body !== void 0 && !(body instanceof ArrayBuffer) && !(body instanceof Blob) && !(body instanceof ReadableStream)) throw new Error(`webworker tunnel: request ${String(id)} body must be an ArrayBuffer, Blob, or ReadableStream`);
	return {
		t: "req",
		id,
		method: frame.method,
		url: frame.url,
		headers,
		body
	};
}
// src/node/notImplementedFail.ts
/** Node's standard code for a feature the running platform cannot provide. */
const FEATURE_UNAVAILABLE_ON_PLATFORM = "ERR_FEATURE_UNAVAILABLE_ON_PLATFORM";
/**
* Build the refusal error and write it to the console first, for stubs that
* cannot be a plain throwing function (constructors, methods on structural
* fakes).
* @param module - module specifier being stubbed.
* @param symbol - unavailable member, named as the importer sees it.
* @returns the error to throw, with `code` set to Node's `ERR_FEATURE_UNAVAILABLE_ON_PLATFORM`.
*/
function notAvailableError(module, symbol) {
	const message = `web-preview: ${module}.${symbol} is not available in the worker host`;
	console.error(message);
	return Object.assign(new Error(message), { code: FEATURE_UNAVAILABLE_ON_PLATFORM });
}
// src/node/internal_modules/module-map.ts
/**
* `internal/modules/esm/module_map` for the Worker, following Node v24.18.0. The Worker evaluates no ES module
* jobs, so a {@link LoadCache} stays empty and refuses to store one.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/node/internal_modules/module-map
*/
/** Job slot Node uses when an import names no `type` attribute. */
const IMPLICIT_TYPE_ATTRIBUTE = "javascript";
/**
* Node's `validateString` refusal.
* @param value - Argument to check.
* @param name - Argument name in the message.
*/
function validateString(value, name) {
	if (typeof value === "string") return;
	throw Object.assign(/* @__PURE__ */ new TypeError(`The "${name}" argument must be of type string. Received ${typeof value}`), { code: "ERR_INVALID_ARG_TYPE" });
}
/** Module jobs keyed by URL, then by import `type` attribute; always empty in the Worker. */
var LoadCache = class extends Map {
	/**
	* Read the job for one URL and type.
	* @param url - Module URL.
	* @param type - Import `type` attribute.
	* @returns The cached job, if any.
	*/
	get(url, type = IMPLICIT_TYPE_ATTRIBUTE) {
		validateString(url, "url");
		validateString(type, "type");
		return super.get(url)?.[type];
	}
	/**
	* Refuse to store a job: the Worker creates no ES module jobs.
	* @param url - Module URL.
	* @param type - Import `type` attribute.
	* @param _job - Job Node would store.
	* @returns Never; always throws.
	*/
	set(url, type = IMPLICIT_TYPE_ATTRIBUTE, _job) {
		validateString(url, "url");
		validateString(type, "type");
		throw notAvailableError("internal/modules/esm/module_map", "LoadCache.set");
	}
	/**
	* Check for the job of one URL and type.
	* @param url - Module URL.
	* @param type - Import `type` attribute.
	* @returns Whether a job is cached.
	*/
	has(url, type = IMPLICIT_TYPE_ATTRIBUTE) {
		validateString(url, "url");
		validateString(type, "type");
		return super.get(url)?.[type] !== void 0;
	}
};
// src/node/internal_modules/esm-loader.ts
/**
* `internal/modules/esm/loader` cascaded loader for the Worker. {@link createCascadedLoader} builds the object with
* the Node v24.18.0 (v2) loader members this repository calls: `resolveSync` and `import` over one Worker module
* loader, and an empty `loadCache`. ES module jobs are unavailable on this platform.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/node/internal_modules/esm-loader
*/
/**
* Translate a Worker resolution failure into the code Node's ESM resolver reports.
* @param error - Failure thrown by the Worker module loader.
* @returns The same failure with an ESM resolution code.
*/
function esmResolutionError(error) {
	if (error instanceof Error && error.code === "MODULE_NOT_FOUND") return Object.assign(error, { code: "ERR_MODULE_NOT_FOUND" });
	return error;
}
/**
* Build the cascaded loader for one Worker module loader.
* @param source - The Worker module loader's resolution and loading operations.
* @returns A cascaded loader delegating to `source`.
*/
function createCascadedLoader(source) {
	return {
		loadCache: new LoadCache(),
		resolveSync(parentURL, request) {
			try {
				const { url, format } = source.resolveSync(request.specifier, parentURL);
				return {
					url,
					format
				};
			} catch (error) {
				throw esmResolutionError(error);
			}
		},
		import(specifier, parentURL) {
			return source.import(specifier, parentURL);
		},
		getOrCreateModuleJob() {
			throw notAvailableError("internal/modules/esm/loader", "getOrCreateModuleJob");
		}
	};
}
// src/module-system/posix-path.ts
var posix_path_exports = /* @__PURE__ */ __exportAll({
	SEP: () => "/",
	basename: () => basename,
	dirname: () => dirname,
	extname: () => extname,
	fileUrlToPath: () => fileUrlToPath,
	isAbsolute: () => isAbsolute,
	join: () => join,
	normalize: () => normalize,
	parse: () => parse$1,
	pathToFileUrl: () => pathToFileUrl,
	relative: () => relative,
	resolve: () => resolve,
	toNamespacedPath: () => toNamespacedPath
});
/**
* Collapse `.` and `..` segments.
* @param path - Path with any number of separators.
* @returns Normalized path; a relative input keeps leading `..` segments.
*/
function normalize(path) {
	const absolute = path.startsWith("/");
	const trailing = path.length > 1 && path.endsWith("/");
	const out = [];
	for (const segment of path.split("/")) {
		if (segment === "" || segment === ".") continue;
		if (segment === ".." && out.length > 0 && out[out.length - 1] !== "..") {
			out.pop();
			continue;
		}
		if (segment === ".." && absolute) continue;
		out.push(segment);
	}
	const body = out.join("/");
	if (absolute) return "/" + body + (trailing && body !== "" ? "/" : "");
	if (body === "") return trailing ? "./" : ".";
	return body + (trailing ? "/" : "");
}
/**
* Join segments and normalize the result.
* @param segments - Path segments.
* @returns Joined path, `.` when nothing remains.
*/
function join(...segments) {
	const joined = segments.filter((segment) => segment !== "").join("/");
	return joined === "" ? "." : normalize(joined);
}
/**
* Resolve segments right to left against a base directory.
* @param segments - Path segments; the first absolute one wins.
* @returns Absolute normalized path.
*/
function resolve(...segments) {
	let path = "";
	for (const segment of [...segments].reverse()) {
		if (segment === "") continue;
		path = path === "" ? segment : `${segment}/${path}`;
		if (segment.startsWith("/")) break;
	}
	return normalize(path.startsWith("/") ? path : `/${path}`);
}
/**
* Directory part of a path, after normalization (see the module note).
* @param path - Path to inspect.
* @returns Parent path; `/` for root children and `.` for bare names.
*/
function dirname(path) {
	const normalized = normalize(path).replace(/\/+$/, "");
	const index = normalized.lastIndexOf("/");
	if (index < 0) return ".";
	if (index === 0) return "/";
	return normalized.slice(0, index);
}
/**
* Last segment of a path, after normalization (see the module note).
* @param path - Path to inspect.
* @param suffix - Optional suffix to strip.
* @returns Final segment.
*/
function basename(path, suffix) {
	const normalized = normalize(path).replace(/\/+$/, "");
	const name = normalized.slice(normalized.lastIndexOf("/") + 1);
	if (suffix !== void 0 && suffix !== name && name.endsWith(suffix)) return name.slice(0, -suffix.length);
	return name;
}
/**
* Extension of the last segment, dot included.
* @param path - Path to inspect.
* @returns Extension, or an empty string when there is none.
*/
function extname(path) {
	const name = basename(path);
	const index = name.lastIndexOf(".");
	return index <= 0 ? "" : name.slice(index);
}
/**
* Report whether a path starts at the root.
* @param path - Path to inspect.
* @returns True for absolute paths.
*/
function isAbsolute(path) {
	return path.startsWith("/");
}
/**
* Relative path from one absolute path to another.
* @param from - Source directory.
* @param to - Target path.
* @returns Relative path using `..` segments.
*/
function relative(from, to) {
	const source = resolve(from).split("/").filter((segment) => segment !== "");
	const target = resolve(to).split("/").filter((segment) => segment !== "");
	let shared = 0;
	while (shared < source.length && shared < target.length && source[shared] === target[shared]) shared += 1;
	return [...new Array(source.length - shared).fill(".."), ...target.slice(shared)].join("/");
}
/**
* Split a path into components, after normalization (see the module note).
* @param path - Path to split.
* @returns Root, directory, base name, extension, and stem.
*/
function parse$1(path) {
	const root = isAbsolute(path) ? "/" : "";
	const base = basename(path);
	const ext = extname(path);
	return {
		root,
		dir: dirname(path),
		base,
		ext,
		name: ext === "" ? base : base.slice(0, -ext.length)
	};
}
/**
* Node's Windows-only namespaced-path conversion.
* @param path - the path to convert.
* @returns The path unchanged; namespaced paths are a Windows concept.
*/
function toNamespacedPath(path) {
	return path;
}
/**
* Convert a VFS path into a `file:` URL string.
* @param path - Absolute VFS path.
* @returns URL text with each segment percent-encoded.
*/
function pathToFileUrl(path) {
	return `file://${resolve(path).split("/").map((segment) => encodeURIComponent(segment)).join("/")}`;
}
/**
* Convert a `file:` URL back into a VFS path.
* @param url - URL text or URL instance.
* @returns Absolute VFS path.
*/
function fileUrlToPath(url) {
	const text = typeof url === "string" ? url : url.href;
	if (!text.startsWith("file://")) throw new Error(`webworker vfs: not a file URL: ${text}`);
	return decodeURIComponent(text.slice(7).replace(/[?#].*$/, "")) || "/";
}
// src/image-layout.ts
/**
* Image layout contract shared by the packer and the worker host: the virtual
* root, where the manifest sits inside the image, where lazy archives sit beside
* it, and the working directories every image carries empty. One definition, two
* consumers — the packer writes this layout, the worker host mounts it.
*/
/** Default virtual root; the runtime mounts the image here unless told otherwise. */
const DEFAULT_ROOT = "/dsh";
/**
* Leaf name of the packed base image: one gzip member holding the ustar archive.
* The app build writes it beside the page and the page's boot fetches it from
* there, so the extension is part of what a deployment serves.
*/
const IMAGE_FILE_NAME = "vfs-image.tar.gz";
/** Image path of the manifest the runtime reads before it wraps a single module. */
const IMAGE_MANIFEST_PATH = "config/vfs-manifest.json";
/**
* Directory, relative to the base image URL, holding the lazy chunk archives:
* `lazy/<sha256 of the archive>.tar.gz`, one page chunk each.
*/
const IMAGE_LAZY_DIRECTORY = "lazy";
/** Home directory under the root; the process shim's `DSH_HOME`/`HOME` default. */
const IMAGE_HOME_DIRECTORY = "home";
/**
* Config fields the Worker requires on Loader rows, by row id. The packer writes them into the image's home patch
* (`home/cordis.patch.yml`) over each row's composed config: the Worker carries no Zstandard codec, so JSONL Session
* logs stay plaintext.
*/
const PLATFORM_ENTRY_CONFIG = { "session-persistence-jsonl": { compression: "none" } };
/** Working directories the host tree expects to exist, empty. */
const IMAGE_EMPTY_DIRECTORIES = [
	"home/",
	"workspace/",
	"tmp/"
];
/**
* Top-level directories an overlay archive may populate. Runtime code,
* configuration, and the lowering manifest remain owned by the base image.
*/
const IMAGE_OVERLAY_DIRECTORIES = ["home", "workspace"];
/**
* Identity of the lowered code shape, recorded in the image manifest by the
* packer and required by the worker host: an image lowered by an older transform
* would otherwise run against newer wrapper semantics. Bump on any change to
* emitted code or to {@link WRAPPER_PARAMS}.
*/
const LOWERING_VERSION = "dsh-worker-transform/1";
/**
* Free variables a lowered body expects from its wrapper, in order.
*
* Part of the image layout rather than of the transform, because the loader
* wraps bodies it never parses: the packer emits against these names and the
* worker binds them, with no compiler in the worker bundle to agree with.
*/
const WRAPPER_PARAMS = [
	"exports",
	"require",
	"module",
	"__filename",
	"__dirname",
	"__dsh$meta",
	"__als"
];
// src/module-system/module-loader.ts
/**
* CommonJS module loader over the worker VFS. It backs the Node internal ESM
* loader that `node-addon-require-builtin` answers, which Cordis's Loader uses
* for every entry import, and the `node:module` `createRequire` proxy that
* `typert-loader`, `client-modules`, and the plugin package inventory resolve
* package metadata through.
*
* Resolution is a narrowed Node `require` algorithm: `exports` walk with a
* fixed condition order, extension probing, and one cache keyed by resolved
* absolute path. Module bodies are wrapped as the image holds them: lowering is
* the packer's job, so nothing here parses JavaScript.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/module-system/module-loader
*/
/** Condition keys honoured in `exports`, in order; `node` is deliberately absent. */
const DEFAULT_CONDITIONS = [
	"browser",
	"require",
	"import",
	"default"
];
/** Extensions probed when a specifier has no usable one. */
const EXTENSIONS = [
	".js",
	".json",
	".mjs",
	".cjs"
];
function isRecord(value) {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}
/** Loader for one VFS mount; construct once per worker. */
var WorkerModuleLoader = class {
	vfs;
	root;
	staticModules;
	staticPrefixes;
	conditions;
	als;
	modules = /* @__PURE__ */ new Map();
	manifests = /* @__PURE__ */ new Map();
	stack = [];
	cascaded;
	/**
	* Resolution and loading over this mount, in Node's v1 argument order. The cascaded ESM loader
	* ({@link cascadedLoader}) and the packer call it; `parentURL` positions relative specifiers and import attributes
	* are ignored.
	*/
	internal;
	constructor(options) {
		this.vfs = options.vfs;
		this.root = options.root ?? "/dsh";
		this.staticModules = new Map(Object.entries(options.staticModules));
		this.staticPrefixes = Object.entries(options.staticModulePrefixes ?? {}).sort(([left], [right]) => right.length - left.length);
		this.conditions = new Set(options.conditions ?? DEFAULT_CONDITIONS);
		this.als = createAlsRuntime(options.alsCausality);
		const resolveInternal = (specifier, parentURL) => {
			const from = parentURL === void 0 ? this.root : this.baseDirectoryOf(parentURL);
			const resolution = this.resolve(specifier, from);
			if (resolution.kind === "static") return {
				format: "builtin",
				url: resolution.specifier
			};
			return {
				format: resolution.path.endsWith(".json") ? "json" : "commonjs",
				url: pathToFileUrl(resolution.path)
			};
		};
		this.internal = {
			version: "worker",
			import: async (specifier, parentURL) => {
				const from = parentURL === void 0 ? this.root : this.baseDirectoryOf(parentURL);
				return this.load(this.resolve(specifier, from));
			},
			resolve: async (specifier, parentURL) => resolveInternal(specifier, parentURL),
			resolveSync: resolveInternal
		};
	}
	/**
	* The `internal/modules/esm/loader` cascaded loader over this mount, created on first use.
	* @returns This loader's one cascaded loader.
	*/
	cascadedLoader() {
		return this.cascaded ??= createCascadedLoader(this.internal);
	}
	/**
	* Throw this loader's diagnostic.
	* @param detail - What failed.
	* @param code - Node error code for a resolution failure Node also reports by code.
	*/
	fail(detail, code) {
		const chain = this.stack.length === 0 ? "" : ` (importer chain: ${this.stack.join(" -> ")})`;
		const error = /* @__PURE__ */ new Error(`webworker modules: ${detail}${chain}`);
		throw code === void 0 ? error : Object.assign(error, { code });
	}
	/** @returns Directory a base path or URL resolves specifiers from. */
	baseDirectoryOf(base) {
		const text = typeof base === "string" ? base : base.href;
		const path = text.startsWith("file://") ? fileUrlToPath(text) : text;
		if (path.endsWith("/")) return resolve(path);
		return this.vfs.existsSync(path) && this.vfs.statSync(path).isDirectory() ? resolve(path) : dirname(path);
	}
	manifestOf(directory) {
		const cached = this.manifests.get(directory);
		if (cached !== void 0) return cached;
		const path = join(directory, "package.json");
		const text = this.vfs.readFileSync(path, "utf8");
		let parsed;
		try {
			parsed = JSON.parse(text);
		} catch (reason) {
			this.fail(`${path} is not valid JSON: ${reason.message}`);
		}
		if (!isRecord(parsed)) this.fail(`${path} does not hold an object`);
		const manifest = parsed;
		this.manifests.set(directory, manifest);
		return manifest;
	}
	/** Walk one `exports` value against the condition set and requested subpath. */
	selectExport(field, subpath, packageName) {
		if (field === null) return void 0;
		if (typeof field === "string") return subpath === "." ? field : void 0;
		if (Array.isArray(field)) {
			for (const candidate of field) {
				const picked = this.selectExport(candidate, subpath, packageName);
				if (picked !== void 0) return picked;
			}
			return;
		}
		const entries = Object.entries(field);
		if (!entries.some(([key]) => key === "." || key.startsWith("./"))) {
			if (subpath !== ".") return void 0;
			return this.selectCondition(field, packageName);
		}
		for (const [key, value] of entries) if (key === subpath) return typeof value === "string" ? value : this.selectCondition(value, packageName, subpath);
		for (const [key, value] of entries) {
			const star = key.indexOf("*");
			if (star < 0) continue;
			const prefix = key.slice(0, star);
			const suffix = key.slice(star + 1);
			if (!subpath.startsWith(prefix) || !subpath.endsWith(suffix)) continue;
			const captured = subpath.slice(prefix.length, subpath.length - suffix.length);
			const target = typeof value === "string" ? value : this.selectCondition(value, packageName, subpath);
			if (target !== void 0) return target.replaceAll("*", captured);
		}
	}
	/** Pick the first condition branch this runtime satisfies. */
	selectCondition(field, packageName, subpath = ".") {
		if (field === null) return void 0;
		if (typeof field === "string") return field;
		if (Array.isArray(field)) {
			for (const candidate of field) {
				const picked = this.selectCondition(candidate, packageName, subpath);
				if (picked !== void 0) return picked;
			}
			return;
		}
		for (const [key, value] of Object.entries(field)) {
			if (!this.conditions.has(key)) continue;
			const picked = this.selectCondition(value, packageName, subpath);
			if (picked !== void 0) return picked;
		}
	}
	/** Extension and directory probing for a concrete path. */
	probe(path, specifier) {
		const candidates = [path, ...EXTENSIONS.map((extension) => path + extension)];
		for (const candidate of candidates) if (this.vfs.existsSync(candidate) && this.vfs.statSync(candidate).isFile()) return candidate;
		if (this.vfs.existsSync(path) && this.vfs.statSync(path).isDirectory()) {
			if (this.vfs.existsSync(join(path, "package.json"))) {
				const main = this.manifestOf(path).main;
				if (main !== void 0) return this.probe(join(path, main), specifier);
			}
			return this.probe(join(path, "index"), specifier);
		}
		return this.fail(`cannot resolve "${specifier}": no file at ${candidates.join(", ")}`, "MODULE_NOT_FOUND");
	}
	/** @returns The Worker-provided implementation of a static specifier. */
	staticModule(specifier) {
		const exact = this.staticModules.get(specifier);
		if (exact !== void 0) return exact;
		for (const [prefix, factory] of this.staticPrefixes) if (specifier.startsWith(prefix)) return factory;
		return this.staticModules.get(`node:${specifier}`);
	}
	/**
	* Resolve a specifier the way the module that requested it would.
	* @param specifier - Bare name, relative path, absolute path, or file URL.
	* @param fromDirectory - Directory of the requesting module.
	* @returns Static module or the resolved VFS path.
	*/
	resolve(specifier, fromDirectory) {
		const staticModule = this.staticModule(specifier);
		if (staticModule !== void 0) return {
			kind: "static",
			specifier,
			factory: staticModule
		};
		if (specifier.startsWith("cordis:") || specifier.startsWith("node:")) return this.fail(`no static module is registered for "${specifier}"`);
		if (specifier.startsWith("file://")) return {
			kind: "file",
			path: this.probe(fileUrlToPath(specifier), specifier)
		};
		if (specifier.startsWith(".")) return {
			kind: "file",
			path: this.probe(join(fromDirectory, specifier), specifier)
		};
		if (isAbsolute(specifier)) return {
			kind: "file",
			path: this.probe(specifier, specifier)
		};
		const segments = specifier.split("/");
		const packageName = specifier.startsWith("@") ? segments.slice(0, 2).join("/") : segments[0] ?? specifier;
		const rest = specifier.slice(packageName.length).replace(/^\//, "");
		const packageDirectory = join(this.root, "node_modules", packageName);
		if (!this.vfs.existsSync(join(packageDirectory, "package.json"))) return this.fail(`cannot resolve "${specifier}": ${packageDirectory}/package.json is not in the image`, "MODULE_NOT_FOUND");
		const manifest = this.manifestOf(packageDirectory);
		const subpath = rest === "" ? "." : `./${rest}`;
		if (manifest.exports !== void 0) {
			const target = this.selectExport(manifest.exports, subpath, packageName);
			if (target === void 0) return this.fail(`"${packageName}" does not export "${subpath}" under conditions [${[...this.conditions].join(", ")}]`, "ERR_PACKAGE_PATH_NOT_EXPORTED");
			return {
				kind: "file",
				path: this.probe(join(packageDirectory, target), specifier)
			};
		}
		const legacy = subpath === "." ? manifest.main ?? "index.js" : rest;
		return {
			kind: "file",
			path: this.probe(join(packageDirectory, legacy), specifier)
		};
	}
	/**
	* Load a resolved module, reusing the cache and tolerating cycles with
	* CommonJS partial-export semantics.
	* @param resolution - Result of {@link resolve}.
	* @returns The module's exports.
	*/
	load(resolution) {
		if (resolution.kind === "static") return resolution.factory();
		const path = resolution.path;
		const cached = this.modules.get(path);
		if (cached !== void 0) return cached.module.exports;
		if (path.endsWith(".json")) {
			const parsed = JSON.parse(this.vfs.readFileSync(path, "utf8"));
			this.modules.set(path, { module: { exports: parsed } });
			return parsed;
		}
		const record = { module: { exports: {} } };
		this.modules.set(path, record);
		this.stack.push(path);
		try {
			const source = this.vfs.readFileSync(path, "utf8");
			const factory = this.compile(source, path);
			const directory = dirname(path);
			factory(record.module.exports, this.requireFrom(directory), record.module, path, directory, {
				url: pathToFileUrl(path),
				resolve: (specifier) => {
					const resolution = this.resolve(specifier, directory);
					return resolution.kind === "static" ? resolution.specifier : pathToFileUrl(resolution.path);
				}
			}, this.als);
			return record.module.exports;
		} catch (reason) {
			this.modules.delete(path);
			throw reason;
		} finally {
			this.stack.pop();
		}
	}
	/**
	* Compile a body the image already lowered.
	*
	* Module syntax reaching here means the image was packed by something other
	* than the packer, or its collector missed the entry. The worker carries no
	* transform to recover with, so it names the image as the thing to rebuild.
	* @param code - Module body as the image holds it.
	* @param path - Resolved VFS path.
	* @returns The wrapper factory.
	*/
	compile(code, path) {
		try {
			return new Function(...WRAPPER_PARAMS, code);
		} catch (reason) {
			if (reason instanceof SyntaxError && /await/i.test(reason.message)) this.fail(`${path} uses top-level await, which cannot run as CommonJS in the worker: ${reason.message}`);
			if (reason instanceof SyntaxError && /import|export/i.test(reason.message)) this.fail(`${path} still carries module syntax, so the image was not lowered by the packer (${reason.message}); rebuild the image`);
			this.fail(`${path} failed to compile: ${reason.message}`);
		}
	}
	/**
	* Build a `require` bound to a directory.
	* @param fromDirectory - Directory relative specifiers resolve against.
	* @returns Callable require with `resolve`.
	*/
	requireFrom(fromDirectory) {
		const require = (specifier) => this.load(this.resolve(specifier, fromDirectory));
		const resolve$1 = ((specifier) => {
			const resolution = this.resolve(specifier, fromDirectory);
			if (resolution.kind === "static") return this.fail(`"${specifier}" is a worker-provided module and has no VFS path`);
			return resolution.path;
		});
		resolve$1.paths = (specifier) => {
			if (this.staticModule(specifier) !== void 0 || specifier.startsWith("node:")) return null;
			if (specifier.startsWith(".")) return [resolve(fromDirectory, ".")];
			return [join(this.root, "node_modules")];
		};
		return Object.assign(require, { resolve: resolve$1 });
	}
	/**
	* `node:module` `createRequire` for the VFS.
	* @param base - Module path, directory path, or `file:` URL.
	* @returns Require bound to that base.
	*/
	createRequire(base) {
		return this.requireFrom(this.baseDirectoryOf(base));
	}
	/**
	* Report what this loader has done, for the host's boot diagnostics.
	* @returns How many module bodies it has run.
	*/
	usage() {
		return { modules: this.modules.size };
	}
};
let active$1;
/**
* Publish the loader the `node:module` proxy resolves through.
* @param loader - Loader built by the worker entry.
*/
function setActiveModuleLoader(loader) {
	active$1 = loader;
}
/**
* Read the published loader.
* @returns The active loader.
*/
function requireActiveModuleLoader() {
	if (active$1 === void 0) throw new Error("webworker modules: no loader is mounted; the worker entry must call setActiveModuleLoader before any createRequire use");
	return active$1;
}
// src/module-proxies.ts
/**
* The worker bundle's module proxy table: the ONLY platform fork of the host
* tree. Entries replace Node builtins, external npm packages, and the native
* flock subpath; other workspace and vendored modules are mounted as they ship.
*
* The build turns these into bundler aliases, and `node/builtins.ts` turns the
* same modules into the loader's static table — one list, two consumers.
*
* The replacement path states the classification. `./node/builtin_modules/implemented/<module>.ts`
* carries the module's real semantics over a worker-side data source (VFS, the
* tunnel, a wasm codec, a browser primitive); `./node/builtin_modules/mock/<module>.ts` is a
* structural placeholder that mounts silently and reports the missing capability
* when a call finally reaches it. External npm replacements live in
* `./externals/`, named after the package they stand in for.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/module-proxies
*/
/**
* Module proxy table — the ONLY platform fork of the worker host. Keys are
* exact module specifiers; the native system package's Landlock entry stays
* unmodified while its flock subpath is replaced.
*/
const MODULE_PROXIES = {
	"node:fs": "./node/builtin_modules/implemented/fs.ts",
	"fs": "./node/builtin_modules/implemented/fs.ts",
	"node:fs/promises": "./node/builtin_modules/implemented/fs/promises.ts",
	"fs/promises": "./node/builtin_modules/implemented/fs/promises.ts",
	"node:path": "./node/builtin_modules/implemented/path.ts",
	"path": "./node/builtin_modules/implemented/path.ts",
	"node:path/posix": "./node/builtin_modules/implemented/path.ts",
	"node:os": "./node/builtin_modules/implemented/os.ts",
	"os": "./node/builtin_modules/implemented/os.ts",
	"node:url": "./node/builtin_modules/implemented/url.ts",
	"node:module": "./node/builtin_modules/implemented/module.ts",
	"node:crypto": "./node/builtin_modules/implemented/crypto.ts",
	"crypto": "./node/builtin_modules/implemented/crypto.ts",
	"node:buffer": "./node/builtin_modules/implemented/buffer.ts",
	"node:http": "./node/builtin_modules/implemented/http.ts",
	"node:async_hooks": "./node/builtin_modules/implemented/async_hooks.ts",
	"node:assert/strict": "./node/builtin_modules/implemented/assert/strict.ts",
	"assert/strict": "./node/builtin_modules/implemented/assert/strict.ts",
	"node:util": "./node/builtin_modules/implemented/util.ts",
	"node:util/types": "./node/builtin_modules/implemented/util/types.ts",
	"node:events": "./node/builtin_modules/implemented/events.ts",
	"node:timers/promises": "./node/builtin_modules/implemented/timers/promises.ts",
	"node:perf_hooks": "./node/builtin_modules/implemented/perf_hooks.ts",
	"node:tty": "./node/builtin_modules/implemented/tty.ts",
	"tty": "./node/builtin_modules/implemented/tty.ts",
	"node:zlib": "./node/builtin_modules/implemented/zlib.ts",
	"node:child_process": "./node/builtin_modules/implemented/child_process.ts",
	"node:https": "./node/builtin_modules/mock/https.ts",
	"node:dns/promises": "./node/builtin_modules/mock/dns/promises.ts",
	"dns/promises": "./node/builtin_modules/mock/dns/promises.ts",
	"node:net": "./node/builtin_modules/mock/net.ts",
	"node:stream": "./node/builtin_modules/implemented/stream.ts",
	"node:string_decoder": "./node/builtin_modules/implemented/string_decoder.ts",
	"node:stream/promises": "./node/builtin_modules/implemented/stream/promises.ts",
	"stream/promises": "./node/builtin_modules/implemented/stream/promises.ts",
	"node:vm": "./node/builtin_modules/mock/vm.ts",
	"node:repl": "./node/builtin_modules/mock/repl.ts",
	"node:worker_threads": "./node/builtin_modules/mock/worker_threads.ts",
	"node:sqlite": "./node/builtin_modules/mock/sqlite.ts",
	"@deepseek-ai/libreoffice-kit": "./node/external_packages/libreoffice-kit.ts",
	"@deepseek-ai/node-addon-system/flock": "./node/external_packages/node-addon-system-flock.ts",
	"koffi": "./node/external_packages/koffi.ts",
	"sharp": "./node/external_packages/sharp.ts",
	"node-pty": "./node/external_packages/node-pty.ts",
	"execa": "./node/external_packages/execa.ts",
	"got": "./node/external_packages/got.ts",
	"@vscode/ripgrep": "./node/external_packages/ripgrep.ts",
	"@earendil-works/pi-ai": "./node/external_packages/pi-ai.ts",
	"@earendil-works/pi-coding-agent": "./node/external_packages/pi-coding-agent.ts",
	"node-addon-require-builtin": "./node/external_packages/node-addon-require-builtin.ts",
	"ws": "./node/external_packages/ws.ts"
};
/** pi-ai subpaths (`/providers/all`, `/api/*.lazy`) share the one structural stub. */
const MODULE_PROXY_PREFIXES = { "@earendil-works/pi-ai/": "./node/external_packages/pi-ai.ts" };
// src/node/external_packages/replaced-externals.ts
/**
* Exact package or subpath specifiers served from the worker bundle. Kept
* import-free for the runtime builtin table and the VFS image collector.
* Whole-package entries are omitted from the image; subpath entries leave
* their parent package available to other consumers.
*/
/** Package or subpath specifiers served from the worker bundle instead of the VFS. */
const REPLACED_EXTERNAL_PACKAGES = [
	"@earendil-works/pi-ai",
	"@earendil-works/pi-coding-agent",
	"@vscode/ripgrep",
	"@deepseek-ai/libreoffice-kit",
	"@deepseek-ai/node-addon-system/flock",
	"koffi",
	"node-pty",
	"execa",
	"got",
	"node-addon-require-builtin",
	"sharp",
	"ws"
];
// src/transport/synthetic-http.ts
const encoder$3 = new TextEncoder();
/**
* Build the request/response pair for one tunnel request.
*
* `res.end()` is the settle point: the captured listener returns void, so the
* response object itself reports completion. `write()` always returns true,
* which skips backpressure waiting the tunnel cannot observe anyway.
* @param frame - Validated request frame.
* @param sink - Frame emitter for the response.
* @returns The pair handed to the captured request listener.
*/
function createSyntheticExchange(frame, sink) {
	const listeners = /* @__PURE__ */ new Map();
	let status = 200;
	let headers = {};
	let streaming = false;
	let finished = false;
	let aborted = false;
	const emit = (event) => {
		for (const callback of [...listeners.get(event) ?? []]) callback();
	};
	const req = {
		url: frame.url,
		method: frame.method,
		headers: frame.headers,
		destroy: () => {
			aborted = true;
		},
		async *[Symbol.asyncIterator]() {
			if (frame.body === void 0) return;
			if (frame.body instanceof Blob) {
				for await (const chunk of frame.body.stream()) {
					if (aborted) return;
					if (chunk.byteLength > 0) yield chunk;
				}
				return;
			}
			if (frame.body instanceof ReadableStream) {
				for await (const chunk of frame.body) {
					if (aborted) return;
					if (!(chunk instanceof Uint8Array)) throw new TypeError("webworker tunnel: request stream produced a non-Uint8Array chunk");
					if (chunk.byteLength > 0) yield chunk;
				}
				return;
			}
			if (aborted || frame.body.byteLength === 0) return;
			yield new Uint8Array(frame.body);
		}
	};
	const res = {
		writeHead: (nextStatus, nextHeaders) => {
			status = nextStatus;
			if (nextHeaders !== void 0) {
				headers = {};
				for (const [key, value] of Object.entries(nextHeaders)) headers[key.toLowerCase()] = String(value);
			}
			return res;
		},
		write: (chunk) => {
			if (finished || aborted) return false;
			if (!streaming) {
				streaming = true;
				sink.head(status, headers);
			}
			sink.chunk(typeof chunk === "string" ? encoder$3.encode(chunk) : chunk);
			return true;
		},
		end: (body) => {
			if (finished) return res;
			finished = true;
			const bytes = body === void 0 ? void 0 : typeof body === "string" ? encoder$3.encode(body) : body;
			if (streaming) {
				if (bytes !== void 0) sink.chunk(bytes);
				sink.end();
			} else sink.end({
				status,
				headers,
				body: bytes
			});
			emit("close");
			return res;
		},
		destroy: () => {
			if (finished) return;
			finished = true;
			sink.fail(`response destroyed for ${frame.method} ${frame.url}`);
			emit("close");
		},
		on: (event, callback) => {
			const set = listeners.get(event) ?? /* @__PURE__ */ new Set();
			set.add(callback);
			listeners.set(event, set);
			return res;
		},
		off: (event, callback) => {
			listeners.get(event)?.delete(callback);
			return res;
		}
	};
	res.once = res.on;
	Object.defineProperty(res, "headersSent", { get: () => streaming });
	Object.defineProperty(res, "writableEnded", { get: () => finished });
	return {
		req,
		res,
		get aborted() {
			return aborted;
		},
		abort: () => {
			if (finished) return;
			aborted = true;
			finished = true;
			emit("aborted");
			emit("close");
		}
	};
}
// src/compile/transform.ts
/**
* The worker's module transform: one acorn parse turns an ES module into a
* CommonJS body **and** routes every suspension point through the ambient-store
* protocol.
*
* Both jobs live in one pass because they are two edits over one syntax tree;
* running a lexer first and a parser second meant two scanners, two sets of
* blind spots, and a second pass reading the first pass's output. Editing is
* interval-based — the original text is sliced and spliced, never reprinted —
* so **line numbers survive**: a stack frame in a transformed module points at
* the same line as the built artifact it came from.
*
* Named and default imports stay live: every reference reads the required
* module object when it runs (`__dsh$m1["x"]`), never a copy taken at the
* import statement, so a module in an import cycle sees a binding once the
* exporting module has initialized it, as ES modules do. Scope analysis tells
* those references apart from same-named locals that shadow them; it runs only
* for a module that declares or assigns an import's name somewhere, because
* elsewhere every read of the name is the import.
*
* The image packer is this transform's only caller: it lowers every JavaScript
* entry it packs and records `LOWERING_VERSION` in the image manifest, so the
* worker wraps those bodies without carrying a compiler of its own.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/compile/transform
*/
const HELPER_SOURCE = {
	def: "const __dsh$def=(t,k,get)=>Object.defineProperty(t,k,{enumerable:true,configurable:true,get});",
	default: "const __dsh$default=(m)=>(m&&m.__esModule&&Object.prototype.hasOwnProperty.call(m,\"default\")?m.default:m);",
	ns: "const __dsh$ns=(m)=>(m&&m.__esModule?m:Object.assign({},m,{default:m}));",
	exportAll: "const __dsh$exportAll=(t,m)=>{for(const k of Object.keys(m))if(k!==\"default\"&&!(k in t))__dsh$def(t,k,()=>m[k]);};",
	dynImport: "const __dsh$dynImport=(s)=>Promise.resolve().then(()=>__dsh$ns(require(s)));"
};
const HELPER_DEPENDENCIES = {
	exportAll: ["def"],
	dynImport: ["ns"]
};
/** Runtime identifier the suspension protocol reaches. */
const ALS = "__als";
/** @returns Number of line breaks in a slice. */
function countNewlines(text) {
	let count = 0;
	for (let index = text.indexOf("\n"); index >= 0; index = text.indexOf("\n", index + 1)) count += 1;
	return count;
}
/** Prefix of the identifiers this transform emits; a module declaring one would capture them. */
const RESERVED_PREFIX = "__dsh$";
var Transformer = class {
	path;
	edits = [];
	source;
	helpers = /* @__PURE__ */ new Set();
	bindings = [];
	modules = 0;
	temporaries = 0;
	moduleSyntax = false;
	moduleRequests = /* @__PURE__ */ new Set();
	metaResolveRequests = /* @__PURE__ */ new Set();
	createRequireBindings = /* @__PURE__ */ new Set();
	/** Module temporary per import declaration with specifiers, assigned before traversal. */
	importModules = /* @__PURE__ */ new Map();
	/** Named and default imports by local name. */
	importedBindings = /* @__PURE__ */ new Map();
	/** Read-position identifiers spelling an import's name, rewritten after traversal. */
	importCandidates = [];
	/**
	* Set when an import name (or the reserved prefix) is declared or assigned.
	* Only then can a candidate be a shadowing local, so only then does scope
	* analysis decide which candidates read the import.
	*/
	scopeAnalysisNeeded = false;
	constructor(source, path) {
		this.path = path;
		this.source = source.startsWith("#!") ? `//${source.slice(2)}` : source;
	}
	fail(detail, index) {
		const line = this.source.slice(0, index).split("\n").length;
		throw new Error(`webworker transform: ${detail} (${this.path}:${line})`);
	}
	helper(name) {
		for (const dependency of HELPER_DEPENDENCIES[name] ?? []) this.helper(dependency);
		this.helpers.add(name);
		return `__dsh$${name}`;
	}
	moduleTemp() {
		this.modules += 1;
		return `__dsh$m${this.modules}`;
	}
	alsTemp() {
		this.temporaries += 1;
		return `__als$${this.temporaries}`;
	}
	/**
	* Replace a range, keeping the module's line count.
	*
	* The padding is the newlines the original range held **minus** the ones the
	* replacement re-emits: a rewrite that splices the original body back in
	* (a desugared loop) already carries that body's newlines, and padding by the
	* whole range again would push every later line down.
	*/
	edit(start, end, build) {
		const original = countNewlines(this.source.slice(start, end));
		this.edits.push({
			start,
			end,
			render: (inner) => {
				const text = build(inner);
				return text + "\n".repeat(Math.max(0, original - countNewlines(text)));
			}
		});
	}
	replace(start, end, text) {
		this.edit(start, end, () => text);
	}
	insert(at, text) {
		this.edits.push({
			start: at,
			end: at,
			render: () => text
		});
	}
	structural(start, end, render) {
		this.edit(start, end, render);
	}
	literal(node) {
		const value = node.value;
		if (typeof value !== "string") this.fail("a module specifier must be a string literal", node.start);
		this.moduleRequests.add(value);
		return JSON.stringify(value);
	}
	/** @returns Static module requests the body makes, in first-appearance order. */
	requests() {
		return [...this.moduleRequests];
	}
	/** @returns Literal `import.meta.resolve()` requests, in first-appearance order. */
	metaRequests() {
		return [...this.metaResolveRequests];
	}
	importDeclaration(node) {
		this.moduleSyntax = true;
		if (Array.isArray(node.attributes) && node.attributes.length > 0) this.fail("import attributes are not supported", node.start);
		const source = node.source;
		const request = `require(${this.literal(source)})`;
		const held = this.importModules.get(node);
		if (held === void 0) {
			this.replace(node.start, node.end, `${request};`);
			return;
		}
		const lines = [`const ${held}=${request};`];
		for (const specifier of node.specifiers) if (specifier.type === "ImportNamespaceSpecifier") lines.push(`const ${nameOf(specifier.local)}=${this.helper("ns")}(${held});`);
		this.replace(node.start, node.end, lines.join(""));
	}
	/**
	* The expression that reads an import when the reference runs. The default
	* form is parenthesized so it stays one operand in any position, including
	* `new` callees and member objects.
	*/
	importAccess(binding) {
		return binding.kind === "default" ? `(${this.helper("default")}(${binding.held}))` : `${binding.held}[${JSON.stringify(binding.imported)}]`;
	}
	/** @returns How a local export reads its binding: imports through their module object. */
	bindingAccess(local) {
		const imported = this.importedBindings.get(local);
		return imported === void 0 ? local : this.importAccess(imported);
	}
	/**
	* Record an identifier that spells an import's name in a read position, and
	* flag one in a declaration or assignment position (or one using the
	* reserved prefix there), which makes scope analysis decide the candidates.
	* @param node - The identifier.
	* @param parent - Its parent node.
	* @param key - The parent property holding it.
	* @param statementStart - Start of the enclosing expression statement when that statement sits in a statement list.
	*/
	identifier(node, parent, key, statementStart) {
		if (parent === void 0 || this.importedBindings.size === 0) return;
		const name = node.name;
		const binding = this.importedBindings.get(name);
		if (binding === void 0 && !name.startsWith(RESERVED_PREFIX)) return;
		if (isBindingPosition(parent, key)) {
			this.scopeAnalysisNeeded = true;
			return;
		}
		if (binding !== void 0 && isReadPosition(parent, key)) this.importCandidates.push({
			node,
			parent,
			key,
			statementStart,
			binding
		});
	}
	/** Flag a declared or assigned name that could shadow an import or capture an emitted temporary. */
	bindingOccurrence(name) {
		if (this.importedBindings.size === 0) return;
		if (this.importedBindings.has(name) || name.startsWith(RESERVED_PREFIX)) this.scopeAnalysisNeeded = true;
	}
	/** Rewrite one identifier that reads an import. */
	importReference({ node, parent, key, statementStart, binding }) {
		const access = this.importAccess(binding);
		if (parent.type === "Property" && parent.shorthand === true) {
			this.replace(node.start, node.end, `${nameOf(node)}:${access}`);
			return;
		}
		const text = (parent.type === "CallExpression" && key === "callee" || parent.type === "TaggedTemplateExpression" && key === "tag") && binding.kind === "named" ? `(0,${access})` : access;
		this.replace(node.start, node.end, text.startsWith("(") && node.start === statementStart ? `;${text}` : text);
	}
	exportNamed(node) {
		this.moduleSyntax = true;
		const declaration = node.declaration;
		const source = node.source;
		const specifiers = node.specifiers;
		if (declaration !== null) {
			this.replace(node.start, declaration.start, "");
			for (const { exported, local } of declaredBindings(declaration, (detail) => this.fail(detail, declaration.start))) this.bindings.push({
				exported,
				local
			});
			return;
		}
		if (source !== null) {
			const held = this.moduleTemp();
			const define = this.helper("def");
			const lines = [`const ${held}=require(${this.literal(source)});`];
			for (const specifier of specifiers) {
				const local = nameOf(specifier.local);
				const exported = nameOf(specifier.exported);
				lines.push(`${define}(exports,${JSON.stringify(exported)},()=>${held}[${JSON.stringify(local)}]);`);
			}
			this.replace(node.start, node.end, lines.join(""));
			return;
		}
		for (const specifier of specifiers) this.bindings.push({
			exported: nameOf(specifier.exported),
			local: nameOf(specifier.local)
		});
		this.replace(node.start, node.end, "");
	}
	exportDefault(node) {
		this.moduleSyntax = true;
		const declaration = node.declaration;
		this.replace(node.start, declaration.start, "exports.default = ");
	}
	exportAll(node) {
		this.moduleSyntax = true;
		const request = `require(${this.literal(node.source)})`;
		const exported = node.exported;
		if (exported === null) {
			this.replace(node.start, node.end, `${this.helper("exportAll")}(exports,${request});`);
			return;
		}
		const held = this.moduleTemp();
		const define = this.helper("def");
		this.replace(node.start, node.end, `const ${held}=${this.helper("ns")}(${request});${define}(exports,${JSON.stringify(nameOf(exported))},()=>${held});`);
	}
	awaitExpression(node) {
		const keywordEnd = node.start + 5;
		if (this.source.slice(node.start, keywordEnd) !== "await") this.fail("unexpected await layout", node.start);
		this.replace(node.start, keywordEnd, `${ALS}.resume(await ${ALS}.pause(`);
		this.insert(node.end, "))");
	}
	/**
	* `for await (L of R) B` becomes an explicit loop over the same protocol.
	* `iterator.return` runs only on abrupt completion, as the language says, and
	* is awaited so teardown still orders before the loop exits.
	*/
	forAwait(node) {
		const left = node.left;
		const right = node.right;
		const body = node.body;
		const iterator = this.alsTemp();
		const step = this.alsTemp();
		const exhausted = this.alsTemp();
		const binding = (inner) => {
			if (left.type !== "VariableDeclaration") return `(${inner(left.start, left.end)})=${step}.value;`;
			const declarations = left.declarations;
			const pattern = declarations[0]?.id;
			if (declarations.length !== 1 || pattern === void 0) this.fail("for-await must declare exactly one binding", left.start);
			return `${String(left.kind)} ${inner(pattern.start, pattern.end)}=${step}.value;`;
		};
		this.structural(node.start, node.end, (inner) => [
			`{const ${iterator}=${ALS}.iterator(${inner(right.start, right.end)});`,
			`let ${step};let ${exhausted}=false;`,
			`try{for(;;){${step}=${ALS}.resume(await ${ALS}.pause(${iterator}.next()));`,
			`if(${step}.done){${exhausted}=true;break}`,
			`{${binding(inner)}${body.type === "BlockStatement" ? inner(body.start, body.end) : `{${inner(body.start, body.end)}}`}}}}`,
			`finally{if(!${exhausted})${ALS}.resume(await ${ALS}.pause(${ALS}.close(${iterator})))}}`
		].join(""));
	}
	/**
	* `yield` resumes with whatever the consumer sent, so the snapshot is taken
	* before suspending and restored when the call completes. `yield*` delegates,
	* which has no expression form here: it is desugared as a statement, and a
	* consumer's `throw()` is not forwarded into the inner iterator (`next` and
	* `return` are).
	*/
	yieldExpression(node, statement) {
		if (node.delegate !== true) {
			this.insert(node.start, `${ALS}.afterYield(${ALS}.snapshot(),`);
			this.insert(node.end, ")");
			return;
		}
		const argument = node.argument;
		if (argument === null) this.fail("yield* without an operand", node.start);
		if (statement === void 0) this.fail("yield* is only supported as a statement", node.start);
		if (statement.expression !== node) this.fail("yield* is only supported as the whole statement expression", node.start);
		const iterator = this.alsTemp();
		const step = this.alsTemp();
		const sent = this.alsTemp();
		const exhausted = this.alsTemp();
		this.structural(statement.start, statement.end, (inner) => [
			`{const ${iterator}=${ALS}.iterator(${inner(argument.start, argument.end)});`,
			`let ${sent};let ${exhausted}=false;`,
			`try{for(;;){const ${step}=${ALS}.resume(await ${ALS}.pause(${iterator}.next(${sent})));`,
			`if(${step}.done){${exhausted}=true;break}`,
			`${sent}=${ALS}.afterYield(${ALS}.snapshot(),yield ${step}.value)}}`,
			`finally{if(!${exhausted})${ALS}.resume(await ${ALS}.pause(${ALS}.close(${iterator})))}}`
		].join(""));
	}
	visit(node, context, parent, key) {
		if (node === null || typeof node !== "object") return;
		if (Array.isArray(node)) {
			for (const child of node) this.visit(child, context, parent, key);
			return;
		}
		const record = node;
		if (typeof record.type !== "string") return;
		let next = context;
		switch (record.type) {
			case "Identifier":
				this.identifier(record, parent, key, context.statementStart);
				break;
			case "ObjectPattern":
				for (const property of record.properties) {
					const value = property.value;
					if (value?.type === "Identifier") this.bindingOccurrence(value.name);
				}
				break;
			case "ImportDeclaration":
				this.importDeclaration(record);
				break;
			case "ExportNamedDeclaration":
				this.exportNamed(record);
				break;
			case "ExportDefaultDeclaration":
				this.exportDefault(record);
				break;
			case "ExportAllDeclaration":
				this.exportAll(record);
				break;
			case "ImportExpression": {
				this.moduleSyntax = true;
				if (!this.source.startsWith("import", record.start)) this.fail("unexpected dynamic import layout", record.start);
				this.replace(record.start, record.start + 6, this.helper("dynImport"));
				const argument = record.source;
				if (argument !== void 0 && typeof argument.value === "string") this.moduleRequests.add(argument.value);
				break;
			}
			case "CallExpression": {
				const callee = record.callee;
				const callArguments = record.arguments;
				if (callee.type === "Identifier" && callee.name === "eval" && record.optional !== true && this.importedBindings.size > 0) this.fail("direct eval cannot see imports once they are read through their module", record.start);
				if (this.isRequireCall(callee, context.moduleScope) && callArguments.length === 1 && typeof callArguments[0]?.value === "string") this.moduleRequests.add(callArguments[0].value);
				if (callee.type === "MemberExpression") {
					const object = callee.object;
					const property = callee.property;
					if (object.type === "MetaProperty" && object.meta.name === "import" && property.type === "Identifier" && property.name === "resolve" && typeof callArguments[0]?.value === "string") this.metaResolveRequests.add(callArguments[0].value);
				}
				break;
			}
			case "MetaProperty":
				if (record.meta.name === "import") {
					this.moduleSyntax = true;
					this.replace(record.start, record.end, "__dsh$meta");
				}
				break;
			case "AwaitExpression":
				if (context.functionDepth === 0) this.fail("top-level await cannot run as CommonJS in the worker", record.start);
				this.awaitExpression(record);
				break;
			case "ForOfStatement":
				if (record.await === true) {
					if (context.functionDepth === 0) this.fail("a top-level for-await loop cannot run as CommonJS", record.start);
					this.forAwait(record);
				}
				next = {
					...next,
					moduleScope: false
				};
				break;
			case "LabeledStatement": {
				const body = record.body;
				if (body.type === "ForOfStatement" && body.await === true) this.fail("a labeled for-await loop is not supported", record.start);
				break;
			}
			case "YieldExpression":
				if (context.asyncGenerator) this.yieldExpression(record, context.statement);
				break;
			case "FunctionDeclaration":
			case "FunctionExpression":
			case "ArrowFunctionExpression":
				next = {
					asyncGenerator: record.async === true && record.generator === true,
					functionDepth: context.functionDepth + 1,
					moduleScope: false
				};
				break;
			case "BlockStatement":
			case "CatchClause":
			case "ClassBody":
			case "ForStatement":
			case "ForInStatement":
			case "SwitchStatement": next = {
				...next,
				moduleScope: false
			};
		}
		if (record.type === "ExpressionStatement") next = {
			...next,
			statement: record,
			statementStart: inStatementList(parent, key) ? record.start : void 0
		};
		for (const [childKey, value] of Object.entries(record)) {
			if (childKey === "type" || childKey === "start" || childKey === "end") continue;
			this.visit(value, next, record, childKey);
		}
	}
	isCreateRequireCall(node) {
		if (node.type !== "CallExpression") return false;
		const callee = node.callee;
		const args = node.arguments;
		if (callee.type !== "Identifier" || !this.createRequireBindings.has(nameOf(callee)) || args.length !== 1) return false;
		const base = args[0];
		if (base.type !== "MemberExpression" || base.computed === true) return false;
		const object = base.object;
		const property = base.property;
		return object.type === "MetaProperty" && object.meta.name === "import" && property.type === "Identifier" && property.name === "url";
	}
	isRequireCall(callee, moduleScope) {
		return callee.type === "Identifier" && callee.name === "require" || moduleScope && this.isCreateRequireCall(callee);
	}
	indexCreateRequireImports(program) {
		for (const statement of program.body) {
			if (statement.type !== "ImportDeclaration") continue;
			const source = statement.source;
			if (source.value !== "node:module" && source.value !== "module") continue;
			for (const specifier of statement.specifiers) {
				if (specifier.type !== "ImportSpecifier" || specifierName(specifier.imported) !== "createRequire") continue;
				this.createRequireBindings.add(specifier.local.name);
			}
		}
	}
	/**
	* Give every import declaration its module temporary and record its named and
	* default bindings before traversal, because references may precede the
	* declaration in source order.
	*/
	indexImports(program) {
		for (const statement of program.body) {
			if (statement.type !== "ImportDeclaration" || statement.specifiers.length === 0) continue;
			const held = this.moduleTemp();
			this.importModules.set(statement, held);
			for (const specifier of statement.specifiers) if (specifier.type === "ImportDefaultSpecifier") this.importedBindings.set(specifier.local.name, {
				held,
				kind: "default",
				imported: "default"
			});
			else if (specifier.type === "ImportSpecifier") this.importedBindings.set(specifier.local.name, {
				held,
				kind: "named",
				imported: specifierName(specifier.imported)
			});
		}
	}
	/**
	* Decide with scope analysis which candidates read an import.
	*
	* Refuses an assignment to an import, which live reads cannot express, and a
	* declaration using the reserved prefix, which would capture an emitted
	* module temporary. Direct `eval` is refused during traversal.
	* @param program - The parsed module, which gains node ranges here.
	* @returns The identifier nodes that read a named or default import.
	*/
	resolveImportReads(program) {
		assignRanges(program);
		const manager = analyze(program, {
			ecmaVersion: 2022,
			sourceType: "module",
			childVisitorKeys: KEYS
		});
		for (const scope of manager.scopes) for (const variable of scope.variables) {
			const definition = variable.defs[0];
			if (definition === void 0 || !variable.name.startsWith(RESERVED_PREFIX)) continue;
			this.fail(`${variable.name} uses the transform's reserved ${RESERVED_PREFIX} prefix`, this.rangeStart(definition.name));
		}
		const reads = /* @__PURE__ */ new Set();
		const moduleScope = manager.scopes.find((scope) => scope.type === "module");
		for (const variable of moduleScope?.variables ?? []) {
			if (!this.importedBindings.has(variable.name) || variable.defs[0]?.type !== "ImportBinding") continue;
			for (const reference of variable.references) {
				if (reference.isWrite()) this.fail(`the import ${variable.name} cannot be assigned`, this.rangeStart(reference.identifier));
				reads.add(reference.identifier);
			}
		}
		return reads;
	}
	/**
	* @param node - A node scope analysis reports.
	* @returns Its source offset; `assignRanges` gave every node a range.
	*/
	rangeStart(node) {
		const start = node.range?.[0];
		if (start === void 0) this.fail("scope analysis reported a node without a source range", 0);
		return start;
	}
	run() {
		if (this.source.includes(`${ALS}.pause(`) || this.source.includes("__als$")) this.fail("the module is already lowered; check the image manifest wiring", 0);
		let program;
		try {
			program = parse(this.source, {
				ecmaVersion: "latest",
				sourceType: "module",
				allowAwaitOutsideFunction: true
			});
		} catch (reason) {
			this.fail(`parse failed: ${reason.message}`, 0);
		}
		this.indexCreateRequireImports(program);
		this.indexImports(program);
		this.visit(program, {
			asyncGenerator: false,
			functionDepth: 0,
			moduleScope: true
		});
		const reads = this.scopeAnalysisNeeded ? this.resolveImportReads(program) : void 0;
		for (const candidate of this.importCandidates) if (reads === void 0 || reads.has(candidate.node)) this.importReference(candidate);
		if (this.edits.length === 0 && !this.moduleSyntax) return this.source;
		const prologue = [];
		if (this.moduleSyntax) prologue.push("\"use strict\";Object.defineProperty(exports,\"__esModule\",{value:true});");
		const getters = this.bindings.map(({ exported, local }) => `__dsh$def(exports,${JSON.stringify(exported)},()=>${this.bindingAccess(local)});`);
		if (this.bindings.length > 0) this.helper("def");
		for (const [name, source] of Object.entries(HELPER_SOURCE)) if (this.helpers.has(name)) prologue.push(source);
		prologue.push(...getters);
		const sorted = [...this.edits].sort((left, right) => left.start - right.start || left.end - right.end);
		const render = (from, to) => {
			let cursor = from;
			let out = "";
			for (const edit of sorted) {
				if (edit.start < cursor || edit.end > to) continue;
				out += this.source.slice(cursor, edit.start) + edit.render(render);
				cursor = edit.end;
			}
			return out + this.source.slice(cursor, to);
		};
		const code = prologue.join("") + render(0, this.source.length);
		try {
			parse(code, {
				ecmaVersion: "latest",
				sourceType: "script",
				allowAwaitOutsideFunction: false
			});
		} catch (reason) {
			this.fail(`the transform produced code that does not parse: ${reason.message}`, 0);
		}
		return code;
	}
};
/** @returns The name a specifier or identifier node carries. */
function nameOf(node) {
	return node.type === "Identifier" ? node.name : String(node.value);
}
/** @returns The name an import or export specifier spells, as an identifier or a string literal. */
function specifierName(node) {
	return node.type === "Identifier" ? node.name : String(node.value);
}
/**
* Give every node the `range` that eslint-scope reads. The parse leaves ranges
* out because only modules that need scope analysis use them.
* @param node - A node or node array of the parsed module.
*/
function assignRanges(node) {
	if (node === null || typeof node !== "object") return;
	if (Array.isArray(node)) {
		for (const child of node) assignRanges(child);
		return;
	}
	const record = node;
	if (typeof record.type !== "string") return;
	record.range = [record.start, record.end];
	for (const value of Object.values(record)) assignRanges(value);
}
/**
* ESTree positions where an identifier names a declared or assigned binding.
* Property values inside an object pattern are checked at the pattern.
*
* Together with the ObjectPattern check, this must recognize every ESTree
* declaration and assignment form: a missed form lets a shadowing local skip
* scope analysis and be rewritten as an import read. A parser upgrade that adds
* a binding form must extend this list and the shadowing cases in
* `transform-imports.spec.ts`.
* @param parent - Parent of the identifier.
* @param key - The parent property holding it.
* @returns Whether the identifier declares or assigns a binding.
*/
function isBindingPosition(parent, key) {
	switch (parent.type) {
		case "VariableDeclarator": return key === "id";
		case "FunctionDeclaration":
		case "FunctionExpression":
		case "ArrowFunctionExpression": return key === "id" || key === "params";
		case "ClassDeclaration":
		case "ClassExpression": return key === "id";
		case "CatchClause": return key === "param";
		case "ArrayPattern":
		case "RestElement":
		case "UpdateExpression": return true;
		case "AssignmentPattern":
		case "AssignmentExpression":
		case "ForInStatement":
		case "ForOfStatement": return key === "left";
		default: return false;
	}
}
/**
* ESTree positions where an identifier, outside a binding position, reads a
* binding rather than naming a property, label, specifier, or meta property.
* Positions not listed read a binding, so the binding positions must be ruled
* out first (`isBindingPosition`).
* @param parent - Parent of the identifier.
* @param key - The parent property holding it.
* @returns Whether the identifier reads a binding.
*/
function isReadPosition(parent, key) {
	switch (parent.type) {
		case "MemberExpression": return key !== "property" || parent.computed === true;
		case "Property":
		case "MethodDefinition":
		case "PropertyDefinition": return key !== "key" || parent.computed === true;
		case "LabeledStatement":
		case "BreakStatement":
		case "ContinueStatement":
		case "ImportSpecifier":
		case "ImportDefaultSpecifier":
		case "ImportNamespaceSpecifier":
		case "ExportSpecifier":
		case "ExportAllDeclaration":
		case "MetaProperty": return false;
		default: return true;
	}
}
/**
* @param parent - Parent of a statement.
* @param key - The parent property holding it.
* @returns Whether the statement sits in a statement list, where it can follow
* a statement that ends without a semicolon.
*/
function inStatementList(parent, key) {
	if (parent === void 0) return false;
	if (parent.type === "SwitchCase") return key === "consequent";
	return key === "body" && (parent.type === "Program" || parent.type === "BlockStatement" || parent.type === "StaticBlock");
}
/** Every binding an exported declaration introduces, including patterns. */
function declaredBindings(declaration, fail) {
	if (declaration.type === "FunctionDeclaration" || declaration.type === "ClassDeclaration") {
		const id = declaration.id;
		if (id === null) fail("an exported declaration must be named");
		const name = id.name;
		return [{
			exported: name,
			local: name
		}];
	}
	if (declaration.type !== "VariableDeclaration") fail(`unsupported exported declaration ${declaration.type}`);
	const bindings = [];
	const collect = (pattern) => {
		switch (pattern.type) {
			case "Identifier":
				bindings.push({
					exported: pattern.name,
					local: pattern.name
				});
				return;
			case "ObjectPattern":
				for (const property of pattern.properties) collect(property.type === "RestElement" ? property.argument : property.value);
				return;
			case "ArrayPattern":
				for (const element of pattern.elements) if (element !== null) collect(element);
				return;
			case "AssignmentPattern":
				collect(pattern.left);
				return;
			case "RestElement":
				collect(pattern.argument);
				return;
			default: fail(`unsupported binding pattern ${pattern.type}`);
		}
	};
	for (const declarator of declaration.declarations) collect(declarator.id);
	return bindings;
}
const cache = /* @__PURE__ */ new Map();
/**
* Transform one module into a body for the worker wrapper.
*
* Results are cached by source text, so a module reached through two paths, or
* a repeated build, parses once.
* @param source - Module source, ESM or CommonJS.
* @param path - Path used in diagnostics.
* @returns The lowered body and the module requests found in it.
*/
function transformDetailed(source, path) {
	const cached = cache.get(source);
	if (cached !== void 0) return cached;
	const transformer = new Transformer(source, path);
	const transformed = {
		code: transformer.run(),
		moduleRequests: transformer.requests(),
		metaResolveRequests: transformer.metaRequests()
	};
	cache.set(source, transformed);
	return transformed;
}
/**
* Lower one module at image-pack time.
*
* The collector calls this for every JavaScript entry it packs and records
* `LOWERING_VERSION` in the image manifest; the loader then wraps those entries
* without parsing them. `lowered: false` reports that the transform would have
* returned the input verbatim (already CommonJS, no suspension point), so the
* entry may be packed as it is.
*
* Throwing is the intended failure mode: a module this transform cannot express
* must fail the build rather than ship an image that breaks at load.
* @param options - Virtual path inside the image and the module source.
* @returns The code to pack and whether it changed.
*/
function lowerModuleSource(options) {
	const { code, moduleRequests, metaResolveRequests } = transformDetailed(options.source, options.filename);
	return {
		code,
		lowered: code !== options.source,
		moduleRequests,
		metaResolveRequests
	};
}
// src/transport/tunnel.ts
/**
* Worker end of the postMessage tunnel. It owns the dispatch lanes and the queue
* that holds requests until the host tree is serving:
*
* - `GET /__boot__` answers from tunnel glue, never from the host API surface,
*   because the page needs the boot payload before its Cordis tree exists.
*   `GET /__assets__` answers the same way with every package asset of the
*   image, which the page maps before any Client bundle runs.
* - Privileged `/api` methods take that same direct entry. The method set is not
*   restated here: a 401 or 403 from the route lane is retried on the direct
*   lane because the page owns the worker and needs no network authentication.
* - Everything else is fed into the real webserver route table through the
*   request listener the app's fake `node:http` captured, keeping the trust
*   fences, byte limits, and status semantics intact.
*
* A boot failure rejects the whole queue with 503 rather than leaving the page
* waiting.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/transport/tunnel
*/
/** Prefix owning the API methods. */
const API_PREFIX = "/api";
/** Host header the synthesized requests carry; the API trust fence requires one. */
const SYNTHETIC_HOST = "127.0.0.1";
const encoder$2 = new TextEncoder();
/**
* Render a failure with everything nested inside it.
*
* A boot failure is usually an `AggregateError` of per-entry failures, each
* wrapping the plugin's own error as `cause`; only the outermost message names
* "loader entries failed to apply", which says nothing about which row broke.
*
* The page logs the rendered text verbatim for refusals; keep it stable for
* anyone matching boot-failure output.
* @param reason - Thrown value.
* @returns One line per nested failure, indented by depth.
*/
function describeFailure(reason) {
	const seen = /* @__PURE__ */ new Set();
	const lines = [];
	const walk = (value, depth) => {
		if (value === null || value === void 0 || seen.has(value) || depth > 6) return;
		seen.add(value);
		const indent = "  ".repeat(depth);
		if (!(value instanceof Error)) {
			const rendered = typeof value === "string" ? value : JSON.stringify(value);
			lines.push(`${indent}${rendered ?? typeof value}`);
			return;
		}
		lines.push(`${indent}${value.name}: ${value.message}`);
		if (value instanceof AggregateError) for (const inner of value.errors) walk(inner, depth + 1);
		walk(value.cause, depth + 1);
	};
	walk(reason, 0);
	return lines.join("\n");
}
/**
* Copy bytes into an exact-size ArrayBuffer so it can be transferred.
*
* Sliced on the ArrayBuffer, not the view: `Uint8Array.prototype.slice` copies,
* but a Node-style Buffer overrides `slice()` with view semantics, and the fs
* bridge hands VFS reads over as Buffer views into the whole mounted image —
* `bytes.slice().buffer` would then post the entire image as the body.
*/
function toTransferable(bytes) {
	return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}
/** Recorded response frames, so a route-lane authentication refusal can be discarded. */
var BufferedSink = class {
	calls = [];
	target;
	settle;
	/** Resolves when the exchange either starts streaming or answers in one frame. */
	settled = new Promise((resolve) => {
		this.settle = resolve;
	});
	sink = {
		head: (status, headers) => {
			this.record(() => {
				this.target?.head(status, headers);
			});
			this.settle?.({
				streamed: true,
				status
			});
		},
		chunk: (bytes) => {
			this.record(() => {
				this.target?.chunk(bytes);
			});
		},
		end: (payload) => {
			this.record(() => {
				this.target?.end(payload);
			});
			this.settle?.({
				streamed: payload === void 0,
				status: payload?.status ?? 200
			});
		},
		fail: (message) => {
			this.record(() => {
				this.target?.fail(message);
			});
			this.settle?.({
				streamed: true,
				status: 500
			});
		}
	};
	record(call) {
		if (this.target === void 0) this.calls.push(call);
		else call();
	}
	/**
	* Send everything recorded so far to a real sink and pass later calls through.
	* @param target - Sink receiving the frames.
	*/
	flushTo(target) {
		this.target = target;
		for (const call of this.calls.splice(0)) call();
	}
};
/** Uplink items of one worker-local logical stream, read once by the Host method as its `uplink`. */
var TunnelUplink = class {
	items = [];
	ended = false;
	closed = false;
	wake;
	push(value) {
		if (this.ended || this.closed) return;
		this.items.push(value);
		this.signal();
	}
	/** Page half-close or stream end; buffered items still drain. */
	end() {
		this.ended = true;
		this.signal();
	}
	[Symbol.asyncIterator]() {
		return this;
	}
	async next() {
		while (true) {
			if (this.closed) return {
				value: void 0,
				done: true
			};
			if (this.items.length > 0) return {
				value: this.items.shift(),
				done: false
			};
			if (this.ended) return {
				value: void 0,
				done: true
			};
			if (this.wake !== void 0) throw new Error("webworker tunnel: stream uplink has one pending read");
			await new Promise((resolve) => {
				this.wake = resolve;
			});
		}
	}
	/** The Host stopped reading: buffered and later items are dropped. */
	return() {
		this.closed = true;
		this.items.length = 0;
		this.signal();
		return Promise.resolve({
			value: void 0,
			done: true
		});
	}
	signal() {
		const wake = this.wake;
		this.wake = void 0;
		wake?.();
	}
};
/** One tunnel per worker; wire {@link TunnelServer.handleMessage} to `onmessage` first. */
var TunnelServer = class {
	port;
	requestListener;
	privilegedMethods;
	unaryApiLane;
	queue = [];
	inFlight = /* @__PURE__ */ new Map();
	/** Uplinks of accepted `stream-open` frames, buffering items that arrive before or while the stream serves. */
	uplinks = /* @__PURE__ */ new Map();
	seams;
	failure;
	listener;
	constructor(options) {
		this.port = options.port;
		this.requestListener = options.requestListener;
		this.privilegedMethods = options.privilegedMethods;
		this.unaryApiLane = options.unaryApiLane ?? "route";
	}
	/**
	* Accept one `postMessage` payload.
	* @param data - Message data from the page.
	*/
	handleMessage(data) {
		const frame = parseInboundFrame(data);
		if (frame.t === "init") throw new Error("webworker tunnel: duplicate init frame; the tunnel is already open");
		if (frame.t === "stream-uplink-item") {
			this.uplinks.get(frame.id)?.push(frame.value);
			return;
		}
		if (frame.t === "stream-uplink-end") {
			this.uplinks.get(frame.id)?.end();
			return;
		}
		if (frame.t === "abort") {
			this.inFlight.get(frame.id)?.abort();
			this.inFlight.delete(frame.id);
			this.dropUplink(frame.id);
			const queued = this.queue.findIndex((request) => request.id === frame.id);
			if (queued !== -1) this.queue.splice(queued, 1);
			return;
		}
		if (this.failure !== void 0) {
			this.refuse(frame, this.failure);
			return;
		}
		if (frame.t === "stream-open") this.uplinks.set(frame.id, new TunnelUplink());
		if (this.seams === void 0) {
			this.queue.push(frame);
			return;
		}
		this.dispatchFrame(frame);
	}
	/**
	* Start serving: drains everything queued during boot.
	* @param seams - Faces that exist only after the host tree is up.
	*/
	serve(seams) {
		this.seams = seams;
		console.info(`webworker tunnel: serving (unary /api lane=${this.unaryApiLane}${this.unaryApiLane === "route" ? " with 401/403 retry" : ""}, privileged set=${this.privilegedMethods === void 0 ? "none" : String(this.privilegedMethods.size)}, queued=${String(this.queue.length)})`);
		for (const frame of this.queue.splice(0)) this.dispatchFrame(frame);
	}
	/**
	* Refuse every queued and future request; the page renders this like a server
	* that failed to start.
	* @param reason - Boot failure to report.
	*/
	fail(reason) {
		const message = describeFailure(reason);
		this.failure = message;
		for (const frame of this.queue.splice(0)) this.refuse(frame, message);
	}
	/**
	* Ask the page to show one text file in its read-only viewer.
	* @param path - Absolute VFS path the viewer names.
	* @param text - File contents.
	*/
	viewText(path, text) {
		this.send({
			t: "view-text",
			path,
			text
		});
	}
	send(frame, transfer) {
		this.port.postMessage(frame, transfer);
	}
	refuse(frame, message) {
		if (frame.t === "stream-open") {
			this.dropUplink(frame.id);
			this.send({
				t: "stream-error",
				id: frame.id,
				failure: {
					kind: "carrier",
					message
				}
			});
			return;
		}
		const body = toTransferable(encoder$2.encode(message));
		this.send({
			t: "res",
			id: frame.id,
			status: 503,
			headers: { "content-type": "text/plain; charset=utf-8" },
			body,
			message
		}, [body]);
	}
	dispatchFrame(frame) {
		if (frame.t === "stream-open") this.serveStream(frame);
		else this.serveRequest(frame);
	}
	async serveStream(frame) {
		if (this.seams === void 0) {
			this.refuse(frame, "webworker tunnel: Remote stream requested before the host tree is serving");
			return;
		}
		const seams = this.seams;
		const uplink = this.uplinks.get(frame.id) ?? new TunnelUplink();
		const controller = new AbortController();
		this.inFlight.set(frame.id, { abort: () => {
			controller.abort();
		} });
		try {
			const source = await seams.openStream(frame.endpoint, frame.payload, uplink, controller.signal);
			for await (const value of source) {
				if (controller.signal.aborted) return;
				this.send({
					t: "stream-item",
					id: frame.id,
					value
				});
			}
			if (!controller.signal.aborted) this.send({
				t: "stream-end",
				id: frame.id
			});
		} catch (error) {
			if (!controller.signal.aborted) {
				const failure = seams.streamFailure(error);
				this.send({
					t: "stream-error",
					id: frame.id,
					failure: {
						kind: "remote",
						...failure
					}
				});
			}
		} finally {
			this.inFlight.delete(frame.id);
			this.dropUplink(frame.id);
		}
	}
	/** The stream is over: settle any Host read still waiting on its uplink and stop buffering. */
	dropUplink(id) {
		const uplink = this.uplinks.get(id);
		this.uplinks.delete(id);
		uplink?.end();
	}
	sinkFor(id) {
		const send = this.send.bind(this);
		const inFlight = this.inFlight;
		return {
			head(status, headers) {
				send({
					t: "res-head",
					id,
					status,
					headers
				});
			},
			chunk(bytes) {
				const buffer = toTransferable(bytes);
				send({
					t: "res-chunk",
					id,
					chunk: buffer
				}, [buffer]);
			},
			end(payload) {
				if (payload === void 0) send({
					t: "res-end",
					id
				});
				else {
					const body = payload.body === void 0 ? void 0 : toTransferable(payload.body);
					send({
						t: "res",
						id,
						status: payload.status,
						headers: payload.headers,
						body
					}, body === void 0 ? void 0 : [body]);
				}
				inFlight.delete(id);
			},
			fail(message) {
				send({
					t: "res-err",
					id,
					message
				});
				inFlight.delete(id);
			}
		};
	}
	/** The page sends an absolute URL; route handlers read `req.url` as a path. */
	pathFrame(frame) {
		const url = new URL(frame.url, `http://${SYNTHETIC_HOST}`);
		return {
			frame: {
				...frame,
				url: `${url.pathname}${url.search}`,
				headers: {
					...frame.headers,
					host: SYNTHETIC_HOST
				}
			},
			path: url.pathname
		};
	}
	async serveRequest(frame) {
		const sink = this.sinkFor(frame.id);
		try {
			const { frame: routed, path } = this.pathFrame(frame);
			if (path === "/__boot__") {
				this.serveBoot(frame, sink);
				return;
			}
			if (path === "/__assets__") {
				this.servePackageAssets(frame, sink);
				return;
			}
			if (path.startsWith(`/api/`)) {
				await this.serveApi(frame, routed, path, sink);
				return;
			}
			this.dispatch(routed, sink);
		} catch (reason) {
			sink.fail(reason instanceof Error ? reason.message : String(reason));
		}
	}
	/**
	* The listener is captured once and reused, so only requests that arrive
	* before the web server binds pay an await.
	* @returns The webserver request listener.
	*/
	async whenListener() {
		this.listener ??= await this.requestListener();
		return this.listener;
	}
	/** Feed the real route table through the captured listener. */
	dispatch(frame, sink, into) {
		const exchange = createSyntheticExchange(frame, into ?? sink);
		this.inFlight.set(frame.id, exchange);
		const listener = this.listener;
		if (listener !== void 0) {
			listener(exchange.req, exchange.res);
			return exchange;
		}
		this.whenListener().then((resolved) => {
			if (!exchange.aborted) resolved(exchange.req, exchange.res);
		}, (reason) => {
			sink.fail(reason instanceof Error ? reason.message : String(reason));
		});
		return exchange;
	}
	/**
	* Unary `/api`: keep the route lane's fences, but fall back to the direct lane
	* when network authentication or trust rejects the worker-owning page.
	*/
	async serveApi(original, routed, path, sink) {
		const method = path.slice(5);
		if (this.unaryApiLane === "direct" || this.privilegedMethods?.has(method) === true) {
			await this.serveDirect(original, sink);
			return;
		}
		const buffered = new BufferedSink();
		const exchange = this.dispatch(routed, sink, buffered.sink);
		let settleAborted = () => {};
		const aborted = new Promise((resolve) => {
			settleAborted = () => {
				resolve("aborted");
			};
		});
		this.inFlight.set(routed.id, { abort: () => {
			exchange.abort();
			settleAborted();
		} });
		const outcome = await Promise.race([buffered.settled, aborted]);
		if (outcome === "aborted" || exchange.aborted) return;
		if (outcome.status === 401 || outcome.status === 403) {
			console.debug(`webworker tunnel: route lane refused ${method} with ${String(outcome.status)}; answering on the direct lane`);
			await this.serveDirect(original, sink);
			return;
		}
		buffered.flushTo(sink);
	}
	servePackageAssets(frame, sink) {
		if (this.seams === void 0) throw new Error("webworker tunnel: package assets requested before the host tree is serving");
		if (frame.method !== "GET") {
			sink.end({
				status: 405,
				headers: { allow: "GET" }
			});
			return;
		}
		sink.end({
			status: 200,
			headers: {
				"content-type": "application/x-tar",
				"cache-control": "no-store"
			},
			body: this.seams.packageAssets()
		});
	}
	serveBoot(frame, sink) {
		if (this.seams === void 0) throw new Error("webworker tunnel: boot payload requested before the host tree is serving");
		if (frame.method !== "GET") {
			sink.end({
				status: 405,
				headers: { allow: "GET" }
			});
			return;
		}
		const body = encoder$2.encode(JSON.stringify(this.seams.bootPayload()));
		sink.end({
			status: 200,
			headers: {
				"content-type": "application/json; charset=utf-8",
				"cache-control": "no-store"
			},
			body
		});
	}
	async serveDirect(frame, sink) {
		if (this.seams === void 0) throw new Error("webworker tunnel: direct fetch requested before the host tree is serving");
		const controller = new AbortController();
		this.inFlight.set(frame.id, { abort: () => {
			controller.abort();
		} });
		const headers = new Headers();
		for (const [key, value] of Object.entries(frame.headers)) try {
			headers.set(key, value);
		} catch {}
		const request = new Request(new URL(frame.url, `http://${SYNTHETIC_HOST}`), {
			method: frame.method,
			headers,
			body: frame.body === void 0 || frame.method === "GET" || frame.method === "HEAD" ? null : frame.body,
			signal: controller.signal
		});
		const response = await this.seams.directFetch(request);
		const responseHeaders = {};
		response.headers.forEach((value, key) => {
			responseHeaders[key] = value;
		});
		if (!(response.body !== null && (responseHeaders["content-type"] ?? "").startsWith("text/event-stream"))) {
			const buffer = await response.arrayBuffer();
			sink.end({
				status: response.status,
				headers: responseHeaders,
				body: buffer.byteLength === 0 ? void 0 : new Uint8Array(buffer)
			});
			return;
		}
		sink.head(response.status, responseHeaders);
		const reader = response.body.getReader();
		this.inFlight.set(frame.id, { abort: () => {
			controller.abort();
			reader.cancel().catch(() => {});
		} });
		try {
			for (;;) {
				const { done, value } = await reader.read();
				if (done) break;
				sink.chunk(value);
			}
			sink.end();
		} catch (reason) {
			sink.fail(reason instanceof Error ? reason.message : String(reason));
		} finally {
			reader.releaseLock();
		}
	}
};
// src/node/process-table.ts
const entries = /* @__PURE__ */ new Map();
/**
* Whether a command with this pid is still running.
* @param pid - pid to look up; a negative value addresses the group, which here
* holds exactly the one process that leads it.
* @returns true while the entry is in the table.
*/
function processAlive(pid) {
	return entries.has(Math.abs(pid));
}
/**
* Deliver a signal to one running command.
*
* `SIGKILL` stops the command whatever it is doing; every other signal asks it
* to stop at its next command boundary. That distinction is real only for a
* worker-backed process — see {@link RunningProcess.destroy}.
* @param pid - pid or negative process-group id.
* @param signal - the signal name to record and deliver.
* @returns true when an entry received it, false when no such process exists.
*/
function signalProcess(pid, signal) {
	const entry = entries.get(Math.abs(pid));
	if (entry === void 0) return false;
	entry.signal ??= signal;
	if (signal === "SIGKILL") entry.process?.destroy();
	else entry.process?.interrupt();
	return true;
}
// src/node/globals/process.ts
/**
* The `process` global the worker needs before any VFS module runs. Cordis
* reads `process.env` and `process.versions.node` while the Loader is
* constructed, and bundle patch rows keep their `!!js process.*` expressions, so
* the configuration bytes stay identical to the Node deployment. Third-party Node
* packages use the presence of `process.title` to avoid browser-only globals.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/node/globals/process
*/
/** Virtual executable identity; the worker has no Node binary behind it. */
const EXEC_PATH = "/dsh/bin/node";
/** Node release whose internal-module members `node-addon-require-builtin` follows in the Worker. */
const NODE_VERSION = "24.18.0";
/**
* Publish `globalThis.process` and Node's `global` alias for the worker lifetime.
*
* `versions.node` names the Node release the Worker's `node-addon-require-builtin` internals follow, so Cordis's
* `ModuleLoader.fromInternal()` reaches them the way it reaches Node's; `execArgv` stays empty, without
* `--expose-internals`.
* @param options - Root, environment, and argument vector.
* @returns The published object, for the module proxy table.
*/
function installProcessGlobal(options) {
	const start = performance.now();
	const write = (target) => (chunk) => {
		console[target](chunk.replace(/\n$/, ""));
		return true;
	};
	const shim = {
		env: { ...options.env },
		argv: [...options.argv ?? [EXEC_PATH]],
		execArgv: [],
		execPath: EXEC_PATH,
		title: "dsh-webworker",
		platform: "linux",
		arch: "x64",
		pid: 1,
		version: `v${NODE_VERSION}`,
		versions: { node: NODE_VERSION },
		cwd: () => options.cwd,
		getBuiltinModule: (id) => {
			let resolution;
			try {
				resolution = requireActiveModuleLoader().resolve(id, "/");
			} catch {
				return;
			}
			return resolution.kind === "static" ? resolution.factory() : void 0;
		},
		kill: (pid, signal = "SIGTERM") => {
			if (signal === 0) {
				if (processAlive(pid)) return true;
				const error = /* @__PURE__ */ new Error("kill ESRCH");
				error.code = "ESRCH";
				error.syscall = "kill";
				throw error;
			}
			return signalProcess(pid, signal);
		},
		nextTick: (callback, ...args) => {
			queueMicrotask(() => {
				callback(...args);
			});
		},
		stdout: { write: write("log") },
		stderr: { write: write("error") },
		on: () => shim,
		off: () => shim,
		once: () => shim,
		prependListener: () => shim,
		prependOnceListener: () => shim,
		removeListener: () => shim,
		removeAllListeners: () => shim,
		listeners: () => [],
		listenerCount: () => 0,
		setMaxListeners: () => shim,
		emit: () => false,
		hrtime: { bigint: () => BigInt(Math.round((performance.now() - start) * 1e6)) },
		uptime: () => (performance.now() - start) / 1e3,
		exit: (code) => {
			console.warn(`webworker process: exit(${String(code ?? 0)}) requested; the worker keeps running`);
		}
	};
	globalThis.global = globalThis;
	globalThis.process = shim;
	return shim;
}
// src/storage/image-gzip.ts
/**
* The image byte envelope. The packer writes one gzip member holding the ustar
* archive, and the worker inflates it with the platform's own decompressor before
* the tar reader sees a byte — `storage/tar.ts` stays a pure ustar reader with no
* codec in it.
*
* Inflation runs on the fetch stream rather than on downloaded bytes: the
* decompressor consumes each chunk as it lands, so unpacking overlaps the
* download instead of following it, and the compressed copy never has to be held
* whole in memory beside the archive it produces.
*
* One format, no negotiation: a body that does not start a gzip member is refused
* by name, in the stream, before the decompressor sees it. Without that check a
* plain tar, a truncated download, or a proxy's HTML error page would reach
* `parseTar` and fail as a corrupt header field, which says nothing about what
* the deployment actually served.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/storage/image-gzip
*/
/** gzip member identification bytes (RFC 1952 §2.3.1). */
const GZIP_MAGIC = [31, 139];
/** Bytes of a refused body quoted in the failure, enough to recognize text served in its place. */
const QUOTED_BYTES = 8;
const hex = (bytes) => [...bytes.slice(0, QUOTED_BYTES)].map((byte) => byte.toString(16).padStart(2, "0")).join(" ");
/**
* A pass-through that refuses a body which is not a gzip member.
*
* The check spans chunks: a transport may deliver the first byte alone, so the
* head is held until it can be judged and then forwarded intact. A body that ends
* before two bytes arrive is refused in `flush`, where "too short" is the only
* thing left to report.
* @param source - the image URL, or how the bytes arrived; named in a refusal.
* @returns The transform to pipe the body through before the decompressor.
*/
function requireGzipMember(source) {
	let head = /* @__PURE__ */ new Uint8Array(0);
	let judged = false;
	const refuse = (read) => /* @__PURE__ */ new Error(`webworker image: ${source} is not the gzip-compressed tar this deployment serves as its image (expected a member starting 1f 8b, read ${read.byteLength === 0 ? "an empty body" : hex(read)}); a host that answered with a Content-Encoding the transport already decoded, or a build that wrote the archive uncompressed, arrives exactly this way`);
	return new TransformStream({
		transform: (chunk, controller) => {
			if (judged) {
				controller.enqueue(chunk);
				return;
			}
			const merged = new Uint8Array(head.byteLength + chunk.byteLength);
			merged.set(head);
			merged.set(chunk, head.byteLength);
			head = merged;
			if (head.byteLength < GZIP_MAGIC.length) return;
			if (GZIP_MAGIC.some((byte, at) => head[at] !== byte)) throw refuse(head);
			judged = true;
			controller.enqueue(head);
		},
		flush: () => {
			if (!judged) throw refuse(head);
		}
	});
}
/**
* Inflate a packed VFS image as it arrives.
* @param body - the image body, straight from `fetch` or wrapped around bytes.
* @param source - the image URL, or how the bytes arrived; named in a refusal.
* @returns the ustar archive the image carries.
* @throws When the body does not start a gzip member, or the member is corrupt.
*/
async function inflateImageStream(body, source) {
	const inflated = body.pipeThrough(requireGzipMember(source)).pipeThrough(new DecompressionStream("gzip"));
	return new Uint8Array(await new Response(inflated).arrayBuffer());
}
/**
* Inflate a packed VFS image held in memory.
*
* The bytes become a body so both entries run the same stream: one decompression
* path, one refusal, whether the image came off the network or out of a caller's
* buffer.
* @param bytes - the image bytes.
* @param source - how the bytes arrived; named in a refusal.
* @returns the ustar archive the image carries.
* @throws When the bytes do not start a gzip member, or the member is corrupt.
*/
async function inflateImage(bytes, source) {
	const body = new Response(bytes).body;
	if (body === null) throw new Error(`webworker image: ${source} produced no readable body`);
	return await inflateImageStream(body, source);
}
// src/storage/tar.ts
/**
* Uncompressed ustar archive: the VFS image format. One fetch delivers the
* whole tree, and the reader hands out subarray views into the fetched buffer,
* so mounting copies nothing and no inflate step runs inside the worker.
*
* Hand-rolled on purpose: both sides need synchronous in-memory operation and
* the reader ships inside the worker bundle, where the streaming tar packages
* would drag Node stream shims back in. The subset is plain ustar — regular
* files and directories, names up to 255 bytes via the name-prefix split — and
* anything outside it fails loud on either side.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/storage/tar
*/
const encoder$1 = new TextEncoder();
const decoder$1 = new TextDecoder();
const BLOCK = 512;
/** Write an octal field: zero-padded digits with a terminating NUL. */
function writeOctal(header, offset, length, value) {
	header.set(encoder$1.encode(value.toString(8).padStart(length - 1, "0")), offset);
}
/**
* Split an entry name into the ustar name and prefix fields.
* @param name - Full entry name.
* @returns The two fields; the prefix is empty when the name fits directly.
* @throws When no slash yields name ≤ 100 and prefix ≤ 155 bytes: the entry
* cannot be archived and a silently truncated name would corrupt the image.
*/
function splitName(name) {
	if (encoder$1.encode(name).length <= 100) return {
		name,
		prefix: ""
	};
	for (let index = name.length - 1; index > 0; index -= 1) {
		if (name[index] !== "/") continue;
		const prefix = name.slice(0, index);
		const rest = name.slice(index + 1);
		if (encoder$1.encode(rest).length <= 100 && encoder$1.encode(prefix).length <= 155) return {
			name: rest,
			prefix
		};
	}
	throw new Error(`vfs tar: entry name does not fit the ustar name+prefix split: ${name}`);
}
/**
* Pack entries into one uncompressed ustar archive.
*
* Entries keep their given order; names ending in a slash become directory
* entries. Contents are written verbatim — compression belongs to the HTTP
* transport, not to the archive.
* @param files - Entry name to content bytes.
* @returns The archive bytes.
*/
function packTar(files) {
	const chunks = [];
	for (const [entryName, bytes] of Object.entries(files)) {
		const directory = entryName.endsWith("/");
		const size = directory ? 0 : bytes.length;
		const { name, prefix } = splitName(entryName);
		const header = new Uint8Array(BLOCK);
		header.set(encoder$1.encode(name), 0);
		writeOctal(header, 100, 8, directory ? 493 : 420);
		writeOctal(header, 108, 8, 0);
		writeOctal(header, 116, 8, 0);
		writeOctal(header, 124, 12, size);
		writeOctal(header, 136, 12, 0);
		header.fill(32, 148, 156);
		header[156] = directory ? 53 : 48;
		header.set(encoder$1.encode("ustar"), 257);
		header.set(encoder$1.encode("00"), 263);
		header.set(encoder$1.encode(prefix), 345);
		let checksum = 0;
		for (const byte of header) checksum += byte;
		header.set(encoder$1.encode(checksum.toString(8).padStart(6, "0")), 148);
		header[154] = 0;
		header[155] = 32;
		chunks.push(header);
		if (size > 0) {
			chunks.push(bytes);
			const padding = size % BLOCK;
			if (padding !== 0) chunks.push(new Uint8Array(BLOCK - padding));
		}
	}
	chunks.push(new Uint8Array(BLOCK * 2));
	const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
	const archive = new Uint8Array(total);
	let offset = 0;
	for (const chunk of chunks) {
		archive.set(chunk, offset);
		offset += chunk.length;
	}
	return archive;
}
/** @returns The NUL-terminated string in one header field. */
function readField(header, offset, length) {
	let end = offset;
	while (end < offset + length && header[end] !== 0) end += 1;
	return decoder$1.decode(header.subarray(offset, end));
}
/**
* Parse an uncompressed ustar archive.
*
* File bytes are subarray views into `archive`, not copies; callers own the
* aliasing. Entry kinds outside the written subset (links, PAX extensions)
* fail loud instead of being skipped.
* @param archive - Archive bytes.
* @returns Entries in archive order.
*/
function parseTar(archive) {
	const entries = [];
	let offset = 0;
	while (offset + BLOCK <= archive.length) {
		const header = archive.subarray(offset, offset + BLOCK);
		if (header.every((byte) => byte === 0)) break;
		const short = readField(header, 0, 100);
		const prefix = readField(header, 345, 155);
		const name = prefix === "" ? short : `${prefix}/${short}`;
		const size = Number.parseInt(readField(header, 124, 12).trim() || "0", 8);
		const mode = Number.parseInt(readField(header, 100, 8).trim() || "0", 8) & 511;
		const typeflag = header[156];
		const directory = typeflag === 53 || name.endsWith("/");
		if (typeflag !== 48 && typeflag !== 0 && typeflag !== 53) throw new Error(`vfs tar: unsupported entry type ${String.fromCharCode(typeflag ?? 0)} for "${name}"`);
		const dataStart = offset + BLOCK;
		entries.push({
			name,
			bytes: archive.subarray(dataStart, dataStart + size),
			directory,
			mode
		});
		offset = dataStart + Math.ceil(size / BLOCK) * BLOCK;
	}
	return entries;
}
// src/storage/memory.ts
/**
* In-memory filesystem behind the worker's `node:fs` proxy. Contents come from
* the build-time image (see {@link loadVfsImage}); this remains the synchronous
* authority when an asynchronous durable sink mirrors selected subtrees.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/storage/memory
*/
const decoder = new TextDecoder();
const encoder = new TextEncoder();
/** Creation default for files, Node's `0o666` under the classic `022` umask. */
const DEFAULT_FILE_MODE = 420;
/** Creation default for directories, Node's `0o777` under the classic `022` umask. */
const DEFAULT_DIRECTORY_MODE = 493;
function fail(code, syscall, path, detail) {
	const error = /* @__PURE__ */ new Error(`${code}: ${detail ?? syscall} failed, ${syscall} '${path}'`);
	error.code = code;
	error.path = path;
	error.syscall = syscall;
	throw error;
}
function encodingOf(options) {
	if (options === null || options === void 0) return void 0;
	if (typeof options === "string") return options;
	return options.encoding ?? void 0;
}
function statsOf(size, mtimeMs, directory, ino, mode) {
	return {
		size,
		ino: Number(ino),
		mtimeMs,
		ctimeMs: mtimeMs,
		atimeMs: mtimeMs,
		birthtimeMs: mtimeMs,
		mtime: new Date(mtimeMs),
		mode: (directory ? 16384 : 32768) | mode & 511,
		isFile: () => !directory,
		isDirectory: () => directory,
		isSymbolicLink: () => false,
		isFIFO: () => false,
		isSocket: () => false,
		isBlockDevice: () => false,
		isCharacterDevice: () => false
	};
}
/**
* The same entry as {@link statsOf}, in the BigInt shape.
*
* Timestamps carry millisecond resolution scaled to nanoseconds, which is what
* the underlying `mtimeMs` holds; the VFS keeps that value strictly increasing
* per entry so two writes inside one millisecond still differ.
* @param size - Byte length; zero for a directory.
* @param mtimeMs - Modification time the entry carries.
* @param directory - Whether the entry is a directory.
* @param ino - Identity of the entry at this path.
* @param mode - Stored permission bits of the entry.
* @returns Stats in the shape Node returns under `{ bigint: true }`.
*/
function bigIntStatsOf(size, mtimeMs, directory, ino, mode, nlink = 1) {
	const milliseconds = BigInt(Math.trunc(mtimeMs));
	const nanoseconds = milliseconds * 1000000n;
	const time = new Date(mtimeMs);
	return {
		size: BigInt(size),
		mode: BigInt((directory ? 16384 : 32768) | mode & 511),
		dev: 1n,
		ino,
		nlink: BigInt(nlink),
		mtimeMs: milliseconds,
		mtimeNs: nanoseconds,
		ctimeMs: milliseconds,
		ctimeNs: nanoseconds,
		atimeMs: milliseconds,
		atimeNs: nanoseconds,
		birthtimeMs: milliseconds,
		birthtimeNs: nanoseconds,
		mtime: time,
		ctime: time,
		atime: time,
		birthtime: time,
		isFile: () => !directory,
		isDirectory: () => directory,
		isSymbolicLink: () => false,
		isFIFO: () => false,
		isSocket: () => false,
		isBlockDevice: () => false,
		isCharacterDevice: () => false
	};
}
/** Parse the Node string flags supported by the compatibility filesystem. */
function openMode(flags) {
	const base = flags[0];
	const suffix = flags.slice(1).split("");
	const validSuffix = suffix.every((flag) => flag === "+" || flag === "x" || flag === "s");
	const uniqueSuffix = new Set(suffix).size === suffix.length;
	if (base !== "r" && base !== "w" && base !== "a" || !validSuffix || !uniqueSuffix || base === "r" && flags.includes("x")) {
		const error = /* @__PURE__ */ new TypeError(`The argument 'flags' is invalid. Received '${flags}'`);
		error.code = "ERR_INVALID_ARG_VALUE";
		throw error;
	}
	return {
		readable: base === "r" || flags.includes("+"),
		writable: base !== "r" || flags.includes("+"),
		append: base === "a",
		create: base === "w" || base === "a",
		truncate: base === "w",
		exclusive: flags.includes("x")
	};
}
/** Resize bytes exactly, preserving the prefix and zero-filling growth. */
function resize(bytes, length) {
	if (!Number.isSafeInteger(length) || length < 0) {
		const error = /* @__PURE__ */ new RangeError(`The value of "len" is out of range. It must be >= 0. Received ${String(length)}`);
		error.code = "ERR_OUT_OF_RANGE";
		throw error;
	}
	const resized = new Uint8Array(length);
	resized.set(bytes.subarray(0, length));
	return resized;
}
/**
* Filesystem held in two maps: one for file bytes, one for directories.
* Every path is normalized to an absolute POSIX path without a trailing
* separator, so callers may pass either form.
*/
var MemoryVfs = class {
	files = /* @__PURE__ */ new Map();
	/** Files a lazy archive supplies on their first asynchronous read. */
	deferred = /* @__PURE__ */ new Map();
	directories = /* @__PURE__ */ new Set(["/"]);
	/** Directory permission bits; absence means {@link DEFAULT_DIRECTORY_MODE}. */
	directoryModes = /* @__PURE__ */ new Map();
	/** Directory mtimes advance when their immediate entry set changes. */
	directoryMtimes = /* @__PURE__ */ new Map();
	mutationListeners = /* @__PURE__ */ new Set();
	sink;
	temporaries = 0;
	identities = /* @__PURE__ */ new Map();
	lastIdentity = 0n;
	/**
	* Build the synchronous filesystem authority.
	* @param options - Optional durable write-behind sink.
	*/
	constructor(options = {}) {
		this.sink = options.sink;
	}
	/**
	* Settle the durable sink without changing in-memory success.
	* @returns A promise that resolves when all recorded mutations are stored.
	*/
	async flush() {
		await this.sink?.flush();
	}
	/**
	* Observe committed runtime mutations. Image seeding is deliberately silent.
	* @param listener - Consumer called after each successful mutation.
	* @returns A disposer that prevents future calls.
	*/
	subscribe(listener) {
		this.mutationListeners.add(listener);
		return () => {
			this.mutationListeners.delete(listener);
		};
	}
	/** Publish after state changes; one faulty observer cannot roll back a write. */
	publish(mutation) {
		const observers = [...this.sink === void 0 ? [] : [(change) => {
			this.sink?.record(change);
		}], ...this.mutationListeners];
		for (const listener of observers) try {
			listener(mutation);
		} catch (error) {
			console.error("webworker vfs: mutation observer failed", error);
		}
	}
	/** Promise face mirroring `node:fs/promises` for the methods the roster uses. */
	promises = {
		readFile: async (path, options) => {
			await this.whenReadable(path);
			return this.readFileSync(path, options);
		},
		writeFile: async (path, data, options) => {
			this.writeFileSync(path, data, options);
		},
		appendFile: async (path, data) => {
			this.appendFileSync(path, data);
		},
		mkdir: async (path, options) => this.mkdirSync(path, options),
		readdir: async (path, options) => this.readdirSync(path, options),
		stat: async (path, options) => this.statSync(path, options),
		lstat: async (path, options) => this.statSync(path, options),
		realpath: async (path) => this.realpathSync(path),
		rename: async (from, to) => {
			this.renameSync(from, to);
		},
		unlink: async (path) => {
			this.unlinkSync(path);
		},
		rm: async (path, options) => {
			this.rmSync(path, options);
		},
		mkdtemp: async (prefix) => this.mkdtempSync(prefix),
		link: async (existing, next) => {
			this.linkSync(existing, next);
		},
		truncate: async (path, length) => {
			this.truncateSync(path, length);
		},
		chmod: async (path, mode) => {
			this.chmodSync(path, mode);
		},
		opendir: async (path) => this.opendir(path),
		open: async (path, flags, mode) => {
			await this.whenReadable(path);
			return this.open(path, flags, mode);
		},
		/** Resolves for any existing path: the VFS grants read and write to everything it holds. */
		access: async (path) => {
			if (!this.existsSync(path)) fail("ENOENT", "access", normalize(resolve(path)));
		}
	};
	/** @returns Absolute path with no trailing separator. */
	key(path) {
		const absolute = normalize(resolve(path));
		return absolute.length > 1 && absolute.endsWith("/") ? absolute.slice(0, -1) : absolute;
	}
	/**
	* Register files whose bytes arrive with one archive on the first
	* asynchronous read of any of them. Their parent directories are created now.
	* @param archive - Archive name, mount root, registered files, and fetch.
	* @throws When a registered path already holds a file, directory, or another registration.
	*/
	deferArchive(archive) {
		const root = this.key(archive.root);
		const files = /* @__PURE__ */ new Map();
		for (const file of archive.files) {
			const target = this.key(join(root, file.path));
			if (this.files.has(target) || this.directories.has(target) || this.deferred.has(target) || files.has(target)) throw new Error(`webworker vfs: ${target} is already registered, so lazy archive ${archive.name} cannot supply it`);
			files.set(target, file.size);
		}
		const state = {
			name: archive.name,
			root,
			load: () => archive.load(),
			files,
			fetching: void 0
		};
		const mtimeMs = Date.now();
		for (const [target, size] of files) {
			this.seedDirectory(dirname(target));
			this.deferred.set(target, {
				size,
				mode: DEFAULT_FILE_MODE,
				mtimeMs,
				archive: state
			});
			this.touchDirectory(dirname(target));
		}
	}
	/**
	* Fetch the lazy archive that supplies a registered path. Concurrent readers
	* of one archive share one fetch; a failed fetch leaves its files registered,
	* so the next read fetches again.
	* @param path - Path about to be read asynchronously.
	* @returns A promise that settles once the archive arrived, or undefined when the path needs no fetch.
	*/
	whenReadable(path) {
		const file = this.deferred.get(this.key(path));
		if (file === void 0) return void 0;
		file.archive.fetching ??= this.fetchArchive(file.archive);
		return file.archive.fetching;
	}
	/** Fetch, validate against the registration, and mount one archive's files. */
	async fetchArchive(state) {
		try {
			let archive;
			try {
				archive = await state.load();
			} catch (error) {
				throw new Error(`webworker vfs: lazy archive ${state.name} could not be fetched`, { cause: error });
			}
			const entries = /* @__PURE__ */ new Map();
			for (const entry of parseTar(archive)) {
				if (entry.directory) continue;
				const relativeName = entry.name.startsWith("./") ? entry.name.slice(2) : entry.name;
				const target = this.key(join(state.root, relativeName));
				const size = state.files.get(target);
				if (size === void 0) throw new Error(`webworker vfs: lazy archive ${state.name} carries unregistered entry "${relativeName}"`);
				if (entry.bytes.length !== size) throw new Error(`webworker vfs: lazy archive ${state.name} holds ${String(entry.bytes.length)} bytes for ${target}, registered as ${String(size)}`);
				entries.set(target, entry.bytes);
			}
			for (const target of state.files.keys()) if (!entries.has(target)) throw new Error(`webworker vfs: lazy archive ${state.name} lacks registered file ${target}`);
			for (const [target, bytes] of entries) {
				const file = this.deferred.get(target);
				if (file?.archive !== state) continue;
				this.deferred.delete(target);
				const identity = this.identities.get(target);
				this.identities.delete(target);
				this.setFile(target, {
					bytes,
					mtimeMs: file.mtimeMs,
					mode: file.mode,
					paths: void 0,
					...identity === void 0 ? {} : { identity }
				});
			}
		} catch (error) {
			state.fetching = void 0;
			throw error;
		}
	}
	/** Refuse a synchronous operation on a registered file whose bytes have not arrived. */
	requireArrived(syscall, target) {
		const file = this.deferred.get(target);
		if (file === void 0) return;
		const error = /* @__PURE__ */ new Error(`ERR_DSH_VFS_DEFERRED: ${syscall} cannot wait for '${target}', which lazy archive ${file.archive.name} supplies on its first asynchronous read`);
		error.code = "ERR_DSH_VFS_DEFERRED";
		error.path = target;
		error.syscall = syscall;
		throw error;
	}
	/**
	* Read a file.
	* @param path - File path.
	* @param options - `'utf8'` or `{encoding}` for text; omitted for bytes.
	* @returns Text or a copy-free view of the stored bytes.
	*/
	readFileSync(path, options) {
		const target = this.key(path);
		this.requireArrived("open", target);
		const node = this.files.get(target);
		if (node === void 0) {
			if (this.directories.has(target)) fail("EISDIR", "read", target);
			fail("ENOENT", "open", target);
		}
		return encodingOf(options) === void 0 ? node.bytes : decoder.decode(node.bytes);
	}
	/**
	* Report whether a path exists.
	* @param path - Path to test.
	* @returns True for files and directories.
	*/
	existsSync(path) {
		const target = this.key(path);
		return this.files.has(target) || this.directories.has(target) || this.deferred.has(target);
	}
	/**
	* Stat a path.
	* @param path - Path to stat.
	* @param options - `bigint` selects the BigInt stats Node returns for it.
	* @returns Stats for the file or directory.
	*/
	statSync(path, options) {
		const target = this.key(path);
		const node = this.files.get(target);
		const deferred = this.deferred.get(target);
		const [size, mtimeMs, directory, mode] = node !== void 0 ? [
			node.bytes.length,
			node.mtimeMs,
			false,
			node.mode
		] : deferred !== void 0 ? [
			deferred.size,
			deferred.mtimeMs,
			false,
			deferred.mode
		] : this.directories.has(target) ? [
			0,
			this.directoryMtimes.get(target) ?? 0,
			true,
			this.directoryModes.get(target) ?? DEFAULT_DIRECTORY_MODE
		] : fail("ENOENT", "stat", target);
		const identity = node === void 0 ? this.identityOf(target) : this.identityOfFile(node);
		return options?.bigint === true ? bigIntStatsOf(size, mtimeMs, directory, identity, mode, node === void 0 ? 1 : this.fileLinkCount(node)) : statsOf(size, mtimeMs, directory, identity, mode);
	}
	/** @returns Stats in the plain shape, for internal callers that read `size`/`mtimeMs`. */
	plainStats(path) {
		return this.statSync(path);
	}
	/** @returns The stable identity of an existing path, assigning one on first observation. */
	identityOf(target) {
		const existing = this.identities.get(target);
		if (existing !== void 0) return existing;
		this.lastIdentity += 1n;
		this.identities.set(target, this.lastIdentity);
		return this.lastIdentity;
	}
	/** @returns The inode-like identity retained by a file node across names. */
	identityOfFile(node) {
		if (node.identity !== void 0) return node.identity;
		this.lastIdentity += 1n;
		node.identity = this.lastIdentity;
		return node.identity;
	}
	/** @returns The number of names currently linked to one file node. */
	fileLinkCount(node) {
		return typeof node.paths === "string" ? 1 : node.paths?.size ?? 0;
	}
	/** Add one map name, promoting the rare hard-link case to a Set. */
	addFilePath(node, path) {
		if (node.paths === void 0) node.paths = path;
		else if (typeof node.paths === "string") node.paths = /* @__PURE__ */ new Set([node.paths, path]);
		else node.paths.add(path);
	}
	/** Remove one map name, collapsing a remaining single link back to a string. */
	removeFilePath(node, path) {
		if (typeof node.paths === "string") {
			node.paths = void 0;
			return;
		}
		if (node.paths === void 0) return;
		node.paths.delete(path);
		if (node.paths.size === 1) {
			const [remaining] = node.paths;
			node.paths = remaining;
		}
	}
	/** Set one file-map entry while maintaining both nodes' reverse path indexes. */
	setFile(path, node) {
		const previous = this.files.get(path);
		if (previous === node) return;
		if (previous !== void 0) this.removeFilePath(previous, path);
		this.files.set(path, node);
		this.addFilePath(node, path);
	}
	/** Delete one file-map entry while retaining an unlinked node held by a descriptor. */
	deleteFile(path) {
		const node = this.files.get(path);
		if (node === void 0) return void 0;
		this.files.delete(path);
		this.removeFilePath(node, path);
		return node;
	}
	/** Publish one linked name after a content or metadata write. */
	publishFilePath(node, path, appendedFrom) {
		this.publish({
			kind: "write",
			path,
			bytes: node.bytes,
			mode: node.mode,
			entryChanged: false,
			...appendedFrom === void 0 ? {} : { appendedFrom }
		});
	}
	/** Publish a content or metadata write for every hard link to one node. */
	publishFile(node, appendedFrom) {
		if (typeof node.paths === "string") {
			this.publishFilePath(node, node.paths, appendedFrom);
			return;
		}
		if (node.paths === void 0) return;
		for (const path of node.paths) this.publishFilePath(node, path, appendedFrom);
	}
	/** Replace bytes on one file identity and notify all linked paths. */
	replaceFile(node, bytes, appendedFrom) {
		node.bytes = bytes;
		node.mtimeMs = this.touchNode(node);
		this.publishFile(node, appendedFrom);
	}
	/** Write at one offset, zero-filling any gap. */
	writeFileNode(node, position, data) {
		const offset = Math.max(0, position);
		const previousLength = node.bytes.length;
		const bytes = new Uint8Array(Math.max(previousLength, offset + data.length));
		bytes.set(node.bytes);
		bytes.set(data, offset);
		this.replaceFile(node, bytes, offset === previousLength ? previousLength : void 0);
		return data.length;
	}
	/** Resize one file identity and notify all linked paths. */
	truncateFile(node, length) {
		this.replaceFile(node, resize(node.bytes, length));
	}
	/** Change one file identity's permission bits and notify every linked path. */
	chmodFile(node, mode) {
		node.mode = mode & 511;
		if (typeof node.paths === "string") this.publish({
			kind: "chmod",
			path: node.paths,
			mode: node.mode
		});
		else if (node.paths !== void 0) for (const path of node.paths) this.publish({
			kind: "chmod",
			path,
			mode: node.mode
		});
	}
	/** @returns Plain stats for an open file, including after its last name is removed. */
	fileStats(node) {
		return statsOf(node.bytes.length, node.mtimeMs, false, this.identityOfFile(node), node.mode);
	}
	/** @returns BigInt stats for an open file, including its device and inode identity. */
	fileBigIntStats(node) {
		return bigIntStatsOf(node.bytes.length, node.mtimeMs, false, this.identityOfFile(node), node.mode, this.fileLinkCount(node));
	}
	/** Forget removed directory identities, so recreated paths report new ones. */
	forgetIdentity(target) {
		this.identities.delete(target);
		const prefix = `${target}/`;
		for (const known of [...this.identities.keys()]) if (known.startsWith(prefix)) this.identities.delete(known);
	}
	/**
	* Modification time for a write, strictly after the entry's previous one.
	*
	* The clock has millisecond resolution and these writes are in memory, so two
	* revisions of one file routinely land in the same millisecond. The filesystem
	* service's stale-write guard compares timestamps, so an equal one would let a
	* stale overwrite through.
	* @param target - Normalized path being written.
	* @returns Now, or one millisecond past the entry's current time.
	*/
	touch(target) {
		return this.touchNode(this.files.get(target));
	}
	/** @returns A modification time strictly newer than one file node's current value. */
	touchNode(node) {
		const previous = node?.mtimeMs;
		const now = Date.now();
		return previous === void 0 ? now : Math.max(now, previous + 1);
	}
	/** Advance a directory's mtime after its immediate children change. */
	touchDirectory(target) {
		const previous = this.directoryMtimes.get(target);
		const now = Date.now();
		this.directoryMtimes.set(target, previous === void 0 ? now : Math.max(now, previous + 1));
	}
	/**
	* List a directory.
	* @param path - Directory path.
	* @param options - `withFileTypes` returns {@link VfsDirent} objects instead of names.
	* @returns Immediate entry names, or directory entries.
	*/
	readdirSync(path, options) {
		const target = this.key(path);
		if (!this.directories.has(target)) {
			if (this.files.has(target) || this.deferred.has(target)) fail("ENOTDIR", "scandir", target);
			fail("ENOENT", "scandir", target);
		}
		const prefix = target === "/" ? "/" : `${target}/`;
		const names = /* @__PURE__ */ new Set();
		for (const candidate of [
			...this.files.keys(),
			...this.deferred.keys(),
			...this.directories
		]) {
			if (!candidate.startsWith(prefix) || candidate === target) continue;
			const rest = candidate.slice(prefix.length);
			if (rest === "") continue;
			const [head = rest] = rest.split("/");
			names.add(head);
		}
		const sorted = [...names].sort();
		if (options?.withFileTypes !== true) return sorted;
		return sorted.map((name) => this.direntOf(target, name));
	}
	/** @returns Directory entry for one child of `directory`. */
	direntOf(directory, name) {
		const stats = this.plainStats(join(directory, name));
		return {
			name,
			parentPath: directory,
			isFile: () => stats.isFile(),
			isDirectory: () => stats.isDirectory(),
			isSymbolicLink: () => false
		};
	}
	/**
	* Resolve a path; the VFS has no symlinks, so this only normalizes.
	* @param path - Path to resolve.
	* @returns Absolute path.
	*/
	realpathSync(path) {
		const target = this.key(path);
		if (!this.existsSync(target)) fail("ENOENT", "realpath", target);
		return target;
	}
	/**
	* Create a directory.
	* @param path - Directory path.
	* @param options - `recursive` creates missing parents.
	* @returns First created path when recursive, otherwise undefined.
	*/
	mkdirSync(path, options) {
		const target = this.key(path);
		if (this.files.has(target) || this.deferred.has(target)) fail("EEXIST", "mkdir", target);
		if (this.directories.has(target)) {
			if (options?.recursive === true) return void 0;
			fail("EEXIST", "mkdir", target);
		}
		const parent = dirname(target);
		if (!this.directories.has(parent)) {
			if (options?.recursive !== true) fail("ENOENT", "mkdir", target);
			this.mkdirSync(parent, options);
		}
		this.directories.add(target);
		this.touchDirectory(target);
		this.touchDirectory(parent);
		const mode = (options?.mode ?? DEFAULT_DIRECTORY_MODE) & 511;
		if (mode !== DEFAULT_DIRECTORY_MODE) this.directoryModes.set(target, mode);
		this.publish({
			kind: "mkdir",
			path: target,
			mode
		});
		return target;
	}
	/**
	* Write a file, replacing existing contents.
	* @param path - File path; its parent directory must exist.
	* @param data - Text or bytes.
	* @param options - `flag` `wx` refuses an existing file, `a` appends.
	*/
	writeFileSync(path, data, options) {
		const target = this.key(path);
		this.requireArrived("open", target);
		if (this.directories.has(target)) fail("EISDIR", "open", target);
		if (!this.directories.has(dirname(target))) fail("ENOENT", "open", target);
		const flag = options?.flag ?? "w";
		if (flag.startsWith("wx") && this.files.has(target)) fail("EEXIST", "open", target);
		if (flag.startsWith("a")) {
			this.appendFileSync(target, data);
			return;
		}
		const previous = this.files.get(target);
		const mode = previous?.mode ?? (options?.mode !== void 0 ? options.mode & 511 : DEFAULT_FILE_MODE);
		const bytes = typeof data === "string" ? encoder.encode(data) : data;
		if (previous !== void 0) {
			this.replaceFile(previous, bytes);
			return;
		}
		const node = {
			bytes,
			mtimeMs: this.touch(target),
			mode,
			paths: void 0
		};
		this.setFile(target, node);
		this.touchDirectory(dirname(target));
		this.publish({
			kind: "write",
			path: target,
			bytes,
			mode,
			entryChanged: true
		});
	}
	/**
	* Open a directory; consumers enumerate entries or just prove it is one.
	* @param path - Directory path.
	* @returns Directory handle.
	*/
	opendir(path) {
		const target = this.key(path);
		const names = this.readdirSync(target);
		let cursor = 0;
		const direntOf = (name) => this.direntOf(target, name);
		return {
			path: target,
			close: async () => {},
			read: async () => {
				const name = names[cursor];
				cursor += 1;
				return name === void 0 ? null : direntOf(name);
			},
			async *[Symbol.asyncIterator]() {
				for (const name of names) yield direntOf(name);
			}
		};
	}
	/**
	* Open a file handle.
	* @param path - File path.
	* @param flags - Node open flags; `r` requires the file, `wx` refuses an existing one.
	* @param mode - Permission bits applied when the open creates the file.
	* @returns File handle.
	*/
	open(path, flags = "r", mode) {
		const target = this.key(path);
		if (this.directories.has(target)) {
			if (!flags.startsWith("r")) fail("EISDIR", "open", target);
			return {
				write: async () => fail("EISDIR", "write", target),
				writeFile: async () => fail("EISDIR", "write", target),
				readFile: async () => fail("EISDIR", "read", target),
				truncate: async () => fail("EISDIR", "ftruncate", target),
				...this.handleTail(target)
			};
		}
		const file = this.openFileSync(target, flags, mode);
		let position = 0;
		let closed = false;
		const current = (syscall) => {
			if (closed) fail("EBADF", syscall, target);
			return file;
		};
		return {
			write: async (data) => {
				const bytes = typeof data === "string" ? encoder.encode(data) : data;
				const descriptor = current("write");
				const offset = descriptor.append ? descriptor.stat().size : position;
				const bytesWritten = descriptor.write(offset, bytes);
				position = offset + bytesWritten;
				return { bytesWritten };
			},
			writeFile: async (data) => {
				const bytes = typeof data === "string" ? encoder.encode(data) : data;
				const descriptor = current("write");
				const offset = descriptor.append ? descriptor.stat().size : position;
				position = offset + descriptor.write(offset, bytes);
			},
			readFile: async (options) => {
				const descriptor = current("read");
				const bytes = descriptor.read(position, Math.max(0, descriptor.stat().size - position));
				position += bytes.length;
				return encodingOf(options) === void 0 ? bytes : decoder.decode(bytes);
			},
			truncate: async (length = 0) => {
				current("ftruncate").truncate(length);
			},
			chmod: async (mode) => {
				current("fchmod").chmod(mode);
			},
			stat: async (options) => options?.bigint === true ? current("fstat").statBigInt() : current("fstat").stat(),
			sync: async () => {
				current("fsync");
				await this.flush();
			},
			datasync: async () => {
				current("fdatasync");
				await this.flush();
			},
			close: async () => {
				closed = true;
			}
		};
	}
	/**
	* Open one synchronous descriptor over a stable file identity.
	* @param path - File path.
	* @param flags - Node open flags.
	* @param mode - Permission bits applied only when a file is created.
	* @returns An open file that survives path rename, replacement, and unlink.
	*/
	openFileSync(path, flags = "r", mode) {
		const target = this.key(path);
		this.requireArrived("open", target);
		const access = openMode(flags);
		const existing = this.files.get(target);
		if (this.directories.has(target)) fail("EISDIR", "open", target);
		if (access.exclusive && existing !== void 0) fail("EEXIST", "open", target);
		if (!access.create && existing === void 0) fail("ENOENT", "open", target);
		if (access.create && existing === void 0) this.writeFileSync(target, /* @__PURE__ */ new Uint8Array(), mode === void 0 ? void 0 : { mode });
		else if (access.truncate && existing !== void 0) this.truncateFile(existing, 0);
		const node = this.files.get(target);
		if (node === void 0) fail("ENOENT", "open", target);
		return {
			readable: access.readable,
			writable: access.writable,
			append: access.append,
			read: (position, length) => {
				if (!access.readable) fail("EBADF", "read", target);
				return node.bytes.subarray(position, position + length);
			},
			write: (position, data) => {
				if (!access.writable) fail("EBADF", "write", target);
				return this.writeFileNode(node, access.append ? node.bytes.length : position, data);
			},
			truncate: (length) => {
				if (!access.writable) fail("EINVAL", "ftruncate", target);
				this.truncateFile(node, length);
			},
			chmod: (mode) => {
				this.chmodFile(node, mode);
			},
			stat: () => this.fileStats(node),
			statBigInt: () => this.fileBigIntStats(node)
		};
	}
	/**
	* Directory-handle members for metadata, durability, and release.
	* `sync`/`datasync` settle an attached durable sink; an ephemeral filesystem
	* resolves immediately and `close` releases nothing.
	* @param target - Normalized path the handle was opened on.
	* @returns Metadata plus the no-op durability and release calls.
	*/
	handleTail(target) {
		return {
			chmod: async (mode) => {
				this.chmodSync(target, mode);
			},
			stat: async (options) => this.statSync(target, options),
			sync: async () => {
				await this.flush();
			},
			datasync: async () => {
				await this.flush();
			},
			close: async () => {}
		};
	}
	/**
	* Append to a file, creating it when absent.
	* @param path - File path.
	* @param data - Text or bytes.
	*/
	appendFileSync(path, data) {
		const target = this.key(path);
		this.requireArrived("open", target);
		const existing = this.files.get(target);
		const addition = typeof data === "string" ? encoder.encode(data) : data;
		if (existing === void 0) {
			this.writeFileSync(target, addition);
			return;
		}
		this.writeFileNode(existing, existing.bytes.length, addition);
	}
	/**
	* Move a file or directory subtree.
	* @param from - Source path.
	* @param to - Destination path.
	*/
	renameSync(from, to) {
		const source = this.key(from);
		const destination = this.key(to);
		this.requireArrived("rename", source);
		this.requireArrived("rename", destination);
		if (source === destination) return;
		const node = this.files.get(source);
		if (node !== void 0) {
			if (this.directories.has(destination)) fail("EISDIR", "rename", destination);
			if (!this.directories.has(dirname(destination))) fail("ENOENT", "rename", destination);
			if (this.files.get(destination) === node) return;
			this.deleteFile(source);
			this.setFile(destination, node);
			this.forgetIdentity(source);
			this.forgetIdentity(destination);
			this.touchDirectory(dirname(source));
			this.touchDirectory(dirname(destination));
			this.publish({
				kind: "remove",
				path: source
			});
			this.publish({
				kind: "write",
				path: destination,
				bytes: node.bytes,
				mode: node.mode,
				entryChanged: true
			});
			return;
		}
		if (!this.directories.has(source)) fail("ENOENT", "rename", source);
		if (this.files.has(destination)) fail("ENOTDIR", "rename", destination);
		if (!this.directories.has(dirname(destination))) fail("ENOENT", "rename", destination);
		const prefix = `${source}/`;
		for (const registered of this.deferred.keys()) if (registered.startsWith(prefix)) this.requireArrived("rename", registered);
		if (this.directories.has(destination)) {
			if (this.readdirSync(destination).length > 0) fail("ENOTEMPTY", "rename", destination);
			this.directories.delete(destination);
			this.directoryModes.delete(destination);
			this.directoryMtimes.delete(destination);
		}
		const movedFiles = [];
		for (const [candidate, value] of [...this.files]) {
			if (!candidate.startsWith(prefix)) continue;
			this.deleteFile(candidate);
			const target = join(destination, candidate.slice(prefix.length));
			this.setFile(target, value);
			movedFiles.push({
				path: target,
				bytes: value.bytes,
				mode: value.mode
			});
		}
		const movedDirectories = [];
		for (const candidate of [...this.directories]) {
			if (!candidate.startsWith(prefix) && candidate !== source) continue;
			const moved = candidate === source ? destination : join(destination, candidate.slice(prefix.length));
			this.directories.delete(candidate);
			this.directories.add(moved);
			const bits = this.directoryModes.get(candidate);
			this.directoryModes.delete(candidate);
			if (bits !== void 0) this.directoryModes.set(moved, bits);
			movedDirectories.push({
				path: moved,
				mode: bits ?? DEFAULT_DIRECTORY_MODE
			});
			const mtime = this.directoryMtimes.get(candidate);
			this.directoryMtimes.delete(candidate);
			if (mtime !== void 0) this.directoryMtimes.set(moved, mtime);
		}
		this.forgetIdentity(source);
		this.forgetIdentity(destination);
		this.touchDirectory(dirname(source));
		this.touchDirectory(dirname(destination));
		this.publish({
			kind: "remove",
			path: source
		});
		for (const directory of movedDirectories) this.publish({
			kind: "mkdir",
			path: directory.path,
			mode: directory.mode
		});
		for (const entry of movedFiles) this.publish({
			kind: "write",
			path: entry.path,
			bytes: entry.bytes,
			mode: entry.mode,
			entryChanged: true
		});
	}
	/**
	* Give existing bytes a second name.
	*
	* Both names retain one file identity, so writes and metadata changes through
	* either name remain visible through the other until that name is removed.
	* @param existing - Source file path.
	* @param next - Additional path; its parent must exist and it must be free.
	*/
	linkSync(existing, next) {
		const source = this.key(existing);
		const target = this.key(next);
		this.requireArrived("link", source);
		const node = this.files.get(source);
		if (node === void 0) fail("ENOENT", "link", source);
		if (this.existsSync(target)) fail("EEXIST", "link", target);
		if (!this.directories.has(dirname(target))) fail("ENOENT", "link", target);
		this.setFile(target, node);
		this.touchDirectory(dirname(target));
		this.publish({
			kind: "write",
			path: target,
			bytes: node.bytes,
			mode: node.mode,
			entryChanged: true
		});
	}
	/**
	* Shorten a file.
	* @param path - File path.
	* @param length - Byte length to keep; defaults to zero.
	*/
	truncateSync(path, length = 0) {
		const target = this.key(path);
		this.requireArrived("truncate", target);
		const node = this.files.get(target);
		if (node === void 0) fail("ENOENT", "truncate", target);
		this.truncateFile(node, length);
	}
	/**
	* Change an entry's permission bits; stat reads back exactly what was set.
	* @param path - File or directory path.
	* @param mode - New permission bits (`0o777` mask).
	*/
	chmodSync(path, mode) {
		const target = this.key(path);
		this.requireArrived("chmod", target);
		const node = this.files.get(target);
		if (node !== void 0) {
			this.chmodFile(node, mode);
			return;
		}
		if (this.directories.has(target)) {
			const bits = mode & 511;
			this.directoryModes.set(target, bits);
			this.publish({
				kind: "chmod",
				path: target,
				mode: bits
			});
			return;
		}
		fail("ENOENT", "chmod", target);
	}
	/**
	* Remove a file.
	* @param path - File path.
	*/
	unlinkSync(path) {
		const target = this.key(path);
		if (this.deleteFile(target) === void 0 && !this.deferred.delete(target)) fail("ENOENT", "unlink", target);
		this.forgetIdentity(target);
		this.touchDirectory(dirname(target));
		this.publish({
			kind: "remove",
			path: target
		});
	}
	/**
	* Remove a file or directory.
	* @param path - Path to remove.
	* @param options - `recursive` removes subtrees, `force` ignores absence.
	*/
	rmSync(path, options) {
		const target = this.key(path);
		if (this.deleteFile(target) !== void 0 || this.deferred.delete(target)) {
			this.forgetIdentity(target);
			this.touchDirectory(dirname(target));
			this.publish({
				kind: "remove",
				path: target
			});
			return;
		}
		if (this.directories.has(target)) {
			if (options?.recursive !== true) fail("ERR_FS_EISDIR", "rm", target);
			const prefix = `${target}/`;
			for (const candidate of [...this.files.keys()]) if (candidate.startsWith(prefix)) this.deleteFile(candidate);
			for (const candidate of [...this.deferred.keys()]) if (candidate.startsWith(prefix)) this.deferred.delete(candidate);
			for (const candidate of [...this.directories]) {
				if (!candidate.startsWith(prefix)) continue;
				this.directories.delete(candidate);
				this.directoryModes.delete(candidate);
				this.directoryMtimes.delete(candidate);
			}
			this.directories.delete(target);
			this.directoryModes.delete(target);
			this.directoryMtimes.delete(target);
			this.forgetIdentity(target);
			this.touchDirectory(dirname(target));
			this.publish({
				kind: "remove",
				path: target
			});
			return;
		}
		if (options?.force !== true) fail("ENOENT", "rm", target);
	}
	/**
	* Create a uniquely named directory beside `prefix`, as `fs.mkdtempSync` does.
	* @param prefix - Path prefix; the suffix is appended without a separator.
	* @returns The created directory path.
	*/
	mkdtempSync(prefix) {
		this.temporaries += 1;
		const target = `${prefix}${Date.now().toString(36)}${this.temporaries.toString(36)}`;
		this.mkdirSync(target, { recursive: true });
		return this.key(target);
	}
	/**
	* Seed a file and its parent directories, for image loading and tests.
	* @param path - File path.
	* @param data - Text or bytes.
	* @param options - Permission bits and modification time supplied by the image or durable store.
	*/
	seed(path, data, options = {}) {
		const target = this.key(path);
		this.seedDirectory(dirname(target));
		this.deferred.delete(target);
		this.setFile(target, {
			bytes: typeof data === "string" ? encoder.encode(data) : data,
			mtimeMs: options.mtimeMs ?? this.touch(target),
			mode: (options.mode ?? DEFAULT_FILE_MODE) & 511,
			paths: void 0
		});
		this.touchDirectory(dirname(target));
	}
	/**
	* Create a directory and its parents.
	* @param path - Directory path.
	* @param options - Permission bits and modification time supplied by the image or durable store.
	*/
	seedDirectory(path, options = {}) {
		const target = this.key(path);
		if (!this.directories.has(target)) {
			const parent = dirname(target);
			if (parent !== target) this.seedDirectory(parent);
			if (this.files.has(target) || this.deferred.has(target)) fail("EEXIST", "mkdir", target);
			this.directories.add(target);
			this.directoryMtimes.set(target, options.mtimeMs ?? Date.now());
			this.touchDirectory(parent);
		}
		if (options.mode !== void 0) this.directoryModes.set(target, options.mode & 511);
		if (options.mtimeMs !== void 0) this.directoryMtimes.set(target, options.mtimeMs);
	}
	/**
	* Report what this filesystem holds, for the host's boot diagnostics.
	* @returns File count, directory count, and total byte size.
	*/
	usage() {
		let bytes = 0;
		for (const node of this.files.values()) bytes += node.bytes.length;
		return {
			files: this.files.size,
			directories: this.directories.size,
			bytes
		};
	}
};
/**
* Mount a tar image produced by the build-time collector.
*
* Entry names are relative to `root` (`node_modules/...`, `config/vfs-manifest.json`);
* an absolute entry name is a collector defect and fails loud. File contents
* stay views into `image` — nothing is copied at mount time.
* @param image - The ustar archive, as `inflateImage` produces it from the fetched image.
* @param root - Virtual root the entries mount under.
* @param vfs - Filesystem to fill; a fresh one by default.
* @returns The filled filesystem.
*/
function loadVfsImage(image, root = "/dsh", vfs = new MemoryVfs()) {
	vfs.seedDirectory(root);
	for (const entry of parseTar(image)) {
		const relativeName = entry.name.startsWith("./") ? entry.name.slice(2) : entry.name;
		if (relativeName.startsWith("/")) throw new Error(`webworker vfs: image entry must be relative to ${root}, received "${entry.name}"`);
		const target = join(root, relativeName);
		if (entry.directory) {
			vfs.seedDirectory(target, { mode: entry.mode });
			continue;
		}
		vfs.seed(target, entry.bytes, { mode: entry.mode });
	}
	return vfs;
}
/**
* Apply one ordered data overlay to an already mounted base image.
*
* Overlay entries may replace files only under the layout's data directories;
* module code, configuration, and the lowering manifest cannot be shadowed.
* Paths containing traversal segments are refused before normalization. Later
* overlays win for files, while file/directory type conflicts fail loud.
* @param image - Uncompressed ustar overlay archive.
* @param root - Virtual root shared with the base image.
* @param vfs - Mounted filesystem to update.
* @returns The same filesystem after applying the overlay.
*/
function loadVfsOverlay(image, root, vfs) {
	for (const entry of parseTar(image)) {
		const relativeName = entry.name.startsWith("./") ? entry.name.slice(2) : entry.name;
		const path = relativeName.endsWith("/") ? relativeName.slice(0, -1) : relativeName;
		const segments = path.split("/");
		if (path === "" || relativeName.startsWith("/") || segments.some((segment) => segment === "" || segment === "." || segment === "..") || !IMAGE_OVERLAY_DIRECTORIES.includes(segments[0] ?? "")) throw new Error(`webworker vfs: overlay entry must stay under ${IMAGE_OVERLAY_DIRECTORIES.join("/ or ")}, received "${entry.name}"`);
		const target = join(root, path);
		if (entry.directory) {
			vfs.seedDirectory(target, { mode: entry.mode });
			continue;
		}
		if (vfs.existsSync(target) && vfs.statSync(target).isDirectory()) throw new Error(`webworker vfs: overlay file cannot replace directory "${target}"`);
		vfs.seed(target, entry.bytes, { mode: entry.mode });
	}
	return vfs;
}
// src/storage/active.ts
let active;
/**
* Publish the filesystem the `node:fs` proxy reads.
* @param vfs - Filesystem mounted by the worker entry.
*/
function setActiveVfs(vfs) {
	active = vfs;
}
/**
* Read the mounted filesystem.
* @returns The active filesystem.
*/
function requireActiveVfs() {
	if (active === void 0) throw new Error("webworker vfs: no filesystem is mounted; the worker entry must call setActiveVfs before any node:fs access");
	return active;
}
// src/package-assets.ts
/**
* Package assets for the preview page. A Client bundle names each image as a
* document-relative reference, `plugins/<package>/assets/<path>`, and resolves
* it through the module loader's synchronous `require.asset`. The page of the
* Worker preview cannot request such a URL itself, so the Worker hands it every
* asset the image holds in one archive and the page maps each reference to a
* Blob URL before any bundle runs.
* @module @deepseek-ai/dsh-experimental-webworker-runtime/src/package-assets
*/
/** Entries of one directory. */
function entriesOf(vfs, directory) {
	return vfs.readdirSync(directory, { withFileTypes: true });
}
/** Files under one directory, as `/`-separated paths relative to it. */
function filesBelow(vfs, directory, prefix = "") {
	return entriesOf(vfs, directory).flatMap((entry) => entry.isDirectory() ? filesBelow(vfs, join(directory, entry.name), `${prefix}${entry.name}/`) : entry.isFile() ? [`${prefix}${entry.name}`] : []);
}
/** Installed package directories under `node_modules`, scoped ones included. */
function packageDirectories(vfs, modules) {
	if (!vfs.existsSync(modules)) return [];
	return entriesOf(vfs, modules).filter((entry) => entry.isDirectory()).flatMap((entry) => entry.name.startsWith("@") ? entriesOf(vfs, join(modules, entry.name)).filter((scoped) => scoped.isDirectory()).map((scoped) => join(modules, entry.name, scoped.name)) : [join(modules, entry.name)]);
}
/**
* Archive every file a package of the image publishes through `"./assets/*": "./lib/assets/*"`.
* @param vfs - Mounted image filesystem.
* @param root - Image root holding `node_modules`.
* @returns An uncompressed tar whose entries are named `plugins/<package>/assets/<path>` with the path unencoded,
* so every name fits the ustar limits the base image already met.
*/
function packPackageAssets(vfs, root) {
	const files = {};
	for (const directory of packageDirectories(vfs, join(root, "node_modules"))) {
		const manifestPath = join(directory, "package.json");
		if (!vfs.existsSync(manifestPath)) continue;
		const manifest = JSON.parse(vfs.readFileSync(manifestPath, "utf8"));
		if (typeof manifest.name !== "string" || manifest.exports?.["./assets/*"] !== "./lib/assets/*") continue;
		const assets = join(directory, "lib", "assets");
		if (!vfs.existsSync(assets)) continue;
		for (const path of filesBelow(vfs, assets)) files[`plugins/${manifest.name}/assets/${path}`] = vfs.readFileSync(join(assets, path));
	}
	return packTar(files);
}
// src/image-manifest.ts
/**
* The base image's manifest: what the packer records about one image and what
* the worker host requires before it wraps a module or boots the tree. Both
* sides use these types; the worker validates the fetched file with
* {@link parseImageManifest}.
*/
function recordOf$1(value) {
	return typeof value === "object" && value !== null && !Array.isArray(value) ? value : void 0;
}
/** A relative path made of non-empty segments without traversal. */
function isRelativePath(value) {
	return typeof value === "string" && value !== "" && !value.startsWith("/") && value.split("/").every((segment) => segment !== "" && segment !== "." && segment !== "..");
}
function isCount(value) {
	return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
/**
* Validate a fetched base-image manifest.
* @param value - Parsed manifest JSON.
* @param path - Manifest path inside the image, named in refusals.
* @returns The detached manifest.
* @throws When the manifest is malformed, or its bodies were lowered against another wrapper contract.
*/
function parseImageManifest(value, path) {
	const record = recordOf$1(value);
	if (record === void 0) throw new Error(`webworker host: ${path} does not hold an object`);
	if (record.lowered !== "dsh-worker-transform/1") throw new Error(`webworker host: image was lowered by ${String(record.lowered)}, this build runs ${LOWERING_VERSION}; rebuild the image`);
	const { root, profile, installation, javascriptEntries, visitedEntries, rewrittenEntries, lazy } = record;
	if (typeof root !== "string" || typeof profile !== "string" || profile === "" || installation !== void 0 && (typeof installation !== "string" || installation === "") || !isCount(javascriptEntries) || !isCount(visitedEntries) || !isCount(rewrittenEntries) || !Array.isArray(lazy)) throw new Error(`webworker host: ${path} lacks a valid root, profile, installation, entry count, or lazy archive list`);
	const archives = [];
	const archiveNames = /* @__PURE__ */ new Set();
	const filePaths = /* @__PURE__ */ new Set();
	for (const entry of lazy) {
		const archive = recordOf$1(entry);
		const name = archive?.archive;
		const files = archive?.files;
		if (!isRelativePath(name) || archiveNames.has(name) || !Array.isArray(files) || files.length === 0) throw new Error(`webworker host: ${path} lists an invalid or repeated lazy archive`);
		archiveNames.add(name);
		const parsed = [];
		for (const candidate of files) {
			const file = recordOf$1(candidate);
			if (file === void 0 || !isRelativePath(file.path) || !isCount(file.size) || filePaths.has(file.path)) throw new Error(`webworker host: ${path} lists an invalid or repeated file in lazy archive ${name}`);
			filePaths.add(file.path);
			parsed.push({
				path: file.path,
				size: file.size
			});
		}
		archives.push({
			archive: name,
			files: parsed
		});
	}
	return {
		root,
		profile,
		...installation === void 0 ? {} : { installation },
		lowered: LOWERING_VERSION,
		javascriptEntries,
		visitedEntries,
		rewrittenEntries,
		lazy: archives
	};
}
function requireGlobalPort(channel) {
	if (channel !== void 0) return channel;
	const post = globalThis.postMessage;
	if (typeof post !== "function") throw new Error("webworker host: no channel; pass options.channel outside a dedicated worker");
	return { postMessage: (message, transfer) => {
		post(message, transfer);
	} };
}
async function readImage(image) {
	if (typeof image !== "string") return await inflateImage(image, "the image bytes given to createWorkerHost");
	const response = await fetch(image);
	if (!response.ok) throw new Error(`webworker host: image fetch failed with ${String(response.status)} for ${image}`);
	if (response.body === null) throw new Error(`webworker host: image response for ${image} carried no body`);
	return await inflateImageStream(response.body, image);
}
/**
* Build the worker host without touching the network or the image.
* @param options - Assembly inputs.
* @returns Handle whose `handleMessage` is ready immediately.
*/
function createWorkerHost(options) {
	const root = options.root ?? "/dsh";
	const port = options.port ?? 3080;
	const tunnel = new TunnelServer({
		port: requireGlobalPort(options.channel),
		requestListener: options.requestListener,
		...options.privilegedMethods === void 0 ? {} : { privilegedMethods: options.privilegedMethods },
		...options.unaryApiLane === void 0 ? {} : { unaryApiLane: options.unaryApiLane }
	});
	let vfs;
	let modules;
	let context;
	const start = async () => {
		try {
			const home = join(root, IMAGE_HOME_DIRECTORY);
			const processShim = installProcessGlobal({
				cwd: root,
				env: {
					DSH_HOME: home,
					HOME: home,
					...options.env
				}
			});
			const [bytes, overlays] = await Promise.all([readImage(options.image), Promise.all((options.overlays ?? []).map(readImage))]);
			const mounted = loadVfsImage(bytes, root);
			for (const overlay of overlays) loadVfsOverlay(overlay, root, mounted);
			for (const directory of IMAGE_EMPTY_DIRECTORIES) mounted.seedDirectory(join(root, directory.replace(/\/$/, "")));
			setActiveVfs(mounted);
			vfs = mounted;
			const manifest = readImageManifest(mounted, options.manifestPath ?? join(root, "config/vfs-manifest.json"));
			deferLazyArchives(mounted, manifest, root, options.image);
			const staticModules = { ...options.staticModules };
			for (const key of ["node:process", "process"]) staticModules[key] ??= () => globalThis.process;
			const loader = new WorkerModuleLoader({
				vfs: mounted,
				root,
				staticModules,
				...options.staticModulePrefixes === void 0 ? {} : { staticModulePrefixes: options.staticModulePrefixes },
				...options.alsCausality === void 0 ? {} : { alsCausality: options.alsCausality }
			});
			setActiveModuleLoader(loader);
			modules = loader;
			const require = loader.requireFrom(root);
			const appBoot = require("@deepseek-ai/dsh-app-boot");
			const { DSH_LAUNCH_ENVIRONMENT_KEY } = require("@deepseek-ai/dsh-launch-environment");
			const cmdline = require("@deepseek-ai/dsh-cmdline");
			const started = Promise.withResolvers();
			const { patches, profile, environment } = prepareProfile(appBoot, mounted, manifest, root, processShim.env);
			const profileConfig = join(profile.dir, "cordis.yml");
			const ctx = await appBoot.boot(BIN_NAME, profileConfig, patches, async (hostCtx) => {
				hostCtx.provide("profileContext", profile);
				hostCtx.provide(DSH_LAUNCH_ENVIRONMENT_KEY, environment);
				installLogSink(hostCtx, require);
				await hostCtx.plugin(appBoot.PluginPackages, {});
				cmdline.provideCmdline(hostCtx, {
					args: [...options.cmdlineArgs ?? [
						"--host",
						"127.0.0.1",
						"--port",
						String(port),
						"--no-open"
					]],
					exit: (code) => {
						console.warn(`webworker host: tree requested exit(${String(code)})`);
					},
					ready: { onReady: (listener) => {
						let pending = true;
						started.promise.then(() => {
							if (pending) listener();
						});
						return () => {
							pending = false;
						};
					} }
				});
			});
			context = ctx;
			const connection = ctx.get("connection");
			if (connection === void 0) throw new Error("webworker host: the tree activated without a Connection service");
			const typertGateway = ctx.get("typertGateway");
			if (typertGateway === void 0) throw new Error("webworker host: the tree activated without a typertGateway service");
			const handler = connection.createSharedFetchHandler("/api");
			const usage = loader.usage();
			console.info(`webworker host: tree active (modules=${String(usage.modules)}, data overlays=${String(overlays.length)}, lazy archives=${String(manifest.lazy.length)}, direct lane=connection.createSharedFetchHandler, als causality=${options.alsCausality === void 0 ? "inert" : "snapshot/restore"}, image lowering=${LOWERING_VERSION})`);
			const peer = connection.browserPeer;
			tunnel.serve({
				directFetch: (request) => handler.fetch(request, peer),
				bootPayload: () => readBootPayload(ctx, peer),
				packageAssets: () => packPackageAssets(mounted, root),
				openStream: (endpoint, payload, uplink, signal) => typertGateway.wireStream.open(endpoint, payload, uplink, peer, signal),
				streamFailure: typertGateway.wireStream.failure
			});
			started.resolve();
		} catch (reason) {
			tunnel.fail(reason);
			throw reason;
		}
	};
	return {
		handleMessage: (data) => {
			tunnel.handleMessage(data);
		},
		start,
		stop: async () => {
			tunnel.fail(/* @__PURE__ */ new Error("webworker host: the tree was disposed"));
			await context?.fiber.dispose();
		},
		get vfs() {
			return vfs;
		},
		get modules() {
			return modules;
		}
	};
}
/**
* Send the tree's own warnings and errors to the worker console.
*
* Cordis's `LoggerService` always exists and always accepts messages, but with
* no exporter mounted it only fills a ring buffer — and no profile in this
* repository mounts one, so `ctx.logger.warn(...)` reaches nothing. A provider
* that fails and is skipped (the skill registry logs exactly that) then looks
* identical to one that found nothing, which is how an empty skill catalog hid a
* filesystem fault twice.
*
* Warnings and errors only: `info`/`debug` from 131 plugin rows would bury the
* page console, and this exists to make failures visible rather than to trace.
* @param ctx - Host context, before any entry mounts.
* @param require - Image resolver, for cordis's own message renderer.
*/
function installLogSink(ctx, require) {
	const { Logger } = require("@deepseek-ai/cordis");
	const exporter = {
		colors: false,
		levels: { default: 2 },
		export: (message) => {
			if (message.type !== "warn" && message.type !== "error") return;
			const line = `${message.name}: ${Logger.format(exporter, message)}`;
			if (message.type === "error") console.error(line);
			else console.warn(line);
		}
	};
	ctx.logger.exporter(exporter);
}
/** Diagnostic prefix the launcher passes to app-boot; the Worker boots the same profile code. */
const BIN_NAME = "dsh";
/**
* Read and validate the mounted image's manifest.
*
* The manifest the packer writes is the single source of truth: the worker holds
* no transform, so an image that was never lowered — or was lowered against
* different wrapper semantics — cannot be recovered at load and must be rebuilt.
* @param vfs - Mounted filesystem.
* @param path - Manifest path inside the image.
* @returns The validated manifest.
* @throws When the manifest is missing, malformed, or names another contract.
*/
function readImageManifest(vfs, path) {
	if (!vfs.existsSync(path)) throw new Error(`webworker host: ${path} is missing, so the image records no lowering; rebuild the image`);
	return parseImageManifest(JSON.parse(vfs.readFileSync(path, "utf8")), path);
}
/**
* Register the manifest's lazy archives; each is fetched beside the image on
* the first asynchronous read of a file it carries.
* @param vfs - Mounted filesystem.
* @param manifest - Validated image manifest.
* @param root - Virtual root the archives' entries mount under.
* @param image - The image bytes or URL; archive names resolve against a URL.
* @throws When the image registers lazy archives but arrived as bytes, leaving them unlocatable.
*/
function deferLazyArchives(vfs, manifest, root, image) {
	if (manifest.lazy.length === 0) return;
	if (typeof image !== "string") throw new Error("webworker host: the image registers lazy archives, which only an image fetched from a URL can locate");
	const base = new URL(image, globalThis.location?.href);
	for (const archive of manifest.lazy) {
		const url = new URL(archive.archive, base).href;
		vfs.deferArchive({
			name: archive.archive,
			root,
			files: archive.files,
			load: () => readImage(url)
		});
	}
}
/**
* Prepare the profile the served launcher would boot: the image's app-boot
* initializes the manifest's shipped profile template under the Worker home
* and composes its bundle layers, user layer, and home layer. The profile
* context carries the launcher's facts with no command-line overlay; the
* image's home patch, written by the packer, carries the Worker's required
* row config (`PLATFORM_ENTRY_CONFIG` in `image-layout.ts`).
* @param appBoot - The image's app-boot.
* @param vfs - Filesystem holding the profile.
* @param manifest - Validated image manifest naming the profile and installation.
* @param root - Virtual root, the process working directory.
* @param env - The process shim's environment, after the launch layers applied.
* @returns Effective boot patches, the profile context, and the launch environment snapshot.
* @throws When the image names no installation manifest to anchor bundle resolution.
*/
function prepareProfile(appBoot, vfs, manifest, root, env) {
	if (manifest.installation === void 0) throw new Error("webworker host: the image names no installation manifest, so no profile bundle can resolve; rebuild the image");
	const home = join(root, IMAGE_HOME_DIRECTORY);
	const installAnchor = join(root, "node_modules", manifest.installation, "package.json");
	const environment = appBoot.loadLayeredEnv(BIN_NAME, root);
	const loaded = appBoot.loadProfile(BIN_NAME, manifest.profile, installAnchor, home);
	appBoot.reportSkippedBundles(BIN_NAME, loaded);
	vfs.writeFileSync(join(loaded.dir, "cordis.yml"), "[]\n");
	const profile = {
		name: loaded.name,
		dir: loaded.dir,
		patchPath: loaded.patchPath,
		installAnchor,
		cwd: root,
		home,
		startedBundles: loaded.layers.map((layer) => layer.packageName),
		overlays: [],
		telemetryDisabledEnv: env.DSH_TELEMETRY_DISABLED
	};
	return {
		patches: appBoot.readProfilePatches(BIN_NAME, profile, loaded),
		profile,
		environment
	};
}
/**
* Assemble the payload the page's pre-Cordis bootstrap needs: the structured
* index injection table the served form renders into index.html. Collected
* from the in-process webserver service, never from the API surface, because
* the page has no Cordis tree yet.
* @param ctx - Booted host context.
* @param peer - the browser Peer the page speaks for.
* @returns Boot payload for `GET /__boot__`.
*/
function readBootPayload(ctx, peer) {
	const webServer = ctx.get("webServer");
	if (webServer === void 0) throw new Error("webworker host: no webServer service, so the page cannot receive its boot injections");
	return { injections: webServer.collectIndexInjections(peer) };
}
// src/fixture-manifest.ts
/** Browser-readable catalog of built-in Preview filesystem overlays. */
/** Manifest format version emitted beside the base VFS image. */
const PREVIEW_FIXTURE_MANIFEST_VERSION = 1;
/** Leaf name resolved beside the base image. */
const PREVIEW_FIXTURE_MANIFEST_FILE = "fixtures.json";
function recordOf(value) {
	return typeof value === "object" && value !== null && !Array.isArray(value) ? value : void 0;
}
/**
* Validate the static fixture catalog before it controls Worker fetches.
* @param value - Parsed JSON response.
* @returns A detached manifest with unique ids and non-empty overlay lists.
*/
function parsePreviewFixtureManifest(value) {
	const record = recordOf(value);
	if (record?.version !== 1 || !Array.isArray(record.fixtures)) throw new Error(`preview fixture manifest must use version ${String(1)}`);
	const fixtures = [];
	const ids = /* @__PURE__ */ new Set();
	for (const value of record.fixtures) {
		const fixture = recordOf(value);
		const id = fixture?.id;
		const label = fixture?.label;
		const description = fixture?.description;
		const overlays = fixture?.overlays;
		const overlayUrls = Array.isArray(overlays) ? overlays.filter((overlay) => typeof overlay === "string" && overlay.length > 0) : [];
		if (typeof id !== "string" || !/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(id) || id === "none" || id === "webfs" || typeof label !== "string" || label.length === 0 || typeof description !== "string" || description.length === 0 || !Array.isArray(overlays) || overlays.length === 0 || overlayUrls.length !== overlays.length) throw new Error("preview fixture manifest contains an invalid fixture entry");
		if (ids.has(id)) throw new Error(`preview fixture manifest repeats id "${id}"`);
		ids.add(id);
		fixtures.push({
			id,
			label,
			description,
			overlays: overlayUrls
		});
	}
	const defaultFixture = record.defaultFixture;
	if (defaultFixture !== null && (typeof defaultFixture !== "string" || !ids.has(defaultFixture))) throw new Error("preview fixture manifest defaultFixture does not name a fixture");
	return {
		version: 1,
		defaultFixture,
		fixtures
	};
}
export { API_PREFIX, DEFAULT_CONDITIONS, DEFAULT_ROOT, IMAGE_EMPTY_DIRECTORIES, IMAGE_FILE_NAME, IMAGE_HOME_DIRECTORY, IMAGE_LAZY_DIRECTORY, IMAGE_MANIFEST_PATH, IMAGE_OVERLAY_DIRECTORIES, LOWERING_VERSION, MODULE_PROXIES, MODULE_PROXY_PREFIXES, MemoryVfs, PLATFORM_ENTRY_CONFIG, PREVIEW_FIXTURE_MANIFEST_FILE, PREVIEW_FIXTURE_MANIFEST_VERSION, REPLACED_EXTERNAL_PACKAGES, SYNTHETIC_HOST, TunnelServer, WRAPPER_PARAMS, WorkerModuleLoader, createAlsRuntime, createSyntheticExchange, createWorkerHost, inflateImage, inflateImageStream, installProcessGlobal, loadVfsImage, loadVfsOverlay, lowerModuleSource, packTar, parseImageManifest, parseInboundFrame, parsePreviewFixtureManifest, parseTar, posix_path_exports as posixPath, requireActiveModuleLoader, requireActiveVfs, setActiveModuleLoader, setActiveVfs };
