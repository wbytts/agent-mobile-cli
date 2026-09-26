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

  // ---------- 能力自检页（真实插件调用） ----------
  var capResult = document.getElementById("cap-result");
  var a11yState = document.getElementById("a11y-state");
  var tauri = window.__TAURI__;
  var invoke = tauri && tauri.core ? tauri.core.invoke : null;

  function refreshA11yStatus() {
    if (!invoke) {
      a11yState.textContent = "不可用（非 Tauri 环境）";
      return;
    }
    invoke("bridge_a11y_status")
      .then(function (res) {
        var enabled = !!(res && res.enabled);
        a11yState.textContent = enabled ? "已开启" : "未开启";
        a11yState.className = "badge " + (enabled ? "badge-connected" : "badge-warn");
      })
      .catch(function (err) {
        a11yState.textContent = "检测失败";
        appendLog("err", "无障碍状态检测失败：" + err);
      });
  }

  // 逐项能力自测：返回 Promise<展示文本>
  var capActions = {
    tap: function () {
      // 点击自检页结果区空白处，避免手势误触列表按钮造成级联自测
      return invoke("bridge_tap", { x: 720, y: 1900 }).then(function () {
        return "已在 (720, 1900) 执行点击";
      });
    },
    swipe: function () {
      return invoke("bridge_swipe", { x1: 540, y1: 1600, x2: 540, y2: 600, durationMs: 300 })
        .then(function () {
          return "已执行 (540,1600)→(540,600) 300ms 滑动";
        });
    },
    input: function () {
      return invoke("bridge_input", { text: "agent-mobile 自检" }).then(function () {
        return "已向当前焦点输入框注入文本";
      });
    },
    key: function () {
      return invoke("bridge_key", { key: "notifications" }).then(function () {
        return "已执行全局动作 notifications（下拉通知栏）";
      });
    },
    uiTree: function () {
      return invoke("bridge_ui_tree").then(function (res) {
        var xml = (res && res.xml) || "";
        return "UI 树获取成功，XML " + xml.length + " 字符";
      });
    },
    screenshot: function () {
      return invoke("bridge_screenshot").then(function (res) {
        var b64 = (res && res.png_base64) || "";
        var kb = Math.round((b64.length * 3) / 4 / 1024);
        return "截图成功，PNG 约 " + kb + " KB";
      });
    }
  };

  document.querySelectorAll(".btn-cap").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var cap = btn.dataset.cap;
      if (!invoke) {
        capResult.textContent = "「" + cap + "」不可用：非 Tauri 环境";
        return;
      }
      capResult.textContent = "「" + cap + "」自测中…";
      capActions[cap]()
        .then(function (text) {
          capResult.textContent = "「" + cap + "」成功：" + text;
          appendLog("ok", "能力自测 " + cap + " 成功：" + text);
        })
        .catch(function (err) {
          capResult.textContent = "「" + cap + "」失败：" + err;
          appendLog("err", "能力自测 " + cap + " 失败：" + err);
        });
    });
  });

  document.getElementById("btn-a11y-settings").addEventListener("click", function () {
    if (!invoke) {
      capResult.textContent = "不可用：非 Tauri 环境";
      return;
    }
    invoke("bridge_open_a11y_settings")
      .then(function () {
        appendLog("info", "已跳转系统无障碍设置");
      })
      .catch(function (err) {
        capResult.textContent = "跳转失败：" + err;
        appendLog("err", "无障碍设置跳转失败：" + err);
      });
  });

  // 进入自检页 / 从设置页返回时刷新权限状态
  tabs.forEach(function (t) {
    t.addEventListener("click", function () {
      if (t.dataset.page === "page-caps") {
        refreshA11yStatus();
      }
    });
  });
  document.addEventListener("visibilitychange", function () {
    if (!document.hidden && document.getElementById("page-caps").classList.contains("active")) {
      refreshA11yStatus();
    }
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

  // mock 日志条目（连接/日志页接线在任务组 4）
  appendLog("info", "App 启动（连接状态为 mock，待任务组 4 接线）");
  appendLog("err", "连接 daemon 失败：ECONNREFUSED 192.168.1.10:18777（mock 示例）");
  appendLog("ok", "心跳 ok，rtt=12ms（mock 示例）");

  renderConnState();
  refreshA11yStatus();
})();
