# LADON Explorer Change Log

## 0.0.1 — 2026-11-20

- 内核（Rust crate `ladon-explorer` 0.0.1）：本地扫描与捕获、Turso 版本账本与哈希链、内容寻址存储（CAS）、`.LADONignore` 策略、Agent 文件夹 API 与标签；命令行程序 `ladon.exe`。
- 图形界面 `ladon-gui.exe`（原生 Slint，不经安装向导安装）：打开与初始化工作区、浏览文件夹、记录文件、版本历史、版本差异比较（仅文本文件）；文件夹列表约每 2 秒轮询刷新一次，仅在浏览页显示时进行；Ctrl+B 折叠侧栏；详情面板默认折叠，选中文件时自动展开。
- Windows 安装向导与试用 zip（`packaging/windows`，入口 `Setup.cmd`）：向导只安装命令行程序 `ladon.exe`；zip 随附图形程序 `ladon-gui.exe`（构建时指定 `-GuiBinary`），解压后可直接运行。
- 测试结果：根 crate 的内核测试（`cargo test --release`，执行记录日期 2026-11-20，单次运行日期未记录）104 项通过、0 项失败；其中 `tc404` 在无法创建符号链接与目录联接且未设置 `LADON_REQUIRE_LINK_TESTS` 时不作断言。图形原型（独立 crate `ui-prototype`，执行日期未记录）模型单元测试 18 项、集成测试 26 项通过。
- 未执行：手工系统测试（ST，30 项）与用户验收测试（UAT，5 项）；单元测试 UT-024～029，集成测试 IT-007～014 与 IT-021～030；依赖真实 NAS、Google Drive、真实账户或真实 Windows shell 会话的用例。
- 未包含：恢复（restore）的图形入口及内核恢复安全加固；图形界面未接入 NAS 与 Google Drive 归档（命令行 `archive` 已有 nas、google_drive 两类 provider 代码，真实环境未验证）；回收站删除；新建、重命名、复制、移动、排序等文件夹操作。
- 质量门禁（覆盖率严格高于 90%）尚未运行，本版本为试用构建。

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
