import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 拉人对话框（L4，需用户显式确认提交；fork 降级语义如实说明）。 */
import { useState } from 'react';
import { Button, Modal } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './SpawnDialog.module.css';
/**
 * 渲染拉起成员表单：name/description/prompt/context 四字段；fork 附降级说明。
 * 提交走真实 spawn RPC（服务端重名 fail-loud 原样展示）。
 * @param props - 本地化、开合与远程源。
 * @returns 拉人对话框。
 */
export function SpawnDialog({ t, open, onClose, source, notify }) {
    const [name, setName] = useState('');
    const [description, setDescription] = useState('');
    const [prompt, setPrompt] = useState('');
    const [context, setContext] = useState('fresh');
    const [pending, setPending] = useState(false);
    const close = () => {
        setName('');
        setDescription('');
        setPrompt('');
        setPending(false);
        onClose();
    };
    if (!open)
        return null;
    return (_jsx(Modal, { open: true, onClose: close, title: t('spawn.title'), closeLabel: t('close'), ...(css.sheet ? { className: css.sheet } : {}), children: _jsxs("form", { className: css.form, onSubmit: (event) => {
                event.preventDefault();
                if (pending || name.trim() === '' || prompt.trim() === '')
                    return;
                setPending(true);
                void source.spawn({ name: name.trim(), description: description.trim(), prompt, context }).then(result => {
                    if (!result.ok) {
                        setPending(false);
                        notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error');
                    }
                    else
                        close();
                });
            }, children: [_jsxs("label", { className: css.field, children: [_jsx("span", { className: css.fieldLabel, children: t('spawn.name') }), _jsx("input", { className: css.input, value: name, onChange: event => { setName(event.target.value); }, required: true })] }), _jsxs("label", { className: css.field, children: [_jsx("span", { className: css.fieldLabel, children: t('spawn.description') }), _jsx("input", { className: css.input, value: description, onChange: event => { setDescription(event.target.value); } })] }), _jsxs("label", { className: css.field, children: [_jsx("span", { className: css.fieldLabel, children: t('spawn.prompt') }), _jsx("textarea", { className: css.body, value: prompt, onChange: event => { setPrompt(event.target.value); }, rows: 3, required: true })] }), _jsxs("fieldset", { className: css.field, children: [_jsx("legend", { className: css.fieldLabel, children: t('spawn.context') }), _jsxs("label", { className: css.radio, children: [_jsx("input", { type: "radio", name: "spawn-context", checked: context === 'fresh', onChange: () => { setContext('fresh'); } }), _jsx("span", { children: t('spawn.context.fresh') })] }), _jsxs("label", { className: css.radio, children: [_jsx("input", { type: "radio", name: "spawn-context", checked: context === 'fork', onChange: () => { setContext('fork'); } }), _jsx("span", { children: t('spawn.context.fork') })] }), context === 'fork' ? _jsx("p", { className: css.forkNote, children: t('spawn.forkNote') }) : null] }), _jsxs("div", { className: css.actions, children: [_jsx(Button, { variant: "ghost", type: "button", onClick: close, children: t('spawn.cancel') }), _jsx(Button, { variant: "primary", type: "submit", disabled: pending, children: t('spawn.submit') })] })] }) }));
}
//# sourceMappingURL=SpawnDialog.js.map