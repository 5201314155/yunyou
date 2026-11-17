import { changedSearchPreferences, defaultSearchPreferences, readSearchPreferences, } from "./search-rules.js";
/**
 * The localStorage key the preferences of every workspace are stored under.
 *
 * Each entry holds whichever of `rules`, `showHidden`, and `respectGitignore`
 * differ from their defaults. Every field is optional on read:
 * {@link readSearchPreferences} gives a missing or mistyped one its current
 * default and reads the others as usual. A change that an earlier build would
 * misread takes a new key.
 */
export const SEARCH_PREFERENCES_KEY = 'dsh.sidebar-files.workspace-search.v1';
/**
 * The storage key of one workspace root: the root as the Host reports it, with
 * trailing `/` and `\` removed except the one that makes a filesystem root
 * (`/`, `C:\`, `C:/`). Case and separators are otherwise kept, so two
 * spellings of one directory keep separate preferences.
 * @param cwd - the Session's working directory.
 * @returns the key its preferences are stored under.
 */
export function workspaceKey(cwd) {
    const trimmed = cwd.replace(/[/\\]+$/, '');
    if (trimmed === '')
        return cwd.slice(0, 1);
    if (/^[A-Za-z]:$/.test(trimmed) && cwd.length > trimmed.length)
        return cwd.slice(0, 3);
    return trimmed;
}
/**
 * One stored entry, without trusting the stored value's type.
 * @param stored - the storage value, as rehydrated.
 * @param key - a workspace key.
 * @returns the entry's value, or `undefined` when the value is not an object or has no such entry.
 */
function storedEntry(stored, key) {
    if (typeof stored !== 'object' || stored === null || Array.isArray(stored) || !Object.hasOwn(stored, key))
        return undefined;
    return Object.getOwnPropertyDescriptor(stored, key)?.value;
}
/**
 * Keep the preferences of every workspace root in one persisted store.
 * @param store - the persisted store; its rehydrated value may be anything JSON can hold.
 * @returns the preferences by workspace root.
 */
export function createSearchPreferenceBook(store) {
    const sources = new Map();
    const source = (cwd) => {
        const key = workspaceKey(cwd);
        const known = sources.get(key);
        if (known !== undefined)
            return known;
        let seen;
        const created = {
            getSnapshot: () => {
                const raw = storedEntry(store.getSnapshot(), key);
                if (seen === undefined || seen.raw !== raw) {
                    seen = { raw, preferences: raw === undefined ? defaultSearchPreferences() : readSearchPreferences(raw) };
                }
                return seen.preferences;
            },
            subscribe: listener => store.subscribe(listener),
        };
        sources.set(key, created);
        return created;
    };
    return {
        source,
        update(cwd, patch) {
            const preferences = { ...source(cwd).getSnapshot(), ...patch };
            const key = workspaceKey(cwd);
            const stored = store.getSnapshot();
            const entries = typeof stored === 'object' && stored !== null && !Array.isArray(stored)
                ? Object.entries(stored).filter(([name]) => name !== key)
                : [];
            const changed = changedSearchPreferences(preferences);
            if (changed !== undefined)
                entries.push([key, changed]);
            store.set(Object.fromEntries(entries));
        },
    };
}
//# sourceMappingURL=search-preferences.js.map