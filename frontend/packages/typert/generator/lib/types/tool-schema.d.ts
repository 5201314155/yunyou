/** Lossless projection into the harness tool-schema subset. */
import type { TypeNodeId } from './model.ts';
import { TypeGraphRenderer } from './renderer.ts';
/** A resolved codec whose type the tool-schema subset cannot represent without weakening constraints. */
export declare class ToolSchemaError extends Error {
    name: string;
}
/**
 * Project one resolved type into the tool-schema subset that `@deepseek-ai/dsh-tools`
 * enforces on tool parameters, tool output, and the generated PTC SDK. A type the
 * subset cannot express without weakening constraints raises {@link ToolSchemaError},
 * which callers report as an absent projection rather than a narrower schema.
 * @param renderer - complete analyzed type graph.
 * @param type - resolved wire type.
 * @param closedObjects - close declared request objects with `additionalProperties: false`.
 * @param optional - omit a top-level undefined alternative supplied by an optional argument.
 * @returns an acyclic schema in the tool-schema subset.
 */
export declare function projectToolSchema(renderer: TypeGraphRenderer, type: TypeNodeId, closedObjects?: boolean, optional?: boolean): Record<string, unknown>;
//# sourceMappingURL=tool-schema.d.ts.map