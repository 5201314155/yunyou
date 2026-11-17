import { jsxs as _jsxs, jsx as _jsx, Fragment as _Fragment } from "react/jsx-runtime";
/**
 * One provider's editor card, hand-written per adapter family: the primary
 * field is a single write-only **API key** input (the page never asks for an
 * environment-variable name — a typed key stores through `credentials/set`
 * under the profile's reference, deriving `<ROUTE>_API_KEY` when the profile
 * has none. The pi-ai profile records that derivation as `apiKeyEnv` only when
 * a key is entered; a blank key materializes a reference-free profile for
 * provider-native authentication);
 * the collapsed 自定义设置 area carries the per-family extras (`baseURL` and
 * `retryPolicy` for both families, DeepSeek's id/name/context-window model
 * catalog, and the display name and wire protocol of a pi-ai route the
 * adapter does not ship — the two fields the create card asked that route
 * for, editable here for the same reason).
 * Reasoning effort is deliberately absent: it is a per-MODEL capability, and
 * the models under one provider disagree about it, so a provider-scoped
 * control can only be set to a value some of them reject. The composer's
 * model picker offers each model its own levels; `cordis.patch.yml` keeps the
 * profile field for a deployment that knows its route. Everything else stays
 * owned by `cordis.patch.yml`. Profile edits land as minimal `settings.mutate`
 * path ops against the stored section — the card names only the fields it can
 * see instead of rebuilding the whole subtree from a partial descriptor.
 */
import { useEffect, useMemo, useState } from 'react';
import { Tooltip } from '@deepseek-ai/dsh-client-ui-primitives';
import { DeepSeekModelsEditor, modelDrafts, validateDeepSeekModels, } from "./DeepSeekModelsEditor.js";
import { apiKeyFailure } from "./apiKey.js";
import { EditorFooter } from "./EditorFooter.js";
import { ModelListEditor } from "./ModelListEditor.js";
import { validateReasoningEfforts } from "./ModelReasoningEfforts.js";
import { deriveKeyRef, protocolChoices, reasoningLevelChoices } from "./store.js";
import { protocolLabel } from "./protocol-label.js";
import { protocolNeedsKey, protocolReasoningLevels } from "./protocol-rules.js";
import { ApiKeyInput } from "./ApiKeyInput.js";
import { RetryPolicyEditor, retryPolicyDefaults, retryPolicyFailure } from "./RetryPolicyEditor.js";
import styles from './ModelsSection.module.css';
/** A user-section subtree as a plain draft object (absent → empty). */
function draftAt(schema, namespace, path) {
    const subtree = schema.getPath(namespace.user, path);
    if (typeof subtree !== 'object' || subtree === null || Array.isArray(subtree))
        return {};
    return structuredClone(subtree);
}
function isRecord(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
/**
 * The minimal path ops carrying `after` over `before`, both as the card sees
 * them. Only keys the card observed are named; fields absent from both sides
 * produce no op, which is why edits are path-addressed rather than a rebuilt
 * section. Objects are diffed field by field, so an omitted field keeps the
 * value an unset restores from the inherited layer instead of a whole-object
 * set discarding it.
 * @param base - path of the edited subtree inside the user section.
 * @param before - the subtree as loaded, or undefined when it is new.
 * @param after - the subtree as edited.
 * @returns ordered set/unset ops; empty when nothing changed.
 */
export function pathOps(base, before, after) {
    const previous = isRecord(before) ? before : {};
    const ops = [];
    for (const [key, value] of Object.entries(after)) {
        const prior = previous[key];
        if (JSON.stringify(prior) === JSON.stringify(value))
            continue;
        if (isRecord(value) && (prior === undefined || isRecord(prior)))
            ops.push(...pathOps([...base, key], prior, value));
        else
            ops.push({ op: 'set', path: [...base, key], value: value });
    }
    for (const key of Object.keys(previous)) {
        if (!(key in after))
            ops.push({ op: 'unset', path: [...base, key] });
    }
    return ops;
}
/** The editor layout the owning namespace selects. */
function layoutOf(ns) {
    if (ns === 'llm-deepseek')
        return 'deepseek';
    if (ns === 'llm-pi-ai')
        return 'pi-ai';
    return 'unknown';
}
/** The credential reference this profile resolves keys through. */
function refFor(schema, namespace, path, provider) {
    const profile = schema.getPath(namespace.value, path);
    const named = typeof profile === 'object' && profile !== null
        ? profile.apiKeyEnv
        : undefined;
    return typeof named === 'string' && named.length > 0 ? named : deriveKeyRef(provider);
}
/**
 * Render one provider's editing card.
 * @param props - the addressed profile plus wire faces and copy.
 * @returns the editor card.
 */
export function ProviderEditor(props) {
    const { namespace, schema, settingsPath, operations, t } = props;
    const [draft, setDraft] = useState(() => {
        const loaded = draftAt(schema, namespace, settingsPath);
        const policy = loaded['retryPolicy'];
        // A stored retry policy without a mode edits under the mode it inherits;
        // the draft names it so a saved policy always carries one.
        if (!isRecord(policy) || policy['mode'] !== undefined)
            return loaded;
        const path = [...settingsPath, 'retryPolicy'];
        const node = schema.nodeAtPath(schema.rehydrate(namespace.schema), path);
        /* v8 ignore next -- a stored retryPolicy implies the schema declares one */
        if (node === undefined)
            return loaded;
        const { mode } = retryPolicyDefaults(schema, node, schema.getPath(namespace.base, path));
        return { ...loaded, retryPolicy: { ...policy, mode } };
    });
    // Undefined is untouched; strings hold new input and a reference stages removal.
    const [keyDraft, setKeyDraft] = useState();
    const keyText = typeof keyDraft === 'string' ? keyDraft : '';
    const keyRemoval = typeof keyDraft === 'object' ? keyDraft : undefined;
    const [busy, setBusy] = useState(false);
    const [listBusy, setListBusy] = useState(false);
    const { onBusyChange } = props;
    useEffect(() => { onBusyChange?.(busy || listBusy); }, [busy, listBusy, onBusyChange]);
    const [failure, setFailure] = useState(undefined);
    const [customizedOpen, setCustomizedOpen] = useState(false);
    // A settings success advances both retry baselines immediately. Keeping the
    // derived fields in the draft prevents a pushed namespace refresh from
    // turning them into deletions when the following credential write is retried.
    const [committedOriginal, setCommittedOriginal] = useState(() => schema.getPath(namespace.user, settingsPath));
    const [expectedRevision, setExpectedRevision] = useState(() => namespace.revision);
    const root = useMemo(() => schema.rehydrate(namespace.schema), [namespace.schema, schema]);
    const node = useMemo(() => schema.nodeAtPath(root, settingsPath), [root, schema, settingsPath]);
    const fallback = schema.getPath(namespace.value, settingsPath);
    const disabled = props.readOnly || busy;
    const layout = layoutOf(namespace.ns);
    const keyRef = refFor(schema, namespace, settingsPath, props.provider);
    // The same schema read the create card makes, so the choices offered here
    // and there cannot drift apart: both come from the adapter's own `Config`.
    // Only the pi-ai layout has a per-route protocol for the read to find, and
    // it rehydrates the whole section schema, so the other layouts skip it.
    const protocols = useMemo(() => layout === 'pi-ai' ? protocolChoices(namespace, schema) : [], [layout, namespace, schema]);
    const stringAt = (source, key) => {
        const value = schema.getPath(source, [key]);
        return typeof value === 'string' && value.trim().length > 0 ? value : undefined;
    };
    const setField = (key, next) => {
        // A value of nothing but whitespace is cleared, not stored: `stringAt`
        // already reports it as absent, so the field would otherwise render empty
        // while the draft still carried the spaces into `cordis.patch.yml`, where
        // both adapters would accept that non-empty string as a real value.
        const value = next === undefined || next.trim().length === 0 ? undefined : next;
        setDraft(current => value === undefined
            ? schema.deletePath(current, [key])
            : schema.setPath(current, [key], value));
    };
    const clearsDerivedRef = keyRemoval !== undefined && layout === 'pi-ai'
        && props.nativeAuth !== undefined && stringAt(draft, 'apiKeyEnv') === deriveKeyRef(props.provider);
    // The model list is validated by the same per-row checker for both families,
    // so a bad row is named by its position rather than by a blanket message.
    // Only a hand-declared pi-ai route's rows edit reasoning levels — the one
    // model list with no installed catalog behind it — so only there are the
    // levels read and the declarations checked.
    const schemaReasoningLevels = useMemo(() => layout === 'pi-ai' && props.declared === true ? reasoningLevelChoices(namespace, schema) : [], [layout, props.declared, namespace, schema]);
    // What the form currently shows, which is what an interrogation must ask and
    // what decides the protocol's own constraints: an edited-but-unsaved protocol.
    const probeApi = stringAt(draft, 'api') ?? stringAt(fallback, 'api');
    const reasoning = protocolReasoningLevels(probeApi, schemaReasoningLevels);
    const editsReasoning = reasoning.levels.length > 0;
    const validateModels = (value) => validateDeepSeekModels(value)
        ?? (editsReasoning ? validateReasoningEfforts(value, reasoning.levelsOnly ? reasoning.levels : undefined) : undefined);
    // Inherited models are checked too where the protocol narrows their levels:
    // switching an inherited route's protocol must not write a route its own models break.
    const modelFailure = validateModels(schema.getPath(draft, ['models'])
        ?? (reasoning.levelsOnly ? schema.getPath(fallback, ['models']) : undefined));
    const retryNode = useMemo(() => layout === 'unknown' ? undefined : schema.nodeAtPath(root, [...settingsPath, 'retryPolicy']), [layout, root, schema, settingsPath]);
    const retryDefaults = retryNode === undefined
        ? undefined
        : retryPolicyDefaults(schema, retryNode, schema.getPath(namespace.base, [...settingsPath, 'retryPolicy']));
    const retryFailure = retryNode === undefined || retryDefaults === undefined || props.credentialOnly === true
        ? undefined
        : retryPolicyFailure(schema, retryNode, schema.getPath(draft, ['retryPolicy']), retryDefaults, t);
    const keyFailure = apiKeyFailure(keyText);
    // What a probe or a write must carry: the typed key with paste whitespace
    // removed. A blank field yields an empty string, which both call sites read
    // as "no key supplied" rather than as a key — that is how a card whose
    // provider already has a stored key is edited without re-entering it.
    const keyValue = keyText.trim();
    // A route whose protocol cannot authenticate without a key needs a key reference or a typed key.
    const protocolNeedsKeyHere = layout === 'pi-ai' && protocolNeedsKey(probeApi)
        && stringAt(draft, 'apiKeyEnv') === undefined && stringAt(fallback, 'apiKeyEnv') === undefined;
    const protocolKeyMissing = protocolNeedsKeyHere && keyValue.length === 0;
    const credentialRequiredFailure = (props.credentialRequired === true
        && keyText.length > 0 && keyValue.length === 0) || protocolKeyMissing
        ? 'keyRequired'
        : undefined;
    const shownKeyFailure = credentialRequiredFailure ?? keyFailure;
    // An edited-but-unsaved endpoint, and a key typed but not yet stored.
    const probeBaseURL = stringAt(draft, 'baseURL') ?? stringAt(fallback, 'baseURL');
    const probe = {
        settingsNs: namespace.ns,
        // Naming the route lets an adapter that already describes it answer from
        // its own registry — better metadata, no network call, no endpoint needed.
        provider: props.provider,
        ...probeBaseURL === undefined ? {} : { baseURL: probeBaseURL },
        ...probeApi === undefined ? {} : { api: probeApi },
        ...keyValue.length === 0 ? {} : { apiKey: keyValue },
    };
    /**
     * The write for this card, or a failure message. Every edit travels as
     * path ops against the STORED section: the draft comes from the redacted
     * descriptor, so a wholesale replace rebuilt from it could delete fields
     * outside the card. Ops name only the fields this card can see.
     */
    const applyOnce = async () => {
        const ns = namespace.ns;
        // Removing a catalog route's conventional reference restores native auth.
        // Other aliases and custom routes keep their named environment fallback.
        const profileDraft = clearsDerivedRef ? schema.deletePath(draft, ['apiKeyEnv']) : draft;
        // A pi-ai profile names the conventional reference only when this page is
        // about to store a key. Otherwise the provider keeps its native auth path.
        const next = layout === 'pi-ai' && stringAt(profileDraft, 'apiKeyEnv') === undefined
            && stringAt(schema.getPath(namespace.base, settingsPath), 'apiKeyEnv') === undefined && keyValue.length > 0
            ? schema.setPath(profileDraft, ['apiKeyEnv'], keyRef)
            : profileDraft;
        if (props.credentialOnly !== true) {
            // The same checker gates the submit button, so a card cannot reach this
            // with a bad row; it stays because the schema check below would refuse
            // the write with a message naming a path instead of the row, and because
            // nothing but this function decides what is written.
            const failure = validateModels(schema.getPath(next, ['models']));
            /* v8 ignore next 3 -- unreachable from the card: the same failure disables submit */
            if (failure !== undefined) {
                return `${t('model')} ${String(failure.index + 1)}: ${t(failure.key)}`;
            }
        }
        /* v8 ignore next -- apply is only reachable from the rendered card, which required a resolved node */
        if (props.credentialOnly !== true && node !== undefined && settingsPath.length === 0) {
            const sectionError = schema.validate(node, next);
            if (sectionError !== undefined)
                return sectionError;
        }
        const materializesNativeProfile = layout === 'pi-ai'
            && fallback === undefined
            && committedOriginal === undefined
            && Object.keys(next).length === 0;
        const ops = props.credentialOnly === true
            ? []
            : materializesNativeProfile
                ? [{ op: 'set', path: [...settingsPath], value: {} }]
                : pathOps(settingsPath, committedOriginal, next);
        if (keyRemoval !== undefined && keyRemoval.removed !== true) {
            const removed = await operations.removeCredential(keyRemoval.ref);
            if (removed !== undefined)
                return removed;
            // A later settings failure cannot restore a secret that was deleted.
            setKeyDraft({ ...keyRemoval, removed: true });
        }
        if (ops.length > 0) {
            const written = await operations.writeSettings(ns, ops, expectedRevision);
            if (written.kind !== 'written')
                return written.kind === 'conflict' ? t('conflict') : written.message;
            setCommittedOriginal(schema.getPath(written.view.user, settingsPath));
            setExpectedRevision(written.view.revision);
            setDraft(next);
        }
        if (keyValue.length > 0) {
            const stored = await operations.storeCredential(keyRef, keyValue);
            if (stored !== undefined)
                return stored;
        }
        setKeyDraft('');
        return undefined;
    };
    const apply = async () => {
        setBusy(true);
        setFailure(undefined);
        try {
            const failure = await applyOnce();
            if (failure !== undefined) {
                setFailure(failure);
                return;
            }
            props.onClose(true);
        }
        finally {
            setBusy(false);
        }
    };
    if (node === undefined) {
        // A directory entry addressing a position its schema cannot resolve is a
        // host-side inconsistency; showing it beats a blank card.
        return _jsxs("p", { className: styles['error'], children: [props.provider, ": ", props.t('settingsPathUnresolvable')] });
    }
    /**
     * The catalog beneath the user layer: what the composition entry pinned, or
     * else the schema default that `resolve` would supply. The effective value
     * cannot answer this — it still carries the stored override until the unset
     * is applied, so reading it would echo that override straight back the
     * moment reset drops it, leaving the rows unchanged until a reload.
     */
    const inheritedModels = () => {
        const pinned = schema.getPath(namespace.base, [...settingsPath, 'models']);
        return pinned ?? schema.nodeAtPath(root, [...settingsPath, 'models'])?.meta.default;
    };
    /**
     * The curated fields of one known adapter family. The family arrives
     * narrowed so the per-family branches below are total: an unknown namespace
     * renders the hint instead and never reaches this body.
     */
    const curatedFields = (family) => {
        // What a hand-declared route names for itself and nothing else can supply.
        // A whole-section `llm-deepseek` profile is a composition fact with no
        // per-route identity for its schema to carry, hence the family test.
        const ownsIdentity = family === 'pi-ai' && props.declared === true;
        const customModels = schema.getPath(draft, ['models']);
        const modelsOverridden = schema.hasPath(draft, ['models']);
        const models = modelDrafts(modelsOverridden ? customModels : inheritedModels());
        const defaultContextWindow = schema.getPath(fallback, ['defaultContextWindow']);
        const defaultMaxTokens = schema.getPath(fallback, ['maxTokens']);
        const defaultInput = schema.getPath(fallback, ['defaultInput']);
        const baseURL = stringAt(fallback, 'baseURL') ?? props.defaultBaseURL;
        const endpointEnv = props.baseURLEnv ?? [];
        const endpointHint = family === 'deepseek' ? t('deepSeekEndpointHint')
            : baseURL === undefined && endpointEnv.length > 0 ? t('baseUrlEnvironment').replace('{env}', () => endpointEnv.join(', ')) : undefined;
        // A named reference is the only credential the adapter reads for this
        // route. Without one, a catalog route authenticates the way the directory
        // reports, and a declared route sends no key at all.
        const namedRef = clearsDerivedRef
            ? stringAt(schema.getPath(namespace.base, settingsPath), 'apiKeyEnv')
            : stringAt(fallback, 'apiKeyEnv');
        const nativeAuth = namedRef === undefined ? props.nativeAuth : undefined;
        // Display method-neutral references without changing the adapter's lookup.
        const authRefs = namedRef === undefined
            ? (nativeAuth?.env ?? []).filter(ref => !endpointEnv.includes(ref)) : [namedRef];
        const visibleRefs = authRefs.filter(ref => !ref.toLowerCase().includes('oauth'));
        const keyHint = visibleRefs[0];
        const checkedRefs = authRefs.map(ref => [
            ref, props.credentialChecks?.[ref] ?? (ref === namedRef ? props.credential : undefined),
        ]);
        const detected = checkedRefs.find(([, info]) => info?.configured === true);
        const detectedRef = detected?.[0];
        const detectedVisibleRef = detectedRef === undefined || !visibleRefs.includes(detectedRef) ? undefined : detectedRef;
        const missingRefs = checkedRefs.length > 0 && checkedRefs.every(([, info]) => info?.configured === false);
        const savedCredential = detected?.[1]?.source === 'file' && detectedVisibleRef !== undefined;
        const credentialHint = props.credentialRequired === true
            ? undefined
            : keyText.length > 0 ? savedCredential ? t('keyReplacePending') : undefined
                : keyRemoval !== undefined ? keyRemoval.removed === true ? t('keyRemoveApplied')
                    : keyHint === undefined ? t('keyRemovePending')
                        : t('keyRemovePendingEnvironment').replace('{hint}', () => keyHint)
                    : keyHint === undefined ? undefined
                        : savedCredential
                            ? t('keySaved')
                            : t(detectedRef === undefined ? missingRefs ? 'keyFallbackMissing' : 'keyFallbackUnchecked'
                                : detectedVisibleRef === undefined ? 'keyFallbackDetectedHidden' : 'keyFallbackDetected')
                                .replace('{hint}', () => detectedVisibleRef ?? keyHint);
        const keyHelp = visibleRefs.length === 0
            ? undefined
            : t('keyHelp').replace('{env}', () => visibleRefs.join(' / '));
        const authHelp = keyHelp === undefined ? null : (_jsx(Tooltip, { label: keyHelp, side: "right", delayMs: 0, focusDelayMs: 0, portal: true, openOnClick: true, children: _jsx("button", { type: "button", className: styles['helpButton'], "aria-label": t('authHelp'), children: "?" }) }));
        // A provider with no native key authentication has no key for this field to store.
        const keylessNativeAuth = nativeAuth?.env.length === 0 && nativeAuth.oauth;
        const keyPlaceholder = keyHint === undefined || props.credentialRequired === true
            ? t('keyPlaceholder')
            : t('keyPlaceholderFallback').replace('{hint}', () => keyHint);
        /** What both family editors take: the rows, whose layer owns them, and the two writes. */
        const catalogProps = {
            models,
            overridden: modelsOverridden,
            t,
            disabled,
            onChange: (next) => {
                setDraft(current => schema.setPath(current, ['models'], next));
            },
            onReset: () => { setDraft(current => schema.deletePath(current, ['models'])); },
        };
        return (_jsxs(_Fragment, { children: [keylessNativeAuth
                    ? null
                    : _jsxs("div", { className: styles['field'], children: [_jsxs("span", { className: styles['fieldLabel'], children: [t('keyInput'), authHelp] }), _jsx(ApiKeyInput, { t: t, value: keyText, ...keyDraft === undefined && savedCredential ? {
                                    saved: {
                                        masked: detected[1].masked ?? '',
                                        ...detected[1].writable
                                            ? { clear: () => { setKeyDraft({ ref: detected[0] }); setFailure(undefined); } } : {},
                                    },
                                } : {}, placeholder: keyPlaceholder, "aria-label": t('keyInput'), "aria-invalid": shownKeyFailure !== undefined && !protocolKeyMissing, "aria-describedby": credentialHint === undefined ? undefined : `${props.provider}-credential-hint`, required: props.credentialRequired === true || protocolNeedsKeyHere, autoFocus: props.autoFocusCredential === true, disabled: disabled || keyRemoval?.removed === true, onChange: (event) => {
                                    setKeyDraft(event.target.value === '' && savedCredential
                                        ? detected[1].writable ? { ref: detected[0] } : undefined
                                        : event.target.value);
                                    setFailure(undefined);
                                } }), credentialHint === undefined ? null : _jsx("p", { id: `${props.provider}-credential-hint`, className: styles[keyRemoval !== undefined || missingRefs ? 'notice' : keyText.length > 0 || detectedRef === undefined ? 'advancedHint' : 'savedNotice'], children: credentialHint }), shownKeyFailure === undefined
                                ? null
                                : _jsx("p", { className: styles[protocolKeyMissing ? 'advancedHint' : 'error'], children: t(shownKeyFailure) })] }), props.credentialOnly === true ? null : _jsxs("details", { className: styles['customized'], onToggle: (event) => { setCustomizedOpen(event.currentTarget.open); }, children: [_jsx("summary", { className: styles['customizedSummary'], children: t('customized') }), _jsxs("div", { className: styles['customizedBody'], children: [ownsIdentity
                                    ? (_jsxs("div", { className: styles['field'], children: [_jsx("span", { className: styles['fieldLabel'], children: t('customDisplayName') }), _jsx("input", { className: styles['input'], type: "text", value: stringAt(draft, 'displayName') ?? '', 
                                                // What this route is called the moment the field is
                                                // cleared, which is the layer beneath the one this field
                                                // edits: a `cordis.yml` may pin a name for a route the
                                                // catalog does not ship, and only when nothing does is
                                                // the answer the route id. Reading the effective value
                                                // instead would echo the stored override back as the
                                                // thing clearing restores.
                                                placeholder: stringAt(schema.getPath(namespace.base, settingsPath), 'displayName')
                                                    ?? props.provider, "aria-label": t('customDisplayName'), disabled: disabled, onChange: (event) => { setField('displayName', event.target.value); } })] }))
                                    : null, _jsxs("div", { className: styles['field'], children: [_jsx("span", { className: styles['fieldLabel'], children: t('baseUrl') }), _jsx("input", { className: styles['input'], type: "text", value: stringAt(draft, 'baseURL') ?? '', placeholder: family === 'deepseek'
                                                ? t('deepSeekBaseUrl')
                                                : baseURL === undefined ? t(endpointEnv.length > 0 ? 'baseUrlConfigured' : 'baseUrlDefault')
                                                    : t('baseUrlPlaceholder').replace('{url}', () => baseURL), "aria-describedby": endpointHint === undefined ? undefined : `${props.provider}-endpoint-hint`, "aria-label": t('baseUrl'), disabled: disabled, onChange: (event) => {
                                                setField('baseURL', event.target.value === '' ? undefined : event.target.value);
                                            } }), endpointHint === undefined ? null : _jsx("span", { id: `${props.provider}-endpoint-hint`, className: styles['advancedHint'], children: endpointHint })] }), ownsIdentity
                                    ? (_jsxs("div", { className: styles['field'], children: [_jsx("span", { className: styles['fieldLabel'], children: t('customApi') }), _jsxs("select", { className: `${styles['input']} ${styles['selectInput']}`, value: probeApi ?? '', "aria-label": t('customApi'), disabled: disabled, onChange: (event) => { setField('api', event.target.value); }, children: [probeApi === undefined ? _jsx("option", { value: "", children: t('customApiUnset') }) : null, protocols.map(choice => _jsx("option", { value: choice, children: protocolLabel(t, choice) }, choice))] })] }))
                                    : null, retryDefaults === undefined
                                    ? null
                                    : (_jsx(RetryPolicyEditor, { policy: schema.getPath(draft, ['retryPolicy']), defaults: retryDefaults, failure: retryFailure, failureId: `${props.provider}-retry-failure`, showFailure: customizedOpen, t: t, disabled: disabled, onChange: (next) => {
                                            setDraft(current => next === undefined
                                                ? schema.deletePath(current, ['retryPolicy'])
                                                : schema.setPath(current, ['retryPolicy'], next));
                                        } })), family === 'deepseek'
                                    ? (_jsx(DeepSeekModelsEditor, { ...catalogProps, defaultContextWindow: typeof defaultContextWindow === 'number'
                                            ? defaultContextWindow
                                            : undefined, defaultMaxTokens: typeof defaultMaxTokens === 'number' ? defaultMaxTokens : undefined }))
                                    : (_jsx(ModelListEditor, { ...catalogProps, catalogProvider: props.declared === true ? undefined : props.provider, reasoningLevels: reasoning.levels, reasoningLevelsOnly: reasoning.levelsOnly, defaultInput: Array.isArray(defaultInput) ? defaultInput : undefined, probe: probe, probeBlocked: keyRemoval !== undefined ? 'keyRemovePending' : shownKeyFailure, operations: operations, onBusyChange: setListBusy }))] })] })] }));
    };
    return (_jsxs("div", { className: props.credentialOnly === true ? styles['addBlock'] : styles['editor'], children: [props.hideTitle === true
                ? null
                : (_jsxs("div", { className: styles['editorHeader'], children: [_jsx("span", { className: styles['editorTitle'], children: props.displayName }), props.provider !== props.displayName
                            ? _jsx("span", { className: styles['editorRoute'], children: props.provider })
                            : null] })), layout === 'unknown'
                ? _jsx("p", { className: styles['advancedHint'], children: `${t('advancedHint')} (${namespace.ns})` })
                : curatedFields(layout), failure !== undefined ? _jsx("p", { className: styles['error'], children: failure }) : null, props.credentialOnly === true || modelFailure === undefined
                ? null
                : (_jsx("p", { className: styles['advancedHint'], children: `${t('model')} ${String(modelFailure.index + 1)}: ${t(modelFailure.key)}` })), retryFailure === undefined || customizedOpen
                ? null
                : _jsx("p", { id: `${props.provider}-retry-failure`, className: styles['error'], children: retryFailure.message }), _jsx(EditorFooter, { t: t, busy: busy, submitDisabled: disabled || layout === 'unknown'
                    || (props.credentialOnly !== true && modelFailure !== undefined)
                    || retryFailure !== undefined
                    || shownKeyFailure !== undefined
                    || (props.credentialRequired === true && keyValue.length === 0), submitLabelKey: props.submitLabelKey ?? 'apply', submitBusyLabelKey: props.submitBusyLabelKey ?? 'applying', cancelLabelKey: keyRemoval?.removed === true ? 'close' : props.cancelLabelKey ?? 'cancel', onCancel: () => { props.onClose(false); }, onSubmit: () => { props.onSubmitCredential?.(); void apply(); } })] }));
}
//# sourceMappingURL=ProviderEditor.js.map