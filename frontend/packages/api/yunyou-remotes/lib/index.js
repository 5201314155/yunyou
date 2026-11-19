import { Context, Service } from "@deepseek-ai/cordis";
// ../../typert/protocol/src/remote-error.ts
/**
* One Remote call failure: a real Error carrying its stable code and typed
* details. Owners throw it at the failure point; the Host Gateway encodes it
* onto the wire unchanged; the Client face rebuilds an instance for the
* `RemoteResult` error branch, so `throw result.error` keeps throw semantics.
* Discrimination is always by `code`, never by instanceof.
*/
var RemoteError = class extends Error {
	code;
	details;
	/** Structural marker: cross-realm/bundle identification never uses instanceof. */
	isDSHRemoteError = true;
	/**
	* @param code - stable failure code declared in {@link RemoteErrorDetailsMap}.
	* @param message - human diagnostic carried across the wire.
	* @param details - structured payload typed by the code.
	* @param options - standard Error options (`cause` survives in-process only).
	*/
	constructor(code, message, details, options) {
		super(message, options);
		this.code = code;
		this.details = details;
		this.name = "RemoteError";
	}
};
// ../../typert/protocol/src/index.ts
/**
* Remote decorators and explicit Gateway bindings backed by versioned
* descriptors carried on decorated class prototypes. Strict reflection
* remains a Typert compiler responsibility.
* @module @deepseek-ai/dsh-typert-protocol
*/
const TYPERT_REMOTE_SEGMENT_PATTERN = /^[A-Za-z0-9_$.-]+$/;
/**
* Test one generated Remote name against the Connection endpoint grammar.
* @param value - namespace, method, lookup, or Context segment.
* @returns whether the value can cross the shared RPC carrier unchanged.
*/
function isTypertRemoteSegment(value) {
	return value !== "." && value !== ".." && TYPERT_REMOTE_SEGMENT_PATTERN.test(value);
}
const REMOTE_METHOD_DESCRIPTOR = "@deepseek-ai/dsh-typert-protocol/remote-methods";
/**
* Bind one visible Service field to a Cordis key and Remote namespace. A
* service that owns a Cordis Context also gives its tree `ctx.invocation`,
* `undefined` outside a Remote call, so no `TypertRemoteService` is needed for
* a Host composition to read it.
* @param service - owning Service instance, normally `this`.
* @param serviceKey - exact Cordis service key.
* @param options - optional wire namespace.
* @returns a frozen, inspectable binding with no compiler-injected metadata.
*/
function bindTypertRemote(service, serviceKey, options = {}) {
	validateName("service key", serviceKey);
	const namespace = options.namespace ?? serviceKey;
	validateName("namespace", namespace);
	const ctx = Reflect.get(service, "ctx");
	if (Context.is(ctx)) provideInvocationAccessor(ctx);
	return Object.freeze({
		service,
		serviceKey,
		namespace
	});
}
/** Cordis Service base that exposes its registered name through Typert Gateway. */
var TypertRemoteService = class extends Service {
	/** Visible binding consumed by the Gateway's source-mode discovery. */
	typertRemote;
	/**
	* Register the Service and bind the same key to Typert Gateway.
	* @param ctx - owning Cordis Context.
	* @param serviceKey - exact Cordis service key and default wire namespace.
	* @param options - optional wire namespace.
	*/
	constructor(ctx, serviceKey, options = {}) {
		super(ctx, serviceKey);
		this.typertRemote = bindTypertRemote(this, this.name, options);
	}
};
/**
* Make `ctx.invocation` read as `undefined` outside a Remote call instead of the
* reflect service's "cannot get property" error; a call-derived Context shadows
* the accessor with its own property. The first Remote Service constructed in a
* tree registers it on the root, where it outlives any one Service.
*/
function provideInvocationAccessor(ctx) {
	if (Object.hasOwn(ctx.root.reflect.props, "invocation")) return;
	ctx.root.accessor("invocation", { get: () => void 0 });
}
function Remote(methodExportOrOptions, context) {
	if (typeof methodExportOrOptions === "string") {
		validateName("Remote export name", methodExportOrOptions);
		return remoteDecorator({ kind: "direct" }, { exportName: methodExportOrOptions });
	}
	if (typeof methodExportOrOptions === "object") return remoteDecorator({ kind: "direct" }, markerOptions(methodExportOrOptions, REMOTE_OPTION_KEYS, "Remote options require name, mode: \"stream\", or resources"));
	if (context === void 0) throw new TypeError("typert-protocol: Remote decorator context is missing");
	addMarkerInitializer(context, { kind: "direct" });
}
const REMOTE_OPTION_KEYS = /* @__PURE__ */ new Set([
	"name",
	"mode",
	"resources"
]);
/** Validate one decorator options object against its accepted keys and select the marker fields. */
function markerOptions(options, accepted, requirement) {
	const keys = Reflect.ownKeys(options);
	if (keys.length === 0 || keys.some((key) => typeof key !== "string" || !accepted.has(key)) || Object.hasOwn(options, "mode") && options.mode !== "stream") throw new TypeError(`typert-protocol: ${requirement}`);
	const { mode, name, resources } = options;
	if (name !== void 0) validateName("Remote export name", name);
	return {
		...mode === void 0 ? {} : { mode },
		...name === void 0 ? {} : { exportName: name },
		...resources === void 0 ? {} : { resources: resourceReaders(resources) }
	};
}
/** Freeze declared readers into the marker record; a misdeclared reader fails at decoration. */
function resourceReaders(resources) {
	const readers = {};
	for (const [kind, reader] of Object.entries(resources)) {
		if (typeof reader !== "function") throw new TypeError(`typert-protocol: Remote resource reader "${kind}" must be a function`);
		readers[kind] = reader;
	}
	if (Object.keys(readers).length === 0) throw new TypeError("typert-protocol: Remote resources require at least one reader");
	return Object.freeze(readers);
}
function remoteDecorator(invocation, options = {}) {
	return function(_method, context) {
		addMarkerInitializer(context, invocation, options);
	};
}
function readRemoteMethodDescriptor(prototype) {
	const property = Object.getOwnPropertyDescriptor(prototype, REMOTE_METHOD_DESCRIPTOR);
	if (property === void 0) return void 0;
	const descriptor = property.value;
	if (descriptor === null || typeof descriptor !== "object") throw new TypeError("typert-protocol: Remote method descriptor must be an object");
	const version = Reflect.get(descriptor, "version");
	if (version !== 1) throw new TypeError(`typert-protocol: unsupported Remote method descriptor version ${String(version)}`);
	const methods = Reflect.get(descriptor, "methods");
	if (!Array.isArray(methods)) throw new TypeError("typert-protocol: Remote method descriptor methods must be an array");
	return descriptor;
}
function addMarkerInitializer(context, invocation, options = {}) {
	if (context.private || context.static || typeof context.name !== "string") throw new TypeError("typert-protocol: Remote decorators require a public instance method with a string name");
	const method = context.name;
	context.addInitializer(function() {
		const prototype = Object.getPrototypeOf(this);
		if (prototype === null) throw new TypeError(`typert-protocol: cannot mark Remote method "${method}" on an object without a prototype`);
		mark(prototype, method, invocation, options);
	});
}
function mark(prototype, method, invocation, { mode, exportName, resources }) {
	const descriptor = readRemoteMethodDescriptor(prototype);
	const marker = Object.freeze({
		method,
		...exportName === void 0 || exportName === method ? {} : { exportName },
		...mode === void 0 ? {} : { mode },
		invocation: Object.freeze(invocation),
		...resources === void 0 ? {} : { resources }
	});
	const current = descriptor?.methods.find((candidate) => candidate.method === method);
	if (current !== void 0) {
		if (current.exportName === marker.exportName && current.mode === marker.mode && current.resources === marker.resources && sameInvocation(current.invocation, invocation)) return;
		throw new Error(`typert-protocol: Remote method "${method}" has conflicting invocation markers`);
	}
	Object.defineProperty(prototype, REMOTE_METHOD_DESCRIPTOR, {
		configurable: true,
		value: Object.freeze({
			version: 1,
			methods: Object.freeze([...descriptor?.methods ?? [], marker])
		})
	});
}
function sameInvocation(left, right) {
	if (left.kind === "direct") return right.kind === "direct";
	if (right.kind === "direct") return false;
	return left.context === right.context;
}
function validateName(subject, value) {
	if (!isTypertRemoteSegment(value)) throw new TypeError(`typert-protocol: ${subject} must contain only RPC endpoint segment characters`);
}
// lib/types/index.js
var __runInitializers = function(thisArg, initializers, value) {
	var useValue = arguments.length > 2;
	for (var i = 0; i < initializers.length; i++) value = useValue ? initializers[i].call(thisArg, value) : initializers[i].call(thisArg);
	return useValue ? value : void 0;
};
var __esDecorate = function(ctor, descriptorIn, decorators, contextIn, initializers, extraInitializers) {
	function accept(f) {
		if (f !== void 0 && typeof f !== "function") throw new TypeError("Function expected");
		return f;
	}
	var kind = contextIn.kind, key = kind === "getter" ? "get" : kind === "setter" ? "set" : "value";
	var target = !descriptorIn && ctor ? contextIn["static"] ? ctor : ctor.prototype : null;
	var descriptor = descriptorIn || (target ? Object.getOwnPropertyDescriptor(target, contextIn.name) : {});
	var _, done = false;
	for (var i = decorators.length - 1; i >= 0; i--) {
		var context = {};
		for (var p in contextIn) context[p] = p === "access" ? {} : contextIn[p];
		for (var p in contextIn.access) context.access[p] = contextIn.access[p];
		context.addInitializer = function(f) {
			if (done) throw new TypeError("Cannot add initializers after decoration has completed");
			extraInitializers.push(accept(f || null));
		};
		var result = (0, decorators[i])(kind === "accessor" ? {
			get: descriptor.get,
			set: descriptor.set
		} : descriptor[key], context);
		if (kind === "accessor") {
			if (result === void 0) continue;
			if (result === null || typeof result !== "object") throw new TypeError("Object expected");
			if (_ = accept(result.get)) descriptor.get = _;
			if (_ = accept(result.set)) descriptor.set = _;
			if (_ = accept(result.init)) initializers.unshift(_);
		} else if (_ = accept(result)) {
			if (kind === "field") initializers.unshift(_);
			else descriptor[key] = _;
		}
	}
	if (target) Object.defineProperty(target, contextIn.name, descriptor);
	done = true;
};
/**
* 形状声明面说明：yunyou-web（Rust 网关）是这些方法的真实实现（W3/W4 已接线，
* 分派键=本面端点逐字）；本 TS 宿主不做业务实现，任何调用诚实返回
* gateway/not-implemented（规格 §0.3 fail-visible，不伪造值）。
*/
/** 线形状声明占位：Rust 网关实现，本宿主恒 not-implemented。 */
function bridgeOnly() {
	throw new RemoteError("gateway/not-implemented", "该方法由 yunyou-web 网关桥接实现", {});
}
/** team 域 face：名册/拉人/成员消息/踢人。 */
let YunyouTeamFace = (() => {
	let _classSuper = TypertRemoteService;
	let _instanceExtraInitializers = [];
	let _roster_decorators;
	let _spawn_decorators;
	let _message_decorators;
	let _kick_decorators;
	return class YunyouTeamFace extends _classSuper {
		static {
			const _metadata = typeof Symbol === "function" && Symbol.metadata ? Object.create(_classSuper[Symbol.metadata] ?? null) : void 0;
			_roster_decorators = [Remote("team.roster")];
			_spawn_decorators = [Remote("team.spawn")];
			_message_decorators = [Remote("team.message")];
			_kick_decorators = [Remote("team.kick")];
			__esDecorate(this, null, _roster_decorators, {
				kind: "method",
				name: "roster",
				static: false,
				private: false,
				access: {
					has: (obj) => "roster" in obj,
					get: (obj) => obj.roster
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			__esDecorate(this, null, _spawn_decorators, {
				kind: "method",
				name: "spawn",
				static: false,
				private: false,
				access: {
					has: (obj) => "spawn" in obj,
					get: (obj) => obj.spawn
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			__esDecorate(this, null, _message_decorators, {
				kind: "method",
				name: "message",
				static: false,
				private: false,
				access: {
					has: (obj) => "message" in obj,
					get: (obj) => obj.message
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			__esDecorate(this, null, _kick_decorators, {
				kind: "method",
				name: "kick",
				static: false,
				private: false,
				access: {
					has: (obj) => "kick" in obj,
					get: (obj) => obj.kick
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			if (_metadata) Object.defineProperty(this, Symbol.metadata, {
				enumerable: true,
				configurable: true,
				writable: true,
				value: _metadata
			});
		}
		/** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
		constructor(ctx) {
			super(ctx, "yunyouTeam", { namespace: "yunyou" });
			__runInitializers(this, _instanceExtraInitializers);
		}
		/** 名册投影（L1）。 */
		roster() {
			return bridgeOnly();
		}
		/** 拉起成员（L4，需显式授权）。 */
		spawn(request) {
			return bridgeOnly();
		}
		/** 成员消息（L2，身份=lead 服务端强制）。 */
		message(request) {
			return bridgeOnly();
		}
		/** 踢人（L3）。 */
		kick(request) {
			return bridgeOnly();
		}
	};
})();
/** tasks 域 face：共享任务板全量投影与状态机操作。 */
let YunyouTasksFace = (() => {
	let _classSuper = TypertRemoteService;
	let _instanceExtraInitializers = [];
	let _list_decorators;
	let _create_decorators;
	let _claim_decorators;
	let _submit_decorators;
	let _review_decorators;
	let _reassign_decorators;
	return class YunyouTasksFace extends _classSuper {
		static {
			const _metadata = typeof Symbol === "function" && Symbol.metadata ? Object.create(_classSuper[Symbol.metadata] ?? null) : void 0;
			_list_decorators = [Remote("tasks.list")];
			_create_decorators = [Remote("tasks.create")];
			_claim_decorators = [Remote("tasks.claim")];
			_submit_decorators = [Remote("tasks.submit")];
			_review_decorators = [Remote("tasks.review")];
			_reassign_decorators = [Remote("tasks.reassign")];
			__esDecorate(this, null, _list_decorators, {
				kind: "method",
				name: "list",
				static: false,
				private: false,
				access: {
					has: (obj) => "list" in obj,
					get: (obj) => obj.list
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			__esDecorate(this, null, _create_decorators, {
				kind: "method",
				name: "create",
				static: false,
				private: false,
				access: {
					has: (obj) => "create" in obj,
					get: (obj) => obj.create
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			__esDecorate(this, null, _claim_decorators, {
				kind: "method",
				name: "claim",
				static: false,
				private: false,
				access: {
					has: (obj) => "claim" in obj,
					get: (obj) => obj.claim
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			__esDecorate(this, null, _submit_decorators, {
				kind: "method",
				name: "submit",
				static: false,
				private: false,
				access: {
					has: (obj) => "submit" in obj,
					get: (obj) => obj.submit
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			__esDecorate(this, null, _review_decorators, {
				kind: "method",
				name: "review",
				static: false,
				private: false,
				access: {
					has: (obj) => "review" in obj,
					get: (obj) => obj.review
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			__esDecorate(this, null, _reassign_decorators, {
				kind: "method",
				name: "reassign",
				static: false,
				private: false,
				access: {
					has: (obj) => "reassign" in obj,
					get: (obj) => obj.reassign
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			if (_metadata) Object.defineProperty(this, Symbol.metadata, {
				enumerable: true,
				configurable: true,
				writable: true,
				value: _metadata
			});
		}
		/** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
		constructor(ctx) {
			super(ctx, "yunyouTasks", { namespace: "yunyou" });
			__runInitializers(this, _instanceExtraInitializers);
		}
		/** 任务板全量投影（L1）。 */
		list() {
			return bridgeOnly();
		}
		/** 创建任务（L2）。 */
		create(request) {
			return bridgeOnly();
		}
		/** 认领（L2，owner 可代派）。 */
		claim(request) {
			return bridgeOnly();
		}
		/** 提交（L2，仅 owner）。 */
		submit(request) {
			return bridgeOnly();
		}
		/** 审查裁决（L3，自审服务端拒绝）。 */
		review(request) {
			return bridgeOnly();
		}
		/** 改派（L3）。 */
		reassign(request) {
			return bridgeOnly();
		}
	};
})();
/** schedule 域 face：三选一定时任务（durable 语义=会话内有效）。 */
let YunyouScheduleFace = (() => {
	let _classSuper = TypertRemoteService;
	let _instanceExtraInitializers = [];
	let _create_decorators;
	let _list_decorators;
	let _delete_decorators;
	return class YunyouScheduleFace extends _classSuper {
		static {
			const _metadata = typeof Symbol === "function" && Symbol.metadata ? Object.create(_classSuper[Symbol.metadata] ?? null) : void 0;
			_create_decorators = [Remote("schedule.create")];
			_list_decorators = [Remote("schedule.list")];
			_delete_decorators = [Remote("schedule.delete")];
			__esDecorate(this, null, _create_decorators, {
				kind: "method",
				name: "create",
				static: false,
				private: false,
				access: {
					has: (obj) => "create" in obj,
					get: (obj) => obj.create
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			__esDecorate(this, null, _list_decorators, {
				kind: "method",
				name: "list",
				static: false,
				private: false,
				access: {
					has: (obj) => "list" in obj,
					get: (obj) => obj.list
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			__esDecorate(this, null, _delete_decorators, {
				kind: "method",
				name: "delete",
				static: false,
				private: false,
				access: {
					has: (obj) => "delete" in obj,
					get: (obj) => obj.delete
				},
				metadata: _metadata
			}, null, _instanceExtraInitializers);
			if (_metadata) Object.defineProperty(this, Symbol.metadata, {
				enumerable: true,
				configurable: true,
				writable: true,
				value: _metadata
			});
		}
		/** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
		constructor(ctx) {
			super(ctx, "yunyouSchedule", { namespace: "yunyou" });
			__runInitializers(this, _instanceExtraInitializers);
		}
		/** 创建（L2，nextFireMs 回显即确认）。 */
		create(request) {
			return bridgeOnly();
		}
		/** 列表（L1）。 */
		list() {
			return bridgeOnly();
		}
		/** 删除（L3）。 */
		delete(request) {
			return bridgeOnly();
		}
	};
})();
export { YunyouScheduleFace, YunyouTasksFace, YunyouTeamFace };
