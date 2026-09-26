//! 集成测试 shim：以 crate::daemon 路径引入 daemon 子树模块（与二进制目标的 crate:: 路径一致）。
//! tests/daemon/ 为真实目录，使 #[path] 的 ../../ 相对穿越可用（内联 mod 会因目录不存在失败）。

#[allow(dead_code)] // 测试目标只引用子集（executor/http 等不在本树内）
#[path = "../../src/daemon/registry.rs"]
pub mod registry;
