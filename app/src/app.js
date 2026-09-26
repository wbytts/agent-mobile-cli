// Agent Mobile Bridge 诊断 UI（纯前端 mock，后续接 Rust 命令）
(function () {
  "use strict";

  // ---------- 页面路由（tab 切换） ----------
  var tabs = document.querySelectorAll(".tab");
  var pages = document.querySelectorAll(".page");

  function switchPage(pageId) {
    pages.forEach(function (p) {
      p.classList.toggle("active", p.id === pageId);
    });
    tabs.forEach(function (t) {
      t.classList.toggle("active", t.dataset.page === pageId);
    });
  }

  tabs.forEach(function (t) {
    t.addEventListener("click", function () {
      switchPage(t.dataset.page);
    });
  });

  // ---------- 连接页（mock 状态机） ----------
  var connBadge = document.getElementById("conn-badge");
  var connStateText = document.getElementById("conn-state-text");
  var deviceIdText = document.getElementById("device-id-text");
  var heartbeatText = document.getElementById("heartbeat-text");
  var btnConnect = document.getElementById("btn-connect");
  var btnScan = document.getElementById("btn-scan");
  var addrInput = document.getElementById("daemon-addr");
  var codeInput = document.getElementById("pair-code");

  var connected = false;

  function renderConnState() {
    connBadge.textContent = connected ? "已连接" : "未连接";
    connBadge.className = "badge " + (connected ? "badge-connected" : "badge-disconnected");
    connStateText.textContent = connected ? "已连接" : "未连接";
    btnConnect.textContent = connected ? "断开" : "连接";
  }

  btnConnect.addEventListener("click", function () {
    if (!connected && !addrInput.value.trim()) {
      appendLog("err", "未填写 daemon 地址");
      addrInput.focus();
      return;
    }
    // TODO(接线): 调用 Rust 命令 connect/disconnect，当前为 mock 切换
    connected = !connected;
    deviceIdText.textContent = connected ? "bridge:mumu-5554 (mock)" : "—";
    heartbeatText.textContent = connected ? new Date().toLocaleTimeString() : "—";
    appendLog(connected ? "ok" : "info",
      connected ? "（mock）已连接 " + addrInput.value.trim() : "（mock）已断开");
    renderConnState();
  });

  btnScan.addEventListener("click", function () {
    // TODO(接线): 相机扫码解析 agent-mobile://pair URI
    appendLog("info", "扫码入口：待接线（相机 + ML Kit/zxing）");
    btnScan.textContent = "扫码配对（待接线）";
  });

  // ---------- 能力自检页（mock） ----------
  var capResult = document.getElementById("cap-result");
  document.querySelectorAll(".btn-cap").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var cap = btn.dataset.cap;
      // TODO(接线): 调用 Rust 命令 self_test(cap)
      capResult.textContent = "「" + cap + "」自测待接线：等待无障碍服务与插件接入";
      appendLog("info", "能力自测 " + cap + "：待接线");
    });
  });

  document.getElementById("btn-a11y-settings").addEventListener("click", function () {
    // TODO(接线): 跳转系统无障碍设置
    capResult.textContent = "无障碍设置跳转待接线";
    appendLog("info", "无障碍设置跳转：待接线");
  });

  // ---------- 日志页 ----------
  var logList = document.getElementById("log-list");

  function appendLog(level, message) {
    var li = document.createElement("li");
    var time = new Date().toLocaleTimeString();
    li.innerHTML =
      '<span class="log-time"></span><span class="log-level-' + level + '">[' + level + "]</span> ";
    li.querySelector(".log-time").textContent = time;
    li.appendChild(document.createTextNode(message));
    logList.insertBefore(li, logList.firstChild);
  }

  // mock 日志条目
  appendLog("info", "App 启动（mock 数据，未接 Rust core）");
  appendLog("info", "检测到无障碍服务：未开启（mock）");
  appendLog("err", "连接 daemon 失败：ECONNREFUSED 192.168.1.10:18777（mock 示例）");
  appendLog("ok", "心跳 ok，rtt=12ms（mock 示例）");

  renderConnState();
})();
