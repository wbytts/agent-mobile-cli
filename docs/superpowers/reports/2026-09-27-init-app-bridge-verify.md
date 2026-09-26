# 验证报告：init-app-bridge

日期：2026-09-27 ｜ verify_mode: full ｜ review_mode: standard

## Summary

| Dimension | Status |
|---|---|
| Completeness | 17/17 tasks，10 需求全部实现（1 场景真机段待用户验收） |
| Correctness | 20 场景：19 实测通过，1（真机相机扫码）未验证 |
| Coherence | 15/15 design 决策遵循，无 spec 漂移 |

## 检查证据（Runtime 记录）

| 检查 | 命令 | 结果 |
|---|---|---|
| host 全量测试 | `cargo test --quiet`（164+93+93） | PASS（exit=0, tier=full） |
| 静态检查 | `cargo clippy --quiet --all-targets -- -D warnings` | PASS |
| 格式 | `cargo fmt --check` | PASS |
| App host 测试 | `cargo test --manifest-path app/src-tauri/Cargo.toml --lib --quiet`（36） | PASS |

OpenSpec 产物：`openspec validate init-app-bridge --strict` PASS；tasks 17/17。

## 场景验收记录（MuMu 127.0.0.1:5555，Android 12 arm64-v8a）

### agent-app
- 手动配置地址与配对码后连接成功：PASS（UI 输码 → online，两轮实测）
- 扫码快速配对：URI 解析路径 PASS（`agent-mobile://pair` intent 自动填入）；**真机相机扫码未验证**（需物理设备）
- 配对码错误提示：PASS（UI 显示「配对失败：配对码错误或已失效」）
- 权限引导：PASS（未授权时桥接快照返回「无障碍服务未开启」结构化错误；UI 状态同步）
- 开启权限后执行点击：PASS（设置页点击生效）
- 脚本调用设备 API：PASS（`mobile.tap` return 值回传；`while(1){}` 25s 超时中断；64MB 内存上限）
- 三页切换与状态展示 / 能力自检：PASS（截图逐项核对）
- 截图命令返回图片：PASS（桥接 screenshot 非空 PNG）

### app-backend
- 两类设备同时在线：PASS（adb 127.0.0.1:5555 + bridge:23116PN5BC 合并枚举）
- 桥接设备快照：PASS（refs 六字段结构与 ADB 后端一致）
- 桥接设备点击：PASS（坐标 tap / @eN 引用 tap 均生效）
- 不支持的命令：PASS（shell → NOT_SUPPORTED 结构化错误）

### bridge-protocol
- 注册后设备可见：PASS
- App 进程被杀后恢复：PASS（offline → 重启 → token 自动重连 online）
- 命令成功/失败回传：PASS
- 显示配对信息：PASS（配对码 + LAN IP + 终端二维码）
- 首次配对成功 / 凭 token 重连免配对：PASS
- 配对码错误被拒：PASS
- 重置配对：PASS（踢线 → 旧 token 被拒 → token 清除 → 输新码恢复 online 全流程闭环）
- 脚本返回执行结果：PASS

### 补充验证
- WS 绑定安全分层：18777 绑 0.0.0.0（LAN 可达，nc 实测 OPEN）、18775 保持 127.0.0.1（LAN CLOSED）
- MuMu 经 LAN IP 192.168.2.38:18777 直连 online（不经 adb reverse）
- tokens.json 0600 / 配置目录 0700

## 集成审查

- Build 阶段 BridgeReview（f33a7a8 前全量 diff）：3 IMPORTANT + 7 MINOR → 全部修复（7945e31）
- 修复轮补充审查 FixReview（7945e31 diff）：见下节结论

## CRITICAL

无。

## WARNING

1. **真机相机扫码未端到端验证**（agent-app「扫码快速配对」场景真机段）：URI 解析分支已实测，相机扫码链路（ScanActivity + ML Kit → URI）未在物理设备跑通。原因：真机 192.168.2.5:5555 adb 掉线，装 APK 需用户重开无线调试。影响范围：仅真机相机入口；手动输入配对码路径不受影响。建议：用户配合完成真机扫码验收，或接受偏差留待后续版本。

## SUGGESTION

（待 FixReview 结论补充）
