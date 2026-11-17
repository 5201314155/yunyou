import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/**
 * The search settings panel edits the navigator's draft of excluded directories.
 * The navigator saves it when the panel closes, including through the magnifier.
 * The `.gitignore` choice above the list is saved as soon as it changes, as
 * the hidden-entry control in the search bar is.
 *
 * It floats over the navigator below the search field. The field that adds a
 * directory stays above the list, which alone scrolls; a new rule goes to the
 * top. The header's restore control appears while the list differs from the
 * default list in any way, order included, and replaces it with that list. The close control, Escape, and focus leaving the panel request
 * closure; focus moving to the magnifier leaves closure to its click.
 */
import { useId, useRef, useState } from 'react';
import clsx from 'clsx';
import { IconCloseOutlineRegular, IconPlusOutlineRegular, IconRefreshOutlineRegular, IconTrashOutlineRegular, Tooltip, } from '@deepseek-ai/dsh-client-ui-primitives';
import { addRule, defaultRules, isDefaultRules, removeRule, setRuleEnabled, } from "./search-rules.js";
import css from './FilesBody.module.css';
/**
 * The settings panel.
 * @param props.rules - the navigator's current draft.
 * @param props.respectGitignore - whether the workspace's search leaves out entries `.gitignore` ignores.
 * @param props.onRespectGitignore - saves a new `.gitignore` choice at once.
 * @param props.trigger - the control that opens and closes the panel; focus moving to it leaves the closing to its click.
 * @param props.onChange - replaces the draft without saving preferences.
 * @param props.onClose - saves and closes the draft, returning focus to the trigger when requested.
 * @param props.t - namespace-bound translate.
 * @returns the panel.
 */
export function SearchSettings({ rules, respectGitignore, onRespectGitignore, trigger, onChange, onClose, t }) {
    const [text, setText] = useState('');
    const [invalid, setInvalid] = useState(undefined);
    const add = useRef(null);
    const title = useId();
    const error = useId();
    // A control that is about to disappear hands the keyboard to the add field first, so the panel keeps it.
    const keepFocus = () => { add.current?.focus(); };
    return (_jsxs("section", { className: css.settings, role: "dialog", "aria-labelledby": title, tabIndex: -1, "data-files-search-settings": true, onKeyDown: (event) => {
            if (event.key !== 'Escape' || event.nativeEvent.isComposing)
                return;
            // Kept from the navigator's own Escape and the overlay's close on the document.
            event.preventDefault();
            event.stopPropagation();
            onClose(true);
        }, onBlur: (event) => {
            const next = event.relatedTarget;
            if (next instanceof Node && (event.currentTarget.contains(next) || trigger.current?.contains(next) === true))
                return;
            onClose(false);
        }, children: [_jsxs("div", { className: css.settingsHeader, children: [_jsx("span", { id: title, className: css.settingsTitle, children: t('settings.title') }), !isDefaultRules(rules) && (_jsx(Tooltip, { label: t('settings.restore'), side: "bottom", portal: true, children: _jsx("button", { type: "button", className: css.tool, "aria-label": t('settings.restore'), "data-files-settings-restore": true, onClick: () => {
                                keepFocus();
                                onChange(defaultRules());
                            }, children: _jsx(IconRefreshOutlineRegular, {}) }) })), _jsx(Tooltip, { label: t('settings.close'), side: "bottom", portal: true, children: _jsx("button", { type: "button", className: css.tool, "aria-label": t('settings.close'), onClick: () => { onClose(true); }, children: _jsx(IconCloseOutlineRegular, {}) }) })] }), _jsx("p", { className: css.settingsDescription, children: t('settings.description') }), _jsx("div", { className: clsx(css.rule, css.settingsOption), children: _jsxs("label", { className: css.ruleLabel, children: [_jsx("input", { type: "checkbox", className: css.ruleCheck, checked: respectGitignore, "data-files-respect-gitignore": true, onChange: (event) => { onRespectGitignore(event.target.checked); } }), _jsx("span", { className: css.ruleName, children: t('settings.respectGitignore') })] }) }), _jsxs("div", { className: clsx(css.rule, css.addRule), children: [_jsx(IconPlusOutlineRegular, { className: css.icon }), _jsx("input", { ref: add, className: css.ruleInput, type: "text", value: text, placeholder: t('settings.add'), "aria-label": t('settings.add'), "aria-invalid": invalid !== undefined, "aria-describedby": invalid === undefined ? undefined : error, autoComplete: "off", spellCheck: false, autoFocus: true, onChange: (event) => {
                            setText(event.target.value);
                            setInvalid(undefined);
                        }, onKeyDown: (event) => {
                            if (event.key !== 'Enter' || event.nativeEvent.isComposing)
                                return;
                            event.preventDefault();
                            const added = addRule(rules, text);
                            if (added.kind === 'invalid') {
                                setInvalid(added.reason);
                                return;
                            }
                            onChange(added.rules);
                            setText('');
                        } })] }), invalid !== undefined && _jsx("p", { id: error, className: css.settingsError, role: "alert", children: t(`settings.invalid.${invalid}`) }), _jsx("ul", { className: css.rules, children: rules.map(rule => (_jsxs("li", { className: clsx(css.rule, !rule.enabled && css.ruleOff), "data-files-rule": rule.path, children: [_jsxs("label", { className: css.ruleLabel, children: [_jsx("input", { type: "checkbox", className: css.ruleCheck, checked: rule.enabled, "aria-label": t('settings.toggle', { path: rule.path }), onChange: (event) => { onChange(setRuleEnabled(rules, rule.path, event.target.checked)); } }), _jsx("span", { className: css.ruleName, children: rule.path })] }), _jsx(Tooltip, { label: t('settings.remove', { path: rule.path }), side: "bottom", portal: true, children: _jsx("button", { type: "button", className: css.tool, "aria-label": t('settings.remove', { path: rule.path }), onClick: () => {
                                    keepFocus();
                                    onChange(removeRule(rules, rule.path));
                                }, children: _jsx(IconTrashOutlineRegular, {}) }) })] }, rule.path))) })] }));
}
//# sourceMappingURL=SearchSettings.js.map