/**
 * The web search and fetch page's staged form over three settings namespaces:
 * `web`, whose `searchProvider` and `fetchProvider` the two provider menus
 * write, `web-deepseek-api` for the DeepSeek Search and Fetch APIs, and
 * `web-search-deepseek` for legacy Messages search.
 *
 * A selection equal to the deployment's composed value clears the override
 * instead of storing a copy of it, and while the deployment composes no
 * provider for a capability its menu also offers automatic selection, which
 * clears the override. The menus offer the providers the Host reports through
 * `web/listProviders`, the official ones first in a fixed order and the rest
 * in registration order, plus a selected id the Host no longer registers. A
 * failed read is reported on the page with a retry.
 *
 * The DeepSeek Search and Fetch APIs take no credential values here: each
 * has a billing menu, written to `searchBilling` or `fetchBilling`, that
 * chooses which DeepSeek credential it sends. Their own endpoint fields are
 * YAML-only: the page reads them to report that they override billing and
 * never writes them. Legacy Messages search keeps its key control.
 *
 * The legacy key's literal never rides a response, so the page learns only
 * whether one is configured and writes it through the credentials domain,
 * addressed by the reference `web-search-deepseek.apiKeyEnv` names. A key
 * draft belongs to the reference its control addressed when it was typed: it
 * is dropped when that reference changes or the page stops showing the
 * control, and a save writes it under that reference. The key is still staged
 * with the rest of the form, so one save covers everything the page shows.
 */
import { createSnapshotStore } from '@deepseek-ai/dsh-client-store';
import { SettingsFormModel, settingsNumberField, settingsTextField, } from '@deepseek-ai/dsh-client-ui-primitives';
/**
 * Namespaces this page edits. Spelled here rather than imported: a client
 * package must not depend on a Host package.
 */
export const WEB_SEARCH_NS = 'web-search-deepseek';
/** Namespace of the DeepSeek Search and Fetch API providers. */
export const WEB_API_NS = 'web-deepseek-api';
/** Namespace of the web service's provider selection. */
export const WEB_NS = 'web';
/** Provider ids of the official providers whose settings this page edits. */
export const PROVIDER_IDS = {
    apiSearch: 'deepseek-search-api',
    legacySearch: 'deepseek-official',
    localFetch: 'http',
    apiFetch: 'deepseek-fetch-api',
};
/**
 * The official providers each menu lists first, in this order, while the Host
 * registers them; the Host's own list stays in registration order.
 */
const MENU_ORDER = {
    search: [PROVIDER_IDS.apiSearch, PROVIDER_IDS.legacySearch],
    fetch: [PROVIDER_IDS.localFetch, PROVIDER_IDS.apiFetch],
};
/** Credential reference legacy Messages search resolves when its section names none. */
const DEFAULT_API_KEY_REF = 'DEEPSEEK_API_KEY';
/** Form field the credential control stages under. */
const API_KEY_FIELD = 'legacyApiKey';
/** Billing values the Host accepts. */
const BILLINGS = ['auto', 'account', 'api-key'];
/** Whether `text` names a billing the Host accepts. */
function isBilling(text) {
    return BILLINGS.includes(text);
}
/** Bridges the settings scopes, the provider lists, and the credentials domain onto the page. */
export class WebSearchCardController {
    scopes;
    ctx;
    selection;
    api;
    legacy;
    store;
    unsubscribe;
    credential;
    credentialReadQueued = false;
    credentialReadSequence = 0;
    /** Reference the key draft was typed for; read only while that draft is non-empty. */
    draftRef;
    /** The draft reference the running or last selection save writes under. */
    savingRef;
    droppingDraft = false;
    providers;
    providersFailed = false;
    providerReadSequence = 0;
    selectionServed = false;
    disposed = false;
    /**
     * @param scopes - the bound settings scopes for the page's namespaces.
     * @param ctx - the page plugin's context, whose `remote.credentials` namespace
     * answers for the credential the legacy key control references, and whose
     * `remote.web` namespace lists the registered providers.
     */
    constructor(scopes, ctx) {
        this.scopes = scopes;
        this.ctx = ctx;
        this.credential = { ref: this.ref(), configured: false, writable: true };
        this.draftRef = this.ref();
        this.savingRef = this.draftRef;
        this.selection = new SettingsFormModel(scopes.web, [
            settingsTextField('searchProvider'), settingsTextField('fetchProvider'),
        ], [{ field: API_KEY_FIELD, write: text => this.writeKey(text) }]);
        this.api = new SettingsFormModel(scopes.api, [
            {
                ...settingsNumberField('count'),
                parse: (text) => {
                    if (text.trim() === '')
                        return { kind: 'clear' };
                    const value = Number(text);
                    return Number.isInteger(value) && value >= 1 && value <= 50 ? { kind: 'set', value } : undefined;
                },
            },
            // Drafts come only from selectBilling, which stages a billing the Host accepts.
            settingsTextField('searchBilling'), settingsTextField('fetchBilling'),
        ]);
        this.legacy = new SettingsFormModel(scopes.legacy, [
            settingsTextField('baseURL'),
            { ...settingsNumberField('maxUses'), parse: (text) => {
                    if (text.trim() === '')
                        return { kind: 'clear' };
                    const value = Number(text);
                    return Number.isInteger(value) && value >= 1 ? { kind: 'set', value } : undefined;
                } },
        ]);
        this.selectionServed = this.selection.shell().available;
        this.store = createSnapshotStore(this.projection());
        this.unsubscribe = [
            ...[this.selection, this.api, this.legacy].map(form => form.bind(() => form.shell()).subscribe(() => { this.publish(); })),
            ...[scopes.web, scopes.api, scopes.legacy].map(scope => scope.subscribe(() => { this.queueCredentialRead(); })),
            scopes.web.subscribe(() => {
                // The provider list comes from the web service; a Host that starts
                // serving it registers providers this page has not read.
                const served = this.selection.shell().available;
                if (served && !this.selectionServed)
                    this.refreshProviders();
                this.selectionServed = served;
            }),
        ];
        void this.readCredential();
        this.refreshProviders();
    }
    /**
     * Republish after a form or scope change. A staged selection can hide the
     * key control without any scope publishing; a scope change can rename the
     * reference itself.
     */
    publish() {
        // Dropping a draft stages an edit, which republishes; publish once after it.
        if (this.droppingDraft)
            return;
        this.dropStaleKeyDraft();
        const ref = this.ref();
        if (ref !== this.credential.ref) {
            this.resetCredentialReference(ref);
            this.queueCredentialRead();
        }
        this.store.set(this.projection());
    }
    /** The provider id a selection field stages. */
    selected(field) {
        return this.selection.field(field).text;
    }
    /**
     * Menu entries for one capability: the registered official providers in
     * {@link MENU_ORDER}, then every other registered provider in Host order,
     * and the selected id when the Host does not register it. Before the list
     * arrives, the menu holds only the selected id.
     */
    options(capability, selected) {
        const registered = this.providers?.[capability];
        if (registered === undefined)
            return selected === '' ? [] : [{ id: selected, missing: false }];
        const first = MENU_ORDER[capability];
        const ordered = [
            ...first.flatMap(id => registered.filter(info => info.id === id)),
            ...registered.filter(info => !first.includes(info.id)),
        ];
        const options = ordered.map(info => ({
            id: info.id,
            ...info.title === undefined ? {} : { title: info.title },
            ...info.description === undefined ? {} : { description: info.description },
            missing: false,
        }));
        if (selected !== '' && !registered.some(info => info.id === selected))
            options.push({ id: selected, missing: true });
        return options;
    }
    /** Whether the Host reports `id` as registered, or has not answered yet. */
    registered(capability, id) {
        return this.providers === undefined || this.providers[capability].some(info => info.id === id);
    }
    searchKind() {
        const id = this.selected('searchProvider');
        if (id === '' || !this.registered('search', id))
            return 'none';
        if (id === PROVIDER_IDS.apiSearch)
            return this.api.shell().available ? 'deepseek' : 'none';
        if (id === PROVIDER_IDS.legacySearch)
            return this.legacy.shell().available ? 'legacy' : 'none';
        return this.providers === undefined ? 'none' : 'plugin';
    }
    fetchKind() {
        const id = this.selected('fetchProvider');
        if (id === '' || !this.registered('fetch', id))
            return 'none';
        if (id === PROVIDER_IDS.localFetch)
            return 'local';
        if (id === PROVIDER_IDS.apiFetch)
            return this.api.shell().available ? 'deepseek' : 'none';
        return this.providers === undefined ? 'none' : 'plugin';
    }
    /** The reference the key control addresses, from the legacy settings or the default. */
    ref() {
        return this.scopes.legacy.getSnapshot().value?.apiKeyEnv || DEFAULT_API_KEY_REF;
    }
    /** Whether the page shows the key control. */
    keyShown() {
        return this.searchKind() === 'legacy';
    }
    /**
     * Drop the key draft once the page no longer shows the control or the
     * control addresses another reference, so a draft is never written under
     * another reference or under one no control shows.
     */
    dropStaleKeyDraft() {
        if (this.selection.field(API_KEY_FIELD).text === '')
            return;
        if (this.keyShown() && this.draftRef === this.ref())
            return;
        this.droppingDraft = true;
        try {
            this.selection.actions().edit(API_KEY_FIELD, '');
        }
        finally {
            this.droppingDraft = false;
        }
    }
    /**
     * Stage a key draft for the reference the control addresses now.
     * @param text - the draft text.
     */
    editKey(text) {
        // The page shows no key control, so a draft would belong to no reference.
        if (!this.keyShown())
            return;
        this.draftRef = this.ref();
        this.selection.actions().edit(API_KEY_FIELD, text);
    }
    projection() {
        const primary = this.selection.shell();
        const served = [primary, this.api.shell(), this.legacy.shell()].filter(item => item.available);
        const shell = {
            available: primary.available,
            writable: served.every(item => item.writable),
            dirty: served.some(item => item.dirty),
            invalid: served.some(item => item.invalid),
            saving: served.some(item => item.saving),
            failed: served.some(item => item.failed),
        };
        const searchSelected = this.selected('searchProvider');
        const fetchSelected = this.selected('fetchProvider');
        const base = this.scopes.web.getSnapshot().base;
        return {
            ...shell,
            selectionBlocked: shell.invalid || shell.failed,
            providersFailed: this.providersFailed,
            search: {
                selected: searchSelected,
                options: this.options('search', searchSelected),
                kind: this.searchKind(),
                auto: base?.searchProvider === undefined,
            },
            fetch: {
                selected: fetchSelected,
                options: this.options('fetch', fetchSelected),
                kind: this.fetchKind(),
                auto: base?.fetchProvider === undefined,
            },
            count: this.api.field('count'),
            searchBilling: this.searchKind() === 'deepseek' ? this.billing('search') : undefined,
            fetchBilling: this.fetchKind() === 'deepseek' ? this.billing('fetch') : undefined,
            messagesBaseURL: this.legacy.field('baseURL'),
            maxUses: this.legacy.field('maxUses'),
            legacyKey: this.keyShown() ? { ...this.selection.field(API_KEY_FIELD), ...this.credential } : undefined,
        };
    }
    /** One capability's billing as its menu renders it. */
    billing(capability) {
        const field = this.api.field(`${capability}Billing`);
        const section = this.scopes.api.getSnapshot().value;
        const advancedOverride = [`${capability}BaseURL`, `${capability}ApiKeyEnv`]
            .filter(name => (section?.[name] ?? '') !== '');
        return { value: isBilling(field.text) ? field.text : 'auto', overridden: field.overridden, advancedOverride };
    }
    /**
     * Re-read the registered providers. Only the latest read publishes; a failed
     * read keeps the previous list and marks the page's provider list as failed
     * until a later read succeeds.
     */
    refreshProviders() {
        void this.readProviders();
    }
    async readProviders() {
        if (this.disposed)
            return;
        const sequence = ++this.providerReadSequence;
        const response = await this.ctx.remote.web.listProviders();
        // oxlint-disable-next-line typescript/no-unnecessary-condition -- dispose() can run during the await.
        if (this.disposed || sequence !== this.providerReadSequence)
            return;
        if (response.ok)
            this.providers = response.value;
        this.providersFailed = !response.ok;
        this.publish();
    }
    /** Clear the old reference's answer and invalidate its pending reads. */
    resetCredentialReference(ref) {
        this.credential = { ref, configured: false, writable: true };
        this.credentialReadSequence++;
    }
    /** Combine settings publications, including connection-reset refreshes, into one credential read. */
    queueCredentialRead() {
        if (this.credentialReadQueued)
            return;
        this.credentialReadQueued = true;
        queueMicrotask(() => {
            this.credentialReadQueued = false;
            void this.readCredential();
        });
    }
    /**
     * Ask the credentials domain about the reference the key control currently names.
     *
     * Only the latest read publishes its answer, and only while its reference
     * is unchanged. A write's readback retains its own result even if
     * another read starts or the reference changes.
     * @param written - a reference a save just wrote, asked about in the same read.
     * @returns the answer by reference, or undefined after failure or disposal.
     */
    async readCredential(written) {
        if (this.disposed)
            return;
        const ref = this.ref();
        if (ref !== this.credential.ref) {
            // A new reference knows nothing yet; keeping the old answer would claim
            // the key is configured under a name nobody has checked.
            this.resetCredentialReference(ref);
            this.store.set(this.projection());
        }
        const sequence = ++this.credentialReadSequence;
        const names = [...new Set([ref, ...written === undefined ? [] : [written]])];
        const response = await this.ctx.remote.credentials.describe(names);
        // oxlint-disable-next-line typescript/no-unnecessary-condition -- dispose() can run during the await.
        if (this.disposed || !response.ok)
            return;
        const views = response.value;
        if (sequence !== this.credentialReadSequence)
            return views;
        if (ref !== this.ref()) {
            // The reference changed outside a publication: drop this answer and read the new one.
            this.publish();
            return views;
        }
        const view = views[ref];
        // An unknown reference is treated as writable: the control stays usable
        // and the Host is what refuses, rather than the page guessing a refusal.
        const next = { ref, configured: view?.configured ?? false, writable: view?.writable ?? true };
        if (next.configured !== this.credential.configured || next.writable !== this.credential.writable) {
            this.credential = next;
            this.publish();
        }
        return views;
    }
    /**
     * Re-read after the Host reports a change to the reference this page watches.
     *
     * A key can be written from somewhere else — the Models page addresses the
     * same reference — and no settings section changes when it is, so without
     * this the badge keeps reporting a state the Host already replaced.
     * @param ref - the reference the Host reports as changed.
     */
    refreshCredential(ref) {
        if (ref !== this.credential.ref)
            return;
        void this.readCredential();
    }
    /**
     * Build the face the page's slot registration injects.
     * @returns the page's snapshot and its form actions.
     */
    inject() {
        return {
            hooks: { webSearchCard: this.store },
            edit: (field, text) => {
                if (field === 'legacyApiKey') {
                    this.editKey(text);
                    return;
                }
                if (field === 'messagesBaseURL' || field === 'maxUses') {
                    this.legacy.actions().edit(field === 'messagesBaseURL' ? 'baseURL' : field, text);
                    return;
                }
                this.api.actions().edit(field, text);
            },
            resetField: (field) => {
                const form = field === 'messagesBaseURL' || field === 'maxUses' ? this.legacy : this.api;
                form.actions().resetField(field === 'messagesBaseURL' ? 'baseURL' : field);
            },
            selectProvider: (field, id) => { this.select(field, id); },
            selectBilling: (capability, billing) => { this.selectBilling(capability, billing); },
            retryProviders: () => { this.refreshProviders(); },
            resolveText: text => this.ctx.locale.resolveText(text),
            save: () => { void this.save(); },
            discard: () => {
                for (const form of [this.selection, this.api, this.legacy])
                    form.actions().discard();
            },
        };
    }
    /**
     * Stage one provider selection. Automatic selection and a target equal to
     * the composed value clear the override, so the deployment default keeps
     * applying.
     * @param field - the selection field.
     * @param id - the provider id to select, or the empty string for automatic selection.
     */
    select(field, id) {
        const composed = this.scopes.web.getSnapshot().base?.[field];
        if (id === '' || composed === id)
            this.selection.actions().resetField(field);
        else
            this.selection.actions().edit(field, id);
    }
    /**
     * Stage one capability's billing. A billing equal to the composed one, which
     * is `auto` when the deployment composes none, clears the override.
     * @param capability - the capability whose billing changes.
     * @param billing - the billing to stage.
     */
    selectBilling(capability, billing) {
        const field = `${capability}Billing`;
        const composed = this.scopes.api.getSnapshot().base?.[field] ?? 'auto';
        if (composed === billing)
            this.api.actions().resetField(field);
        else
            this.api.actions().edit(field, billing);
    }
    /**
     * Save the API section before the selection, so a selection takes effect
     * only after the settings it uses have landed. A refused section stops the
     * selection from saving and keeps every remaining draft for correction. The
     * key draft's reference is fixed when the selection save plans its writes,
     * so a provider change during the save does not redirect a key the save
     * already planned.
     * @returns settlement after every attempted write.
     */
    async save() {
        const state = this.projection();
        if (!state.available || state.invalid || state.saving)
            return;
        for (const form of [this.api, this.legacy]) {
            if (!form.shell().available || !form.shell().dirty)
                continue;
            await form.save();
            if (form.shell().failed)
                return;
        }
        if (!this.selection.shell().dirty)
            return;
        this.savingRef = this.draftRef;
        await this.selection.save();
    }
    /**
     * Write the staged key under the reference its draft belonged to when the
     * save started, then re-read whether the Host now holds it.
     * @param value - the staged credential literal.
     * @returns whether the Host reports a configured credential afterwards.
     */
    async writeKey(value) {
        const ref = this.savingRef;
        // Refusals surface through the re-read below: the Host is the only
        // authority on whether the key now exists.
        await this.ctx.remote.credentials.set(ref, value);
        return (await this.readCredential(ref))?.[ref]?.configured ?? false;
    }
    /** Release subscriptions and prevent queued reads or late answers from publishing. */
    dispose() {
        this.disposed = true;
        for (const off of this.unsubscribe)
            off();
        for (const form of [this.selection, this.api, this.legacy])
            form.dispose();
    }
}
//# sourceMappingURL=web-search-card-controller.js.map