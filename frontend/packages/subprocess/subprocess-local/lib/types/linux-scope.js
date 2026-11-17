/** Linux user-systemd scope launch and managed-range ownership. */
import { controlPipe } from "./control-spawn.js";
import { execFile, spawn, spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as sleepMs } from 'node:timers/promises';
import { loadLinuxExecve } from "./linux-execve.js";
import { cleanupLinuxLaunchFiles, createLinuxLaunchFiles, deserializeRunnerError, readLinuxStartupError, } from "./runner-protocol.js";
import { runnerEnvironment, runnerInvocationAvailable, runnerStdio, spawnRunnerInvocation, } from "./runner-launch.js";
import { childEnv } from "./spawn.js";
const SYSTEMCTL_TIMEOUT_MS = 5_000;
const SCOPE_INITIAL_POLL_INTERVAL_MS = 50;
const MISSING_UNIT = /\bunit\b[^\r\n]*(?:could not be found|not found|not loaded)/iu;
const LITERAL_ARGV_OPTION = '--expand-environment=no';
// systemd-run before v254 rejects the option; its scope mode passes argv to execvpe unexpanded.
const LITERAL_ARGV_OPTION_REJECTED = /unrecognized option\b.*expand-environment/u;
// The spelling before systemctl v252 and an unambiguous prefix of the later `--kill-whom`.
const KILL_ALL_MEMBERS = '--kill-who=all';
/** Flags bit that /proc/net/unix sets on a listening socket (__SO_ACCEPTCON). */
const UNIX_SOCKET_ACCEPTING = 0x10000;
function managerEnvironment() {
    const environment = childEnv({ LC_ALL: 'C' });
    delete environment.SYSTEMD_LOG_TARGET;
    return environment;
}
function quietSystemdEnvironment() {
    return childEnv({ LC_ALL: 'C', SYSTEMD_LOG_TARGET: 'null' });
}
function querySystemctl(command, args) {
    return new Promise((resolveResult) => {
        execFile(command, [...args], {
            encoding: 'utf8',
            env: managerEnvironment(),
            timeout: SYSTEMCTL_TIMEOUT_MS,
        }, (error, stdout, stderr) => {
            const code = error === null ? 0 : error.code;
            resolveResult({
                status: typeof code === 'number' ? code : null,
                stdout,
                stderr,
                ...error === null ? {} : { error },
            });
        });
    });
}
function unitStem(prefix) {
    return `${prefix}-${String(process.pid)}-${randomBytes(6).toString('hex')}`;
}
function sleepWithAbort(delayMs, signal) {
    return sleepMs(delayMs, undefined, { signal });
}
/**
 * Confirm this exact runner entry and libc execve binding without a probe mode.
 * @param internals - optional runner and libc-binding seams used by tests.
 * @returns whether the bootstrap can enter the final target.
 */
export function probeLinuxBootstrap(internals = {}) {
    try {
        ;
        (internals.loadLinuxExecve ?? loadLinuxExecve)();
        const invocation = internals.runnerInvocation
            ?? (internals.resolveRunnerInvocation ?? spawnRunnerInvocation)();
        return (internals.runnerAvailable ?? runnerInvocationAvailable)(invocation);
    }
    catch {
        return false;
    }
}
/**
 * Confirm current literal-argv transient-scope support before selecting native launch.
 * @param internals - optional systemd command seams used by tests.
 * @returns the options that keep target argv literal, or undefined when the user manager cannot create the scope.
 */
export function probeLinuxScope(internals = {}) {
    const attempt = (literalArgv) => {
        const unitBase = unitStem('dsh-subprocess-probe');
        const result = (internals.spawnSync ?? spawnSync)(internals.systemdRun ?? 'systemd-run', [
            '--user',
            '--scope',
            '--quiet',
            '--collect',
            ...literalArgv,
            `--unit=${unitBase}`,
            '--',
            internals.systemctl ?? 'systemctl',
            '--user',
            'show',
            `${unitBase}.scope`,
            '--property=ActiveState',
            '--value',
        ], {
            encoding: 'utf8',
            env: quietSystemdEnvironment(),
            stdio: ['ignore', 'ignore', 'pipe'],
            timeout: SYSTEMCTL_TIMEOUT_MS,
        });
        return { created: result.error === undefined && result.status === 0, stderr: result.stderr };
    };
    const current = attempt([LITERAL_ARGV_OPTION]);
    if (current.created)
        return [LITERAL_ARGV_OPTION];
    if (!LITERAL_ARGV_OPTION_REJECTED.test(current.stderr))
        return undefined;
    return attempt([]).created ? [] : undefined;
}
/**
 * Check every Linux native prerequisite on each call.
 * @param internals - optional native capability seams used by tests.
 * @returns the literal-argv scope options when Linux native containment is available, otherwise undefined.
 */
export function probeLinuxNative(internals = {}) {
    return probeLinuxBootstrap(internals) ? probeLinuxScope(internals) : undefined;
}
/**
 * Query the user manager once through systemctl.
 * @param internals - optional systemctl seam used by tests.
 * @returns whether the query succeeds.
 */
export function probeLinuxManager(internals = {}) {
    const result = (internals.spawnSync ?? spawnSync)(internals.systemctl ?? 'systemctl', [
        '--user',
        'show',
        '--property=Version',
        '--value',
    ], { env: managerEnvironment(), stdio: 'ignore', timeout: SYSTEMCTL_TIMEOUT_MS });
    return result.error === undefined && result.status === 0;
}
/**
 * Find the socket a process in this network namespace listens on at path. The kernel lists a
 * bound socket only while a process holds it open, so a socket file that a stopped or crashed
 * listener left behind does not count.
 */
function unixSocketListener(path, table) {
    let text;
    try {
        text = readFileSync(table, 'utf8');
    }
    catch (_unreadableSocketTable) {
        // Without the socket table nothing is known, so callers fall back to systemctl or keep their verdict.
        return undefined;
    }
    const suffix = ` ${path}`;
    for (const line of text.split('\n')) {
        if (!line.endsWith(suffix))
            continue;
        const fields = line.trim().split(/\s+/u);
        if (fields.slice(7).join(' ') === path && (Number.parseInt(String(fields[3]), 16) & UNIX_SOCKET_ACCEPTING) !== 0) {
            return String(fields[6]);
        }
    }
    return undefined;
}
/**
 * Identify the user manager's listener on `$XDG_RUNTIME_DIR/systemd/private`, the socket
 * `systemd-run --user` connects through.
 * @param internals - optional socket-table seam used by tests.
 * @returns the listening socket's inode from the kernel socket table, or undefined when
 *   XDG_RUNTIME_DIR is unset or nothing in this network namespace listens there.
 */
export function linuxManagerListener(internals = {}) {
    const runtimeDirectory = childEnv().XDG_RUNTIME_DIR;
    if (runtimeDirectory === undefined || runtimeDirectory === '')
        return undefined;
    return unixSocketListener(join(runtimeDirectory, 'systemd', 'private'), internals.unixSocketTable ?? '/proc/net/unix');
}
/**
 * Identify the systemd-run binary that scope launches resolve through PATH, so a package upgrade
 * that replaces it triggers another probe of its literal-argv options.
 * @param internals - optional systemd-run seam used by tests.
 * @returns the device, inode, size, and modification time of the first match, or undefined when none resolves.
 */
export function systemdRunIdentity(internals = {}) {
    const command = internals.systemdRun ?? 'systemd-run';
    const candidates = command.includes('/')
        ? [command]
        : (childEnv().PATH ?? '').split(':').filter(directory => directory !== '').map(directory => join(directory, command));
    for (const candidate of candidates) {
        let stats;
        try {
            stats = statSync(candidate, { throwIfNoEntry: false });
        }
        catch (_unreadableCandidate) {
            // A PATH entry that is not a searchable directory cannot hold systemd-run.
            continue;
        }
        if (stats?.isFile() === true)
            return `${String(stats.dev)}:${String(stats.ino)}:${String(stats.size)}:${String(stats.mtimeMs)}`;
    }
    return undefined;
}
/**
 * Confirm the user manager before a scope launch. A listener on its private socket confirms it
 * without a child process; otherwise one systemctl query decides. A hung manager that still
 * listens passes, and its scope launch fails with LinuxScopeNotStartedError.
 * @param internals - optional socket-table and systemctl seams used by tests.
 * @returns whether a scope launch can reach the user manager.
 */
export function confirmLinuxManager(internals = {}) {
    return linuxManagerListener(internals) !== undefined || probeLinuxManager(internals);
}
/** A scope launch whose bootstrap never consumed its launch request, so its target never ran. */
export class LinuxScopeNotStartedError extends Error {
    name = 'LinuxScopeNotStartedError';
}
class LinuxScopeStartup {
    files;
    kind;
    terminationSignals = new Set();
    constructor(files, kind) {
        this.files = files;
        this.kind = kind;
    }
    resolveOutcome(outcome) {
        const startup = readLinuxStartupError(this.files.startupErrorPath);
        if (startup !== undefined)
            throw deserializeRunnerError(startup.error);
        if (existsSync(this.files.requestPath)
            && !(outcome.signal !== null && this.terminationSignals.has(outcome.signal))) {
            throw new LinuxScopeNotStartedError(`${this.kind} scope exited before its bootstrap consumed the launch request; the provider probes native containment again before its next command`);
        }
        return outcome;
    }
}
class SystemdScopeOwner {
    unit;
    startup;
    direct;
    systemctl;
    runSync;
    query;
    sleep;
    establishment = 'pending';
    stopped = false;
    terminationRequested = false;
    observation;
    killFailure;
    directKillSettlement;
    wakeGeneration = 0;
    wakeWaiter;
    constructor(unit, startup, direct, systemctl, runSync, query, sleep) {
        this.unit = unit;
        this.startup = startup;
        this.direct = direct;
        this.systemctl = systemctl;
        this.runSync = runSync;
        this.query = query;
        this.sleep = sleep;
    }
    inspectTaskCount() {
        const result = this.runSync(this.systemctl, [
            '--user', 'show', '--property=LoadState', '--property=ActiveState', '--property=TasksCurrent', this.unit,
        ], { encoding: 'utf8', env: managerEnvironment(), timeout: SYSTEMCTL_TIMEOUT_MS });
        if (result.error !== undefined || result.status !== 0 || typeof result.stdout !== 'string')
            return undefined;
        const state = this.parseUnitState(result.stdout);
        return state.loadState === 'loaded' && state.activeState === 'active' ? state.tasksCurrent : undefined;
    }
    signal(signal) {
        if (this.stopped)
            return;
        this.terminationRequested = true;
        if (this.direct.running())
            this.startup.terminationSignals.add(signal);
        this.observeRequestConsumption();
        const directFallbackRequired = this.establishment === 'pending';
        let directSignalled = false;
        if (directFallbackRequired && this.direct.running())
            directSignalled = this.direct.signal(signal);
        const result = this.runSync(this.systemctl, [
            '--user',
            'kill',
            KILL_ALL_MEMBERS,
            `--signal=${signal}`,
            this.unit,
        ], { encoding: 'utf8', env: managerEnvironment(), timeout: SYSTEMCTL_TIMEOUT_MS });
        this.wakeObservation();
        if (result.error === undefined && result.status === 0) {
            if (signal === 'SIGKILL') {
                this.killFailure = undefined;
                this.directKillSettlement = undefined;
            }
            return;
        }
        if (!directFallbackRequired && this.direct.running())
            directSignalled = this.direct.signal(signal);
        if (signal === 'SIGKILL') {
            const output = `${result.stdout}\n${result.stderr}`;
            if (!MISSING_UNIT.test(output)) {
                this.killFailure = result.error ?? new Error(`systemctl could not signal ${this.unit}: ${output.trim() || `exit ${String(result.status)}`}`);
                // The direct outcome retains errors; this barrier only joins its physical settlement.
                this.directKillSettlement = directSignalled
                    ? this.direct.settled.then(() => { }, () => { })
                    : undefined;
            }
        }
    }
    terminateForHostExit() {
        if (this.stopped)
            return;
        try {
            if (this.direct.running())
                this.direct.signal('SIGKILL');
        }
        catch { /* Continue with the native owner. */ }
        try {
            this.runSync(this.systemctl, [
                '--user',
                'kill',
                KILL_ALL_MEMBERS,
                '--signal=SIGKILL',
                this.unit,
            ], { env: managerEnvironment(), stdio: 'ignore', timeout: SYSTEMCTL_TIMEOUT_MS });
        }
        catch {
            // Host exit cannot report one range; the runtime continues with the rest.
        }
    }
    observeRequestConsumption() {
        if (this.establishment === 'pending' && !existsSync(this.startup.files.requestPath)) {
            this.establishment = 'established';
        }
    }
    absentUnit() {
        this.observeRequestConsumption();
        if (this.establishment === 'established')
            return false;
        if (!this.direct.running() && existsSync(this.startup.files.requestPath)) {
            return false;
        }
        if (this.killFailure !== undefined)
            throw this.killFailure;
        return true;
    }
    /**
     * Prove an active unit with no processes is the empty managed range rather
     * than a launch still placing its payload. systemd ends a scope only on the
     * populated-to-empty transition, so a payload killed before it entered the
     * cgroup leaves the unit active forever. A departed client cannot add another
     * payload; a consumed request proves the payload already entered the scope,
     * even while its direct-process exit notification is pending.
     */
    emptyRange(tasksCurrent) {
        return this.terminationRequested && tasksCurrent === 0
            && (!this.direct.running() || !existsSync(this.startup.files.requestPath));
    }
    /** Release a leftover empty scope so the transient unit is collected and cannot accumulate. */
    releaseEmptyRange() {
        try {
            this.runSync(this.systemctl, ['--user', 'stop', this.unit], {
                env: managerEnvironment(),
                stdio: 'ignore',
                timeout: SYSTEMCTL_TIMEOUT_MS,
            });
        }
        catch {
            // The range is already empty; a failed cleanup leaves only the transient unit.
        }
    }
    parseUnitState(stdout) {
        const values = new Map();
        for (const line of stdout.split(/\r?\n/u)) {
            if (line === '')
                continue;
            const separator = line.indexOf('=');
            if (separator <= 0) {
                throw new Error(`systemctl returned malformed state for ${this.unit}: ${JSON.stringify(stdout.trim())}`);
            }
            const name = line.slice(0, separator);
            if (values.has(name)) {
                throw new Error(`systemctl returned duplicate ${name} for ${this.unit}`);
            }
            values.set(name, line.slice(separator + 1));
        }
        const loadState = values.get('LoadState');
        const activeState = values.get('ActiveState');
        // The manager prints this sentinel for a property the unit does not carry.
        const reportedTasks = values.get('TasksCurrent');
        const tasksCurrent = reportedTasks === '[not set]' ? undefined : reportedTasks;
        if (values.size !== (reportedTasks === undefined ? 2 : 3)
            || loadState === undefined || activeState === undefined) {
            throw new Error(`systemctl returned incomplete state for ${this.unit}: ${JSON.stringify(stdout.trim())}`);
        }
        if (tasksCurrent !== undefined && !/^\d+$/u.test(tasksCurrent)) {
            throw new Error(`systemctl returned a non-numeric TasksCurrent for ${this.unit}: ${JSON.stringify(tasksCurrent)}`);
        }
        return {
            loadState,
            activeState,
            tasksCurrent: tasksCurrent === undefined ? undefined : Number(tasksCurrent),
        };
    }
    async rangeActive() {
        this.observeRequestConsumption();
        const generation = this.wakeGeneration;
        const directRunning = this.direct.running();
        const result = await this.query(this.systemctl, [
            '--user',
            'show',
            this.unit,
            '--property=LoadState',
            '--property=ActiveState',
            '--property=TasksCurrent',
        ]);
        // A signal invalidates state queried before its delivery and direct fallback.
        if (generation !== this.wakeGeneration)
            return true;
        const output = `${result.stdout}\n${result.stderr}`;
        if (result.status === 0) {
            const { loadState, activeState, tasksCurrent } = this.parseUnitState(result.stdout);
            if (loadState === 'not-found' && activeState === 'inactive')
                return this.absentUnit();
            if (loadState !== 'loaded') {
                throw new Error(`systemctl returned unknown state for ${this.unit}: ${JSON.stringify({ loadState, activeState })}`);
            }
            this.establishment = 'established';
            if (activeState === 'inactive' || activeState === 'failed')
                return false;
            if (!['active', 'activating', 'reloading', 'deactivating'].includes(activeState)) {
                throw new Error(`systemctl returned unknown ActiveState for ${this.unit}: ${JSON.stringify(activeState)}`);
            }
            if (this.emptyRange(tasksCurrent)) {
                this.releaseEmptyRange();
                return false;
            }
            if (this.killFailure !== undefined) {
                if (directRunning && this.directKillSettlement !== undefined) {
                    const settlement = this.directKillSettlement;
                    this.directKillSettlement = undefined;
                    await settlement;
                    // A query preceding direct exit cannot prove that its signalled processes survived.
                    return this.rangeActive();
                }
                throw this.killFailure;
            }
            return true;
        }
        if (!MISSING_UNIT.test(output)) {
            if (result.error !== undefined)
                throw result.error;
            throw new Error(`systemctl could not read ${this.unit}: ${output.trim() || `exit ${String(result.status)}`}`);
        }
        return this.absentUnit();
    }
    wakeObservation() {
        this.wakeGeneration += 1;
        this.wakeWaiter?.resolve();
        this.wakeWaiter = undefined;
    }
    async waitForPoll(delayMs, generation) {
        if (generation !== this.wakeGeneration)
            return;
        const wake = Promise.withResolvers();
        const waiter = { generation, resolve: wake.resolve };
        const sleepController = new AbortController();
        this.wakeWaiter = waiter;
        try {
            await Promise.race([this.sleep(delayMs, sleepController.signal), wake.promise]);
        }
        finally {
            sleepController.abort();
            if (this.wakeWaiter === waiter)
                this.wakeWaiter = undefined;
        }
    }
    async waitForExit() {
        if (this.stopped)
            return;
        this.observation ??= (async () => {
            let pollIntervalMs = SCOPE_INITIAL_POLL_INTERVAL_MS;
            let generation = this.wakeGeneration;
            while (await this.rangeActive()) {
                await this.waitForPoll(pollIntervalMs, generation);
                generation = this.wakeGeneration;
                // Keep establishment responsive, then reduce systemctl process churn
                // while systemd remains the authoritative owner of an active range.
                if (this.establishment === 'established') {
                    pollIntervalMs = Math.min(pollIntervalMs * 2, SYSTEMCTL_TIMEOUT_MS);
                }
            }
            this.stopped = true;
        })().catch((error) => {
            this.observation = undefined;
            throw error;
        });
        await this.observation;
    }
    cleanup() {
        cleanupLinuxLaunchFiles(this.startup.files);
    }
}
function scopeArgs(unitBase, literalArgv, invocation, argv) {
    return [
        '--user',
        '--scope',
        '--quiet',
        '--collect',
        ...literalArgv,
        `--unit=${unitBase}`,
        '--',
        ...invocation,
        '--',
        ...argv,
    ];
}
function directOutcome(child, startup) {
    return new Promise((resolveOutcome, rejectOutcome) => {
        let settled = false;
        child.once('error', (error) => {
            if (settled)
                return;
            settled = true;
            rejectOutcome(error);
        });
        child.once('exit', (exitCode, signal) => {
            if (settled)
                return;
            settled = true;
            try {
                resolveOutcome(startup.resolveOutcome({ exitCode, signal }));
            }
            catch (error) {
                /* v8 ignore next -- Node filesystem operations throw Error instances. */
                const failure = error instanceof Error ? error : new Error(String(error));
                rejectOutcome(failure);
            }
        });
    });
}
/**
 * Send a direct-process signal, distinguishing an absent PID from failed delivery.
 * @param pid - owned direct-process identity whose exit notification can still be pending.
 * @param send - platform signal operation; true means the signal was submitted.
 * @returns whether the signal was submitted or the owned PID is already absent.
 */
export function signalLinuxDirectProcess(pid, send) {
    try {
        if (send())
            return true;
    }
    catch { /* A failed signal still permits an independent absence observation. */ }
    try {
        process.kill(pid, 0);
        return false;
    }
    catch (error) {
        return error.code === 'ESRCH';
    }
}
function signalChildGroup(child, signal) {
    let groupSignalled = false;
    try {
        groupSignalled = process.kill(-child.pid, signal);
    }
    catch { /* A missing or inaccessible group still permits a direct-process attempt. */ }
    if (groupSignalled && signal === 'SIGTERM')
        return true;
    // Group success can reflect another member; joining direct exit requires its own SIGKILL submission.
    // ChildProcess.kill can emit an error that settles directOutcome before the real exit.
    return signalLinuxDirectProcess(child.pid, () => process.kill(child.pid, signal));
}
/**
 * Prepare one Linux PTY scope using the same launch request and bootstrap core.
 * @param spec - terminal target request.
 * @param targetEnv - validated complete target environment.
 * @param literalArgv - options from the provider's positive scope probe.
 * @param internals - optional runner and systemd seams used by tests.
 * @returns invocation and ownership callbacks; requested termination preserves the observed signal even before bootstrap consumption.
 */
export function prepareLinuxTerminalScope(spec, targetEnv, literalArgv, internals = {}) {
    const invocation = internals.runnerInvocation ?? spawnRunnerInvocation();
    const files = createLinuxLaunchFiles({ cwd: spec.cwd, env: targetEnv });
    const startup = new LinuxScopeStartup(files, 'terminal');
    const unitBase = unitStem('dsh-terminal');
    return {
        command: internals.systemdRun ?? 'systemd-run',
        args: scopeArgs(unitBase, literalArgv, invocation, spec.argv),
        cwd: process.cwd(),
        env: runnerEnvironment(files.requestPath, invocation),
        bindOwner: direct => new SystemdScopeOwner(`${unitBase}.scope`, startup, direct, internals.systemctl ?? 'systemctl', internals.spawnSync ?? spawnSync, internals.systemctlQuery ?? querySystemctl, internals.sleep ?? sleepWithAbort),
        resolveOutcome: outcome => startup.resolveOutcome(outcome),
        cleanup: () => { cleanupLinuxLaunchFiles(files); },
    };
}
/**
 * Launch one ordinary target inside a transient user scope.
 * @param spec - ordinary target request.
 * @param targetEnv - validated complete target environment.
 * @param literalArgv - options from the provider's positive scope probe.
 * @param internals - optional runner and systemd seams used by tests.
 * @returns streams, result, and scope owner; requested termination preserves the observed signal even before bootstrap consumption.
 */
export function launchLinuxScope(spec, targetEnv, literalArgv, internals = {}) {
    const invocation = internals.runnerInvocation ?? spawnRunnerInvocation();
    const stdio = runnerStdio(spec, false);
    const files = createLinuxLaunchFiles({
        cwd: spec.cwd, env: targetEnv,
        pipeFds: stdio.slice(0, 3).flatMap((disposition, fd) => disposition === 'pipe' ? [fd] : []),
        ...spec.stdio.control === undefined ? {} : { control: spec.stdio.control },
    });
    const startup = new LinuxScopeStartup(files, 'subprocess');
    const unitBase = unitStem('dsh-subprocess');
    let child;
    try {
        child = (internals.spawn ?? spawn)(internals.systemdRun ?? 'systemd-run', scopeArgs(unitBase, literalArgv, invocation, spec.argv), {
            cwd: process.cwd(),
            env: runnerEnvironment(files.requestPath, invocation),
            stdio,
            detached: true,
        });
    }
    catch (error) {
        cleanupLinuxLaunchFiles(files);
        throw error;
    }
    const direct = directOutcome(child, startup);
    const owner = new SystemdScopeOwner(`${unitBase}.scope`, startup, {
        running: () => child.pid !== undefined && child.exitCode === null && child.signalCode === null,
        signal: signal => signalChildGroup(child, signal),
        settled: direct,
    }, internals.systemctl ?? 'systemctl', internals.spawnSync ?? spawnSync, internals.systemctlQuery ?? querySystemctl, internals.sleep ?? sleepWithAbort);
    return {
        stdin: child.stdin,
        stdout: child.stdout,
        stderr: child.stderr,
        control: controlPipe(child, spec.stdio.control),
        direct,
        owner,
    };
}
//# sourceMappingURL=linux-scope.js.map