import { fileAddressFor } from '@deepseek-ai/dsh-util-workspace-path';
import { rankByName } from '@deepseek-ai/dsh-client-ui-primitives';
import { SkillRow } from "./SkillRow.js";
import { SkillReference } from "./SkillReference.js";
import { skillReferenceRule } from "./user-text-rules.js";
import { en, ja, es, fr, de, ru, pt, NS, zh } from "./locales.js";
/**
 * Normalize one catalog pull across the official `{skills:[…]}` face and the
 * web.v1 `{items:[{id,name,description,source}]}` face (§9.2). Unknown shapes
 * drop to an honest empty catalog.
 * @param value - the Remote result value.
 * @returns the entry list.
 */
function normalizeSkillCatalog(value) {
    const items = value !== null && typeof value === 'object' && Array.isArray(value.skills)
        ? value.skills
        : value !== null && typeof value === 'object' && Array.isArray(value.items)
            ? value.items
            : [];
    return items.flatMap((item) => {
        if (item === null || typeof item !== 'object')
            return [];
        const record = item;
        const name = typeof record.name === 'string' && record.name !== '' ? record.name
            : typeof record.id === 'string' ? record.id : undefined;
        if (name === undefined)
            return [];
        const source = record.source === 'builtin' || record.source === 'project' || record.source === 'plugin'
            ? record.source
            : undefined;
        return [{
                name,
                description: typeof record.description === 'string' ? record.description : '',
                modelInvocable: false,
                ...(source !== undefined ? { source } : {}),
            }];
    });
}
/** Required services: reference source faces plus the tool-row and locale registries. */
export const inject = ['inputTriggers', 'sessions', 'slots', 'locale', 'remote', 'remote.skills', 'sidebarRight', 'uiConversation'];
/**
 * Client plugin body: register the '/' source, dictionaries, and keyed tool row.
 * @param ctx - client root context.
 */
export function apply(ctx) {
    ctx.effect(() => ctx.locale.register(NS, { zh, en, ja, es, fr, de, ru, pt }), 'ui-skill: dictionaries');
    ctx.uiConversation.userText.register(skillReferenceRule);
    ctx.slots.inject('tool.call.toolview', () => ctx.slots.register({ name: 'tool.call.toolview', key: 'skill', locale: NS }, SkillRow));
    const skills = ctx.remote.skills;
    const sessions = ctx.sessions;
    // Session-keyed catalog cache; single-flight per key. Plugin-closure state:
    // the fiber effect below is its teardown boundary.
    const fetches = new Map();
    // Session-keyed skill-body cache (web.v1 §3: a skill executes by the user
    // editing its expanded body and sending it explicitly). Filled best-effort
    // alongside the catalog; a deployment without `skills/get` keeps the
    // literal `/name` pick path.
    const bodies = new Map();
    // Per-session lexicon invalidation listeners (subscribeLexicon consumers).
    const lexiconListeners = new Map();
    const notifyLexicon = (sessionId) => {
        for (const listener of [...(lexiconListeners.get(sessionId) ?? [])]) {
            try {
                listener();
            }
            catch (error) {
                // Contain listener failures: settlement notifies from an ignored
                // promise chain (a throw would surface as an unhandled rejection)
                // and one faulty consumer must not starve the others.
                console.error('[ui-skill] lexicon listener failed:', error);
            }
        }
    };
    const fetchCatalog = (sessionId) => {
        const existing = fetches.get(sessionId);
        if (existing !== undefined)
            return existing;
        const abort = new AbortController();
        const promise = (async () => {
            if (sessions.binding(sessionId) === undefined) {
                throw new Error(`skill catalog requires a retained session "${sessionId}"`);
            }
            return sessions.using(sessionId, { source: 'skillCatalog', signal: abort.signal }, async (reference) => {
                abort.signal.throwIfAborted();
                const state = reference.binding.session.getSnapshot();
                if (state.openState !== 'open') {
                    throw state.openError ?? new Error(`session "${sessionId}" is not open`);
                }
                const result = await skills.list({ sessionId }, abort.signal);
                abort.signal.throwIfAborted();
                if (!result.ok)
                    throw new Error(`skills/list failed: ${result.error.code}: ${result.error.message}`);
                return normalizeSkillCatalog(result.value);
            });
        })();
        const entry = { promise, abort };
        fetches.set(sessionId, entry);
        promise.then(
        // Settled snapshot backs the synchronous lexicon reads.
        (skills) => {
            entry.settled = skills;
            notifyLexicon(sessionId);
            void prefetchBodies(sessionId, skills);
        }, 
        // A failed fetch must not poison the key: the next consumer retries.
        () => {
            if (fetches.get(sessionId) === entry)
                fetches.delete(sessionId);
        });
        return entry;
    };
    /**
     * Best-effort body prefetch for one session's catalog: each failure is
     * contained (a missing body only keeps that skill's literal pick path).
     * @param sessionId - owning session.
     * @param catalogEntries - the settled catalog entries.
     */
    const prefetchBodies = (sessionId, catalogEntries) => {
        const cache = bodies.get(sessionId) ?? new Map();
        bodies.set(sessionId, cache);
        for (const skill of catalogEntries) {
            if (cache.has(skill.name))
                continue;
            void sessions.using(sessionId, { source: 'skillCatalog' }, async (reference) => {
                const state = reference.binding.session.getSnapshot();
                if (state.openState !== 'open')
                    throw state.openError ?? new Error('session is not open');
                const result = await skills.get({ sessionId, id: skill.name });
                if (!result.ok)
                    throw new Error(result.error.message);
                cache.set(skill.name, result.value.body);
            }).then(() => { }, () => { });
        }
    };
    const invalidate = (key) => {
        const entry = fetches.get(key);
        if (entry === undefined)
            return;
        fetches.delete(key);
        bodies.delete(key);
        entry.abort.abort();
        notifyLexicon(key);
    };
    const clearAll = () => {
        for (const key of [...fetches.keys()])
            invalidate(key);
    };
    // The bound translate resolves against the registered dictionaries with the
    // locale service's own fallback ladder; candidate-time reads stay plain text.
    const t = ctx.locale.bind(NS);
    const source = {
        trigger: '/',
        name: 'skill',
        order: 2,
        async candidates(session, { query, signal }) {
            if (sessions.subagentAddress(session.sessionId) !== undefined)
                return [];
            const skills = await fetchCatalog(session.sessionId).promise;
            // Superseded keystroke: the shared fetch stays warm, this caller yields.
            if (signal.aborted)
                return [];
            // The same ranking as the command group of this menu: case-insensitive
            // ordered subsequence, prefix hits first.
            return rankByName(skills, query)
                .map(skill => ({
                name: skill.name,
                // The user-only marker rides the description (the menu's only
                // secondary text); `hint` is the claim-state ghost text, not a badge.
                // The web.v1 source layer joins the same slot when served (§3).
                description: `${skill.modelInvocable ? '' : `${t('menu.userOnly')} · `}${skill.source !== undefined ? `${t(`menu.source.${skill.source}`)} · ` : ''}${skill.description}`,
            }));
        },
        warm(session) {
            // Fire-and-forget scope-birth prewarm; the shared fetch reports
            // through candidates.
            if (sessions.subagentAddress(session.sessionId) !== undefined)
                return;
            fetchCatalog(session.sessionId).promise.catch(() => { });
        },
        lexicon(session) {
            return fetches.get(session.sessionId)?.settled?.map(skill => skill.name);
        },
        subscribeLexicon(session, listener) {
            const key = session.sessionId;
            const listeners = lexiconListeners.get(key) ?? new Set();
            listeners.add(listener);
            lexiconListeners.set(key, listeners);
            return () => {
                listeners.delete(listener);
                if (listeners.size === 0)
                    lexiconListeners.delete(key);
            };
        },
        openReference(session, { ref }) {
            if (sessions.subagentAddress(session.sessionId) !== undefined)
                return false;
            const cwd = sessions.list.getSnapshot().byId[session.sessionId]?.cwd;
            const open = (catalog) => {
                const path = catalog.find(skill => `/${skill.name}` === ref)?.path;
                if (path === undefined)
                    return false;
                ctx.sidebarRight.openResource(fileAddressFor(session.sessionId, cwd, path));
                return true;
            };
            const settled = fetches.get(session.sessionId)?.settled;
            if (settled !== undefined)
                return open(settled);
            const entry = fetchCatalog(session.sessionId);
            void entry.promise.then((catalog) => {
                if (!entry.abort.signal.aborted)
                    open(catalog);
            }).catch((error) => {
                if (!entry.abort.signal.aborted)
                    console.error('[ui-skill] reference preview failed:', error);
            });
            return true;
        },
        onPick({ candidate, session }) {
            // web.v1 §3: the expanded body fills the composer for the user to edit
            // and send explicitly. Without a served body (an official Host keeps the
            // literal path) the pick lands the literal and the prompt ships the same
            // literal — host-side determinism (the dsh-tool-skill pre-step) stays
            // intact. A name shared with a host command still resolves to the
            // command: adjudication claims the line client-side first.
            const body = bodies.get(session.sessionId)?.get(candidate.name);
            return { text: body ?? `/${candidate.name} ` };
        },
    };
    ctx.slots.inject('conversation.user.text.node', () => ctx.slots.register({
        name: 'conversation.user.text.node',
        key: 'skill',
        priority: 100,
        inject: (sessionId) => ({
            openSkill: (name) => {
                if (sessions.scope(sessionId) === undefined)
                    return;
                source.openReference({ sessionId }, { ref: `/${name}` });
            },
        }),
    }, SkillReference));
    const inputTriggers = ctx.get('inputTriggers');
    // A preset decides which skill providers an agent reads, so a switched
    // session's cached catalog belongs to the composition it no longer runs.
    ctx.remote.$on('agent-preset/selected', invalidate);
    // The Host announces every manager operation after its reload without
    // naming what changed, and a switched plugin may register providers in the
    // global layer every session reads, so each one drops every cached catalog.
    ctx.remote.$on('plugin-manager/changed', clearAll);
    ctx.on('connection/reset', clearAll);
    ctx.effect(() => {
        const unregister = inputTriggers.registerSource(source);
        return () => {
            unregister();
            clearAll();
        };
    }, 'ui-skill: source');
}
//# sourceMappingURL=index.js.map