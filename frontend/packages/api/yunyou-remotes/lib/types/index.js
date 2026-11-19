var __runInitializers = (this && this.__runInitializers) || function (thisArg, initializers, value) {
    var useValue = arguments.length > 2;
    for (var i = 0; i < initializers.length; i++) {
        value = useValue ? initializers[i].call(thisArg, value) : initializers[i].call(thisArg);
    }
    return useValue ? value : void 0;
};
var __esDecorate = (this && this.__esDecorate) || function (ctor, descriptorIn, decorators, contextIn, initializers, extraInitializers) {
    function accept(f) { if (f !== void 0 && typeof f !== "function") throw new TypeError("Function expected"); return f; }
    var kind = contextIn.kind, key = kind === "getter" ? "get" : kind === "setter" ? "set" : "value";
    var target = !descriptorIn && ctor ? contextIn["static"] ? ctor : ctor.prototype : null;
    var descriptor = descriptorIn || (target ? Object.getOwnPropertyDescriptor(target, contextIn.name) : {});
    var _, done = false;
    for (var i = decorators.length - 1; i >= 0; i--) {
        var context = {};
        for (var p in contextIn) context[p] = p === "access" ? {} : contextIn[p];
        for (var p in contextIn.access) context.access[p] = contextIn.access[p];
        context.addInitializer = function (f) { if (done) throw new TypeError("Cannot add initializers after decoration has completed"); extraInitializers.push(accept(f || null)); };
        var result = (0, decorators[i])(kind === "accessor" ? { get: descriptor.get, set: descriptor.set } : descriptor[key], context);
        if (kind === "accessor") {
            if (result === void 0) continue;
            if (result === null || typeof result !== "object") throw new TypeError("Object expected");
            if (_ = accept(result.get)) descriptor.get = _;
            if (_ = accept(result.set)) descriptor.set = _;
            if (_ = accept(result.init)) initializers.unshift(_);
        }
        else if (_ = accept(result)) {
            if (kind === "field") initializers.unshift(_);
            else descriptor[key] = _;
        }
    }
    if (target) Object.defineProperty(target, contextIn.name, descriptor);
    done = true;
};
import { Remote, RemoteError, TypertRemoteService } from '@deepseek-ai/dsh-typert-protocol';
/**
 * 形状声明面说明：yunyou-web（Rust 网关）是这些方法的真实实现（W3/W4 已接线，
 * 分派键=本面端点逐字）；本 TS 宿主不做业务实现，任何调用诚实返回
 * gateway/not-implemented（规格 §0.3 fail-visible，不伪造值）。
 */
/** 线形状声明占位：Rust 网关实现，本宿主恒 not-implemented。 */
function bridgeOnly() {
    throw new RemoteError('gateway/not-implemented', '该方法由 yunyou-web 网关桥接实现', {});
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
            _roster_decorators = [Remote('team.roster')];
            _spawn_decorators = [Remote('team.spawn')];
            _message_decorators = [Remote('team.message')];
            _kick_decorators = [Remote('team.kick')];
            __esDecorate(this, null, _roster_decorators, { kind: "method", name: "roster", static: false, private: false, access: { has: obj => "roster" in obj, get: obj => obj.roster }, metadata: _metadata }, null, _instanceExtraInitializers);
            __esDecorate(this, null, _spawn_decorators, { kind: "method", name: "spawn", static: false, private: false, access: { has: obj => "spawn" in obj, get: obj => obj.spawn }, metadata: _metadata }, null, _instanceExtraInitializers);
            __esDecorate(this, null, _message_decorators, { kind: "method", name: "message", static: false, private: false, access: { has: obj => "message" in obj, get: obj => obj.message }, metadata: _metadata }, null, _instanceExtraInitializers);
            __esDecorate(this, null, _kick_decorators, { kind: "method", name: "kick", static: false, private: false, access: { has: obj => "kick" in obj, get: obj => obj.kick }, metadata: _metadata }, null, _instanceExtraInitializers);
            if (_metadata) Object.defineProperty(this, Symbol.metadata, { enumerable: true, configurable: true, writable: true, value: _metadata });
        }
        /** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
        constructor(ctx) {
            super(ctx, 'yunyouTeam', { namespace: 'yunyou' });
            __runInitializers(this, _instanceExtraInitializers);
        }
        /** 名册投影（L1）。 */
        roster() {
            return bridgeOnly();
        }
        /** 拉起成员（L4，需显式授权）。 */
        spawn(request) {
            void request;
            return bridgeOnly();
        }
        /** 成员消息（L2，身份=lead 服务端强制）。 */
        message(request) {
            void request;
            return bridgeOnly();
        }
        /** 踢人（L3）。 */
        kick(request) {
            void request;
            return bridgeOnly();
        }
    };
})();
export { YunyouTeamFace };
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
            _list_decorators = [Remote('tasks.list')];
            _create_decorators = [Remote('tasks.create')];
            _claim_decorators = [Remote('tasks.claim')];
            _submit_decorators = [Remote('tasks.submit')];
            _review_decorators = [Remote('tasks.review')];
            _reassign_decorators = [Remote('tasks.reassign')];
            __esDecorate(this, null, _list_decorators, { kind: "method", name: "list", static: false, private: false, access: { has: obj => "list" in obj, get: obj => obj.list }, metadata: _metadata }, null, _instanceExtraInitializers);
            __esDecorate(this, null, _create_decorators, { kind: "method", name: "create", static: false, private: false, access: { has: obj => "create" in obj, get: obj => obj.create }, metadata: _metadata }, null, _instanceExtraInitializers);
            __esDecorate(this, null, _claim_decorators, { kind: "method", name: "claim", static: false, private: false, access: { has: obj => "claim" in obj, get: obj => obj.claim }, metadata: _metadata }, null, _instanceExtraInitializers);
            __esDecorate(this, null, _submit_decorators, { kind: "method", name: "submit", static: false, private: false, access: { has: obj => "submit" in obj, get: obj => obj.submit }, metadata: _metadata }, null, _instanceExtraInitializers);
            __esDecorate(this, null, _review_decorators, { kind: "method", name: "review", static: false, private: false, access: { has: obj => "review" in obj, get: obj => obj.review }, metadata: _metadata }, null, _instanceExtraInitializers);
            __esDecorate(this, null, _reassign_decorators, { kind: "method", name: "reassign", static: false, private: false, access: { has: obj => "reassign" in obj, get: obj => obj.reassign }, metadata: _metadata }, null, _instanceExtraInitializers);
            if (_metadata) Object.defineProperty(this, Symbol.metadata, { enumerable: true, configurable: true, writable: true, value: _metadata });
        }
        /** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
        constructor(ctx) {
            super(ctx, 'yunyouTasks', { namespace: 'yunyou' });
            __runInitializers(this, _instanceExtraInitializers);
        }
        /** 任务板全量投影（L1）。 */
        list() {
            return bridgeOnly();
        }
        /** 创建任务（L2）。 */
        create(request) {
            void request;
            return bridgeOnly();
        }
        /** 认领（L2，owner 可代派）。 */
        claim(request) {
            void request;
            return bridgeOnly();
        }
        /** 提交（L2，仅 owner）。 */
        submit(request) {
            void request;
            return bridgeOnly();
        }
        /** 审查裁决（L3，自审服务端拒绝）。 */
        review(request) {
            void request;
            return bridgeOnly();
        }
        /** 改派（L3）。 */
        reassign(request) {
            void request;
            return bridgeOnly();
        }
    };
})();
export { YunyouTasksFace };
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
            _create_decorators = [Remote('schedule.create')];
            _list_decorators = [Remote('schedule.list')];
            _delete_decorators = [Remote('schedule.delete')];
            __esDecorate(this, null, _create_decorators, { kind: "method", name: "create", static: false, private: false, access: { has: obj => "create" in obj, get: obj => obj.create }, metadata: _metadata }, null, _instanceExtraInitializers);
            __esDecorate(this, null, _list_decorators, { kind: "method", name: "list", static: false, private: false, access: { has: obj => "list" in obj, get: obj => obj.list }, metadata: _metadata }, null, _instanceExtraInitializers);
            __esDecorate(this, null, _delete_decorators, { kind: "method", name: "delete", static: false, private: false, access: { has: obj => "delete" in obj, get: obj => obj.delete }, metadata: _metadata }, null, _instanceExtraInitializers);
            if (_metadata) Object.defineProperty(this, Symbol.metadata, { enumerable: true, configurable: true, writable: true, value: _metadata });
        }
        /** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
        constructor(ctx) {
            super(ctx, 'yunyouSchedule', { namespace: 'yunyou' });
            __runInitializers(this, _instanceExtraInitializers);
        }
        /** 创建（L2，nextFireMs 回显即确认）。 */
        create(request) {
            void request;
            return bridgeOnly();
        }
        /** 列表（L1）。 */
        list() {
            return bridgeOnly();
        }
        /** 删除（L3）。 */
        delete(request) {
            void request;
            return bridgeOnly();
        }
    };
})();
export { YunyouScheduleFace };
//# sourceMappingURL=index.js.map