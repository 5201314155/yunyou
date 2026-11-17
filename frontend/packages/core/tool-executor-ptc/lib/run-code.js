import { n as Config, r as createRunCodeTransport, t as PtcCalls } from "./_chunks/ptc-calls-CGry9Giw.js";
import { ToolExecutionPipeline } from "@deepseek-ai/dsh-tools/pipeline";
// lib/types/run-code.js
/**
* Create a tool whose programs can call only the supplied private definitions.
* Nested calls retain approvals, guards, result policy, dispatch logs, and deferred contexts.
* The owning context cancels and drains active Agent programs when disposed if the runtime
* retains state or enforces a sandbox. Agent disposal releases retained runtime state.
* @param ctx - Context with tools, lifecycle ownership, and policies; Agent calls also require workingDirectory.
* @param options - Fixed runtime, tools, presentation, and configurable limits.
* @returns An unregistered definition; register it in the owning scope with ctx.tools.register.
* @throws Error if private tool names repeat or configured limits are invalid.
*/
function createRunCodeTool(ctx, options) {
	const definitions = /* @__PURE__ */ new Map();
	for (const definition of options.tools) {
		if (definitions.has(definition.name)) throw new Error(`Program tool ${options.name} has duplicate binding ${definition.name}`);
		definitions.set(definition.name, definition);
	}
	const pipeline = new ToolExecutionPipeline(ctx, { get: (name) => definitions.get(name) });
	return createRunCodeTransport(ctx, pipeline, Config(options.limits ?? {}), new PtcCalls(ctx), {
		...options,
		tools: [...definitions.values()]
	});
}
export { Config as RunCodeConfig, createRunCodeTool };
