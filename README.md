# LADON Explorer V1.3 — IPA 全套工程、原生 Rust 内核与现代侘寂 UI

**发布日期：2026-11-18 · 设计/源码基线候选 · 非已验收成品。**

本包在 **V1.2** 的所有文件、14 张现代侘寂 SVG、Windows 传统 Shell COM 桥接、Turso 历史引擎、.NEMA、.LADONignore、Agent Folder API、IP​​A 风格文档上**增量升级**，不是另起炉灶。

## 最重要的四项架构改变

1. **Google Drive 先行、无常驻后端的个人历史**：以本地 Turso + CAS + Drive 加密不可变对象为源。个人应用私有 `appDataFolder` 模式与团队普通 Drive 目录 / Shared Drive 模式明确分离。通过规则把内容、历史、事件段、加密索引、checkpoint、密文聊天归档到 Drive；不把 Drive 当数据库。
2. **Vercel 最小控制面**：多租户不能依靠 Drive 自定义存储规则实现强隔离。`server/vercel` 在服务器验证 Google OIDC 和成员权限，存储成员、角色、设备、绑定及**短期**聊天密文投递队列；原文和长期历史不会交给 Vercel。完整的个人离线功能不依赖 Vercel。
3. **真自适应 Modern Wabi-Sabi UI**：侧栏从 236px 折到 60px，文件区自动占满释放宽度；Hover 显示浮层、不重新排版；底部只保留一个含状态信息的聊天 Dock，历史向上展开；普通状态提示闲置淡出、危险常显。
4. **涌现式治理**：多个独立事件信号经过要件锚点聚合，以低 Token、非强制方式给出证据充分的建议；从不未经批准直接修改文档、代码或测试。

## 文件导航

| 文件夹 | 内容 |
|---|---|
| `docs/01_requirements` | REQ-001～100 与业务需求、验收条件；V1.3 增量要件在 `15_*.md` |
| `docs/02_basic_design` | IPA 风格基本设计；新增 27～29 的 Drive/Vercel、UI、事件融合设计 |
| `docs/03_detailed_design` | 原有详细设计 + 44～49 的存储规则、同步冲突、零知识边界、Dock、Provider、涌现式设计 |
| `docs/04_tests` | 55 UT、30 IT、74 ST + UAT / 灾备 / 失效注入，统一 Excel 追踪工作簿 |
| `docs/05_operations` | 部署、Google/Vercel 配置、数据迁移/恢复、运营 |
| `src/` | 既有 Rust 核心 + `storage_rules.rs`、`cloud_journal.rs`、`ui_layout.rs`、`emergence.rs` |
| `native/`, `build.rs` | V1.2 Windows Shell 经典扩展兼容宿主源码；真实 Windows 11 尚未测试 |
| `sql/` | 001 历史核心、002 协作与成员、003 归档规则/同步/短期聊天队列 |
| `server/vercel/` | Next.js 16 + Turso Serverless 及 Google OIDC 控制面样例，未部署 |
| `api/`、`protocol/` | Agent API V1.3 目标 OpenAPI；Drive 规则与同步协议 |
| `ui-modern-svg/` | V1.2 原 14 张 SVG/PNG，保留 |
| `ui-v13-responsive/` | **6 张新版 SVG + PNG + 总览 + 可交互 HTML** |
| `ui-reference-png/` | 上轮美术参考图 |
| `scripts/` | 原 CI/验收脚本 + 新增 SVG 生成/静态 QA 脚本 |

## 核心运行模式

```text
个人（免常驻后端）
  原生 GUI ↔ 本地 Turso + CAS ↔ .LADONignore
                      │
                      └─→ 客户端 AES-GCM → Google Drive appDataFolder / 用户普通目录

团队（可信最小控制面）
  原生 GUI ↔ 本地 Turso + CAS ──→ 团队普通 Drive/Shared Drive 加密历史
       │                                    ↑
       └─→ Vercel Functions ↔ 独立 Turso Cloud（身份、成员、短期密文投递）
                │
                └─→ 真实 Google OIDC 验证及成员服务端授权
```

Google Drive 物理存储并非真正的 POSIX 目录树。`protocol/drive-layout-v1.yaml` 的路径为**逻辑键**；最终需由 Provider 映射到 Drive 文件 ID。团队共享数据还必须同时有 Drive ACL 与 LADON 角色权限，并解决端到端密钥分发。

## 快速查看新版原型

- 在浏览器直接打开 `ui-v13-responsive/interactive-preview.html`，实际点击侧栏折叠、Hover、底部聊天、模拟右键、账号/云盘菜单。
- 浏览 `ui-v13-responsive/00_V1.3_6场景总览.png`，打开对应可编辑 `.svg`。
- 原来的 `ui-modern-svg/index.html` 仍可查看 V1.2 的 14 场景。

> 预览页是**交互示意**，不是可以操作真实文件的 Slint 运行版本；Windows 右键原生菜单由 `native/shell_menu.cpp` 负责，网页中仅模拟 LADON 新命令。

## 本地内核测试（需要 Rust 1.85+）

```bash
cargo build --release
cargo test --all-targets
# 新模块专项
cargo test ut_rules
cargo test ut_journal
cargo test ut_ui
cargo test ut_emergence
```

CLI 主要功能见上一版 README/使用指南：`ladon --root <workspace> init`、`capture`、`history`、`verify`、`archive`、`restore`、`tag-folder`、`sensitive-add`、`shell-menu`。`LADON_GOOGLE_MODE=appdata` 在开发环境下使用 `drive.appdata` token 选择私有父文件夹，不能与 `LADON_GOOGLE_FOLDER_ID` 同时使用。

## Vercel 控制面（非生产部署）

```bash
cd server/vercel
cp .env.example .env.local
npm install
npm run typecheck
npm run build
```

OAuth Client ID、远程 Turso token 与真实身份验证需要部署者完成配置。默认示例只有 Google ID Token 的后端验证、成员过滤查询与临时密文聊天；它**不包含**生产级注册、自助租户管理、团队密钥分发、即时聊天推送、完整恶意行为限流及密钥吊销，不能直接开启真实多租户业务。

## 敏感信息

`.LADONignore` 禁止原件、Diff、快照、事件、标签与云端 AI 数据流外传。云存储并不是安全策略执行者；必须由本地 Resource Runtime + 服务端授权共同执行。新增敏感文件后需要额外清查已经上云的历史副本，单纯增加规则不能收回旧数据。

## 当前完成和未完成的事实

**已提供源码（尚未运行 Rust 构建）**：按作用域生成非明文对象逻辑键、敏感拒绝、加密事件段发布与读回校验、设备事件序号/同基线冲突初判、侧栏/聊天布局与状态渐隐的纯 Rust 规则、简单多信号建议器、Google Drive appDataFolder 对象上传开关、Vercel/Turso 认证权限示例。SQL、JSON/YAML、SVG、ZIP 可在当前环境静态校验。

**待实施及实机验收**：真正的 Slint 自适应界面；完整 Drive OAuth PKCE/索引/同步/自动续传/配额；NAS/S3 正式适配；客户端安全凭据存储；多用户密钥交换与设备撤销；完整分布式合并；真实 Serverless 部署/用户登录；MCP / 全量 Agent HTTP 服务；Windows 11 新式右键/第三方扩展/多选菜单支持。当前环境无 Rust 工具链，`cargo build/test` 未执行；无 Google/Vercel 正式凭证，不能标记为生产完成。

## 正式验收原则

若本地未确认版本能在云端被读回、解密且 Hash 一致，则**不得驱逐本地历史副本**。当网络、配额或敏感策略出现故障，系统应保留 WAL 并在 Dock 显示不可淡出的警告。全套要件、设计、实现与测试状态可通过 `docs/04_tests/要求追踪_测试管理.xlsx` 追查。
