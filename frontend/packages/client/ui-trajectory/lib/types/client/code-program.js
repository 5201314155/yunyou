/**
 * Identify tools supported by the trajectory code inspector.
 * @param name - Recorded tool name, absent when the call is outside the loaded window.
 * @returns Whether this tool executes a program shown by the code inspector.
 */
export function isProgramTool(name) {
    return name === 'run_code' || name === 'cua_run_code';
}
function record(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value)
        ? value
        : undefined;
}
function parseRecord(value) {
    if (value === undefined)
        return undefined;
    try {
        return record(JSON.parse(value));
    }
    catch {
        return undefined;
    }
}
function recordedLanguage(schemaRaw) {
    const schema = parseRecord(schemaRaw);
    const declaredLanguage = typeof schema?.description === 'string'
        ? /^Execute a program in `([^`]+)`/.exec(schema.description)?.[1]?.toLowerCase()
        : undefined;
    if (declaredLanguage !== undefined
        && declaredLanguage !== 'typescript' && declaredLanguage !== 'python')
        return undefined;
    const properties = record(record(schema?.parameters)?.properties);
    const description = record(properties?.code)?.description;
    const typescript = declaredLanguage === 'typescript'
        || (typeof description === 'string' && /\bTypeScript\b/i.test(description));
    const python = declaredLanguage === 'python'
        || (typeof description === 'string' && /\bPython\b/i.test(description));
    if (typescript === python)
        return undefined;
    return typescript ? 'typescript' : 'python';
}
/**
 * Resolve a program without guessing its language from source or current runtime settings.
 * Unknown explicit languages or conflicting recorded hints leave the source unhighlighted.
 * @param cell - Recorded tool arguments and the schema visible at call time.
 * @returns The program, or undefined for another tool or unsupported arguments.
 */
export function codeProgram(cell) {
    if (cell.kind !== 'tool' && cell.kind !== 'subtool')
        return undefined;
    if (!isProgramTool(cell.toolName) || cell.inputDetail === undefined)
        return undefined;
    const args = parseRecord(cell.inputDetail);
    if (typeof args?.code !== 'string')
        return undefined;
    if (args.description !== undefined && typeof args.description !== 'string')
        return undefined;
    return {
        rawInput: cell.inputDetail,
        source: args.code,
        description: typeof args.description === 'string' && args.description.trim() !== ''
            ? args.description
            : args.code.split(/\r?\n/).find(line => line.trim() !== '')?.trim() ?? '',
        arguments: args,
        language: recordedLanguage(cell.schemaDetail),
    };
}
//# sourceMappingURL=code-program.js.map