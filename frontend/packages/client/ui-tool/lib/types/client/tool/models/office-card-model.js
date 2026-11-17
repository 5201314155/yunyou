import { abbreviateHomePath, isAbsoluteWorkspacePath, relativizeToCwd } from '@deepseek-ai/dsh-util-workspace-path';
import { recordedAbsolutePath } from "./recorded-path.js";
const TITLE_KEYS = {
    office_render: 'tool.title.officeRender',
    office_convert: 'tool.title.officeConvert',
    office_recalculate: 'tool.title.officeRecalculate',
};
function completeText(block, key) {
    const value = block.args.complete(key) ? block.args.text(key) : undefined;
    // Decoding text can discover an invalid escape and make complete() false.
    return value !== undefined && value !== '' && block.args.complete(key) ? value : undefined;
}
/**
 * Derive an Office row from wire values.
 * @param toolName - wire tool name used for the title and format summary.
 * @param block - preparing, running, or settled Office block.
 * @param cwd - Session workspace root used to shorten paths.
 * @param home - Host account home shown as `~`.
 * @returns the row title, summary, and openable path.
 */
export function officeRowModel(toolName, block, cwd, home) {
    const titleKey = TITLE_KEYS[toolName] ?? 'tool.title.generic';
    const input = completeText(block, 'input');
    const format = toolName === 'office_render' ? undefined : completeText(block, 'format');
    const short = (path) => abbreviateHomePath(relativizeToCwd(path, cwd), home);
    const summary = input === undefined ? '' : format === undefined ? short(input) : `${short(input)} → ${format}`;
    const recorded = 'kind' in block ? recordedAbsolutePath(block.meta, 'path') : undefined;
    const filePath = recorded ?? (input !== undefined && isAbsoluteWorkspacePath(input) ? input : undefined);
    return { titleKey, summary, filePath };
}
//# sourceMappingURL=office-card-model.js.map