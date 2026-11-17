import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/**
 * The provider retry-policy fields of an editor card, built from the
 * `retryPolicy` schema the owning namespace publishes: its union branches are
 * the modes, each branch's fields are the inputs, and the field bounds and
 * defaults come from the schema. Number fields render as inputs, string lists
 * as a code picker, and nested objects as a fold. An absent `retryPolicy`
 * keeps the inherited policy; choosing a mode writes a user-layer policy whose
 * empty fields keep the inherited values.
 */
import { useState } from 'react';
import { Checkbox } from '@deepseek-ai/dsh-client-ui-primitives';
import styles from './ModelsSection.module.css';
/** Labels of known modes and fields by schema key; any other key shows verbatim. */
const LABELS = new Map([
    ['normal', 'retryModeNormal'],
    ['always', 'retryModeAlways'],
    ['maxRetries', 'retryMaxRetries'],
    ['retryableCodes', 'retryCodes'],
    ['backoff', 'retryBackoff'],
    ['initialDelayMs', 'retryInitialDelay'],
    ['maxDelayMs', 'retryMaxDelay'],
    ['jitterRatio', 'retryJitter'],
]);
/** Hints shown under the mode select, by mode. */
const MODE_HINTS = new Map([['always', 'retryAlwaysHint']]);
/** Labels of known failure codes; a code without one shows verbatim. */
const CODE_LABELS = new Map([
    ['EMPTY_RESPONSE', 'retryCodeEmptyResponse'],
    ['RATE_LIMIT', 'retryCodeRateLimit'],
    ['SERVER', 'retryCodeServer'],
    ['TIMEOUT', 'retryCodeTimeout'],
    ['TRANSPORT', 'retryCodeTransport'],
    ['STREAM_CLOSED', 'retryCodeStreamClosed'],
    ['MALFORMED_RESPONSE', 'retryCodeMalformedResponse'],
    ['QUOTA', 'retryCodeQuota'],
]);
/** Plain decimal text; prefixed forms such as `0x10` stay invalid text. */
const DECIMAL = /^[+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?$/iu;
/** The pair `resolveRetryPolicy` orders beyond any single field bound: initial delay at most the maximum. */
const DELAY_ORDER = [['backoff', 'initialDelayMs'], ['backoff', 'maxDelayMs']];
function isRecord(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
/** Read a nested field of a plain policy object. */
function fieldAt(value, path) {
    return path.reduce((node, key) => isRecord(node) ? node[key] : undefined, value);
}
/** The mode a union branch selects, read from its `mode` constant. */
function modeOf(branch) {
    const mode = branch.dict?.['mode']?.value;
    return typeof mode === 'string' ? mode : undefined;
}
/** The constant choices a list field's item schema publishes, in schema order. */
function choicesOf(node) {
    // A union item always carries `list`; any other item publishes no choices.
    return (node.inner?.list ?? []).flatMap((branch) => {
        const value = branch.type === 'const' ? branch.value : undefined;
        return typeof value === 'string' ? [value] : [];
    });
}
/** The editable fields of one object node, in schema order, without the `mode` discriminant. */
function fieldsOf(node) {
    /* v8 ignore next -- object schema nodes always carry `dict` */
    return Object.entries(node.dict ?? {}).filter(([key]) => key !== 'mode');
}
/**
 * Resolve the modes and inherited policy beneath the user layer.
 * @param schema - settings schema operations.
 * @param node - the `retryPolicy` schema union.
 * @param inherited - the policy the composition layer sets, if any.
 * @returns the modes and the inherited mode and field values.
 */
export function retryPolicyDefaults(schema, node, inherited) {
    // A schema that is not a union of mode branches has no `list` and offers no mode.
    const branches = new Map((node.list ?? []).flatMap((branch) => {
        const mode = modeOf(branch);
        return mode === undefined ? [] : [[mode, branch]];
    }));
    const modes = [...branches.keys()];
    const stored = schema.getPath(inherited, ['mode']);
    return {
        modes,
        mode: typeof stored === 'string' && branches.has(stored) ? stored : modes[0] ?? '',
        branch: mode => typeof mode === 'string' ? branches.get(mode) : undefined,
        value: (branch, path) => {
            const value = schema.getPath(inherited, path);
            if (value !== undefined)
                return value;
            const fallback = schema.nodeAtPath(branch, path)?.meta.default;
            return fallback;
        },
    };
}
/** The first bound of a number field's schema that `value` breaks, as copy. */
function numberFailure(node, value, t) {
    if (typeof value !== 'number' || !Number.isFinite(value))
        return t('retryValueNumber');
    const { min, max, step } = node.meta;
    if (step === 1 && !Number.isInteger(value))
        return t('retryValueInteger');
    // `Number.MIN_VALUE` is the schema idiom for a positive bound.
    if (min === Number.MIN_VALUE && value <= 0)
        return t('retryValuePositive');
    if (typeof min === 'number' && value < min)
        return t('retryValueMin').replace('{min}', () => String(min));
    if (typeof max === 'number' && value > max)
        return t('retryValueMax').replace('{max}', () => String(max));
    return undefined;
}
/**
 * The first refused field of a drafted policy, checked against the selected
 * branch's field bounds with empty fields inherited, then against the
 * cross-field rules `resolveRetryPolicy` enforces beyond the schema, then
 * against the whole schema.
 * @param schema - settings schema operations.
 * @param node - the `retryPolicy` schema union.
 * @param policy - the drafted user-layer policy; undefined inherits.
 * @param defaults - the inherited policy beneath the draft.
 * @param t - section copy.
 * @returns the failure, or undefined when the policy is valid.
 */
export function retryPolicyFailure(schema, node, policy, defaults, t) {
    if (policy === undefined)
        return undefined;
    const whole = () => {
        const message = schema.validate(node, policy);
        return message === undefined ? undefined : { message };
    };
    if (!isRecord(policy))
        return whole();
    const branch = defaults.branch(policy['mode']);
    if (branch === undefined)
        return whole();
    const read = (path) => schema.getPath(policy, path) ?? defaults.value(branch, path);
    const check = (owner, base) => {
        for (const [key, field] of fieldsOf(owner)) {
            const path = [...base, key];
            if (field.type === 'object') {
                const nested = check(field, path);
                if (nested !== undefined)
                    return nested;
            }
            else if (field.type === 'number') {
                const value = read(path);
                // A field with neither a value nor a default is left to the schema check below.
                const message = value === undefined ? undefined : numberFailure(field, value, t);
                if (message !== undefined)
                    return { message, path };
            }
            else if (field.type === 'array') {
                const stored = schema.getPath(policy, path);
                if (stored === undefined)
                    continue;
                if (schema.validate(field, stored) !== undefined)
                    return { message: t('retryCodesInvalid'), path };
                if (Array.isArray(stored) && new Set(stored).size !== stored.length)
                    return { message: t('retryCodesDuplicate'), path };
            }
        }
        return undefined;
    };
    const failure = check(branch, []);
    if (failure !== undefined)
        return failure;
    const [lowerPath, upperPath] = DELAY_ORDER;
    const lower = read(lowerPath);
    const upper = read(upperPath);
    if (typeof lower === 'number' && typeof upper === 'number' && lower > upper) {
        return { message: t('retryDelayOrder'), path: lowerPath };
    }
    return whole();
}
function textOf(value) {
    return typeof value === 'number' || typeof value === 'string' ? String(value) : '';
}
/** Set or remove one nested field of a plain policy object, dropping emptied parents. */
function withField(policy, path, value) {
    const [key, ...rest] = path;
    /* v8 ignore next -- paths come from schema fields and are never empty */
    if (key === undefined)
        return policy;
    const result = { ...policy };
    const prior = result[key];
    const next = rest.length === 0 ? value : withField(isRecord(prior) ? prior : {}, rest, value);
    if (next === undefined || (isRecord(next) && Object.keys(next).length === 0))
        Reflect.deleteProperty(result, key);
    else
        result[key] = next;
    return result;
}
/**
 * Render the retry mode and its policy fields.
 * @param props - drafted policy, inherited values, and the draft writer.
 * @returns the retry-policy field group.
 */
export function RetryPolicyEditor(props) {
    const { t, disabled, defaults, failure } = props;
    const current = isRecord(props.policy) ? props.policy : {};
    const branch = defaults.branch(current['mode']);
    const mode = branch === undefined ? undefined : String(current['mode']);
    // Raw text keeps partial input such as `0.` visible while it is typed.
    const [texts, setTexts] = useState({});
    const [added, setAdded] = useState({});
    // A nested fold starts open only when the draft already customizes it.
    const [openFolds] = useState(() => new Set(Object.keys(current).filter(key => isRecord(current[key]))));
    const label = (key) => {
        const labelKey = LABELS.get(key);
        return labelKey === undefined ? key : t(labelKey);
    };
    const invalidAt = (path) => failure?.path?.join('.') === path.join('.');
    const numberField = (root, path, node, className) => {
        const id = path.join('.');
        const invalid = invalidAt(path);
        const storedValue = fieldAt(current, path);
        return (_jsxs("label", { className: className, children: [_jsx("span", { className: styles['modelFieldLabel'], children: label(path[path.length - 1]) }), _jsx("input", { className: styles['input'], type: "text", inputMode: node.meta.step === 1 ? 'numeric' : 'decimal', value: texts[id] ?? textOf(storedValue), placeholder: textOf(defaults.value(root, path)), "aria-invalid": invalid, "aria-describedby": invalid ? props.failureId : undefined, disabled: disabled, onChange: (event) => {
                        const trimmed = event.target.value.trim();
                        setTexts(previous => ({ ...previous, [id]: trimmed.length === 0 ? '' : event.target.value }));
                        const value = DECIMAL.test(trimmed) ? Number(trimmed) : trimmed;
                        props.onChange(withField(current, path, value === '' ? undefined : value));
                    } })] }, id));
    };
    const codesField = (root, path, node) => {
        const id = path.join('.');
        const strings = (value) => Array.isArray(value) ? value.filter(code => typeof code === 'string') : [];
        const inherited = strings(defaults.value(root, path));
        const storedCodes = fieldAt(current, path);
        const selected = Array.isArray(storedCodes) ? strings(storedCodes) : inherited;
        // Codes added during this edit stay listed after they are unchecked.
        const choices = [...new Set([...choicesOf(node), ...inherited, ...selected, ...added[id] ?? []])];
        const invalid = invalidAt(path);
        const select = (next) => {
            // Selecting exactly the inherited codes keeps the field inherited.
            const inherits = next.length === inherited.length && next.every(value => inherited.includes(value));
            props.onChange(withField(current, path, inherits ? undefined : [...next]));
        };
        const draftCode = (texts[id] ?? '').trim();
        const add = () => {
            if (draftCode.length === 0)
                return;
            setAdded(previous => ({ ...previous, [id]: [...new Set([...previous[id] ?? [], draftCode])] }));
            setTexts(previous => ({ ...previous, [id]: '' }));
            if (!selected.includes(draftCode))
                select([...selected, draftCode]);
        };
        return (_jsxs("fieldset", { className: styles['modelInputTypes'], "aria-invalid": invalid, "aria-describedby": invalid ? props.failureId : undefined, children: [_jsx("legend", { className: styles['modelFieldLabel'], children: label(path[path.length - 1]) }), _jsx("div", { className: styles['retryCodeChoices'], children: choices.map((code) => {
                        const labelKey = CODE_LABELS.get(code);
                        return (_jsx(Checkbox, { 
                            // A code the list does not name shows verbatim.
                            label: labelKey === undefined ? code : t(labelKey), checked: selected.includes(code), disabled: disabled || (selected.length === 1 && selected.includes(code)), onChange: (checked) => {
                                select(choices.filter(value => value === code ? checked : selected.includes(value)));
                            } }, code));
                    }) }), _jsxs("div", { className: styles['retryCodeAdd'], children: [_jsx("input", { className: styles['input'], type: "text", value: texts[id] ?? '', placeholder: t('retryCodeCustom'), "aria-label": t('retryCodeCustom'), disabled: disabled, onChange: (event) => {
                                const value = event.target.value;
                                setTexts(previous => ({ ...previous, [id]: value }));
                            }, onKeyDown: (event) => {
                                // Enter that confirms an IME composition belongs to the composition.
                                if (event.key !== 'Enter' || event.nativeEvent.isComposing)
                                    return;
                                // Enter adds the code; it never submits the settings form.
                                event.preventDefault();
                                add();
                            } }), _jsx("button", { type: "button", className: styles['secondaryButton'], disabled: disabled || draftCode.length === 0, onClick: add, children: t('retryCodeAdd') })] })] }, path.join('.')));
    };
    // Top-level number fields share the width of one nested column so they line up with it.
    const fields = (root, owner, base) => fieldsOf(owner).map(([key, node]) => {
        const path = [...base, key];
        if (node.type === 'number') {
            return numberField(root, path, node, base.length === 0 ? `${styles['modelField']} ${styles['retryCount']}` : styles['modelField']);
        }
        if (node.type === 'array')
            return codesField(root, path, node);
        if (node.type === 'object') {
            return (_jsxs("details", { className: styles['retryFold'], open: openFolds.has(key), children: [_jsx("summary", { className: styles['customizedSummary'], children: label(key) }), _jsx("div", { className: styles['retryBackoff'], children: fields(root, node, path) })] }, key));
        }
        return null;
    });
    const labelId = `${props.failureId}-label`;
    const hint = mode === undefined ? undefined : MODE_HINTS.get(mode);
    return (_jsxs("div", { className: styles['field'], role: "group", "aria-labelledby": labelId, children: [_jsx("span", { id: labelId, className: styles['fieldLabel'], children: t('retryPolicy') }), _jsxs("select", { className: `${styles['input']} ${styles['selectInput']}`, value: mode ?? '', "aria-labelledby": labelId, "aria-describedby": failure !== undefined && failure.path === undefined ? props.failureId : undefined, disabled: disabled, onChange: (event) => {
                    const next = defaults.branch(event.target.value);
                    setTexts({});
                    if (next === undefined) {
                        props.onChange(undefined);
                        return;
                    }
                    // A switched policy keeps only the fields its new mode declares.
                    const declared = new Set(fieldsOf(next).map(([key]) => key));
                    const kept = Object.fromEntries(Object.entries(current).filter(([key]) => declared.has(key)));
                    props.onChange({ ...kept, mode: event.target.value });
                }, children: [_jsx("option", { value: "", children: t('retryModeDefault').replace('{mode}', () => label(defaults.mode)) }), defaults.modes.map(value => _jsx("option", { value: value, children: label(value) }, value))] }), hint === undefined ? null : _jsx("span", { className: styles['advancedHint'], children: t(hint) }), branch === undefined ? null : _jsx("div", { className: styles['retryBody'], children: fields(branch, branch, []) }), failure === undefined || !props.showFailure
                ? null
                : _jsx("p", { id: props.failureId, className: styles['error'], children: failure.message })] }));
}
//# sourceMappingURL=RetryPolicyEditor.js.map