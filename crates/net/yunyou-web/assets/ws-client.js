// 云悠 web 连接层：WS 生命周期 + 协议分派。
// 词汇 = 服务端 protocol.rs 一一对应；未知词汇静默忽略（前向兼容——服务端
// 新词汇不炸旧页面）。重连策略：不自动重连——token 随进程铸造，服务重启后
// 旧 token 必失效，自动重连只会刷 403；页面给出「用新地址打开」的人话指引。
(function () {
  "use strict";
  var R = window.YunyouRender;
  var ws = null;

  function connect() {
    var params = new URLSearchParams(location.search);
    var token = params.get("token") || "";
    var proto = location.protocol === "https:" ? "wss://" : "ws://";
    ws = new WebSocket(proto + location.host + "/ws?token=" + encodeURIComponent(token));
    ws.onopen = function () {
      // 等 hello 帧再亮灯（握手成功≠通道语义就绪）
    };
    ws.onmessage = function (evt) {
      var msg;
      try { msg = JSON.parse(evt.data); } catch (e) { return; }
      dispatch(msg);
    };
    ws.onclose = function () { R.disconnected(); };
    ws.onerror = function () { R.disconnected(); };
  }

  function dispatch(m) {
    switch (m.type) {
      case "hello": R.hello(m.model); break;
      case "user-message": R.userMessage(m.text); break;
      case "assistant-delta": R.assistantDelta(m.text); break;
      case "reasoning-delta": R.reasoningDelta(m.text); break;
      case "tool-call": R.toolCall(m); break;
      case "tool-result": R.toolResult(m); break;
      case "assistant-message": R.assistantFinal(m.text, m.reasoning); break;
      case "turn-complete": R.turnComplete(m); break;
      case "command-output": R.commandOutput(m.text, m.ok); break;
      case "error": R.errorMessage(m.text); break;
      case "bye": R.bye(m.text); break;
      case "session-list": R.sessionMenu(m.sessions); break;
      case "history": R.replayHistory(m); break;
      default: break; // 未知词汇：前向兼容
    }
  }

  function sendRaw(obj) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(obj));
      return true;
    }
    return false;
  }
  function send(text) {
    return sendRaw({ type: "submit", text: text });
  }
  function listSessions() {
    return sendRaw({ type: "list-sessions" });
  }
  function replaySession(id) {
    return sendRaw({ type: "replay-session", id: id });
  }

  window.YunyouWs = { connect: connect, send: send, listSessions: listSessions, replaySession: replaySession };
})();
