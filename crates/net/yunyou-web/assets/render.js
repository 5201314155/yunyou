// 云悠 web 渲染层：视觉块与《CLI终端显示设计规范》§4 同构——
// 用户回显(4.3)/等待指示(4.4)/流式回复(4.5)/工具卡(4.6-4.7)/思考收敛/落款(4.12)。
// 线性流淌纪律的 web 等价：只追加节点，已渲染的历史块不回改；
// 「活动块」=流式中的正文/思考/工具卡，事件收口时冻结。
// 符号与帧序列一律取 tokens.js 注入常量（不散写字面量）。
(function () {
  "use strict";
  var stream = document.getElementById("stream");
  var input = document.getElementById("input");
  var sendBtn = document.getElementById("send");
  var SYM = window.YUNYOU_SYM || {};
  var FRAMES = window.YUNYOU_SPINNER_FRAMES || ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

  // —— 活动块状态（同一时刻至多一个活动正文块/思考块；工具卡按 call_id 归档）——
  var textBlock = null;  // { el, pre }
  var thinkBlock = null; // { el, summary, pre, chars, frozen }
  var toolRows = {};     // call_id → { row, timer }

  function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text !== undefined) node.textContent = text;
    return node;
  }
  function scrollBottom() { stream.scrollTop = stream.scrollHeight; }

  function setConnected(ok, text) {
    var dot = document.getElementById("dot");
    var statusText = document.getElementById("status-text");
    dot.classList.toggle("ok", ok);
    dot.classList.toggle("err", !ok);
    statusText.textContent = text;
    input.disabled = !ok;
    sendBtn.disabled = !ok;
  }

  // —— 连接问候：模型名进顶栏，hello 卡让位于对话流，会话按钮点亮 ——
  function hello(model) {
    document.getElementById("model").textContent = model || "";
    setConnected(true, "已连接");
    document.getElementById("sessions-btn").disabled = false;
    var card = stream.querySelector(".hello");
    if (card) card.remove();
    scrollBottom();
  }
  function disconnected() {
    setConnected(false, "已断开（服务已退出或连接关闭；重启后需用新 token 地址打开）");
    document.getElementById("sessions-btn").disabled = true;
  }
  function bye(text) {
    setConnected(false, "已断开（/quit）");
    stream.appendChild(el("div", "msg-bye", text));
    scrollBottom();
  }

  // —— §4.3 用户消息回显：❯ 文本（用户的话与云悠的话颜色区分归属） ——
  function userMessage(text) {
    freezeAll();
    var row = el("div", "msg-user");
    row.appendChild(el("span", "sym prompt", SYM.prompt || "❯"));
    row.appendChild(el("span", "text", text));
    stream.appendChild(row);
    scrollBottom();
  }

  // —— §4.4/4.5 等待指示 + 流式回复 ——
  function ensureTextBlock() {
    freezeThinking();
    if (textBlock) return textBlock;
    var wrap = el("div", "msg-assistant");
    var pre = el("div", "text");
    wrap.appendChild(pre);
    stream.appendChild(wrap);
    textBlock = { el: wrap, pre: pre };
    return textBlock;
  }
  function waiting() {
    var b = ensureTextBlock();
    if (!b.pre.querySelector(".waiting")) {
      b.pre.appendChild(el("span", "waiting", (SYM.pending || "●") + " 云悠正在想…"));
      scrollBottom();
    }
  }
  function assistantDelta(text) {
    var b = ensureTextBlock();
    var w = b.pre.querySelector(".waiting");
    if (w) w.remove(); // 首字节到达，等待指示让位正文（§4.4）
    b.pre.textContent += text;
    scrollBottom();
  }
  // —— 思考收敛（CLI Collapse 同口径：计数活动行，冻结为折叠条；碎思考吞不落条） ——
  function ensureThinking() {
    if (textBlock) freezeText();
    if (thinkBlock) return thinkBlock;
    var det = el("details", "msg-thinking");
    var summary = el("summary", "", "（思考）0 字");
    var pre = el("div", "think-text");
    det.appendChild(summary);
    det.appendChild(pre);
    stream.appendChild(det);
    thinkBlock = { el: det, summary: summary, pre: pre, chars: 0, frozen: false };
    return thinkBlock;
  }
  function reasoningDelta(text) {
    var t = ensureThinking();
    t.chars += text.length;
    t.summary.textContent = "（思考）" + t.chars + " 字…";
    t.pre.textContent += text;
    scrollBottom();
  }
  function freezeThinking() {
    if (!thinkBlock) return;
    var t = thinkBlock;
    thinkBlock = null;
    if (!t.chars) { t.el.remove(); return; } // 碎思考吞（CLI 同规）
    t.summary.textContent = "（思考）" + t.chars + " 字（已收敛，点击展开）";
  }
  function freezeText() {
    if (!textBlock) return;
    var b = textBlock;
    textBlock = null;
    // 只剩等待指示的空块 → 清除，不留「无内容」假块
    var w = b.pre.querySelector(".waiting");
    if (w && !b.pre.textContent.replace(w.textContent, "").trim()) b.el.remove();
  }
  function freezeAll() { freezeText(); freezeThinking(); finishRunningTools("已取消"); }

  // —— 落盘终态：以会话 JSONL 文本为准冻结（防增量漏字——会话是唯一事实源） ——
  function assistantFinal(text, reasoning) {
    freezeThinking();
    if (textBlock) {
      var w = textBlock.pre.querySelector(".waiting");
      if (w) w.remove();
      if (typeof text === "string") textBlock.pre.textContent = text;
      textBlock = null;
    }
    void reasoning; // P2 不双显思考正文（折叠条已带计数）；全文查看走会话 JSONL
  }

  // —— §4.6/4.7 工具卡：spinner → ✓/✗（成功补结果摘要，失败必须给原因） ——
  function toolCall(call) {
    freezeAll();
    var row = el("div", "tool-card running");
    var spin = el("span", "spinner", FRAMES[0]);
    row.appendChild(spin);
    row.appendChild(el("span", "tool-name", call.name));
    // 实时事件带 arguments_summary（120 字符截断）；回放事件带完整 arguments——同一卡位取近取全
    var args = call.arguments_summary || call.arguments || "";
    row.appendChild(el("span", "tool-args", args ? " · " + args : ""));
    stream.appendChild(row);
    var frame = 0;
    var timer = setInterval(function () {
      frame = (frame + 1) % FRAMES.length;
      spin.textContent = FRAMES[frame];
    }, window.YUNYOU_SPINNER_INTERVAL_MS || 80);
    toolRows[call.call_id] = { row: row, timer: timer };
    scrollBottom();
  }
  function settleSym(row, ok, cancelled) {
    var sym = el("span", "sym " + (ok ? "ok" : "err"),
      cancelled ? "–" : (ok ? (SYM.success || "✓") : (SYM.failure || "✗")));
    var spinnerEl = row.querySelector(".spinner");
    if (spinnerEl) spinnerEl.replaceWith(sym); else row.prepend(sym);
    row.classList.remove("running");
  }
  function toolResult(res) {
    var entry = toolRows[res.call_id];
    var row;
    if (entry) {
      clearInterval(entry.timer);
      delete toolRows[res.call_id];
      row = entry.row;
    } else {
      row = el("div", "tool-card"); // 快路径：无 call 行直接落定（不假装忙碌）
      stream.appendChild(row);
    }
    settleSym(row, !res.is_error, false);
    var extra = [];
    if (res.exit_code !== null && res.exit_code !== undefined && res.exit_code !== 0) {
      extra.push("退出码 " + res.exit_code); // 失败优先原则：退出码必给
    }
    var summary = res.summary || replayContentSummary(res.content);
    if (summary) extra.push(summary);
    if (extra.length) row.appendChild(el("span", "tool-args", " · " + extra.join(" · ")));
    // 详情折叠：canonical value 原样（完整无损在会话 JSONL——与 CLI 同纪律）
    var det = el("details", "tool-detail");
    det.appendChild(el("summary", "", "详情"));
    det.appendChild(el("pre", "", contentText(res.content)));
    row.appendChild(det);
    scrollBottom();
  }
  // 回放形态适配：JSONL 的 tool-result 无 summary 字段，内容摘首行
  function replayContentSummary(content) {
    if (typeof content !== "string") return "";
    var first = content.split("\n")[0];
    return first.length > 48 ? first.slice(0, 48) + "…" : first;
  }
  // 实时 content=对象（canonical value）；回放 content=字符串（渲染后文本或 JSON 文本）——尝试解析还原
  function contentText(v) {
    if (v == null) return "";
    if (typeof v === "string") {
      try { return JSON.stringify(JSON.parse(v), null, 2); } catch (e) { return v; }
    }
    return safeJson(v);
  }
  function finishRunningTools(label) {
    Object.keys(toolRows).forEach(function (id) {
      var entry = toolRows[id];
      clearInterval(entry.timer);
      delete toolRows[id];
      settleSym(entry.row, false, true);
      entry.row.appendChild(el("span", "tool-args", " · " + label));
    });
  }
  function safeJson(v) {
    try { return JSON.stringify(v, null, 2); } catch (e) { return String(v); }
  }

  // —— §4.12 状态落款行（回合完结一次；截断必告示） ——
  function turnComplete(meta) {
    freezeText();
    freezeThinking();
    var parts = ["☁ 本回合 " + formatElapsed(meta.elapsed_ms)];
    if (meta.prompt_tokens != null) parts.push("↑" + fmtNum(meta.prompt_tokens));
    if (meta.completion_tokens != null) parts.push("↓" + fmtNum(meta.completion_tokens));
    if (meta.model) parts.push(meta.model);
    stream.appendChild(el("div", "turn-done", "── " + parts.join(" · ") + " ──"));
    if (meta.truncated) {
      stream.appendChild(el("div", "turn-truncated", "（已达单回合步数上限，输出截断）"));
    }
    scrollBottom();
  }
  function formatElapsed(ms) {
    if (ms == null) return "—";
    if (ms < 60000) return (ms / 1000).toFixed(1) + "s";
    var m = Math.floor(ms / 60000);
    var s = Math.round((ms % 60000) / 1000);
    return m + "m" + (s < 10 ? "0" : "") + s + "s";
  }
  function fmtNum(n) { return Number(n).toLocaleString("en-US"); }

  // —— 壳层词汇：命令输出 / 错误 ——
  function commandOutput(text, ok) {
    freezeAll();
    var row = el("div", "msg-command" + (ok ? "" : " failed"));
    row.textContent = text;
    stream.appendChild(row);
    scrollBottom();
  }
  function errorMessage(text) {
    finishRunningTools("已中断");
    stream.appendChild(el("div", "msg-error", (SYM.failure || "✗") + " " + text));
    scrollBottom();
  }

  // —— P4 会话面：列表下拉 + 历史回放（§4.13：视觉块与实时一致） ——
  function sessionMenu(sessions) {
    var menu = document.getElementById("session-menu");
    menu.innerHTML = "";
    if (!sessions.length) {
      menu.appendChild(el("div", "session-empty", "（该目录暂无会话文件）"));
    }
    sessions.forEach(function (s) {
      var item = el("button", "session-item");
      item.type = "button";
      item.textContent = s.id + " · " + new Date(s.modified_ms).toLocaleString() +
        " · " + fmtSize(s.size_bytes);
      item.addEventListener("click", function () {
        closeMenu();
        window.YunyouWs.replaySession(s.id);
      });
      menu.appendChild(item);
    });
    menu.classList.add("open");
  }
  function closeMenu() {
    var menu = document.getElementById("session-menu");
    if (menu) menu.classList.remove("open");
  }
  function fmtSize(bytes) {
    if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + "MB";
    if (bytes >= 1024) return (bytes / 1024).toFixed(1) + "KB";
    return bytes + "B";
  }
  // 回放：清流重建（历史块与实时同款视觉；命令 run 侧为纯日志配对不呈现）
  function replayHistory(m) {
    stream.innerHTML = "";
    textBlock = null;
    thinkBlock = null;
    Object.keys(toolRows).forEach(function (id) { clearInterval(toolRows[id].timer); });
    toolRows = {};
    var head = "☁ 已恢复会话 " + m.session_id + " · " + m.events.length + " 条事件" +
      (m.truncated ? "（仅尾部，完整历史在会话文件）" : "");
    stream.appendChild(el("div", "session-head", head));
    m.events.forEach(function (ev) {
      switch (ev.type) {
        case "user-message": userMessage(ev.text); break;
        case "assistant-message": assistantFinal(ev.text, ev.reasoning); break;
        case "tool-call": toolCall(ev); break;
        case "tool-result": toolResult(ev); break;
        case "system-message":
          stream.appendChild(el("div", "msg-system", ev.text));
          break;
        case "command-done": commandOutput(ev.text, ev.kind === "success"); break;
        default: break; // command-run=纯日志配对；未知类型静默（ignorable 同源）
      }
    });
    finishRunningTools("无结果"); // 尾部悬空的 tool-call（被打断的历史回合）
    scrollBottom();
  }

  window.YunyouRender = {
    hello: hello,
    disconnected: disconnected,
    bye: bye,
    userMessage: userMessage,
    waiting: waiting,
    assistantDelta: assistantDelta,
    reasoningDelta: reasoningDelta,
    assistantFinal: assistantFinal,
    toolCall: toolCall,
    toolResult: toolResult,
    turnComplete: turnComplete,
    commandOutput: commandOutput,
    errorMessage: errorMessage,
    sessionMenu: sessionMenu,
    closeMenu: closeMenu,
    replayHistory: replayHistory
  };
})();
