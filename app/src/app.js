// Agent Mobile Bridge 诊断 UI：连接/日志页经 Tauri 命令与事件接线（任务组 4）
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

  var tauri = window.__TAURI__;
  var invoke = tauri && tauri.core ? tauri.core.invoke : null;
  var tauriEvent = tauri && tauri.event ? tauri.event : null;

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

  // ---------- 连接页（真实连接状态机，经 bridge://state 事件推送） ----------
  var connBadge = document.getElementById("conn-badge");
  var connStateText = document.getElementById("conn-state-text");
  var deviceIdText = document.getElementById("device-id-text");
  var heartbeatText = document.getElementById("heartbeat-text");
  var btnConnect = document.getElementById("btn-connect");
  var btnScan = document.getElementById("btn-scan");
  var addrInput = document.getElementById("daemon-addr");
  var codeInput = document.getElementById("pair-code");

  var currentState = { state: "disconnected" };

  function renderConnState() {
    var s = currentState;
    var connected = s.state === "connected";
    var busy = connected || s.state === "connecting";
    if (connected) {
      connBadge.textContent = "已连接";
      connBadge.className = "badge badge-connected";
      connStateText.textContent = "已连接";
      deviceIdText.textContent = s.device_id || "—";
    } else if (s.state === "connecting") {
      connBadge.textContent = "连接中";
      connBadge.className = "badge badge-warn";
      connStateText.textContent = "连接中…";
      deviceIdText.textContent = "—";
    } else if (s.state === "pairing") {
      connBadge.textContent = "配对失败";
      connBadge.className = "badge badge-warn";
      connStateText.textContent = "配对失败：" + (s.reason || "未知原因");
      deviceIdText.textContent = "—";
    } else {
      connBadge.textContent = "未连接";
      connBadge.className = "badge badge-disconnected";
      connStateText.textContent = "未连接";
      deviceIdText.textContent = "—";
      heartbeatText.textContent = "—";
    }
    btnConnect.textContent = busy ? "断开" : "连接";
  }

  // 「host:port」或裸 host（默认端口 18777）
  function parseAddr() {
    var text = addrInput.value.trim();
    if (!text) {
      return null;
    }
    var idx = text.lastIndexOf(":");
    if (idx > 0) {
      var port = parseInt(text.slice(idx + 1), 10);
      if (port > 0 && port <= 65535) {
        return { host: text.slice(0, idx), port: port };
      }
    }
    return { host: text, port: 18777 };
  }

  function startConnect() {
    if (!invoke) {
      appendLog("err", "非 Tauri 环境，无法连接");
      return;
    }
    var addr = parseAddr();
    if (!addr) {
      appendLog("err", "未填写 daemon 地址");
      addrInput.focus();
      return;
    }
    var code = codeInput.value.trim();
    invoke("bridge_connect", {
      host: addr.host,
      port: addr.port,
      pairingCode: code ? code : null,
    }).catch(function (err) {
      appendLog("err", "发起连接失败：" + err);
    });
  }

  btnConnect.addEventListener("click", function () {
    if (currentState.state === "connected" || currentState.state === "connecting") {
      invoke("bridge_disconnect").catch(function (err) {
        appendLog("err", "断开失败：" + err);
      });
      return;
    }
    startConnect();
  });

  // 扫码配对：相机扫码 → 解析 agent-mobile://pair URI → 自动填入并连接
  btnScan.addEventListener("click", function () {
    if (!invoke) {
      appendLog("err", "非 Tauri 环境，无法扫码");
      return;
    }
    btnScan.disabled = true;
    btnScan.textContent = "扫码中…";
    invoke("bridge_scan_pair_qr")
      .then(function (text) {
        return invoke("bridge_parse_pair_uri", { uri: text });
      })
      .then(function (info) {
        addrInput.value = info.host + ":" + info.port;
        codeInput.value = info.code;
        appendLog("ok", "扫码成功：" + info.host + ":" + info.port);
        startConnect();
      })
      .catch(function (err) {
        appendLog("err", "扫码配对失败：" + err);
      })
      .finally(function () {
        btnScan.disabled = false;
        btnScan.textContent = "扫码配对";
      });
  });

  // 连接状态 / 日志 / 心跳事件订阅
  if (tauriEvent) {
    tauriEvent.listen("bridge://state", function (e) {
      currentState = e.payload;
      renderConnState();
    });
    tauriEvent.listen("bridge://log", function (e) {
      appendLog(e.payload.level, e.payload.message);
    });
    tauriEvent.listen("bridge://heartbeat", function () {
      heartbeatText.textContent = new Date().toLocaleTimeString();
    });
  }

  // ---------- 能力自检页（真实插件调用） ----------
  var capResult = document.getElementById("cap-result");
  var a11yState = document.getElementById("a11y-state");

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

  // ---------- 初始化 ----------
  appendLog("info", "App 启动");
  renderConnState();
  refreshA11yStatus();
  if (invoke) {
    invoke("bridge_state")
      .then(function (s) {
        currentState = s;
        renderConnState();
      })
      .catch(function () {});
    // 回填上次成功连接的 daemon 地址（持久化于 SharedPreferences）
    invoke("bridge_last_address")
      .then(function (addr) {
        if (addr && addr.host) {
          addrInput.value = addr.host + ":" + addr.port;
        }
      })
      .catch(function () {});
    // 冷启动自动连接：有保存地址即自动发起（有 token 免配对）
    invoke("bridge_auto_connect")
      .then(function (triggered) {
        if (triggered) {
          appendLog("info", "检测到上次连接地址，自动连接中");
        }
      })
      .catch(function (err) {
        appendLog("err", "自动连接失败：" + err);
      });
  }
})();
