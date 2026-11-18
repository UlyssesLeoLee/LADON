# LADON Explorer Change Log

## V1.3 — 2026-11-18

- 保留 V1.2 的 72 功能要件、28 非功能要件、Windows 原生 Shell 菜单桥、Agent API、文档任务和 14 SVG。
- 新增 REQ-073～100，NFR-029～043：Drive 个人/团队存储规则、Vercel 最小控制面、跨终端历史和授权、UI 侧栏真回流、底部状态融合、涌现式建议。
- 新增 `src/storage_rules.rs`、`src/cloud_journal.rs`、`src/ui_layout.rs`、`src/emergence.rs` 及 Rust 单测；已有 Drive Provider 增加 appDataFolder 选择。
- Vercel/Turso 示例新增 Google OIDC 验签、成员空间查询及 7 天密文聊天中继（长期聊天归档仍在用户 Drive/NAS）。
- 增加 SQL 003、Drive 对象协议、V1.3 Agent OpenAPI 新目标接口。
- 新增 6 SVG + PNG、场景总览和自适应/聊天/右键/渐隐的交互 HTML。
- 测试规格扩充至 UT 55 / IT 30 / ST 74；全部执行状态保留 `未执行`。
- 明确未实现和安全边界；本版本为工程设计与源码基线候选。

## V1.2

V1.2 的实现与文档变更参见 `docs/00_guide/02_V1.2变更影响与实施等级.md`。
