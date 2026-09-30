# AI 安装说明

把下面这段话发给 AI，让 AI 在你的本机环境里完成安装、配置 skill 和验证。

```text
请帮我安装 agent-mobile-cli：https://github.com/wbytts/agent-mobile-cli

要求：
1. 优先使用 npm 安装：npm install -g agent-mobile-cli。
2. skill 从 npm 包内置的 skills/agent-mobile-cli 复制到 ~/.agents/skills/agent-mobile-cli，并在我使用的 Agent 平台目录（~/.claude/skills、~/.codex/skills、~/.config/agents/skills 等，按实际存在的平台）创建指向主安装目录的软链接；复制前先把安装计划展示给我确认。
3. 验证 ADB 直连模式：本机需要有 Android platform-tools（adb）；执行 agent-mobile-cli devices、connect <我的设备>、snapshot 验证可用。
4. 如果我需要使用调试 App 代理模式（无 adb 环境/免 root 脚本沙盒）：引导我从 GitHub Releases 下载 Agent Mobile Bridge APK 装到手机，开启无障碍服务，然后执行 agent-mobile-cli pair 显示二维码，让我用 App 扫码完成配对。
5. 如果 npm 包暂未包含当前平台二进制，回退源码构建：cargo build --release。
```

## 1. 安装 CLI

优先使用 npm 全局安装：

```bash
npm install -g agent-mobile-cli
agent-mobile-cli --help
```

npm 包的 CLI 二进制在安装时从 GitHub Releases 下载（逐源 sha256 校验）。GitHub 访问慢时脚本会自动尝试内置加速代理（ghfast.top / gh-proxy.com / gh.llkk.cc）；也可显式指定代理前缀：

```bash
AGENT_MOBILE_CLI_GH_PROXY=https://ghfast.top npm install -g agent-mobile-cli
```

未覆盖的平台会给出警告并提示源码构建，安装本身不会失败。

源码构建回退：

```bash
git clone https://github.com/wbytts/agent-mobile-cli.git
cd agent-mobile-cli
cargo build --release
./target/release/agent-mobile-cli --help
```

前置条件（ADB 直连模式）：主机可获取 `adb`。探测顺序：配置 `adb_path` → `ANDROID_HOME`/`ANDROID_SDK_ROOT` → PATH → 常见安装路径。首个设备命令会自动拉起常驻 daemon（HTTP `127.0.0.1:18775`，桥接 WS `18777`）。

## 2. 安装 skill

npm 全局安装后，skill 来源位于 npm 包内置目录：

```text
<npm 全局根>/node_modules/agent-mobile-cli/skills/agent-mobile-cli
```

源码构建时，来源为仓库内同名目录 `skills/agent-mobile-cli`。

默认实体安装目录：

```text
~/.agents/skills/agent-mobile-cli
```

各 Agent 平台目录只创建指向主安装目录的软链接，不复制多份实体文件：

```text
~/.claude/skills/agent-mobile-cli        -> ~/.agents/skills/agent-mobile-cli
~/.codex/skills/agent-mobile-cli         -> ~/.agents/skills/agent-mobile-cli
~/.config/agents/skills/agent-mobile-cli -> ~/.agents/skills/agent-mobile-cli
~/.cursor/skills/agent-mobile-cli        -> ~/.agents/skills/agent-mobile-cli
```

安装命令（先把计划展示给用户确认，再执行）：

```bash
SKILL_SRC="$(npm root -g)/agent-mobile-cli/skills/agent-mobile-cli"   # 源码安装则用仓库内路径
mkdir -p ~/.agents/skills
cp -R "$SKILL_SRC" ~/.agents/skills/agent-mobile-cli
for d in ~/.claude/skills ~/.codex/skills ~/.config/agents/skills ~/.cursor/skills; do
  [ -d "$d" ] && ln -sfn ~/.agents/skills/agent-mobile-cli "$d/agent-mobile-cli"
done
```

已存在的非软链接实体路径不要覆盖，跳过时提示用户手动处理。

## 3. 验证 ADB 直连模式

```bash
agent-mobile-cli devices                        # 枚举设备
agent-mobile-cli connect 127.0.0.1:5555         # 网络设备/模拟器按需连接
agent-mobile-cli snapshot --device <serial>     # UI 快照（@eN 元素引用）
agent-mobile-cli screenshot --device <serial>   # 截图保存 PNG
```

成功时 `devices` 返回 `ok: true` 与设备列表；`snapshot` 返回简化 UI 树与可交互元素表。多设备在线时控制命令必须带 `--device <serial>`。

## 4.（可选）调试 App 代理模式

适用场景：设备不便开 adb、需要设备端脚本沙盒（QuickJS + `mobile.*` API）、免 root 的 UI 自动化。App 名 **Agent Mobile Bridge**。

### 4.1 安装 APK

从 GitHub Releases 下载最新 APK 安装到手机（downloads 页或让 AI 代下）：

```text
https://github.com/wbytts/agent-mobile-cli/releases
```

也可 `adb install -r agent-mobile-bridge.apk`。源码构建见仓库 `app/BUILD.md`（Tauri 2 + Android NDK 工具链）。

### 4.2 开启无障碍服务

App 的设备操作能力（UI 树/点击/滑动/截图）依赖无障碍服务：

- 手动：系统设置 → 无障碍 → 已安装的服务 → Agent Mobile Bridge → 开启
- 有 adb 时可直接：

```bash
adb shell settings put secure enabled_accessibility_services com.agentmobile.bridge/com.agentmobile.bridge.BridgeAccessibilityService
adb shell settings put secure accessibility_enabled 1
```

### 4.3 配对连接

手机与电脑处于同一局域网。电脑端执行：

```bash
agent-mobile-cli pair    # 显示 6 位配对码 + 候选局域网 IP + 终端配对二维码
```

手机端任选其一：

- **扫码**：App 连接页点「扫码配对」，对准终端二维码（URI 形如 `agent-mobile://pair?host=<ip>&port=18777&code=<code>`）
- **手动**：App 连接页输入电脑局域网 IP:18777 与 6 位配对码，点「连接」

配对码一次性（首次使用或 daemon 重启后失效）；配对成功后 App 凭长期 token 自动重连免配对。`agent-mobile-cli pair --reset` 可重置全部配对。

注意：App 的地址/配对码输入框会记住上次内容，重新输入前先清空。

### 4.4 验证桥接模式

```bash
agent-mobile-cli devices                          # 应出现 bridge:<设备名> 条目
agent-mobile-cli snapshot --device bridge:<名字>
echo 'return mobile.uiTree().xml.length' | agent-mobile-cli script - --device bridge:<名字>
```

桥接设备不支持的能力（shell/logcat/stop）会返回结构化 `NOT_SUPPORTED` 错误。

### 4.5（可选）公网代理远程调试

手机与 CLI 主机不在同一局域网时（蜂窝网络、云端 Agent），部署仓库根目录的 `mobile-debug-proxy-server`：

1. 公网主机构建并启动：`cargo build --release --manifest-path mobile-debug-proxy-server/Cargo.toml`，运行 `--bind 0.0.0.0:28777`（记录首启打印的 owner token）；生产环境前置反向代理终结 TLS。
2. CLI 主机 `~/.agent-mobile-cli/config.json` 增加 `"proxy": { "url": "http://<proxy-host>:28777", "token": "<owner-token>" }`。
3. `agent-mobile-cli pair --proxy` 输出代理配对码与二维码；手机 App 扫码连接，或手动输入 `wss://<域名>`（经反代）/`ws://<host:port>`。
4. 验证：`agent-mobile-cli devices` 出现 `proxy:<设备名>`；`agent-mobile-cli snapshot --device proxy:<名字>` 正常返回。

代理设备命令语义与本地桥接一致；不支持的 shell/logcat/stop 同样返回 `NOT_SUPPORTED`。

## 5. 使用入口

完整命令 SOP 见安装后的：

```text
~/.agents/skills/agent-mobile-cli/SKILL.md
```

源码仓库对应文件：`skills/agent-mobile-cli/SKILL.md`。桥接协议细节见仓库 `docs/bridge-protocol.md`。

## 6. 卸载

```bash
npm uninstall -g agent-mobile-cli
rm -rf ~/.agents/skills/agent-mobile-cli
# 各平台目录下的软链接一并删除；手机端卸载 Agent Mobile Bridge 并关闭其无障碍服务
agent-mobile-cli daemon-stop 2>/dev/null  # 卸载前停止常驻 daemon
```
