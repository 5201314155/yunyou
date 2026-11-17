// 云悠 web 装配入口：输入事件与发送。
// P2：普通文本=回合提交（服务端经 agent-loop 同路径）；斜杠命令 P3 接命令缝。
// Enter 发送 / Shift+Enter 换行；输入框随内容自适应增高（上限 160px）。
(function () {
  "use strict";
  var input = document.getElementById("input");
  var sendBtn = document.getElementById("send");

  function submit() {
    var text = input.value.trim();
    if (!text) return;
    if (!window.YunyouWs.send(text)) return; // 未连接：渲染层已提示，不装送达
    input.value = "";
    autosize();
  }

  input.addEventListener("keydown", function (e) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  });
  sendBtn.addEventListener("click", submit);

  // P4 会话面：会话按钮 → 请求列表；点页面其他处收起下拉
  var sessionsBtn = document.getElementById("sessions-btn");
  sessionsBtn.addEventListener("click", function (e) {
    e.stopPropagation();
    window.YunyouWs.listSessions();
  });
  document.addEventListener("click", function () {
    window.YunyouRender.closeMenu();
  });

  function autosize() {
    input.style.height = "auto";
    input.style.height = Math.min(input.scrollHeight, 160) + "px";
  }
  input.addEventListener("input", autosize);

  window.YunyouWs.connect();
})();
