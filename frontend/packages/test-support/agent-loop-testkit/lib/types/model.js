/**
 * Create an unsupported Model placeholder for structural Agent stubs.
 * @returns a Model whose operations throw at the first unexpected dependency.
 */
export function unsupportedModel() {
    const reject = () => {
        throw new Error('this test Agent does not support model selection');
    };
    return { current: reject, select: reject };
}
//# sourceMappingURL=model.js.map