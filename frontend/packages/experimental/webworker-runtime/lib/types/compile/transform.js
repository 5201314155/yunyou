/**
 * The worker's module transform: one acorn parse turns an ES module into a
 * CommonJS body **and** routes every suspension point through the ambient-store
 * protocol.
 *
 * Both jobs live in one pass because they are two edits over one syntax tree;
 * running a lexer first and a parser second meant two scanners, two sets of
 * blind spots, and a second pass reading the first pass's output. Editing is
 * interval-based — the original text is sliced and spliced, never reprinted —
 * so **line numbers survive**: a stack frame in a transformed module points at
 * the same line as the built artifact it came from.
 *
 * Named and default imports stay live: every reference reads the required
 * module object when it runs (`__dsh$m1["x"]`), never a copy taken at the
 * import statement, so a module in an import cycle sees a binding once the
 * exporting module has initialized it, as ES modules do. Scope analysis tells
 * those references apart from same-named locals that shadow them; it runs only
 * for a module that declares or assigns an import's name somewhere, because
 * elsewhere every read of the name is the import.
 *
 * The image packer is this transform's only caller: it lowers every JavaScript
 * entry it packs and records `LOWERING_VERSION` in the image manifest, so the
 * worker wraps those bodies without carrying a compiler of its own.
 * @module @deepseek-ai/dsh-experimental-webworker-runtime/src/compile/transform
 */
import { parse } from 'acorn';
import { analyze } from 'eslint-scope';
import { KEYS } from 'eslint-visitor-keys';
const HELPER_SOURCE = {
    def: 'const __dsh$def=(t,k,get)=>Object.defineProperty(t,k,{enumerable:true,configurable:true,get});',
    default: 'const __dsh$default=(m)=>(m&&m.__esModule&&Object.prototype.hasOwnProperty.call(m,"default")?m.default:m);',
    ns: 'const __dsh$ns=(m)=>(m&&m.__esModule?m:Object.assign({},m,{default:m}));',
    exportAll: 'const __dsh$exportAll=(t,m)=>{for(const k of Object.keys(m))if(k!=="default"&&!(k in t))__dsh$def(t,k,()=>m[k]);};',
    dynImport: 'const __dsh$dynImport=(s)=>Promise.resolve().then(()=>__dsh$ns(require(s)));',
};
const HELPER_DEPENDENCIES = {
    exportAll: ['def'],
    dynImport: ['ns'],
};
/** Runtime identifier the suspension protocol reaches. */
const ALS = '__als';
/** @returns Number of line breaks in a slice. */
function countNewlines(text) {
    let count = 0;
    for (let index = text.indexOf('\n'); index >= 0; index = text.indexOf('\n', index + 1))
        count += 1;
    return count;
}
/** Prefix of the identifiers this transform emits; a module declaring one would capture them. */
const RESERVED_PREFIX = '__dsh$';
class Transformer {
    path;
    edits = [];
    source;
    helpers = new Set();
    bindings = [];
    modules = 0;
    temporaries = 0;
    moduleSyntax = false;
    moduleRequests = new Set();
    metaResolveRequests = new Set();
    createRequireBindings = new Set();
    /** Module temporary per import declaration with specifiers, assigned before traversal. */
    importModules = new Map();
    /** Named and default imports by local name. */
    importedBindings = new Map();
    /** Read-position identifiers spelling an import's name, rewritten after traversal. */
    importCandidates = [];
    /**
     * Set when an import name (or the reserved prefix) is declared or assigned.
     * Only then can a candidate be a shadowing local, so only then does scope
     * analysis decide which candidates read the import.
     */
    scopeAnalysisNeeded = false;
    constructor(source, path) {
        this.path = path;
        // A `#!` line is only legal at offset zero, and the prologue takes that spot;
        // commenting it out in place keeps every offset and the line count intact.
        this.source = source.startsWith('#!') ? `//${source.slice(2)}` : source;
    }
    fail(detail, index) {
        const line = this.source.slice(0, index).split('\n').length;
        throw new Error(`webworker transform: ${detail} (${this.path}:${line})`);
    }
    helper(name) {
        for (const dependency of HELPER_DEPENDENCIES[name] ?? [])
            this.helper(dependency);
        this.helpers.add(name);
        return `__dsh$${name}`;
    }
    moduleTemp() {
        this.modules += 1;
        return `__dsh$m${this.modules}`;
    }
    alsTemp() {
        this.temporaries += 1;
        return `__als$${this.temporaries}`;
    }
    /**
     * Replace a range, keeping the module's line count.
     *
     * The padding is the newlines the original range held **minus** the ones the
     * replacement re-emits: a rewrite that splices the original body back in
     * (a desugared loop) already carries that body's newlines, and padding by the
     * whole range again would push every later line down.
     */
    edit(start, end, build) {
        const original = countNewlines(this.source.slice(start, end));
        this.edits.push({
            start,
            end,
            render: (inner) => {
                const text = build(inner);
                return text + '\n'.repeat(Math.max(0, original - countNewlines(text)));
            },
        });
    }
    replace(start, end, text) {
        this.edit(start, end, () => text);
    }
    insert(at, text) {
        this.edits.push({ start: at, end: at, render: () => text });
    }
    structural(start, end, render) {
        this.edit(start, end, render);
    }
    literal(node) {
        const value = node.value;
        if (typeof value !== 'string')
            this.fail('a module specifier must be a string literal', node.start);
        this.moduleRequests.add(value);
        return JSON.stringify(value);
    }
    /** @returns Static module requests the body makes, in first-appearance order. */
    requests() {
        return [...this.moduleRequests];
    }
    /** @returns Literal `import.meta.resolve()` requests, in first-appearance order. */
    metaRequests() {
        return [...this.metaResolveRequests];
    }
    // --- module syntax --------------------------------------------------------
    importDeclaration(node) {
        this.moduleSyntax = true;
        if (Array.isArray(node.attributes) && node.attributes.length > 0) {
            this.fail('import attributes are not supported', node.start);
        }
        const source = node.source;
        const request = `require(${this.literal(source)})`;
        const held = this.importModules.get(node);
        if (held === undefined) {
            this.replace(node.start, node.end, `${request};`);
            return;
        }
        // Named and default imports declare nothing here: each reference reads the
        // held module object instead (`importReference`).
        const lines = [`const ${held}=${request};`];
        for (const specifier of node.specifiers) {
            if (specifier.type === 'ImportNamespaceSpecifier') {
                lines.push(`const ${nameOf(specifier.local)}=${this.helper('ns')}(${held});`);
            }
        }
        this.replace(node.start, node.end, lines.join(''));
    }
    /**
     * The expression that reads an import when the reference runs. The default
     * form is parenthesized so it stays one operand in any position, including
     * `new` callees and member objects.
     */
    importAccess(binding) {
        return binding.kind === 'default'
            ? `(${this.helper('default')}(${binding.held}))`
            : `${binding.held}[${JSON.stringify(binding.imported)}]`;
    }
    /** @returns How a local export reads its binding: imports through their module object. */
    bindingAccess(local) {
        const imported = this.importedBindings.get(local);
        return imported === undefined ? local : this.importAccess(imported);
    }
    /**
     * Record an identifier that spells an import's name in a read position, and
     * flag one in a declaration or assignment position (or one using the
     * reserved prefix there), which makes scope analysis decide the candidates.
     * @param node - The identifier.
     * @param parent - Its parent node.
     * @param key - The parent property holding it.
     * @param statementStart - Start of the enclosing expression statement when that statement sits in a statement list.
     */
    identifier(node, parent, key, statementStart) {
        if (parent === undefined || this.importedBindings.size === 0)
            return;
        const name = node.name;
        const binding = this.importedBindings.get(name);
        if (binding === undefined && !name.startsWith(RESERVED_PREFIX))
            return;
        if (isBindingPosition(parent, key)) {
            this.scopeAnalysisNeeded = true;
            return;
        }
        if (binding !== undefined && isReadPosition(parent, key)) {
            this.importCandidates.push({ node, parent, key, statementStart, binding });
        }
    }
    /** Flag a declared or assigned name that could shadow an import or capture an emitted temporary. */
    bindingOccurrence(name) {
        if (this.importedBindings.size === 0)
            return;
        if (this.importedBindings.has(name) || name.startsWith(RESERVED_PREFIX))
            this.scopeAnalysisNeeded = true;
    }
    /** Rewrite one identifier that reads an import. */
    importReference({ node, parent, key, statementStart, binding }) {
        const access = this.importAccess(binding);
        // A shorthand property holds the name as both key and value; only the value
        // is a candidate.
        if (parent.type === 'Property' && parent.shorthand === true) {
            this.replace(node.start, node.end, `${nameOf(node)}:${access}`);
            return;
        }
        const called = (parent.type === 'CallExpression' && key === 'callee')
            || (parent.type === 'TaggedTemplateExpression' && key === 'tag');
        // `(0, m.f)()` calls with `this` undefined, as calling an imported binding does.
        const text = called && binding.kind === 'named' ? `(0,${access})` : access;
        // A leading parenthesis would extend a preceding statement that ends without a semicolon.
        this.replace(node.start, node.end, text.startsWith('(') && node.start === statementStart ? `;${text}` : text);
    }
    exportNamed(node) {
        this.moduleSyntax = true;
        const declaration = node.declaration;
        const source = node.source;
        const specifiers = node.specifiers;
        if (declaration !== null) {
            // `export const x = 1` keeps its declaration; only the keyword goes.
            this.replace(node.start, declaration.start, '');
            for (const { exported, local } of declaredBindings(declaration, detail => this.fail(detail, declaration.start))) {
                this.bindings.push({ exported, local });
            }
            return;
        }
        if (source !== null) {
            const held = this.moduleTemp();
            const define = this.helper('def');
            const lines = [`const ${held}=require(${this.literal(source)});`];
            for (const specifier of specifiers) {
                const local = nameOf(specifier.local);
                const exported = nameOf(specifier.exported);
                lines.push(`${define}(exports,${JSON.stringify(exported)},()=>${held}[${JSON.stringify(local)}]);`);
            }
            this.replace(node.start, node.end, lines.join(''));
            return;
        }
        // A bare `export {}` is a module marker with nothing to publish.
        for (const specifier of specifiers) {
            this.bindings.push({ exported: nameOf(specifier.exported), local: nameOf(specifier.local) });
        }
        this.replace(node.start, node.end, '');
    }
    exportDefault(node) {
        this.moduleSyntax = true;
        const declaration = node.declaration;
        this.replace(node.start, declaration.start, 'exports.default = ');
    }
    exportAll(node) {
        this.moduleSyntax = true;
        const request = `require(${this.literal(node.source)})`;
        const exported = node.exported;
        if (exported === null) {
            this.replace(node.start, node.end, `${this.helper('exportAll')}(exports,${request});`);
            return;
        }
        const held = this.moduleTemp();
        const define = this.helper('def');
        this.replace(node.start, node.end, `const ${held}=${this.helper('ns')}(${request});${define}(exports,${JSON.stringify(nameOf(exported))},()=>${held});`);
    }
    // --- suspension points ----------------------------------------------------
    awaitExpression(node) {
        const keywordEnd = node.start + 'await'.length;
        if (this.source.slice(node.start, keywordEnd) !== 'await')
            this.fail('unexpected await layout', node.start);
        this.replace(node.start, keywordEnd, `${ALS}.resume(await ${ALS}.pause(`);
        this.insert(node.end, '))');
    }
    /**
     * `for await (L of R) B` becomes an explicit loop over the same protocol.
     * `iterator.return` runs only on abrupt completion, as the language says, and
     * is awaited so teardown still orders before the loop exits.
     */
    forAwait(node) {
        const left = node.left;
        const right = node.right;
        const body = node.body;
        const iterator = this.alsTemp();
        const step = this.alsTemp();
        const exhausted = this.alsTemp();
        const binding = (inner) => {
            if (left.type !== 'VariableDeclaration')
                return `(${inner(left.start, left.end)})=${step}.value;`;
            const declarations = left.declarations;
            const pattern = declarations[0]?.id;
            if (declarations.length !== 1 || pattern === undefined) {
                this.fail('for-await must declare exactly one binding', left.start);
            }
            return `${String(left.kind)} ${inner(pattern.start, pattern.end)}=${step}.value;`;
        };
        this.structural(node.start, node.end, inner => [
            `{const ${iterator}=${ALS}.iterator(${inner(right.start, right.end)});`,
            `let ${step};let ${exhausted}=false;`,
            `try{for(;;){${step}=${ALS}.resume(await ${ALS}.pause(${iterator}.next()));`,
            `if(${step}.done){${exhausted}=true;break}`,
            `{${binding(inner)}${body.type === 'BlockStatement' ? inner(body.start, body.end) : `{${inner(body.start, body.end)}}`}}}}`,
            `finally{if(!${exhausted})${ALS}.resume(await ${ALS}.pause(${ALS}.close(${iterator})))}}`,
        ].join(''));
    }
    /**
     * `yield` resumes with whatever the consumer sent, so the snapshot is taken
     * before suspending and restored when the call completes. `yield*` delegates,
     * which has no expression form here: it is desugared as a statement, and a
     * consumer's `throw()` is not forwarded into the inner iterator (`next` and
     * `return` are).
     */
    yieldExpression(node, statement) {
        if (node.delegate !== true) {
            this.insert(node.start, `${ALS}.afterYield(${ALS}.snapshot(),`);
            this.insert(node.end, ')');
            return;
        }
        const argument = node.argument;
        if (argument === null)
            this.fail('yield* without an operand', node.start);
        if (statement === undefined)
            this.fail('yield* is only supported as a statement', node.start);
        if (statement.expression !== node) {
            // Anything around the delegation (`x = yield* g()`, `f(yield* g())`)
            // would be silently dropped by the statement-wide rewrite below; the
            // all-or-nothing lowering contract demands a loud refusal instead.
            this.fail('yield* is only supported as the whole statement expression', node.start);
        }
        const iterator = this.alsTemp();
        const step = this.alsTemp();
        const sent = this.alsTemp();
        const exhausted = this.alsTemp();
        this.structural(statement.start, statement.end, inner => [
            `{const ${iterator}=${ALS}.iterator(${inner(argument.start, argument.end)});`,
            `let ${sent};let ${exhausted}=false;`,
            `try{for(;;){const ${step}=${ALS}.resume(await ${ALS}.pause(${iterator}.next(${sent})));`,
            `if(${step}.done){${exhausted}=true;break}`,
            `${sent}=${ALS}.afterYield(${ALS}.snapshot(),yield ${step}.value)}}`,
            `finally{if(!${exhausted})${ALS}.resume(await ${ALS}.pause(${ALS}.close(${iterator})))}}`,
        ].join(''));
    }
    // --- traversal ------------------------------------------------------------
    visit(node, context, parent, key) {
        if (node === null || typeof node !== 'object')
            return;
        if (Array.isArray(node)) {
            for (const child of node)
                this.visit(child, context, parent, key);
            return;
        }
        const record = node;
        if (typeof record.type !== 'string')
            return;
        let next = context;
        switch (record.type) {
            case 'Identifier':
                this.identifier(record, parent, key, context.statementStart);
                break;
            case 'ObjectPattern':
                // A property value here is a declaration or assignment target, which its
                // own parent (a Property) cannot tell apart from an object literal value.
                for (const property of record.properties) {
                    const value = property.value;
                    if (value?.type === 'Identifier')
                        this.bindingOccurrence(value.name);
                }
                break;
            case 'ImportDeclaration':
                this.importDeclaration(record);
                break;
            case 'ExportNamedDeclaration':
                this.exportNamed(record);
                break;
            case 'ExportDefaultDeclaration':
                this.exportDefault(record);
                break;
            case 'ExportAllDeclaration':
                this.exportAll(record);
                break;
            case 'ImportExpression': {
                this.moduleSyntax = true;
                if (!this.source.startsWith('import', record.start))
                    this.fail('unexpected dynamic import layout', record.start);
                this.replace(record.start, record.start + 'import'.length, this.helper('dynImport'));
                // A computed dynamic import stays out of the request list; resolution
                // then happens (and fails loud) at runtime, never silently at pack time.
                const argument = record.source;
                if (argument !== undefined && typeof argument.value === 'string')
                    this.moduleRequests.add(argument.value);
                break;
            }
            case 'CallExpression': {
                // CommonJS bodies pass through untransformed, but literal calls through
                // the wrapper's `require` remain module requests. The ESM case accepts
                // only a direct module-scope createRequire call with the importer URL.
                const callee = record.callee;
                const callArguments = record.arguments;
                // Module code is strict, so `eval` cannot be rebound: a call through the
                // bare name is a direct eval, whose code could name an import that no
                // longer exists as a local. An optional call (`eval?.()`) is indirect.
                if (callee.type === 'Identifier' && callee.name === 'eval' && record.optional !== true
                    && this.importedBindings.size > 0) {
                    this.fail('direct eval cannot see imports once they are read through their module', record.start);
                }
                if (this.isRequireCall(callee, context.moduleScope) && callArguments.length === 1
                    && typeof callArguments[0]?.value === 'string') {
                    this.moduleRequests.add(callArguments[0].value);
                }
                // `import.meta.resolve('lit')` is the third static request face: the
                // loader answers it from the image, so the pack sweep must keep the
                // target. A computed argument stays out, same as dynamic import —
                // resolution then fails loud at runtime, never silently at pack time.
                if (callee.type === 'MemberExpression') {
                    const object = callee.object;
                    const property = callee.property;
                    if (object.type === 'MetaProperty' && object.meta.name === 'import'
                        && property.type === 'Identifier' && property.name === 'resolve'
                        && typeof callArguments[0]?.value === 'string') {
                        this.metaResolveRequests.add(callArguments[0].value);
                    }
                }
                break;
            }
            case 'MetaProperty': {
                // `new.target` is a MetaProperty too, and it must survive untouched:
                // the abstract-seam guards in the roster read it (`new.target === X`).
                const meta = record.meta;
                if (meta.name === 'import') {
                    this.moduleSyntax = true;
                    this.replace(record.start, record.end, '__dsh$meta');
                }
                break;
            }
            case 'AwaitExpression':
                if (context.functionDepth === 0) {
                    this.fail('top-level await cannot run as CommonJS in the worker', record.start);
                }
                this.awaitExpression(record);
                break;
            case 'ForOfStatement':
                if (record.await === true) {
                    if (context.functionDepth === 0)
                        this.fail('a top-level for-await loop cannot run as CommonJS', record.start);
                    this.forAwait(record);
                }
                next = { ...next, moduleScope: false };
                break;
            case 'LabeledStatement': {
                const body = record.body;
                if (body.type === 'ForOfStatement' && body.await === true) {
                    this.fail('a labeled for-await loop is not supported', record.start);
                }
                break;
            }
            case 'YieldExpression':
                if (context.asyncGenerator)
                    this.yieldExpression(record, context.statement);
                break;
            case 'FunctionDeclaration':
            case 'FunctionExpression':
            case 'ArrowFunctionExpression':
                next = {
                    asyncGenerator: record.async === true && record.generator === true,
                    functionDepth: context.functionDepth + 1,
                    moduleScope: false,
                };
                break;
            case 'BlockStatement':
            case 'CatchClause':
            case 'ClassBody':
            case 'ForStatement':
            case 'ForInStatement':
            case 'SwitchStatement':
                next = { ...next, moduleScope: false };
                break;
            default: break;
        }
        if (record.type === 'ExpressionStatement') {
            next = { ...next, statement: record, statementStart: inStatementList(parent, key) ? record.start : undefined };
        }
        for (const [childKey, value] of Object.entries(record)) {
            if (childKey === 'type' || childKey === 'start' || childKey === 'end')
                continue;
            this.visit(value, next, record, childKey);
        }
    }
    isCreateRequireCall(node) {
        if (node.type !== 'CallExpression')
            return false;
        const callee = node.callee;
        const args = node.arguments;
        if (callee.type !== 'Identifier' || !this.createRequireBindings.has(nameOf(callee)) || args.length !== 1) {
            return false;
        }
        const base = args[0];
        if (base.type !== 'MemberExpression' || base.computed === true)
            return false;
        const object = base.object;
        const property = base.property;
        return object.type === 'MetaProperty'
            && object.meta.name === 'import'
            && property.type === 'Identifier'
            && property.name === 'url';
    }
    isRequireCall(callee, moduleScope) {
        return (callee.type === 'Identifier' && callee.name === 'require')
            || (moduleScope && this.isCreateRequireCall(callee));
    }
    indexCreateRequireImports(program) {
        for (const statement of program.body) {
            if (statement.type !== 'ImportDeclaration')
                continue;
            const source = statement.source;
            if (source.value !== 'node:module' && source.value !== 'module')
                continue;
            for (const specifier of statement.specifiers) {
                if (specifier.type !== 'ImportSpecifier' || specifierName(specifier.imported) !== 'createRequire')
                    continue;
                this.createRequireBindings.add(specifier.local.name);
            }
        }
    }
    /**
     * Give every import declaration its module temporary and record its named and
     * default bindings before traversal, because references may precede the
     * declaration in source order.
     */
    indexImports(program) {
        for (const statement of program.body) {
            if (statement.type !== 'ImportDeclaration' || statement.specifiers.length === 0)
                continue;
            const held = this.moduleTemp();
            this.importModules.set(statement, held);
            for (const specifier of statement.specifiers) {
                if (specifier.type === 'ImportDefaultSpecifier') {
                    this.importedBindings.set(specifier.local.name, { held, kind: 'default', imported: 'default' });
                }
                else if (specifier.type === 'ImportSpecifier') {
                    this.importedBindings.set(specifier.local.name, { held, kind: 'named', imported: specifierName(specifier.imported) });
                }
            }
        }
    }
    /**
     * Decide with scope analysis which candidates read an import.
     *
     * Refuses an assignment to an import, which live reads cannot express, and a
     * declaration using the reserved prefix, which would capture an emitted
     * module temporary. Direct `eval` is refused during traversal.
     * @param program - The parsed module, which gains node ranges here.
     * @returns The identifier nodes that read a named or default import.
     */
    resolveImportReads(program) {
        assignRanges(program);
        // acorn's node types model directives differently from `@types/estree`; the
        // tree itself is ESTree. eslint-scope distinguishes only ES5 and ES2015+
        // scoping, so any later edition selects the module rules.
        const manager = analyze(program, { ecmaVersion: 2022, sourceType: 'module', childVisitorKeys: KEYS });
        for (const scope of manager.scopes) {
            for (const variable of scope.variables) {
                const definition = variable.defs[0];
                if (definition === undefined || !variable.name.startsWith(RESERVED_PREFIX))
                    continue;
                this.fail(`${variable.name} uses the transform's reserved ${RESERVED_PREFIX} prefix`, this.rangeStart(definition.name));
            }
        }
        const reads = new Set();
        const moduleScope = manager.scopes.find(scope => scope.type === 'module');
        for (const variable of moduleScope?.variables ?? []) {
            if (!this.importedBindings.has(variable.name) || variable.defs[0]?.type !== 'ImportBinding')
                continue;
            for (const reference of variable.references) {
                if (reference.isWrite())
                    this.fail(`the import ${variable.name} cannot be assigned`, this.rangeStart(reference.identifier));
                reads.add(reference.identifier);
            }
        }
        return reads;
    }
    /**
     * @param node - A node scope analysis reports.
     * @returns Its source offset; `assignRanges` gave every node a range.
     */
    rangeStart(node) {
        const start = node.range?.[0];
        if (start === undefined)
            this.fail('scope analysis reported a node without a source range', 0);
        return start;
    }
    run() {
        // Transforming a lowered body again would nest the protocol inside itself:
        // it still runs, only slower and unreadable, so a mis-wired manifest must
        // surface here rather than as a silent tax on every load.
        if (this.source.includes(`${ALS}.pause(`) || this.source.includes('__als$')) {
            this.fail('the module is already lowered; check the image manifest wiring', 0);
        }
        let program;
        try {
            program = parse(this.source, {
                ecmaVersion: 'latest',
                sourceType: 'module',
                allowAwaitOutsideFunction: true,
            });
        }
        catch (reason) {
            this.fail(`parse failed: ${reason.message}`, 0);
        }
        this.indexCreateRequireImports(program);
        this.indexImports(program);
        this.visit(program, { asyncGenerator: false, functionDepth: 0, moduleScope: true });
        // Without a declaration or assignment of an import's name, nothing can
        // shadow it, so every candidate reads the import.
        const reads = this.scopeAnalysisNeeded ? this.resolveImportReads(program) : undefined;
        for (const candidate of this.importCandidates) {
            if (reads === undefined || reads.has(candidate.node))
                this.importReference(candidate);
        }
        if (this.edits.length === 0 && !this.moduleSyntax)
            return this.source;
        const prologue = [];
        if (this.moduleSyntax)
            prologue.push('"use strict";Object.defineProperty(exports,"__esModule",{value:true});');
        // Reading an exported import can register the default helper, so the
        // getters are built before the helper sources are emitted.
        const getters = this.bindings.map(({ exported, local }) => `__dsh$def(exports,${JSON.stringify(exported)},()=>${this.bindingAccess(local)});`);
        if (this.bindings.length > 0)
            this.helper('def');
        for (const [name, source] of Object.entries(HELPER_SOURCE)) {
            if (this.helpers.has(name))
                prologue.push(source);
        }
        prologue.push(...getters);
        const sorted = [...this.edits].sort((left, right) => left.start - right.start || left.end - right.end);
        const render = (from, to) => {
            let cursor = from;
            let out = '';
            for (const edit of sorted) {
                if (edit.start < cursor || edit.end > to)
                    continue;
                out += this.source.slice(cursor, edit.start) + edit.render(render);
                cursor = edit.end;
            }
            return out + this.source.slice(cursor, to);
        };
        const code = prologue.join('') + render(0, this.source.length);
        // Proof that the emitted body is CommonJS a wrapper can compile: any leftover
        // module syntax, or any mis-spliced interval, fails here rather than at load.
        try {
            parse(code, { ecmaVersion: 'latest', sourceType: 'script', allowAwaitOutsideFunction: false });
        }
        catch (reason) {
            this.fail(`the transform produced code that does not parse: ${reason.message}`, 0);
        }
        return code;
    }
}
/** @returns The name a specifier or identifier node carries. */
function nameOf(node) {
    return node.type === 'Identifier' ? node.name : String(node.value);
}
/** @returns The name an import or export specifier spells, as an identifier or a string literal. */
function specifierName(node) {
    return node.type === 'Identifier' ? node.name : String(node.value);
}
/**
 * Give every node the `range` that eslint-scope reads. The parse leaves ranges
 * out because only modules that need scope analysis use them.
 * @param node - A node or node array of the parsed module.
 */
function assignRanges(node) {
    if (node === null || typeof node !== 'object')
        return;
    if (Array.isArray(node)) {
        for (const child of node)
            assignRanges(child);
        return;
    }
    const record = node;
    if (typeof record.type !== 'string')
        return;
    record.range = [record.start, record.end];
    for (const value of Object.values(record))
        assignRanges(value);
}
/**
 * ESTree positions where an identifier names a declared or assigned binding.
 * Property values inside an object pattern are checked at the pattern.
 *
 * Together with the ObjectPattern check, this must recognize every ESTree
 * declaration and assignment form: a missed form lets a shadowing local skip
 * scope analysis and be rewritten as an import read. A parser upgrade that adds
 * a binding form must extend this list and the shadowing cases in
 * `transform-imports.spec.ts`.
 * @param parent - Parent of the identifier.
 * @param key - The parent property holding it.
 * @returns Whether the identifier declares or assigns a binding.
 */
function isBindingPosition(parent, key) {
    switch (parent.type) {
        case 'VariableDeclarator': return key === 'id';
        case 'FunctionDeclaration':
        case 'FunctionExpression':
        case 'ArrowFunctionExpression': return key === 'id' || key === 'params';
        case 'ClassDeclaration':
        case 'ClassExpression': return key === 'id';
        case 'CatchClause': return key === 'param';
        case 'ArrayPattern':
        case 'RestElement':
        case 'UpdateExpression': return true;
        case 'AssignmentPattern':
        case 'AssignmentExpression':
        case 'ForInStatement':
        case 'ForOfStatement': return key === 'left';
        default: return false;
    }
}
/**
 * ESTree positions where an identifier, outside a binding position, reads a
 * binding rather than naming a property, label, specifier, or meta property.
 * Positions not listed read a binding, so the binding positions must be ruled
 * out first (`isBindingPosition`).
 * @param parent - Parent of the identifier.
 * @param key - The parent property holding it.
 * @returns Whether the identifier reads a binding.
 */
function isReadPosition(parent, key) {
    switch (parent.type) {
        case 'MemberExpression': return key !== 'property' || parent.computed === true;
        case 'Property':
        case 'MethodDefinition':
        case 'PropertyDefinition': return key !== 'key' || parent.computed === true;
        case 'LabeledStatement':
        case 'BreakStatement':
        case 'ContinueStatement':
        case 'ImportSpecifier':
        case 'ImportDefaultSpecifier':
        case 'ImportNamespaceSpecifier':
        case 'ExportSpecifier':
        case 'ExportAllDeclaration':
        case 'MetaProperty': return false;
        default: return true;
    }
}
/**
 * @param parent - Parent of a statement.
 * @param key - The parent property holding it.
 * @returns Whether the statement sits in a statement list, where it can follow
 * a statement that ends without a semicolon.
 */
function inStatementList(parent, key) {
    if (parent === undefined)
        return false;
    if (parent.type === 'SwitchCase')
        return key === 'consequent';
    return key === 'body' && (parent.type === 'Program' || parent.type === 'BlockStatement' || parent.type === 'StaticBlock');
}
/** Every binding an exported declaration introduces, including patterns. */
function declaredBindings(declaration, fail) {
    if (declaration.type === 'FunctionDeclaration' || declaration.type === 'ClassDeclaration') {
        const id = declaration.id;
        if (id === null)
            fail('an exported declaration must be named');
        const name = id.name;
        return [{ exported: name, local: name }];
    }
    if (declaration.type !== 'VariableDeclaration')
        fail(`unsupported exported declaration ${declaration.type}`);
    const bindings = [];
    const collect = (pattern) => {
        switch (pattern.type) {
            case 'Identifier':
                bindings.push({ exported: pattern.name, local: pattern.name });
                return;
            case 'ObjectPattern':
                for (const property of pattern.properties) {
                    collect((property.type === 'RestElement' ? property.argument : property.value));
                }
                return;
            case 'ArrayPattern':
                for (const element of pattern.elements)
                    if (element !== null)
                        collect(element);
                return;
            case 'AssignmentPattern':
                collect(pattern.left);
                return;
            case 'RestElement':
                collect(pattern.argument);
                return;
            default:
                fail(`unsupported binding pattern ${pattern.type}`);
        }
    };
    for (const declarator of declaration.declarations)
        collect(declarator.id);
    return bindings;
}
const cache = new Map();
/**
 * Transform one module into a body for the worker wrapper.
 *
 * Results are cached by source text, so a module reached through two paths, or
 * a repeated build, parses once.
 * @param source - Module source, ESM or CommonJS.
 * @param path - Path used in diagnostics.
 * @returns The lowered body and the module requests found in it.
 */
function transformDetailed(source, path) {
    const cached = cache.get(source);
    if (cached !== undefined)
        return cached;
    const transformer = new Transformer(source, path);
    const transformed = { code: transformer.run(), moduleRequests: transformer.requests(), metaResolveRequests: transformer.metaRequests() };
    cache.set(source, transformed);
    return transformed;
}
/**
 * Lower one module at image-pack time.
 *
 * The collector calls this for every JavaScript entry it packs and records
 * `LOWERING_VERSION` in the image manifest; the loader then wraps those entries
 * without parsing them. `lowered: false` reports that the transform would have
 * returned the input verbatim (already CommonJS, no suspension point), so the
 * entry may be packed as it is.
 *
 * Throwing is the intended failure mode: a module this transform cannot express
 * must fail the build rather than ship an image that breaks at load.
 * @param options - Virtual path inside the image and the module source.
 * @returns The code to pack and whether it changed.
 */
export function lowerModuleSource(options) {
    const { code, moduleRequests, metaResolveRequests } = transformDetailed(options.source, options.filename);
    return { code, lowered: code !== options.source, moduleRequests, metaResolveRequests };
}
//# sourceMappingURL=transform.js.map