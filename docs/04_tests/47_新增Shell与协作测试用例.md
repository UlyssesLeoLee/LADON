# 47. V1.2 增补 UT／IT／ST 测试详细设计

> Pass 的判定：测试函数的完整名称出现在 2026-11-24 全量运行的通过记录中，或在 `46_测试执行记录与缺陷.md` 中记为较早运行通过且名称在其中给出时为 Pass。不存在的测试函数不为 Pass；仅有文件级计数的测试为未执行；没有测试函数的用例不为 Pass。其余用例为未执行（包括仅有设计、手工检查，以及依赖真实环境的用例）。**Windows Shell 实机测试与 ST 用例未在当前环境执行。**

## 测试先决条件

- UT：Rust stable、Cargo；本文件中有测试函数的 UT 与 IT 用例，执行结果见 `46_测试执行记录与缺陷.md` 第 1.1 节；未纳入运行记录的测试函数对应的用例为未执行（如 `src/policy.rs` 中仅在 unix 下编译的 ut_policy_symlinked_root_rule_file_is_applied）；静态权限矩阵不等于 OIDC 服务端认证。
- IT：测试目录使用独立临时目录；不得使用真实用户数据；Windows Shell 用隔离测试账户。
- 安装包与 Shell：`Install-LADON.ps1` 与 `Uninstall-LADON.ps1` 只写入当前用户位置（用户 PATH、开始菜单、HKCU 卸载登记、示例工作区）；`packaging/windows/` 与 MSI 均不注册 Windows Shell 右键扩展。右键菜单由 `ladon shell-menu` 调用（`native/shell_menu.cpp` 经 `build.rs` 编译）。`LADON-Shell.ps1` 是开始菜单「LADON 命令行」快捷方式（`LADON 命令行.lnk`）的启动脚本，不属于 Shell 扩展，本文件无对应用例；安装与卸载流程的验证未执行。
- ST：Windows 11 最新稳定版、TortoiseGit、7-Zip、WinRAR（可选）、多个租户的测试账户、Google Drive 测试文件夹、NAS 假/真环境。

## 测试条目与预期结果

| 分类 | Test ID | 测试名称 | 驱动/前置 | 测试函数或场景 | 判定标准 | 实现状态/要件 |
|---|---|---|---|---|---|---|
| UT | UT-029 | Full NEMA Grammar | src/nema.rs | (future) | 全 139 例按最终编解码规范执行 | Not Run（未执行） |
| UT | UT-030 | 右键敏感规则写入 | src/policy.rs | ut_policy_04_add_sensitive | 在临时目录写入 `secret project.md`，对该相对文件名调用 add_sensitive，重新加载 LadonPolicy 后，`excluded` 对相对路径 `secret project.md` 返回 true | Pass |
| UT | UT-031 | Shell 响应状态区分 | src/shell.rs | ut_shell_result_distinct | 敏感与历史命令 ID 不冲突 | Pass |
| UT | UT-032 | Agent API 能力清单 | src/agent_api.rs | ut_agent_capability_status | 未实现接口明确设计状态 | Pass |
| UT | UT-033 | 跨租户访问拒绝 | src/auth.rs | ut_auth_cross_tenant | 不同租户无法读取工作区资源 | Pass |
| UT | UT-034 | 角色权限矩阵 | src/auth.rs | ut_auth_reviewer_no_write | Reviewer 拒绝写入 | Pass |
| UT | UT-035 | Viewer 敏感文件拒绝 | src/auth.rs | ut_auth_viewer_sensitive | Viewer 无权读取敏感文件 | Pass |
| UT | UT-036 | Agent 作用域 | src/auth.rs | ut_auth_agent_scoped | 作用域外写入拒绝 | Pass |
| UT | UT-037 | Shell COM 转发 | native/shell_menu.cpp | (future) | WM_MENUCHAR / DRAWITEM / QueryContextMenu 组合正确 | Not Run（未执行） |
| UT | UT-038 | 忽略规则并发写 | src/policy.rs | (future) | 并发更新不丢规则，保留注释和格式 | Not Run（未执行） |
| UT | UT-039 | Turso 迁移可重入 | tests/it_store.rs | tc112_init_is_idempotent | 迁移 002 只应用一次：首次 init 后删除 002 的表 `agent_grants`，再次 init 不重建，已有数据仍可读（以集成测试实现，`cargo test --test it_store` 运行） | Pass |
| UT | UT-040 | Auth 真实 Token 校验 | future | (future) | 令牌伪造、过期、改租户均拒绝 | Not Run（未执行） |
| IT | IT-014 | Turso cloud DLP | future | 配置云同步同时含敏感事件 | 远端 DB 不出现敏感数据 | Not Run（未执行） |
| IT | IT-015 | Agent 本地文件夹到历史 | tests/it_agent_folder.rs | it_agent_folder_snapshot_history | 目录列表含 requirements.md；历史的 FileRef 与捕获结果的文件标识一致，版本 1 条，链有效 | Pass |
| IT | IT-016 | Shell 扩展宿主隔离 | scripts/st_windows_shell_matrix.ps1 | 恶意扩展/子进程退出注入 | 主 GUI 不崩溃，回退 Explorer | Not Run（未执行） |
| IT | IT-017 | 端到端历史外传与审计 | future | Google Drive 完整版本读取、Diff、恢复 | 每版本对象可校验，敏感不上传 | Not Run（未执行） |
| IT | IT-018 | 跨租户访问与消息 | future | 跨租户会话、workspace、Provider 探测 | 所有 HTTP、聊天均拒绝越权 | Not Run（未执行） |
| IT | IT-019 | 聊天引用文档任务 | future | 消息引用 REQ 且触发 Agent 分析 | 任务、来源、审批、提交事件可追溯 | Not Run（未执行） |
| IT | IT-020 | Agent 并发 CAS | future | 双 Agent 同文件准备提交 | 一成功一冲突，审计完整 | Not Run（未执行） |
| ST | ST-051 | Windows Shell 单文件扩展 | TortoiseGit+7-Zip 已安装 | 右键 Git 仓库单文件 | 原生 Git/7-Zip 命令可调用或有 Explorer 回退 | Not Run（未执行）（REQ-051） |
| ST | ST-052 | Windows 11 现代扩展 | Windows11 注册现代命令 | 右键文件执行现代扩展 | 可直接使用或有真实 Explorer 兼容入口 | Not Run（未执行）（REQ-052） |
| ST | ST-053 | 文件夹/空白区/多选菜单 | 不同路径与多选文件 | 调用系统右键菜单 | 一一对照 Explorer 矩阵、失败项有回退 | Not Run（未执行）（REQ-053） |
| ST | ST-054 | 底部聊天无历史 | 窗口任意大小 | 展开/隐藏聊天记录 | 输入始终最底部且不覆盖主要浏览 | Not Run（未执行）（REQ-055） |
| ST | ST-055 | AI／团队通信隔离 | 创建 AI 和团队会话 | 切换、提及、引用文件 | 未经授权的成员消息不作为 AI 上下文 | Not Run（未执行）（REQ-056） |
| ST | ST-056 | 账号与租户 | OIDC 实验身份/两个租户 | 登录、退出、切换、读取 | 权限由服务端校验 | Not Run（未执行）（REQ-057） |
| ST | ST-057 | 跨设备离线同步 | 设备 A、B、短期断网 | 更新聊天和文件历史后重连 | 不丢版本、无重复、冲突可见 | Not Run（未执行）（REQ-059） |
| ST | ST-058 | Agent 写入计划审批 | Agent 准备修改文档 | 无审批/旧 hash 提交 | 均被拒绝且原文件无变化 | Not Run（未执行）（REQ-061） |
| ST | ST-059 | 事件订阅游标 | 多 Agent 订阅与断联 | 订阅、ACK、过期恢复 | 不静默丢失已确认事件 | Not Run（未执行）（REQ-062） |
| ST | ST-060 | 悬停折叠键盘 | Windows 缩放 100~150% | hover、键盘 Tab/Shift-F10、Esc | 无功能仅靠 hover，可通过键盘操作 | Not Run（未执行）（REQ-063） |
| ST | ST-061 | Modern Wabi-Sabi UI | 12+场景 | 色彩、层次、响应式复核 | Explorer 风格简洁与无障碍验收通过 | Not Run（未执行）（REQ-064） |
| ST | ST-062 | 历史权限与敏感路径 | 敏感文件/已撤权成员 | 查版本时间线/附件/分享 URL | 跨端不得泄露内容和元数据 | Not Run（未执行）（REQ-066） |
| ST | ST-063 | 云盘租户绑定 | 两个租户各自 Google Drive | 切换租户读取备份 | 不允许跨租户恢复和操作 | Not Run（未执行）（REQ-068） |
| ST | ST-064 | 聊天消息到文档任务 | 带条款引用的成员消息 | 创建任务/Agent 处理/审批 | 消息与版本链双向可追踪 | Not Run（未执行）（REQ-069） |
| ST | ST-065 | Explorer 标准命令 | 已安装第三方 Shell 扩展 | 文件/目录/多选右键、快捷键 | 兼容清单所有用例有通过或系统回退 | Not Run（未执行）（REQ-071） |
| ST | ST-066 | 文档即任务侧栏 | REQ 条款含批注与 UT/ST | 切换条款和收起面板 | 关联准确、可键盘访问且不遮挡底栏 | Not Run（未执行）（REQ-072） |
| ST | ST-075 | 同内容副本移动后的外传拒绝 | 文件曾位于排除路径（`restricted/a.md`），之后移动为同内容副本（`plain/a.md`） | 对 `plain/a.md` 的最新版本执行外部归档 | 归档被拒绝；外部存储不出现该内容的版本与历史 | Not Run（未执行）（REQ-050） |

## 详细执行顺序

1. 先运行 `cargo fmt --all -- --check` 与 `cargo test --all-targets`；失败需登记构建缺陷，不得进入 ST。
2. 运行 SQL 001→002 迁移检验，查看每个租户表外键、租户索引、加密消息字段。
3. 在 Windows 实机按 `scripts/st_windows_shell_matrix.ps1` 的 Matrix 逐项填写原生显示、调用成功、回退成功与异常退出。
4. 在 Slint 完成 GUI 后用视觉原型执行底部聊天、折叠、右键、历史、文档任务和租户 UI 人工测试。
5. 在真实身份服务/云盘完成联调后验证泄漏、撤权、同步冲突及灾备。

## 风险与缺陷登记

每失败项记录环境（Win 版本/CPU/文件系统/扩展版本）、trace/event ID、步骤、复现率、严重度、责任模块、修复提交、复测结果。禁止将未运行的用例标为通过。
