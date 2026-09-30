# Rulings

## 代理设备采用实时查询（替代 uplink 轮询注册表）

- status: active
- workflow: classic
- date: 2026-09-29
- tasks: 72e7a208-7fc1-4201-8542-de631748c1e9 (4.2)
- decision: daemon 内 ProxyBackend 不做 5s 轮询缓存注册表，改为每次 devices/命令时实时 `GET /devices` 查询代理服务。
- reason: 轮询缓存引入过期状态与离线标记同步复杂度；实时查询数据新鲜、代码量少约一半，且直接满足 spec「代理服务不可达或凭证无效时返回明确结构化错误」（轮询模式下枚举只在缓存过期时间接体现）。spec 验收场景（合并枚举、命令路由、认证失败显式报错、歧义参与）均不受实现方式影响。
- impact: design.md 决策 2/11 的「轮询刷新注册表」描述以本 ruling 为准；`devices` 命令在代理不可达时降级并附 `proxy_error` 字段；目标显式指向 `proxy:` 前缀时传播原始错误。

## 代理配对由 CLI 直连代理服务（替代经 daemon 中转）

- status: active
- workflow: classic
- date: 2026-09-29
- tasks: 5416fda1-424e-4667-b74c-1afdd8999786 (4.3)
- decision: `agent-mobile-cli pair --proxy` 由 CLI 短进程直接用 ureq 调代理服务 `POST /pairing-codes|pairing-reset`，不经 daemon HTTP 中转。
- reason: CLI 进程本身持有 config.json 中的 proxy.url/token，直连省去 daemon 侧一对转发端点（/proxy-pair-info 等）与对应测试；行为与输出契约不变。
- impact: design.md 决策 5 的「经 daemon 调用代理」以本 ruling 为准。
