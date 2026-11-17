import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/**
 * The web search and fetch settings page: one section per capability, each
 * with a provider menu and its explanation. The selected provider's settings
 * below the menu sit in one gray panel: the result count and billing for the
 * DeepSeek Search API, billing for the DeepSeek Fetch API, and the key,
 * endpoint, and search limit for legacy Messages search. A provider with no
 * settings here shows no panel.
 */
import { useRef, useState } from 'react';
import { Button, IconChevronDownOutlineRegular, Menu, SettingsForm, SettingsSecretField, SettingsValueField, Tag, } from '@deepseek-ai/dsh-client-ui-primitives';
import { formLabels } from "./locales.js";
import styles from './WebSearchCard.module.css';
import { PROVIDER_IDS, } from "./web-search-card-controller.js";
/** Dictionary names of the official providers; any other provider shows its registered title or id. */
const PROVIDER_LABELS = {
    [PROVIDER_IDS.apiSearch]: 'providerDeepSeek',
    [PROVIDER_IDS.legacySearch]: 'providerLegacy',
    [PROVIDER_IDS.localFetch]: 'providerLocal',
    [PROVIDER_IDS.apiFetch]: 'providerDeepSeekFetch',
};
/**
 * The menu label of one provider.
 * @param t - the page's locale reader.
 * @param option - the provider entry.
 * @returns the localized name, marked when the Host does not register it.
 */
function providerLabel(t, option) {
    const key = PROVIDER_LABELS[option.id];
    const name = key === undefined ? option.title ?? option.id : t(key);
    return option.missing ? t('providerMissing', { id: name }) : name;
}
/**
 * Render the page's one-liner or its settings form, as the Plugins page asks.
 * @param props - the view asked for, locale copy, the form snapshot, and its actions.
 * @returns the one-liner, or the form.
 */
export function WebSearchCard(props) {
    const { t } = props;
    const state = props.useWebSearchCard(snapshot => snapshot);
    if (props.view === 'summary')
        return t('description');
    const disabled = !state.writable || state.saving;
    const menuDisabled = disabled || state.selectionBlocked;
    const { search, fetch } = state;
    // The selected provider's own description, in the active language; an empty translation shows nothing.
    const describe = (section) => {
        const description = section.options.find(option => option.id === section.selected)?.description;
        return description === undefined ? undefined : props.resolveText(description) || undefined;
    };
    return (_jsxs(SettingsForm, { labels: formLabels(t), state: state, onSave: props.save, onDiscard: props.discard, children: [_jsxs("section", { className: styles.section, "aria-labelledby": "plugin-config-web-search-section", children: [_jsx("h4", { id: "plugin-config-web-search-section", className: styles.sectionTitle, children: t('searchSection') }), _jsx(ProviderSelect, { id: "plugin-config-web-search-provider", labelledBy: "plugin-config-web-search-section", t: t, section: search, disabled: menuDisabled, field: "searchProvider", onSelect: props.selectProvider, providersFailed: state.providersFailed, onRetry: props.retryProviders, description: describe(search), hint: search.kind === 'deepseek'
                            ? _jsx("a", { className: styles.link, href: t('searchGuideUrl'), target: "_blank", rel: "noopener noreferrer", children: t('searchGuide') })
                            : search.kind === 'plugin' ? t('pluginProviderHint') : undefined }), search.kind === 'deepseek' && (_jsxs("div", { className: styles.panel, children: [_jsx(SettingsValueField, { id: "plugin-config-web-search-count", label: t('count'), hint: t('countHint'), numeric: true, overriddenLabel: t('overridden'), resetLabel: t('reset'), invalidLabel: t('invalidCount'), disabled: disabled, ...state.count, onEdit: (text) => { props.edit('count', text); }, onReset: () => { props.resetField('count'); } }), state.searchBilling !== undefined && (_jsx(BillingField, { capability: "search", t: t, billing: state.searchBilling, disabled: disabled, onSelect: props.selectBilling, resolveText: props.resolveText }))] })), search.kind === 'legacy' && state.legacyKey !== undefined && (_jsxs("div", { className: styles.panel, children: [_jsx(SettingsSecretField, { id: "plugin-config-web-search-key", label: t('credential'), hint: t('legacyKeyHint', { ref: state.legacyKey.ref }), 
                                // The credentials domain accepts a key even when the settings document
                                // itself is read-only; they are separate stores with separate refusals,
                                // so the settings document's writability does not disable this control.
                                // The key's own writability does (a key sourced from the process
                                // environment cannot be written from here), and so does a save in
                                // flight, which would discard an edit made before it settles.
                                disabled: !state.legacyKey.writable || state.saving, text: state.legacyKey.text, configured: state.legacyKey.configured, stateLabel: state.legacyKey.configured ? t('apiKeySet') : t('apiKeyUnsetApi'), onEdit: (text) => { props.edit('legacyApiKey', text); } }), _jsx(SettingsValueField, { id: "plugin-config-web-search-endpoint", label: t('baseUrl'), hint: t('baseUrlHint', { path: '/messages' }), overriddenLabel: t('overridden'), resetLabel: t('reset'), invalidLabel: t('invalidNumber'), disabled: disabled, ...state.messagesBaseURL, onEdit: (text) => { props.edit('messagesBaseURL', text); }, onReset: () => { props.resetField('messagesBaseURL'); } }), _jsx(SettingsValueField, { id: "plugin-config-web-search-max-uses", label: t('maxUses'), hint: t('maxUsesHint'), numeric: true, overriddenLabel: t('overridden'), resetLabel: t('reset'), invalidLabel: t('invalidMaxUses'), disabled: disabled, ...state.maxUses, onEdit: (text) => { props.edit('maxUses', text); }, onReset: () => { props.resetField('maxUses'); } })] }))] }), _jsxs("section", { className: styles.section, "aria-labelledby": "plugin-config-web-fetch-section", children: [_jsx("h4", { id: "plugin-config-web-fetch-section", className: styles.sectionTitle, children: t('fetchSection') }), _jsx(ProviderSelect, { id: "plugin-config-web-fetch-provider", labelledBy: "plugin-config-web-fetch-section", t: t, section: fetch, disabled: menuDisabled, field: "fetchProvider", onSelect: props.selectProvider, providersFailed: state.providersFailed, onRetry: props.retryProviders, description: describe(fetch), hint: fetch.kind === 'plugin' ? t('pluginProviderHint') : undefined }), state.fetchBilling !== undefined && (_jsx("div", { className: styles.panel, children: _jsx(BillingField, { capability: "fetch", t: t, billing: state.fetchBilling, disabled: disabled, onSelect: props.selectBilling, resolveText: props.resolveText }) }))] })] }));
}
/** Menu id of the automatic-selection entry, matching a selection that names no provider. */
const AUTO_ID = '';
/**
 * A menu button that shows its current value and opens a list of choices.
 * Its accessible name is the element `labelledBy` names followed by the current
 * value, so the label does not replace the value.
 * @param props - the control id, the id of the element naming it, the entries, the current entry and its
 * label, the lock state, and the selection callback.
 * @returns the anchored menu.
 */
function Dropdown(props) {
    const [open, setOpen] = useState(false);
    const trigger = useRef(null);
    // A disabled menu stays closed even if it was open when the lock arrived.
    const shown = open && !props.disabled;
    const valueId = `${props.id}-value`;
    const anchor = (_jsxs("button", { ref: trigger, id: props.id, type: "button", className: styles.selector, "aria-haspopup": "menu", "aria-expanded": shown, "aria-labelledby": `${props.labelledBy} ${valueId}`, disabled: props.disabled, onClick: () => { setOpen(value => !value); }, children: [_jsx("span", { id: valueId, className: styles.selectorLabel, children: props.valueLabel }), _jsx(IconChevronDownOutlineRegular, { className: styles.chevron })] }));
    return (_jsx(Menu, { open: shown, onClose: () => { setOpen(false); }, items: props.items, selectedId: props.selectedId, onSelect: (id) => {
            trigger.current?.focus({ preventScroll: true });
            setOpen(false);
            if (id !== props.selectedId)
                props.onSelect(id);
        }, portal: true, autoFocus: true, minWidthFromAnchor: true, anchor: anchor }));
}
/** Dictionary names of each billing and its explanation, in menu order. */
const BILLING_COPY = {
    'auto': { nameKey: 'billingAuto', hintKey: 'billingAutoHint' },
    'account': { nameKey: 'billingAccount', hintKey: 'billingAccountHint' },
    'api-key': { nameKey: 'billingApiKey', hintKey: 'billingApiKeyHint' },
};
/** Billing entries in menu order. */
const BILLINGS = ['auto', 'account', 'api-key'];
/** Separator between the field names of the advanced-override tag, by language. */
const FIELD_SEPARATOR = { en: ', ', zh: '、', ja: '、', es: ', ', fr: ', ', de: ', ', ru: ', ', pt: ', ' };
/**
 * One capability's billing menu with the selected billing's explanation. While the capability's own endpoint fields
 * override billing, a tag beside the label names them and the menu is locked.
 * @param props - the capability, locale copy, the billing state, the settings lock, the selection callback, and the
 * locale resolver for the field-name separator.
 * @returns the labelled billing menu.
 */
function BillingField(props) {
    const { t, billing, capability } = props;
    const id = `plugin-config-web-${capability}-billing`;
    const labelId = `${id}-label`;
    return (_jsxs("div", { className: styles.panelField, children: [_jsxs("div", { className: styles.panelHead, children: [_jsx("span", { id: labelId, className: styles.panelLabel, children: t('billing') }), billing.advancedOverride.length > 0 && (_jsx(Tag, { tone: "neutral", children: t('billingAdvancedOverride', { fields: billing.advancedOverride.join(props.resolveText(FIELD_SEPARATOR)) }) }))] }), _jsx(Dropdown, { id: id, labelledBy: labelId, items: BILLINGS.map(mode => ({ id: mode, label: t(BILLING_COPY[mode].nameKey) })), selectedId: billing.value, valueLabel: t(BILLING_COPY[billing.value].nameKey), disabled: props.disabled || billing.advancedOverride.length > 0, 
                // The menu offers only BILLINGS entries, so the fallback never applies.
                /* v8 ignore next */
                onSelect: (mode) => { props.onSelect(capability, BILLINGS.find(entry => entry === mode) ?? billing.value); } }), _jsx("p", { className: styles.hint, children: t(BILLING_COPY[billing.value].hintKey) })] }));
}
/**
 * A notice that a list could not be read, with a retry.
 * @param props - the notice copy, the retry label, and the retry callback.
 * @returns the status line.
 */
function ReadFailure(props) {
    return (_jsxs("div", { className: styles.notice, role: "status", children: [_jsx("span", { children: props.message }), _jsx(Button, { variant: "outline", size: "sm", onClick: props.onRetry, children: props.retry })] }));
}
/**
 * One section's provider menu, with the selected provider's description and
 * the page's own line about it below, and a notice with a retry while the provider list could not be read.
 * @param props - the control id, the id of the section title naming it, the section state, the selection callback, and the list read state.
 * @returns the labelled menu.
 */
function ProviderSelect(props) {
    const { t, section } = props;
    const current = section.options.find(option => option.id === section.selected);
    const items = [
        ...section.auto ? [{ id: AUTO_ID, label: t('providerAuto') }] : [],
        ...section.options.map(option => ({ id: option.id, label: providerLabel(t, option) })),
    ];
    return (_jsxs("div", { className: styles.field, children: [_jsx(Dropdown, { id: props.id, labelledBy: props.labelledBy, items: items, selectedId: section.selected, valueLabel: current === undefined ? t('providerAuto') : providerLabel(t, current), disabled: props.disabled, onSelect: (id) => { props.onSelect(props.field, id); } }), props.description !== undefined && _jsx("p", { className: styles.hint, children: props.description }), props.hint !== undefined && _jsx("p", { className: styles.hint, children: props.hint }), props.providersFailed && _jsx(ReadFailure, { message: t('providersFailed'), retry: t('retry'), onRetry: props.onRetry })] }));
}
//# sourceMappingURL=WebSearchCard.js.map