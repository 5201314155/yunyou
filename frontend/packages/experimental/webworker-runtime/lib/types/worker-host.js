/**
 * Worker assembly entry: the whole harness Cordis tree inside one dedicated
 * Web Worker.
 *
 * Every platform object arrives through options — the `node:*` proxy table, the
 * request listener the app's fake `node:http` captured, the image bytes — so this
 * package never reaches back into the application that composes it. **Platform
 * readiness before the call is the caller's responsibility**: anything the
 * proxies need initialized (the zstd WebAssembly module, for one) must be ready
 * before {@link startWorkerHost} runs.
 *
 * Construction is split in two on purpose. {@link createWorkerHost} is
 * synchronous so the worker can accept messages and queue requests that arrive
 * during boot; {@link WorkerHost.start} then mounts the image, the module
 * loader, and the tree. {@link startWorkerHost} performs both and installs the
 * message handler before its first await.
 *
 * The tree itself boots through the host's own `boot()` glue loaded from the
 * image, over the profile the image's own app-boot initializes from the shipped
 * template and composes from the bundles the image carries, so profile
 * composition, entry mounting, the activation audit, and its diagnostics are the
 * same code the Node deployment runs. The Worker supplies module loading, the
 * platform overlay, and the command line.
 * @module @deepseek-ai/dsh-experimental-webworker-runtime/src/worker-host
 */
import { setActiveModuleLoader, WorkerModuleLoader } from "./module-system/module-loader.js";
import { join } from "./module-system/posix-path.js";
import { installProcessGlobal } from "./node/globals/process.js";
import { TunnelServer } from "./transport/tunnel.js";
import { inflateImage, inflateImageStream } from "./storage/image-gzip.js";
import { loadVfsImage, loadVfsOverlay } from "./storage/memory.js";
import { setActiveVfs } from "./storage/active.js";
import { setTextViewer } from "./shell/process/xdg-open.js";
import { packPackageAssets } from "./package-assets.js";
import { DEFAULT_ROOT, IMAGE_EMPTY_DIRECTORIES, IMAGE_HOME_DIRECTORY, IMAGE_MANIFEST_PATH, LOWERING_VERSION, } from "./image-layout.js";
import { parseImageManifest } from "./image-manifest.js";
export { DEFAULT_ROOT } from "./image-layout.js";
/** Port reported to the tree when the caller names none; the bind is fake either way. */
export const DEFAULT_PORT = 3080;
function requireGlobalPort(channel) {
    if (channel !== undefined)
        return channel;
    const scope = globalThis;
    const post = scope.postMessage;
    if (typeof post !== 'function') {
        throw new Error('webworker host: no channel; pass options.channel outside a dedicated worker');
    }
    return { postMessage: (message, transfer) => { post(message, transfer); } };
}
async function readImage(image) {
    if (typeof image !== 'string')
        return await inflateImage(image, 'the image bytes given to createWorkerHost');
    const response = await fetch(image);
    if (!response.ok)
        throw new Error(`webworker host: image fetch failed with ${String(response.status)} for ${image}`);
    if (response.body === null)
        throw new Error(`webworker host: image response for ${image} carried no body`);
    // Inflated off the response stream: the archive is built while the rest of the
    // image is still arriving.
    return await inflateImageStream(response.body, image);
}
/**
 * Build the worker host without touching the network or the image.
 * @param options - Assembly inputs.
 * @returns Handle whose `handleMessage` is ready immediately.
 */
export function createWorkerHost(options) {
    const root = options.root ?? DEFAULT_ROOT;
    const port = options.port ?? DEFAULT_PORT;
    const tunnel = new TunnelServer({
        port: requireGlobalPort(options.channel),
        requestListener: options.requestListener,
        ...options.privilegedMethods === undefined ? {} : { privilegedMethods: options.privilegedMethods },
        ...options.unaryApiLane === undefined ? {} : { unaryApiLane: options.unaryApiLane },
    });
    setTextViewer((path, text) => { tunnel.viewText(path, text); });
    let vfs;
    let modules;
    let context;
    const start = async () => {
        try {
            const home = join(root, IMAGE_HOME_DIRECTORY);
            const processShim = installProcessGlobal({ cwd: root, env: { DSH_HOME: home, HOME: home, ...options.env } });
            const [bytes, overlays] = await Promise.all([
                readImage(options.image),
                Promise.all((options.overlays ?? []).map(readImage)),
            ]);
            const mounted = loadVfsImage(bytes, root);
            for (const overlay of overlays)
                loadVfsOverlay(overlay, root, mounted);
            // Belt and braces over the image's own empty-directory entries: a hand
            // -built image without them still boots.
            for (const directory of IMAGE_EMPTY_DIRECTORIES) {
                mounted.seedDirectory(join(root, directory.replace(/\/$/, '')));
            }
            setActiveVfs(mounted);
            vfs = mounted;
            const manifestPath = options.manifestPath ?? join(root, IMAGE_MANIFEST_PATH);
            const manifest = readImageManifest(mounted, manifestPath);
            deferLazyArchives(mounted, manifest, root, options.image);
            const staticModules = { ...options.staticModules };
            // Read at require time, not here: the table entry then answers whichever
            // global `installProcessGlobal` left in place, in this role's order.
            for (const key of ['node:process', 'process']) {
                staticModules[key] ??= () => globalThis.process;
            }
            const loader = new WorkerModuleLoader({
                vfs: mounted,
                root,
                staticModules,
                ...options.staticModulePrefixes === undefined ? {} : { staticModulePrefixes: options.staticModulePrefixes },
                ...options.alsCausality === undefined ? {} : { alsCausality: options.alsCausality },
            });
            setActiveModuleLoader(loader);
            modules = loader;
            const require = loader.requireFrom(root);
            const appBoot = require('@deepseek-ai/dsh-app-boot');
            const { DSH_LAUNCH_ENVIRONMENT_KEY } = require('@deepseek-ai/dsh-launch-environment');
            const cmdline = require('@deepseek-ai/dsh-cmdline');
            // Resolved once the tunnel serves; a failed start never resolves it.
            const started = Promise.withResolvers();
            const { patches, profile, environment } = prepareProfile(appBoot, mounted, manifest, root, processShim.env);
            const profileConfig = join(profile.dir, 'cordis.yml');
            const ctx = await appBoot.boot(BIN_NAME, profileConfig, patches, async (hostCtx) => {
                hostCtx.provide('profileContext', profile);
                hostCtx.provide(DSH_LAUNCH_ENVIRONMENT_KEY, environment);
                installLogSink(hostCtx, require);
                // Package lookup without a runtime resolution: the Worker loader owns
                // module resolution, so only manifest lookup over the image applies.
                await hostCtx.plugin(appBoot.PluginPackages, {});
                cmdline.provideCmdline(hostCtx, {
                    args: [...(options.cmdlineArgs ?? ['--host', '127.0.0.1', '--port', String(port), '--no-open'])],
                    exit: (code) => { console.warn(`webworker host: tree requested exit(${String(code)})`); },
                    ready: {
                        onReady: (listener) => {
                            let pending = true;
                            void started.promise.then(() => { if (pending)
                                listener(); });
                            return () => { pending = false; };
                        },
                    },
                });
            });
            context = ctx;
            const connection = ctx.get('connection');
            if (connection === undefined)
                throw new Error('webworker host: the tree activated without a Connection service');
            const typertGateway = ctx.get('typertGateway');
            if (typertGateway === undefined) {
                throw new Error('webworker host: the tree activated without a typertGateway service');
            }
            const handler = connection.createSharedFetchHandler('/api');
            const usage = loader.usage();
            console.info(`webworker host: tree active (modules=${String(usage.modules)}, data overlays=${String(overlays.length)}, lazy archives=${String(manifest.lazy.length)}, direct lane=connection.createSharedFetchHandler, als causality=${options.alsCausality === undefined ? 'inert' : 'snapshot/restore'}, image lowering=${LOWERING_VERSION})`);
            // The worker tree serves only the page that spawned it, so every call speaks for the browser Peer.
            const peer = connection.browserPeer;
            tunnel.serve({
                directFetch: (request) => handler.fetch(request, peer),
                bootPayload: () => readBootPayload(ctx, peer),
                packageAssets: () => packPackageAssets(mounted, root),
                openStream: (endpoint, payload, uplink, signal) => typertGateway.wireStream.open(endpoint, payload, uplink, peer, signal),
                streamFailure: typertGateway.wireStream.failure,
            });
            started.resolve();
        }
        catch (reason) {
            tunnel.fail(reason);
            throw reason;
        }
    };
    return {
        handleMessage: (data) => { tunnel.handleMessage(data); },
        start,
        stop: async () => {
            tunnel.fail(new Error('webworker host: the tree was disposed'));
            await context?.fiber.dispose();
        },
        get vfs() {
            return vfs;
        },
        get modules() {
            return modules;
        },
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
export function installLogSink(ctx, require) {
    const { Logger } = require('@deepseek-ai/cordis');
    const exporter = {
        colors: false,
        // cordis compares `exporter.levels ?? logger.level ?? INFO` against the
        // message level and drops anything higher, and its scale counts UP with
        // verbosity (ERROR 0, INFO 1, WARN 2, DEBUG 3). An exporter that declares no
        // level therefore admits errors and info but silently drops every warning —
        // which is what the built-in ring-buffer exporter does, so the skipped-provider
        // warning this sink exists for never even reached the buffer.
        levels: { default: 2 },
        export: (message) => {
            if (message.type !== 'warn' && message.type !== 'error')
                return;
            const line = `${message.name}: ${Logger.format(exporter, message)}`;
            if (message.type === 'error')
                console.error(line);
            else
                console.warn(line);
        },
    };
    ctx.logger.exporter(exporter);
}
/** Diagnostic prefix the launcher passes to app-boot; the Worker boots the same profile code. */
const BIN_NAME = 'dsh';
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
    if (!vfs.existsSync(path)) {
        throw new Error(`webworker host: ${path} is missing, so the image records no lowering; rebuild the image`);
    }
    return parseImageManifest(JSON.parse(vfs.readFileSync(path, 'utf8')), path);
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
    if (manifest.lazy.length === 0)
        return;
    if (typeof image !== 'string') {
        throw new Error('webworker host: the image registers lazy archives, which only an image fetched from a URL can locate');
    }
    const base = new URL(image, globalThis.location?.href);
    for (const archive of manifest.lazy) {
        const url = new URL(archive.archive, base).href;
        vfs.deferArchive({ name: archive.archive, root, files: archive.files, load: () => readImage(url) });
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
    if (manifest.installation === undefined) {
        throw new Error('webworker host: the image names no installation manifest, so no profile bundle can resolve; rebuild the image');
    }
    const home = join(root, IMAGE_HOME_DIRECTORY);
    const installAnchor = join(root, 'node_modules', manifest.installation, 'package.json');
    const environment = appBoot.loadLayeredEnv(BIN_NAME, root);
    const loaded = appBoot.loadProfile(BIN_NAME, manifest.profile, installAnchor, home);
    appBoot.reportSkippedBundles(BIN_NAME, loaded);
    // The empty root list the include anchors at; the launcher rewrites it on every boot.
    vfs.writeFileSync(join(loaded.dir, 'cordis.yml'), '[]\n');
    const profile = {
        name: loaded.name, dir: loaded.dir, patchPath: loaded.patchPath, installAnchor, cwd: root, home,
        startedBundles: loaded.layers.map(layer => layer.packageName),
        overlays: [], telemetryDisabledEnv: env.DSH_TELEMETRY_DISABLED,
    };
    return { patches: appBoot.readProfilePatches(BIN_NAME, profile, loaded), profile, environment };
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
    const webServer = ctx.get('webServer');
    if (webServer === undefined) {
        throw new Error('webworker host: no webServer service, so the page cannot receive its boot injections');
    }
    return { injections: webServer.collectIndexInjections(peer) };
}
/**
 * Install the message handler and boot the tree.
 *
 * The handler is attached before the first await, so requests that arrive
 * during boot queue instead of being dropped. A boot failure refuses the queue
 * with 503 and rejects.
 * @param options - Assembly inputs; `channel` also replaces the message source.
 * @returns Resolves once the tunnel is serving.
 */
export async function startWorkerHost(options) {
    const host = createWorkerHost(options);
    if (options.channel === undefined) {
        const scope = globalThis;
        if (typeof scope.addEventListener !== 'function') {
            throw new Error('webworker host: no message source; pass options.channel outside a dedicated worker');
        }
        scope.addEventListener('message', (event) => { host.handleMessage(event.data); });
    }
    await host.start();
}
//# sourceMappingURL=worker-host.js.map