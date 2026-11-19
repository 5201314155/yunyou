import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 详情页三步文档序（§6.1：locale meta.detail→包 README→仓库 README）。 */
import { useEffect, useState } from 'react';
import { Button } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './PluginDetail.module.css';
/**
 * 渲染详情：拉取三步文档序步骤并按序呈现；失败保留返回路径（fail-visible）。
 * @param props - 条目、远程源与本地化。
 * @returns 详情面板。
 */
export function PluginDetail({ t, entry, source, onBack }) {
    const [detail, setDetail] = useState(undefined);
    const [error, setError] = useState(undefined);
    useEffect(() => {
        let live = true;
        void source.detail(entry.id).then(result => {
            if (!live)
                return;
            if (result.ok)
                setDetail(result.value);
            else
                setError(`${result.code}: ${result.message}`);
        });
        return () => { live = false; };
    }, [entry.id, source]);
    return (_jsxs("section", { className: css.detail, "aria-label": entry.name, children: [_jsxs("header", { className: css.head, children: [_jsx(Button, { variant: "ghost", onClick: onBack, children: t('detail.back') }), _jsx("h2", { className: css.title, children: entry.name })] }), _jsx("h3", { className: css.stepsTitle, children: t('detail.stepsTitle') }), error !== undefined ? _jsx("p", { className: css.error, children: t('opFailed', { message: error }) }) : null, detail !== undefined ? (_jsx("ol", { className: css.steps, children: detail.steps.map(step => (_jsxs("li", { className: css.step, children: [_jsx("h4", { className: css.stepTitle, children: step.title }), _jsx("pre", { className: css.stepBody, children: step.body })] }, step.title))) })) : null, _jsx("p", { className: css.installNote, children: t('install.unavailable') })] }));
}
//# sourceMappingURL=PluginDetail.js.map