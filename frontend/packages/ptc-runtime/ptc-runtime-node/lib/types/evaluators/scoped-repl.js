/** Scope chain for scoped REPL programs: acorn scanning, copied bindings between programs, and Script compilation. */
import { getLineInfo, parse } from 'acorn';
import { Script } from 'node:vm';
/** Prefix of every generated name; programs that contain it anywhere are rejected. */
export const RESERVED_PREFIX = '__$binding_';
/** Generated parameter receiving the values copied from earlier programs. */
export const BINDING_VALUES = `${RESERVED_PREFIX}vals`;
/** Generated parameter receiving one read function per binding, in binding order. */
export const BINDING_GETTERS = `${RESERVED_PREFIX}getters`;
/**
 * Parse a program once and list its own top-level bindings, leading strict directive, and final expression.
 * Every program uses script rules with top-level await: strictness comes from the generated Script or the program's directive,
 * and V8 reports strict-mode violations when compiling. Script rules reject top-level `using` and static `import`.
 * Top-level declarations are read, and so are var declarations inside blocks and other statements, which hoist to the program's scope;
 * declarations inside nested functions and classes are not read.
 * @param code - Type-stripped JavaScript submitted for one evaluation.
 * @param options - Filename shown by syntax errors.
 * @returns Declarations in source order, whether a "use strict" directive leads, and the final expression offsets.
 * @throws {SyntaxError} Laid out like V8's for parse errors, top-level return, a hashbang, and any occurrence of {@link RESERVED_PREFIX}.
 */
export function scanProgram(code, options) {
    // A hashbang is valid only at the start of a Script, and the generated prefix precedes the program.
    if (code.startsWith('#!'))
        throw programSyntaxError(code, options.filename, 0, 'Hashbang lines are not supported');
    const reserved = code.indexOf(RESERVED_PREFIX);
    if (reserved >= 0)
        throw programSyntaxError(code, options.filename, reserved, `Names starting with '${RESERVED_PREFIX}' are reserved`);
    let program;
    try {
        program = parse(code, { ecmaVersion: 'latest', sourceType: 'script', allowAwaitOutsideFunction: true });
    }
    catch (error) {
        throw parseFailure(code, options.filename, error);
    }
    const declarations = new Map();
    const declare = (identifier, kind) => {
        // A function declaration initializes the name before any statement runs, so a var of the same name never replaces it.
        if (kind === 'var' && declarations.get(identifier.name) === 'function')
            return;
        declarations.set(identifier.name, kind);
    };
    const collectVar = (statement) => {
        switch (statement.type) {
            case 'VariableDeclaration':
                if (statement.kind === 'var')
                    for (const declarator of statement.declarations)
                        bindingNames(declarator.id, (identifier) => { declare(identifier, 'var'); });
                break;
            case 'BlockStatement':
                statement.body.forEach(collectVar);
                break;
            case 'IfStatement':
                collectVar(statement.consequent);
                if (statement.alternate)
                    collectVar(statement.alternate);
                break;
            case 'ForStatement':
                if (statement.init?.type === 'VariableDeclaration')
                    collectVar(statement.init);
                collectVar(statement.body);
                break;
            case 'ForInStatement':
            case 'ForOfStatement':
                if (statement.left.type === 'VariableDeclaration')
                    collectVar(statement.left);
                collectVar(statement.body);
                break;
            case 'WhileStatement':
            case 'DoWhileStatement':
            case 'LabeledStatement':
            case 'WithStatement':
                collectVar(statement.body);
                break;
            case 'TryStatement':
                collectVar(statement.block);
                if (statement.handler)
                    collectVar(statement.handler.body);
                if (statement.finalizer)
                    collectVar(statement.finalizer);
                break;
            case 'SwitchStatement':
                for (const branch of statement.cases)
                    branch.consequent.forEach(collectVar);
                break;
            default: break; // Functions and classes start their own var scope; other statements declare nothing.
        }
    };
    for (const statement of program.body) {
        switch (statement.type) {
            case 'VariableDeclaration': {
                const kind = statement.kind === 'var' || statement.kind === 'let' ? statement.kind : 'const';
                for (const declarator of statement.declarations)
                    bindingNames(declarator.id, (identifier) => { declare(identifier, kind); });
                break;
            }
            case 'FunctionDeclaration':
                declare(statement.id, 'function');
                break;
            case 'ClassDeclaration':
                declare(statement.id, 'class');
                break;
            default: collectVar(statement);
        }
    }
    let useStrict = false;
    for (const statement of program.body) {
        if (statement.type !== 'ExpressionStatement' || statement.directive === undefined)
            break;
        if (statement.directive === 'use strict')
            useStrict = true;
    }
    const last = program.body.findLast(statement => statement.type !== 'EmptyStatement');
    if (last?.type !== 'ExpressionStatement')
        return { declarations, useStrict };
    const finalExpression = { statementStart: last.start, expressionStart: last.expression.start, expressionEnd: last.expression.end };
    return { declarations, useStrict, finalExpression };
}
/**
 * Build the Script source that copies earlier bindings, exposes read functions, and runs the program.
 * The generated prefix occupies line 0, so compiling with `lineOffset: -1` reports program line N as line N.
 * A copy exists exactly for each carried binding. A name the program declares again starts from its earlier value through a `var` copy,
 * other `var` bindings are copied as `var`, and all others as `let`.
 * With `strict` off, a program whose directives include "use strict" gets that directive at the start of its function body.
 * @param code - Program text that produced `scan`.
 * @param scan - Result of {@link scanProgram} for `code`.
 * @param carried - Earlier bindings to copy, in the order of the copied values: names the program does not declare,
 * and names it declares again only with `var`.
 * @param options - Whether the whole Script is strict, and whether the final expression becomes the completion value.
 * @returns Source evaluating to a factory, and every binding in the order of the read functions it pushes.
 */
export function buildProgramSource(code, scan, carried, options) {
    const copiedOnly = carried.filter(({ name }) => !scan.declarations.has(name));
    const bindings = [...copiedOnly, ...Array.from(scan.declarations, ([name, kind]) => ({ name, kind }))];
    // A copy only carries the earlier value in; declaring it const would only forbid later assignments.
    const copies = carried.map(({ name, kind }, index) => {
        const keyword = kind === 'var' || scan.declarations.has(name) ? 'var' : 'let';
        return ` ${keyword} ${name} = ${BINDING_VALUES}[${index}];`;
    }).join('');
    const getters = bindings.length === 0 ? '' : ` ${BINDING_GETTERS}.push(${bindings.map(({ name }) => `() => ${name}`).join(', ')});`;
    const final = options.finalExpression ? scan.finalExpression : undefined;
    const body = final === undefined ? code : `${code.slice(0, final.statementStart)}return ${code.slice(final.statementStart, final.expressionStart)}{ value: (${code.slice(final.expressionStart, final.expressionEnd)}) }${code.slice(final.expressionEnd)}`;
    // The copies precede the program, so a directive the program wrote no longer leads the function body unless repeated here.
    const directive = !options.strict && scan.useStrict ? '"use strict";' : '';
    const prefix = `${options.strict ? '"use strict";' : ''}(function (${BINDING_VALUES}, ${BINDING_GETTERS}) { return async function () {${directive}${copies}${getters}`;
    // The closing line stays separate because the program's last line may end with a line comment.
    return { source: `${prefix}\n${body}\n} })`, bindings };
}
/**
 * Compile programs that each run in their own scope and start with copies of the top-level bindings earlier programs left.
 * A program reads earlier bindings when it is compiled: values earlier code assigns afterwards stay with that code.
 * A var redeclaration starts from the earlier value. An earlier name whose let, const, or class redeclaration never finished initializing
 * keeps the value it had before; an uninitialized new name is dropped.
 */
export class ScopedReplScopes {
    options;
    bindings = new Map();
    replaced = new Map();
    compiled;
    /**
     * Start an empty scope chain.
     * @param options - Strictness and the redeclaration observer shared by every program.
     */
    constructor(options) {
        // Source child processes run under Node's erasable-only type stripping, which rejects parameter properties.
        this.options = options;
    }
    /**
     * Scan and compile one program against the bindings current at this moment.
     * Bindings of the previously compiled program apply once its body has started; a syntax error leaves the chain unchanged.
     * @param code - Type-stripped JavaScript of the program.
     * @param options - Filename, shared context, completion choice, and import handling.
     * @returns The program, which the caller runs once.
     * @throws {SyntaxError} From scanning or V8 compilation, with its location and caret and without compiler frames.
     */
    compile(code, options) {
        this.adoptCompiled();
        const strict = this.options.strict;
        const scan = scanProgram(code, { filename: options.filename });
        const carried = [];
        const values = [];
        const replaced = new Map();
        const redeclarations = [];
        for (const [name, binding] of this.bindings) {
            const current = this.currentValue(name, binding);
            if (current === undefined)
                continue;
            const kind = scan.declarations.get(name);
            // A var redeclaration starts from the earlier value, as a global var does in Node's REPL, so an unreached one keeps it.
            if (kind === undefined || kind === 'var') {
                carried.push({ name, kind: current.kind });
                values.push(current.value);
            }
            if (kind !== undefined) {
                replaced.set(name, current);
                redeclarations.push({ name, previous: current.kind, current: kind });
            }
        }
        const { source, bindings } = buildProgramSource(code, scan, carried, { strict, finalExpression: options.finalExpression });
        const scriptOptions = {
            filename: options.filename,
            lineOffset: -1,
            ...options.importModuleDynamically === undefined ? {} : { importModuleDynamically: options.importModuleDynamically },
        };
        let factory;
        try {
            factory = new Script(source, scriptOptions).runInContext(options.context);
        }
        catch (error) {
            throw withoutCompilerFrames(error);
        }
        if (redeclarations.length > 0)
            this.options.onRedeclaration?.(redeclarations);
        const getters = [];
        const program = factory(values, getters);
        this.compiled = { bindings, getters, replaced };
        return program;
    }
    adoptCompiled() {
        const program = this.compiled;
        this.compiled = undefined;
        // The prefix pushes every read function before the program body runs; an unstarted program is skipped.
        if (program === undefined || program.getters.length < program.bindings.length)
            return;
        const { getters } = program;
        this.bindings = new Map(program.bindings.map(({ name, kind }, index) => [name, { kind, read: getters[index] }]));
        this.replaced = program.replaced;
    }
    currentValue(name, binding) {
        try {
            return { kind: binding.kind, value: binding.read() };
        }
        catch (_error) {
            // The declaration never finished initializing; later programs see the value it replaced, if any.
            return this.replaced.get(name);
        }
    }
}
/** Drop the compiler frames V8 appends after a compile error's heading; the location and caret precede the heading. */
function withoutCompilerFrames(error) {
    /* v8 ignore next -- V8 reports every compile failure of the generated source as a SyntaxError. */
    if (!(error instanceof SyntaxError))
        return error;
    const heading = `\n${error.name}: ${error.message}`;
    error.stack = `${String(error.stack).split(heading, 1).join('')}${heading}`;
    return error;
}
/** Collect identifiers bound by a declaration pattern. */
function bindingNames(pattern, visit) {
    switch (pattern.type) {
        case 'Identifier':
            visit(pattern);
            break;
        case 'ObjectPattern':
            for (const property of pattern.properties)
                bindingNames(property.type === 'RestElement' ? property.argument : property.value, visit);
            break;
        case 'ArrayPattern':
            for (const element of pattern.elements)
                if (element !== null)
                    bindingNames(element, visit);
            break;
        case 'RestElement':
            bindingNames(pattern.argument, visit);
            break;
        case 'AssignmentPattern':
            bindingNames(pattern.left, visit);
            break;
        /* v8 ignore next -- Declaration patterns never bind member expressions; acorn rejects them first. */
        case 'MemberExpression': break;
    }
}
/** Convert an acorn parse error to a positioned program SyntaxError. */
function parseFailure(code, filename, error) {
    /* v8 ignore next -- acorn raises every parse failure, including exhausted parser stack, as a positioned SyntaxError. */
    if (!(error instanceof SyntaxError) || !('pos' in error) || typeof error.pos !== 'number')
        return error;
    const detail = error.message.replace(/ \(\d+:\d+\)$/, '');
    const message = detail === "'return' outside of function" ? 'Illegal return statement'
        : detail === 'Unexpected token' && error.pos >= code.length ? 'Unexpected end of input'
            : detail;
    return programSyntaxError(code, filename, error.pos, message);
}
/** Create a SyntaxError whose stack shows the location, source line, and caret like V8's. */
function programSyntaxError(code, filename, offset, message) {
    const { line, column } = getLineInfo(code, offset);
    const rest = code.slice(offset - column);
    const end = rest.search(/[\n\r\u2028\u2029]/);
    const error = new SyntaxError(message);
    error.stack = `${filename}:${line}\n${end < 0 ? rest : rest.slice(0, end)}\n${' '.repeat(column)}^\n\n${error.name}: ${message}`;
    return error;
}
//# sourceMappingURL=scoped-repl.js.map