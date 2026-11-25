> 设计基线候选 · 2026-11-24 · IPA 共通フレーム参照 · 尚待设计评审/签字。


# 58. 文档任务与 Hermes CLI 开发 WBS

## 1. 目标与范围

### 1.1 目标
交付“文档任务版本”：文档任务（文档即任务）作为核心功能完整可用，并新增交互控制台 `ladon console`，所有者在控制台内启动 Hermes，由 Hermes 通过 LADON 自带的 MCP 服务器 `ladon mcp-serve` 操作文档任务。设计依据为 `57_文档任务模块与HermesCLI详细设计.md`（以下称“57 号文档”）；本文给出工作包、顺序、验证方式、风险与发布标准。

### 1.2 范围
| 范围内 | 范围外（在文档任务版本发布后另行设计与接线） |
|---|---|
| 基线、锚点、批注、任务、状态机、审计链、依赖与并行调度、方案与受控写入、报告与 .NEMA 导出、文件移动谱系 | 左侧导航快速浏览、底部 AI 聊天栏、GUI 中的 Google Drive 链接 |
| 一次性 CLI、MCP 服务器、交互控制台、Hermes 受控启动与自检、`ladon shell-menu` 菜单中的“文档即任务”入口接到控制台 | GUI 文档任务页接线（保持示例内容及提示条）、聊天转任务、涌现式建议落地、Concept Guard、FileSpace 租约与多 Agent 运行时 |
| 文件图标与可执行文件图标（WP20，所有者在 2026-11-24 的决定：随文档任务版本发布） | 多文件原子写入、docx/pdf/xlsx 内容改写、角色体系、跨设备同步 |
除 WP20 外，本 WBS 不修改 `ui-prototype/` 下任何文件。

### 1.3 编号与文档
本文档号 58，设计文档为 57；本交付不新增其他编号的文档。需求条目以 `design/requirements.json` 为唯一来源（与 REQ-073～100 的做法一致），57 号文档第 15.3 节给出目录，业务规则写入 `11_文档治理业务要件.md` 的新增章节，UT/IT/ST 用例行写入 `41`、`42`、`43` 号文档（WP00）。Hermes 启动与运维说明写入用户手册 `53_用户操作说明.md`（WP19）。新编号：REQ-101～115、NFR-044～047、UT-141～162、IT-047～074、ST-076～089、TC-820～1029（分段见 57 号文档第 15.6 节）、RSK-21～40；UAT 复用 UAT-01 与 UAT-05。

## 2. 里程碑与顺序
| 里程碑 | 包含工作包 | 完成后可交付的能力 |
|---|---|---|
| M0 前置 | WP00、WP00A、WP00B、WP00C | 需求目录与设计基线、`verify` 流程与覆盖率基线、Hermes 集成探针结论与金标、命令入口库化（`main.rs` 缩减为一次调用） |
| M1 内核与一次性 CLI | WP01～WP11 | 文档任务从基线、批注、任务、状态机、依赖、调度、对账到只分析方案的提交与批准、报告与 .NEMA 导出全部可经 `ladon doc` 使用，不需要 Hermes；不含编辑方案的批准链路与应用（WP14） |
| M2 MCP 与受控写入 | WP12～WP14 | `ladon mcp-serve` 的 13 个工具（WP13），编辑方案、`doc_plan_submit`（第 14 个工具）、人工批准与应用、恢复、回滚链（WP14） |
| M3 Hermes 与控制台 | WP15～WP18 | 交互控制台、Hermes 受控启动与自检、收件箱批准、菜单入口（`ladon shell-menu`） |
| M4 发布准备 | WP19、WP20 | 端到端验收、覆盖率结论、文档终态、图标、发布前检查 |
关键路径：WP00 → WP00A → WP01 → WP05 → WP06 → WP07 → WP08 → WP11 → WP13 → WP14 → WP16 → WP17 → WP18 → WP19（合计 149 小时）。WP00B 在 WP00 之后与 WP01～WP08 并行，须先于 WP12 与 WP16；WP00C 在 WP00A 之后进行，须先于 WP11。除 WP00B 外，所有产生或修改生产源文件的包依赖 WP00A（覆盖率度量与 `verify` 流程在其中确立）。可并行的只有修改不同源文件的包：WP01（`store.rs`）、WP04（`engine.rs`、`policy.rs`）、WP20（`build.rs`、GUI 源文件）彼此之间、以及它们与 `lib.rs` 链上的包之间可并行；`lib.rs` 链（第 4 节）上的包严格串行，因此 WP02 与 WP03、WP09 与 WP10、WP12 与 WP15 不并行。

## 3. WBS

### 3.1 每个工作包的统一步骤
1. 设计提交：先更新 `.quality/docs/<源文件>.md`（技术设计）与 `.quality/test-design/<源文件>.test.md`（真实测试用例、输入、期望、异常与边界、回归、位置），运行 `python <用户目录>/.uqh-global/global_dispatcher.py prepare <源文件>` 封印，仅含这些文档与 `.quality/plans/<源文件>.plan.json` 的设计提交，确认 `ready <源文件>` 之后才能改生产代码。源路径一律使用正斜杠；计划文件与设计文档使用 LF 换行（`core.autocrlf=true` 下需把计划文件转为 LF，否则 `ready` 报告计划未提交）。
2. 实现与测试：先写失败的测试，再写代码。
3. 修订：把设计文档与测试设计修订到实现后的终态，更新 `Source-SHA256`，只保留终态描述。
4. 验证：按第 5 节的通道运行；每个变更源文件的行覆盖率必须严格高于 90%，度量与 `verify` 流程由 WP00A 确立，在其落地之前任何包都不声称覆盖率达标，也不留下未提交的生产源文件改动（Stop 钩子会运行 `verify`）。
5. 提交：提交到工作分支，不向 `develop`、`master` 提交或合并；推送、打标签、发布一律先征得所有者同意。
6. 编辑既有源文件的工作包（`store.rs`、`engine.rs`、`policy.rs`、`main.rs`、`lib.rs`）先刷新其陈旧的 `.quality` 文档（`main.rs.md` 中应为 `grep` 命令而不是 `search`，并补全 `annotate`、`task`、`parallel` 等命令；`engine.rs.md` 的哈希已与源码不符）。7. 零新增依赖，适用于包括 WP20 在内的所有工作包：`Cargo.toml`、`Cargo.lock`、`ui-prototype/Cargo.lock` 不得出现改动（以 `git diff --stat` 为检查）。根 crate 的任何新增依赖（含 build-dependency）都会改变 `ui-prototype/Cargo.lock`，因为 GUI crate 以路径依赖根 crate，而 `release.yml` 以 `--locked` 构建两者。包内发现零依赖不可行时停止并上报，由所有者决定，不在包内自行引入。
8. 每个包的测试设计含“验收条目—TC 编号—输入—断言”对照表（57 号文档第 15.1 节第 5 点），在该包的设计提交之前由 WP00 的只读核对脚本（不入库）检查：每条验收对应至少一个 TC，TC 编号落在该包的区段内。

### 3.2 工作包表
表中 `.quality/…` 记作 `Q:<源文件>`，表示 `.quality/docs/src/<源文件>.md`、`.quality/test-design/src/<源文件>.test.md`、`.quality/plans/src/<源文件>.plan.json` 三个文件。“GUI”列为 是 的包修改 `src/store.rs` 或 `src/engine.rs`、资源与构建脚本或 GUI 源文件（WP01、WP04、WP14、WP20），或是以整条发布流水线为验收的发布包（WP19），其验收包含 release 工作流“Build release binaries”步骤中 `ladon-gui` 的编译通过。

| ID | 工作包 | 交付物 | 文件 | 依赖 | 验收 | 测试用例 | 工时 | GUI |
|---|---|---|---|---|---|---|---:|---|
| WP00 | 需求目录与设计基线 | 登记 REQ-101～115、NFR-044～047、UT/IT/ST 编号、TC 区段、RSK-21～40；既有文档任务相关需求（57 号文档第 15.3 节的处置表）的状态写入 `design/requirements.json`；REQ 条文与终态迁移表、人工专属操作、链定义写入 11 号文档新增章节；UT/IT/ST 用例行写入 41、42、43 号文档；36 号文档更新；所有者对未决问题的决定记入文档 | `design/requirements.json`、`design/testcases.json`、`design/nfrs.json`、`docs/01_requirements/11_文档治理业务要件.md`、`docs/03_detailed_design/36_数据库事务与变更策略.md`、`docs/04_tests/41_UT详细设计.md`、`docs/04_tests/42_IT详细设计.md`、`docs/04_tests/43_ST详细设计.md`、`docs/00_guide/01_实施状态与关键风险.md` | — | 编号唯一且 JSON 可解析；每个 REQ 至少对应一个测试编号且每个测试编号对应一个 REQ；11 号文档写入终态迁移表、人工专属操作与链定义；43 号文档“受 GUI 阻塞的 ST”清单覆盖全部依赖 GUI 的用例；57 号文档第 15.3 节的需求处置表与 `design/requirements.json` 一致；不新增编号文档；不触碰 `src/`、`ui-prototype/src` | 脚本：JSON 解析、编号唯一、需求—测试双向映射、每个包的验收条目与 TC 区段对照（无构建，只读，不入库） | 8 | 否 |
| WP00A | 质量门禁与覆盖率度量 | 所有者对 `verify` 流程的决定记录；`.quality/qguard.json`（`coverage_commands` 为 argv 数组，不放占位或伪造报告）；CI 中产出 LCOV 的步骤；`src/store.rs`、`src/engine.rs`、`src/policy.rs`、`src/main.rs`、`src/lib.rs` 的基线覆盖率度量；不改动任何 `src/` 文件 | `.quality/qguard.json`、`.github/workflows/coverage.yml`（新增，须所有者批准）、`docs/04_tests/46_测试执行记录与缺陷.md` | WP00 | 所有者已决定 `verify` 流程且记录在案，候选为：A. CI 产出 LCOV，`verify` 读取该次提交对应的工件；B. 本机 gnullvm 的 `cargo llvm-cov`，仅在提交内存允许时可用；C. 所有者批准的临时流程，允许在 CI 证据到位之前提交生产源码，门槛仍为严格高于 90%（三者都不降低标准，也不把 `excluded_sources` 放宽）；`qguard.json` 有效，`verify` 读到一份真实的 LCOV；CI 的覆盖率步骤在 `windows-latest` 上成功并产出 `lcov.info`；基线覆盖率逐文件记录，低于 90% 的文件列出未覆盖行数；在此之前不得声称任何包的覆盖率达标；工作流变更与第三方工具 `cargo-llvm-cov` 的安装均先征得所有者同意 | TC-1010～1014 | 8 | 否 |
| WP00B | Hermes 集成探针 | 不含 LADON 代码的探针，脚本留在仓库之外：（1）微型 stdio MCP 服务器加覆盖层，经真实启动器链与 `HERMES_MANAGED_DIR` 运行 `hermes mcp test`，记录覆盖层是否生效、`HERMES_MANAGED_DIR` 与 `HERMES_EPHEMERAL_SYSTEM_PROMPT` 是否穿过 `rewrite-and-run.ps1`（探针服务器只回显环境变量名与哈希）、发现窗口耗时；（2）以 D: 盘 Hermes 虚拟环境中的 `mcp` SDK 客户端对该服务器录得精确字节序列，存为金标，并以 SDK 的 `mcp.types` 模型验证样例回复；（3）选定 `hermes_route` 默认值，ST-076 的探针部分结果与 Hermes 版本记入 46 号文档与 57 号文档第 12.1 节 | `tests/fixtures/mcp_client/*.jsonl`（金标）、`docs/04_tests/46_测试执行记录与缺陷.md`、`docs/03_detailed_design/57_文档任务模块与HermesCLI详细设计.md`（第 12.1 节） | WP00 | 探针结论（覆盖层是否生效、两个环境变量是否透传、Hermes 版本）记录在案并选定路线，`HERMES_MANAGED_DIR` 的每次启动设置超出其文档意图这一依赖被明确记录；金标是 SDK 客户端实际发出的字节，每行一个 JSON 对象，LADON 样例回复通过 SDK 模型验证；不读取启动器脚本、任何 `.env`、认证或令牌文件，不启动聊天，只运行 `hermes mcp test` 与 `--help`；WP12 与 WP16 在结论之后开始 | TC-1015～1019 | 8 | 否 |
| WP00C | 命令入口库化（须所有者批准：重构既有代码，行为不变） | 把 `main.rs` 的全部逻辑迁入 `src/entry.rs`：`Cli`、`Command` 的 clap 定义与分派；`watch`（事件源与停止条件可注入，一批事件的处理为独立函数）、`google_login` 与 `google_logout`（浏览器打开与令牌存储可注入）、`tag_folder`（HTTP 客户端可注入）、shell 宿主分支（宿主调用可注入）、`resolve_record`、旧语义的 `annotate` 与 `task`；`main.rs` 缩减为对 `ladon_explorer::entry::main_entry()` 的一次调用；为每个遗留命令补特征化测试 | `src/main.rs`、`src/entry.rs`、`src/lib.rs`、`tests/it_entry_legacy.rs`、`tests/it_cli_binary_smoke.rs`、`Q:main.rs`、`Q:entry.rs`、`Q:lib.rs` | WP00A | 每个遗留命令的输出与退出行为同迁移前一致（特征化测试先在迁移前的二进制上记录，再对 `entry.rs` 通过）；`main.rs` 只含一次调用且被二进制冒烟测试覆盖；`entry.rs` 行覆盖率严格高于 90%，未达标时按 WP00A 的度量报告为阻断并交所有者决定，不降低标准；`Cargo.toml` 与两份 `Cargo.lock` 无改动，`ladon-gui` 仍编译；此后 WP11、WP13、WP16、WP17、WP18 不修改 `main.rs` | TC-1020～1029 | 18 | 否 |
| WP01 | 迁移 004 与 Store 基础 | `sql/004_document_tasks.sql`（见 57 号文档 3.2）；`ensure_schema`（`Store::init` 不变，004 只在此应用）与迁移前备份（成对复制 `meta.turso` 与 `-wal`，打开副本按行数核对，不可验证时的确认与 `dt.migration.applied` 记录）；`connect` 为 `pub(crate)`，带有界重试、抖动与可注入打开器；`file_by_id`；turso 行为探针；内核 API 冻结测试 | `src/store.rs`、`sql/004_document_tasks.sql`、`tests/it_doctask_migration.rs`、`tests/it_kernel_api_freeze.rs`、`Q:store.rs` | WP00A | 全新库经 `init` 与 `ensure_schema` 后 `schema_migrations` 恰含版本 2 与 4，`Store::init` 与 `Engine::init` 单独调用时不应用 004；0.0.3 形态库升级后旧行逐字节不变、旧表无新增行；已记录版本不重复执行且被删除的表不被重建（`tc112`）；迁移前备份：`meta.turso` 与 `-wal` 成对复制并按行数核对，不可验证时 004 不执行、确认后写入 `backup_verified:false`，空库不备份；`connect` 以假打开器验证对锁错误重试 6 次、退避 25～400 ms 加抖动，最终信息指明其他进程占用，非锁错误不重试；004 不含注释、触发器、CHECK、`ALTER`、部分索引；探针结果（影响行数、`BEGIN IMMEDIATE`、CHECK 与外键观测、唯一索引违反的变体、回滚后 `seq` 是否连续、跨进程锁错误的变体与文本）写入测试设计；`tc101`～`tc114`、`it_05` 不变且通过；GUI 使用的全部内核签名通过冻结测试 | UT-162、IT-047、IT-048、IT-068、IT-073；TC-820～829 | 13 | 是 |
| WP02 | 模型（纯） | 状态、主体、操作表与迁移表、守卫输入、`DocError` 与错误码、记录类型、句柄与选择器解析、哈希函数、限制与文本校验、`schedule_batches` | `src/doctask_model.rs`、`src/lib.rs`、`Q:doctask_model.rs`、`Q:lib.rs` | WP00A | 迁移表与 57 号文档 4.2（T1～T11）一致，矩阵穷举，T11 只允许 System；人工专属操作对 Agent、System 返回 `E_HUMAN_ONLY`；状态字符串与旧 7 个完全一致；事件哈希、方案哈希、证据哈希有金标且对任一字段敏感；调度输出批次与冲突组且环被拒；模块不依赖 `store`、`engine` | UT-141～146；TC-830～839 | 8 | 否 |
| WP03 | 大纲、条款编号与锚点分类（纯） | `doctask_outline.rs`：大纲、条款编号、`ROW` 虚拟条款、分类、章节文本与字节级拼接（原始缓冲、行起始偏移、严格 UTF-8） | `src/doctask_outline.rs`、`src/lib.rs`、`Q:doctask_outline.rs` | WP00A | 标题条款键与 `diff::section_hashes` 在含重复标题与 NEMA 域的语料上逐键相等；分类矩阵逐项符合 57 号文档 5.4（改名且正文相同为 `relocated`、正文改动为 `needs_review`、多个相同正文为 `ambiguous`、前置重复标题为 `relocated`（`full_hash`）、短正文不自动重定位）；字节级拼接：CRLF、混合换行、BOM、缺失末尾换行时范围外字节逐字节不变，无效 UTF-8、NUL、UTF-16 目标被拒绝；`diff.rs` 不改动且 `ut_diff_01`～`03` 通过 | UT-147～150；TC-840～849 | 10 | 否 |
| WP04 | 引擎外传门禁 | `Engine::remote_read_allowed`（文件级）、`Engine::remote_version_allowed`（版本级）与规则新鲜度检查 `refresh_policy_if_stale`（根与全局规则文件按修改时间重载并等待就绪，嵌套规则由后台刷新、新规则就绪前沿用上一份有效规则）；`PendingPolicy` 的就绪构造与就绪查询、遍历计数 | `src/engine.rs`、`src/policy.rs`、`tests/it_engine_gate.rs`、`Q:engine.rs`、`Q:policy.rs` | WP00A | 根规则、嵌套规则、全局规则与否定规则的允许与拒绝符合预期；被排除路径内容的副本被拒绝；版本级：旧版本曾被标记敏感、旧版本字节与被排除文件相同时被拒绝；规则无效（含全局规则文件不可读）时一律拒绝；修改根或全局规则文件后下一次调用即看到新规则；规则未变时连续调用不启动规则遍历、一次规则编辑恰好启动一次（遍历计数断言）；`tc301`～`tc328` 通过；`capture`、`PendingVersion`、`CaptureResult`、`reload_policy` 不变 | IT-054（引擎部分）；TC-850～859 | 7 | 是 |
| WP05 | DocStore | `doctask_store.rs`：Immediate 事务、计数器、全部新表的 SQL、CAS、链追加（`prev_hash` 唯一索引与重试）与校验、存量回填（旧事件白名单导入、`dt.task.imported`）、`verify` 所需查询 | `src/doctask_store.rs`、`src/lib.rs`、`tests/it_doctask_store.rs`、`Q:doctask_store.rs` | WP01、WP02 | 第二条语句失败时第一条回滚，不会出现无事件的记录；两个并发 CAS 只有一个成功；链可校验，篡改 `payload_json` 在对应 `seq` 被检出；回填按 `created_at` 顺序分配编号、批注与任务为 `local_only`、事件标记未知、重复执行无变化；旧批注与旧任务标题只以哈希与长度入链，任何链载荷都不含它们的文本，`task.state_changed` 的合法状态以明文导入；每个旧任务有 `dt.task.imported`，回滚到 0.0.3 使用后再升级并重新导入，`verify` 无错误级发现；两个连接并发追加时唯一索引使后者重读链尾重试，链保持完整 | IT-049、IT-050；TC-860～869 | 11 | 否 |
| WP06 | 服务核心 | `DocTaskService::open` 与 `attach`、授权表、门禁特征与 `EngineGate`、`AgentView` 投影、`resolve_file`、基线、批注、任务创建与编辑（含可见性）、读取视图、旧命令兼容路径；人工专属操作的服务签名带展示时的 `expected_*` | `src/doctask_service.rs`、`src/lib.rs`、`tests/it_doctask_service.rs`、`tests/it_nema_task.rs`（加强 IT-005）、`Q:doctask_service.rs` | WP03、WP04、WP05 | 无基线时创建返回 `E_NO_BASELINE`；任务钉在基线版本且章节解析取自该版本字节；Agent 对基线、编辑、公开批注返回 `E_HUMAN_ONLY`；Agent 视图经 `AgentView`：省略 `local_only` 与门禁拒绝文件上的记录并以 `hidden` 计数，被拒绝、`local_only` 与不存在的引用得到完全相同的 `E_NOT_FOUND` 且无候选，历史版本逐版本判定，事件不含载荷；`attach` 不写 `meta`、不落库对账；人工专属操作的 `expected_*` 在主体变化时得到 `E_STALE_SUBJECT`，现存文件未捕获时基线登记、`ack`、`rebase` 得到 `E_FILE_CHANGED`，`--yes` 无 `--expect-hash` 被拒；基线撤销后同一版本可重新登记；旧创建路径保持旧语义（`unchecked`、`legacy`）；IT-005 断言 `base_version_id`、章节与批注读回 | IT-051、IT-054（服务部分）；TC-870～879 | 13 | 否 |
| WP07 | 服务流程 | `doctask_flow.rs`：迁移 T1～T11、批准与驳回、只分析方案的提交（`doc plan new` 的服务层）、提案、依赖、关联、证据、配额、收件箱、调度、`verify`；`EditPlanSupport` 接口，未注册时拒绝编辑方案 | `src/doctask_flow.rs`、`src/lib.rs`、`tests/it_doctask_flow.rs`、`Q:doctask_flow.rs` | WP06 | 每条合法边写入文档规定的事件且同事务；每条非法边被拒且不写入；验收需要通过证据或书面豁免、先行任务完成、锚点健康；Agent 在服务层不能批准、驳回、验收、取消、重开、采纳、忽略；Agent 任务为提案且采纳前不能迁移；UAT-05 对应用例得到一个冲突组加一个并行任务；`verify` 检出绕过 `set_task_status` 的 `STATUS_DRIFT`；只分析方案的 `open → done` 全生命周期（不含编辑方案）可经服务层走通；T3 守卫 4 过期时方案 `superseded`、任务回到 `analysis`、状态已提交并返回 `E_STALE_PLAN`；Agent 写入的通过证据不满足 T6，`--accept-agent-evidence` 与书面豁免各自通过且证据 id 写入批准载荷；`relocated` 未确认时 T3、T6 被拒；未采纳的提案上迁移与方案提交得到 `E_TRANSITION_DENIED` 并提示采纳 | IT-052、IT-053、IT-057；TC-880～899 | 14 | 否 |
| WP08 | 对账与文件移动谱系 | `doctask_reconcile.rs`：对账、确认、重定位、重定基、锚点迁移报告、谱系 | `src/doctask_reconcile.rs`、`src/lib.rs`、`tests/it_doctask_reconcile.rs`、`Q:doctask_reconcile.rs` | WP06、WP07 | 外部改标题且正文相同得到 `relocated` 并记事件，须 `ack` 或 `relocate` 后才可批准、验收、应用；前置重复标题得到 `relocated`（`full_hash`），正文未达重定位下限的不自动重定位；正文改动得到 `needs_review`，确认后可验收且 `base_version_id` 不变；`ack`、`relocate`、`rebase`、谱系确认的 `expected_*` 不符得到 `E_STALE_SUBJECT`，现存文件摘要与账本不符得到 `E_FILE_CHANGED`；文件删除得 `file_missing`，冷字节得 `unknown` 而不编造；对账重复执行不产生重复事件；移动文件产生仅人工可确认的谱系候选，确认后列表跟随新路径 | IT-055、IT-056；TC-900～909 | 8 | 否 |
| WP09 | 渲染与文案（纯） | `doctask_render.rs`：中英文案目录、CJK 宽度、表格、各屏幕、错误渲染 | `src/doctask_render.rs`、`src/lib.rs`、`Q:doctask_render.rs` | WP02 | 中英混排列对齐；各屏幕与 golden 文本一致；纯文本模式无 ANSI；每个错误码有中英文消息与提示；中英文案键集一致；追踪屏包含批注、下游任务、关联、Agent 变更与提交、测试证据、回滚链；批准屏显示 `expected_hash`、`result_hash` 的短哈希与目标章节同基线是否一致（单独一行） | UT-145（渲染部分）、UT-151、UT-152；TC-910～919 | 6 | 否 |
| WP10 | 报告与 .NEMA 导出（纯） | `doctask_report.rs`：报告（md/json）、.NEMA 写出与回读校验 | `src/doctask_report.rs`、`src/lib.rs`、`Q:doctask_report.rs` | WP02 | 导出的 .nema 经 `parse_graph_edges` 解析且 `topo_batches` 成功；制表符、U+3000、换行被净化，记号不含空格、`<`、`>`、`=`；同一输入与 `--generated-at` 输出逐字节相同，连续导出两次也相同（数据截止点 `cutoff_seq`，`dt.export.generated` 不进入内容）；未开启时不含 `local_only` 批注与任务的文本；报告含回滚链与恢复命令 | UT-153、UT-154；TC-920～929 | 7 | 否 |
| WP11 | 一次性 CLI 与旧命令路由 | `doctask_cli.rs`：`doc` 命令树、注册表、`--json`、退出码、人工专属确认流程（`--yes` 需 `--expect-hash`）；只分析方案的 `doc plan new`、`approve`、`reject`；`annotate`、`task` 经服务层（改 `entry.rs` 中的库调用，`main.rs` 不改动） | `src/doctask_cli.rs`、`src/entry.rs`、`src/lib.rs`、`tests/it_doctask_cli.rs`、`tests/it_doctask_cli_binary.rs`、`Q:doctask_cli.rs`、`Q:entry.rs` | WP00C、WP06、WP07、WP08、WP09、WP10 | 57 号文档 9.2 中除编辑方案选项（`--file --section --kind --text-file`）、`plan show --diff`、`apply`、`recover` 之外的全部命令可用并输出规定文案与 JSON，含只分析方案的 `plan new`、`approve`、`reject`；人工专属命令在管道与伪终端上退出 5；退出码映射正确；`annotate`、`task` 保持位置参数并首行输出 `annotation id: `、`task id: `；`entry.rs` 的新增分支各为一次库调用且不含 SQL（词元扫描）；从 `init` 到导出的一次性全流程（只分析方案）不需要 Hermes | IT-060、IT-061、IT-066；TC-930～939 | 12 | 否 |
| WP12 | MCP 协议层（纯） | `mcp.rs`：JSON-RPC 与 MCP 生命周期、`ToolHost`、行读取与限长 | `src/mcp.rs`、`src/lib.rs`、`Q:mcp.rs` | WP00A、WP00B | 协商 2025-11-25、2025-06-18、2025-03-26、2024-11-05 且对未知版本回复 2025-11-25；WP00B 的金标逐行被协议层解析；通知无回复、`ping` 回 `{}`、未知方法 -32601 并按原类型回显 id；坏 JSON、无效 UTF-8、超长行得到错误回复且循环继续；每行输出恰为一个 JSON 对象（由 IT-062 与词元扫描证明；模块级 `deny` 属性只在运行 clippy 时生效，见 5.1） | UT-155、UT-156；TC-940～949 | 5 | 否 |
| WP13 | MCP 工具与 `mcp-serve` | `doctask_mcp.rs`：13 个工具（`doc_plan_submit` 由 WP14 追加）、`serve()`（`--global-ignore`、延迟的 Schema 检查、`CatchPanic`、panic 钩子、披露记录先写后返回）、桩客户端辅助程序、架构守卫（词元扫描）；`initialize` 不打开数据库，首次 `tools/call` 才打开 `Engine` 并使用 `DocTaskService::attach` | `src/doctask_mcp.rs`、`src/entry.rs`、`src/lib.rs`、`tests/support/mcp_stub.rs`、`tests/it_mcp_stdio.rs`、`tests/it_arch_guard.rs`、`tests/it_doctask_concurrency.rs`、`Q:doctask_mcp.rs`、`Q:entry.rs` | WP07、WP08、WP11、WP12 | 工具集与 57 号文档 10.2 一致（13 个，`doc_plan_submit` 由 WP14 追加，UT-157 快照为 13 个），无人工专属工具，`readOnlyHint` 恰在只读工具上；数据库被占用时 `initialize` 200 ms 内回复，工具调用得到普通结果 `E_STORE_BUSY`；业务拒绝、非法参数（`E_INVALID_ARG`）、`E_STORE_BUSY`、`E_SCHEMA_OUTDATED` 都是不带 `error` 键也不带 `isError` 的普通结果，不使熔断计数增加；Schema 低于 4 时进程不退出且不改动数据库；门禁覆盖历史版本、调度、关联、追踪、阻塞者与 `AgentView` 省略；配额与重复创建行为正确；一系列只读调用之后 `doc_task_events`、`doc_anchors`、`doc_task_meta` 逐字节不变，只有 `doc_agent_disclosures` 增加且先于内容返回；`--global-ignore` 经 `env_clear()` 子进程生效；`CatchPanic` 使 panic 得到 `E_INTERNAL`，panic 标记文本不出现在 stderr；EOF 后 2 秒内以 0 退出且 stdout 仅含协议行；架构守卫通过；子进程与测试进程并发读写全部成功 | UT-157、IT-062、IT-063、IT-067、IT-069、IT-072、IT-074；TC-950～959 | 12 | 否 |
| WP14 | 方案、批准、应用与恢复 | `doctask_plan.rs`（实现 `EditPlanSupport`：目标校验、字节级拼接、差异预览）；`Engine::commit_text`（规范化与重解析点检查、方案 id 派生的暂存与备份路径、`capture` 返回 `unchanged` 的处理；与 `restore` 共用私有辅助函数，`restore` 不变）；`doc plan new` 的编辑选项、`plan show --diff`、`apply`、`recover`；`doc_plan_submit` 工具（第 14 个） | `src/doctask_plan.rs`、`src/engine.rs`、`src/doctask_cli.rs`、`src/doctask_mcp.rs`、`src/lib.rs`、`tests/it_doctask_plan.rs`、`Q:doctask_plan.rs`、`Q:engine.rs` | WP07、WP08、WP11、WP13 | 应用只写入方案规定的字节，创建账本 `modified` 事件且原因以任务号开头，系统证据链接新版本；批准界面显示的差异应用于 `expected_version_id` 的字节后，与结果字节及 `commit_text` 实际写入的字节逐字节相等（基线之后又被编辑的目标文件同样成立）；`result_hash` 等于 `expected_hash` 的方案被拒；批准与应用之间现存文件被外部修改则不写入、方案 `conflicted`、任务保持 `working`；Agent、未批准、已过期方案的应用被拒；字节级拼接：CRLF、混合换行、BOM、缺失末尾换行时范围外字节不变，非 UTF-8 目标被拒；`.ladon`、`.git` 路径被拒；`capture` 返回 `unchanged`（`watch` 抢先捕获）时 A4 仍链接正确的版本，`doc recover` 同样；注入点测试覆盖 A1 至 A4 每两步之间的中断、并发 `capture` 与 `mark_deleted`，`doc recover` 的各行（目标为旧内容、目标缺失而备份在、目标为新内容且账本已有或没有新版本、第三种内容、目标与备份都缺失）收敛为旧内容、新内容或保留现场且账本与事件链一致；`cfg(windows)` 的 junction 目录组件使应用被拒且不移动文件；`--out` 与 `--text-file` 拒绝设备名与非普通文件；`restore` 与 `capture` 既有测试不变；`trace` 与报告含回滚链 | IT-058、IT-059、UT-157（第 14 个工具的快照）；TC-960～969 | 14 | 是 |
| WP15 | Windows 控制台辅助（含 FFI） | `winconsole.rs`：控制台模式保存与恢复、VT 开启、Ctrl+C 门、kill-on-close 作业、`is_real_console`（`GetConsoleMode`）与 `console_kind`（msys 管道名匹配）；非 Windows 空桩 | `src/winconsole.rs`、`src/lib.rs`、`Q:winconsole.rs` | WP00A | 标准输出为管道时全部函数返回而不 panic，`is_real_console` 对管道句柄为假；msys、cygwin 管道名匹配是纯函数且有单元测试；作业只设置 `KILL_ON_JOB_CLOSE`、加入失败为警告；Ctrl+C 处理例程对 `CTRL_C`、`CTRL_BREAK` 返回 TRUE，对关闭、注销、关机返回 FALSE，模块内不出现 `SetConsoleCtrlHandler(NULL, TRUE)`；64 位作业结构大小断言；非 Windows 桩的编译在 ubuntu 作业中通过，其 clippy 结论按 5.1 报告（证据通道建立之前为“未验证”） | UT-161；TC-970～974 | 6 | 否 |
| WP16 | Hermes 启动器与自检 | `hermes.rs`：设置、启动器解析、覆盖层（含 `--global-ignore`）与种子、不区分大小写的环境白名单、命令行、预检、`ProcessRunner`、doctor、print-config、status、披露检查（`doc_agent_disclosures`、`policy_fp`、`hermes resume`）；`ladon hermes` 子命令；两层链假启动器测试 | `src/hermes.rs`、`src/entry.rs`、`src/lib.rs`、`tests/it_hermes_launch.rs`、`Q:hermes.rs`、`Q:entry.rs` | WP00B、WP13、WP14、WP15 | argv 与 57 号文档 12.5 完全一致且不含用户文本；子进程环境不含任何 `LADON_*`（含 `Ladon_*` 等混合大小写）与被禁的 `HERMES_*`，`Path`、`SystemRoot`、`ProgramFiles(x86)` 保持原大小写，含 `HERMES_MANAGED_DIR` 与 `HERMES_EPHEMERAL_SYSTEM_PROMPT`，LADON 不设置 `HERMES_HOME`（所有者环境中已有时原样转发）；预置 `HERMES_MANAGED_DIR`、不是真实 Windows 控制台、Schema 过低、`.cmd` 参数不安全时拒绝；自检子进程以 `env_clear()` 加 Hermes 基线环境、与覆盖层相同的 `args` 启动；覆盖层写出后回读比对，退出后目录被清除；指令文本通过威胁模式扫描且不超过 4000 字符；假启动器为 `.cmd` → `powershell -File` 两层链，断言 `$args` 逐元素（`ladon,clarify` 为单个元素）与指令文本的 SHA-256 和长度；`hermes resume` 在规则文件变化、敏感标记变化、已披露内容的副本进入排除路径、上次会话未正常结束时开启新会话，一切不变时恢复；退出码被记录；`print-config` 与 `profile` 路线不写任何 Hermes 文件 | UT-159、UT-160、IT-065、IT-071；TC-975～984 | 13 | 否 |
| WP17 | 交互控制台 | `console.rs`：分词、命令、收件箱与批准流程（含采纳提案、接受 Agent 证据后验收）、启动屏、Hermes 交接、脚本模式、设置；`ladon console` | `src/console.rs`、`src/entry.rs`、`src/lib.rs`、`tests/it_console.rs`、`tests/it_console_binary.rs`、`Q:console.rs`、`Q:entry.rs` | WP11、WP14、WP15、WP16 | 每个 `doc` 命令在控制台中带或不带前缀均可用，clap 错误不退出控制台；收件箱以展示哈希绑定，内容变化得到 `E_STALE_SUBJECT`；只有精确键入任务号才批准；脚本模式下人工专属命令与 `hermes` 以退出 5 拒绝；`hermes` 经记录型 `ProcessRunner` 启动，返回后显示摘要与待处理队列；只有 Agent 通过证据时收件箱只提供接受证据与书面豁免；中英文案键集一致；NFR-044 夹具的启动屏在 CI 上不超过 10 秒（回归报警，记录实测值），3 秒目标由 ST-077 在所有者机器上手工度量 | UT-158、IT-064；TC-985～994 | 10 | 否 |
| WP18 | 菜单入口接到控制台 | `entry.rs` 中 `shell-host-internal` 的 0x7003 分支以 `std::process::Command` 直接启动 `ladon.exe`（参数为列表，`creation_flags(CREATE_NEW_CONSOLE)`，从不经过 `cmd`、`start` 或 `powershell`，与 `shell.rs` 中既有的列表式 `spawn_shell_host` 同形）运行 `console --open 文件`；`console --open` 参数。现有入口是 `ladon shell-menu <文件>` 所宿主的菜单，MSI 没有登记资源管理器右键谓词，是否新增登记属所有者的另一个决定（不在本包内） | `src/entry.rs`、`src/console.rs`、`docs/03_detailed_design/40_WindowsShell菜单与隔离.md`、`Q:entry.rs` | WP17 | `--open 文件` 使控制台以该文件为当前文件并显示其大纲；占位文案消失，40 号文档描述终态行为；`shell.rs` 对 0x7003 的映射不变；启动参数以 `Command::get_args()` 断言，文件名含 `&`、`^`、`%` 时作为单个 argv 元素到达，另以回显 argv 的测试辅助程序在 Windows 上端到端验证；ST-088 按真实入口 `ladon shell-menu <文件>` 描述 | TC-995～996；ST-088 | 3 | 否 |
| WP19 | 端到端验收与发布准备 | 所有者一天端到端测试；覆盖率度量与结论；手工 ST 脚本与记录模板；用户手册、CHANGELOG、追踪工作簿；散落文件清理；版本号建议 | `tests/it_doctask_owner_day.rs`、`docs/05_operations/53_用户操作说明.md`、`docs/04_tests/46_测试执行记录与缺陷.md`、`docs/04_tests/41_UT详细设计.md`、`docs/04_tests/42_IT详细设计.md`、`docs/04_tests/43_ST详细设计.md`、`docs/04_tests/要求追踪_测试管理.xlsx`、`design/testcases.json`、`CHANGELOG.md` | WP14、WP17、WP18、WP20 | 所有者一天端到端测试在 CI 上通过（记录型 `ProcessRunner` 中的“Hermes”是桩 MCP 客户端，对接真实 `mcp-serve` 子进程：提案、批注，所有者采纳，“Hermes”在已采纳任务上提交方案，再批准、应用、证据、验收、校验、导出并回读 .NEMA）；每个变更源文件的行覆盖率结果按 WP00A 确立的流程记录在案，项目整体门禁结论如实报告；57 号文档第 15.3 节的需求处置与 `design/requirements.json`、发布说明一致，并写明本版本不是 1.0 及仍未完成的 P0；手工 ST-076～089 备有执行脚本与结果模板，未执行者保持 Not Run；手册只描述终态并写明已知限制；两份 `Cargo.lock` 与 `Cargo.toml` 无改动；散落的零字节文件经所有者同意后清理 | IT-070；UAT-01、UAT-05 记录；TC-997～999 | 10 | 是 |
| WP20 | 文件图标与可执行文件图标 | `ladon.exe` 与 `ladon-gui.exe` 嵌入 LADON 图标；GUI 文件列表按文件类型显示系统图标并有回退。设计要点见 3.3 | `build.rs`、`ui-prototype/build.rs`（新增）、`build/rc_tool.rs`（新增，无依赖，两个 `build.rs` 以 `#[path]` 引入）、`ui-prototype/src/fileicon.rs`（新增）、`ui-prototype/src/lib.rs`、`ui-prototype/src/main.rs`、`ui-prototype/ui/app.slint`、`tests/it_build_rc_tool.rs`、`tests/it_exe_icon.rs`、`logo/platform/windows/LADON_Explorer.ico`（既有，不改）；`.quality` 文档：`build.rs`、`build/rc_tool.rs`、`ui-prototype/build.rs`、`ui-prototype/src/fileicon.rs` 新建，`ui-prototype/src/main.rs`、`ui-prototype/src/lib.rs` 先刷新（现有四份 GUI 设计文档的哈希已与源码不符，并含已不存在的内容） | WP00A | `ladon.exe` 的 PE 资源目录含 `RT_GROUP_ICON` 与 `RT_ICON`（`tests/it_exe_icon.rs`）；`ladon-gui.exe` 的图标由 ST-089 确认，自动检查须修改 `release.yml` 或 `packaging/windows/Build-Package.ps1`，先征得所有者同意，未获同意时保持仅手工确认；文件列表对目录、`.md`、`.txt`、`.exe` 得到非空位图，未知扩展名取系统通用图标，位图缺失时显示字形占位；`fileicon.rs` 与 `build/rc_tool.rs` 的单元测试使每文件行覆盖率严格高于 90%，`build.rs` 只做接线、`main.rs` 只增加一行映射，二者不在 llvm-cov 可报告范围，覆盖率结论如实记录；既有 GUI 测试通过；`ladon-gui` 在 release 工作流中编译通过；`Cargo.toml` 与两份 `Cargo.lock` 无改动 | ST-089；TC-1000～1009 | 12 | 是 |
合计 236 小时（估算〔假设〕，按每包含设计提交、实现、测试与文档修订计；不含 WP01、WP04、WP14 对 `store.rs`、`engine.rs`、`policy.rs` 既有未覆盖行的补测，该部分的条件性工时待 WP00A 的基线度量给出未覆盖行数后追加）。WP00C 的 18 小时以每个遗留命令至少一个特征化测试估算，须所有者批准；不批准时 `main.rs` 的覆盖率按 WP00A 的度量报告为阻断，由所有者决定，不降低标准。

### 3.3 WP20 设计要点
〔决定〕
1. 依赖：零新增依赖（见 3.1 第 7 点）。资源编译由只依赖 `std` 的构建脚本完成，系统图标取用手写 `extern "system"` FFI（风格同 `src/google_oauth.rs:415-519`，非 Windows 为返回 `None` 的桩）。给 GUI crate 增加 `build.rs` 不需要改 `Cargo.toml`，Cargo 自动识别包根下的 `build.rs`。
2. 可执行文件图标：图标源为既有的 `logo/platform/windows/LADON_Explorer.ico`（16–256 px 多分辨率，〔事实 logo/README.md〕）。两个 `build.rs` 在 Windows 目标上把含 `1 ICON "<.ico 绝对路径>"` 的 `.rc` 写入 `OUT_DIR`，调用资源编译器，以 `cargo:rustc-link-arg-bins=<产物>` 交给链接器（该指令只作用于本包自己的二进制，所以 `ladon.exe` 与 `ladon-gui.exe` 各自嵌入），并以 `cargo:rerun-if-changed` 指向 `.ico`。资源编译器的发现与 `.rc` 生成放在 `build/rc_tool.rs`，发现顺序：环境变量 `RC`；PATH 中的 `rc.exe`；`%ProgramFiles(x86)%\Windows Kits\10\bin\<最高版本>\x64\rc.exe`；PATH 中的 `windres`、`llvm-windres`、`x86_64-w64-mingw32-windres`（gnu、gnullvm 目标，产物为 COFF 目标文件）。全部找不到时输出 `cargo:warning` 并继续构建，使缺少资源编译器的开发机仍能构建；产物是否带图标由第 6 条的检查判定。
3. 文件图标：库模块 `ui-prototype/src/fileicon.rs`。纯逻辑部分：图标键（目录，或小写扩展名，无扩展名为空串）、缓存 `HashMap<键, Option<IconPixels>>`、`IconPixels{width,height,rgba}`。`cfg(windows)` 部分：`SHGetFileInfoW` 以扩展名加 `FILE_ATTRIBUTE_NORMAL`（目录用 `FILE_ATTRIBUTE_DIRECTORY`）与 `SHGFI_USEFILEATTRIBUTES | SHGFI_ICON | SHGFI_SMALLICON` 查询，不访问磁盘；`GetIconInfo` 与 `GetDIBits` 取得 32 位位图并转为非预乘 RGBA；释放 `HICON` 与 GDI 对象。`exe`、`ico`、`lnk` 显示其类型图标而不是文件自身内嵌的图标（后者需要工作区根与逐行磁盘访问，不在范围内，写入手册的已知限制）。
4. 接线：`FileItem` 增加 `icon: image` 与 `has-icon: bool`；文件列表行在名称之前放一个 18 px 的 `Image`，`has-icon` 为假时显示既有风格的字形占位。`file_item`（`ui-prototype/src/main.rs`）是 `FileItem` 的唯一构造点，由在 UI 线程运行的 `apply_reply` 调用；它增加一行对 `fileicon` 的映射调用，位图经 `SharedPixelBuffer` 在 UI 线程转为 `slint::Image`。`app::Command`、`app::Reply` 与工作线程协议不变，因此不触发 `apply_reply` 的穷尽匹配。
5. 质量门禁：逻辑全部放在 `fileicon.rs` 与 `build/rc_tool.rs` 并带单元测试（`build/rc_tool.rs` 由根 crate 的 `tests/it_build_rc_tool.rs` 以 `#[path]` 引入后测试，故可被 llvm-cov 报告）；`main.rs` 只加一行映射，`build.rs` 只做接线。`main.rs` 与 `build.rs` 无法单独达到每文件行覆盖率严格高于 90%（GUI 的 `main.rs` 没有自动化测试，`build.rs` 不在 llvm-cov 的报告范围内），结论如实记录，不降低标准。
6. 验证范围：`ladon.exe` 由 `tests/it_exe_icon.rs`（`#[cfg(windows)]`，仅 `std`）读取 `CARGO_BIN_EXE_ladon` 的 PE 资源目录并断言存在 `RT_GROUP_ICON` 与 `RT_ICON` 条目。`ladon-gui.exe` 只在 release 工作流的发布构建步骤产生（GUI 测试步骤使用 `--no-default-features --features kernel`，不构建该二进制），自动检查需要修改 `release.yml` 或 `packaging/windows/Build-Package.ps1`，须先征得所有者同意；未获同意时只由 ST-089 在所有者机器上确认。`fileicon.rs` 的 `#[cfg(windows)]` 测试断言目录、`.md`、`.txt`、`.exe` 得到非空位图且缓存命中不重复查询。
〔假设〕windows-latest 运行器安装了 Windows SDK 且第 2 条的路径之一可用；`SHGetFileInfoW` 与 `GetDIBits` 在运行器的非交互会话中可用；本机 gnullvm 加 llvm-mingw 提供第 2 条的某个 `windres`。三项均由 CI 与所有者机器的首次构建确认；任一不成立时停止并上报，不在包内改用新增依赖。

## 4. 包间顺序与并行
```
WP00 → WP00A → { WP01 → WP05 ; WP02 → { WP09 ; WP10 } ; WP03 ; WP04 ; WP00C ; WP15 ; WP20 }
WP00 → WP00B → { WP12 ; WP16 }
WP03、WP04、WP05 → WP06 → WP07 → WP08
WP00C、WP06、WP07、WP08、WP09、WP10 → WP11
WP07、WP08、WP11、WP12 → WP13 → WP14（WP14 另依赖 WP07、WP08、WP11）
WP00B、WP13、WP14、WP15 → WP16
WP11、WP14、WP15、WP16 → WP17 → WP18
WP14、WP17、WP18、WP20 → WP19
```
依赖以第 3.2 节表中的“依赖”列为准。同一源文件被多个包修改时严格串行：`src/lib.rs`（WP00C、WP02、WP03、WP05、WP06、WP07、WP08、WP09、WP10、WP11、WP12、WP13、WP14、WP15、WP16、WP17：每个包的 `pub mod` 行、`lib.rs` 的 `.quality` 文档与封印计划的基线哈希都以上一次提交的 blob 为准，所以同一时刻只有一个包持有 `lib.rs`）、`src/main.rs`（只有 WP00C）、`src/entry.rs`（WP00C、WP11、WP13、WP16、WP17、WP18）、`src/engine.rs`（WP04、WP14）、`src/policy.rs`（只有 WP04）、`src/store.rs`（只有 WP01）、`src/doctask_cli.rs`（WP11、WP14）、`src/doctask_mcp.rs`（WP13、WP14）、`src/console.rs`（WP17、WP18）。因此 WP02 与 WP03、WP09 与 WP10、WP12 与 WP15 不并行；WP01、WP04、WP20 的文件不含 `lib.rs`，可与 `lib.rs` 链上的包并行。

## 5. 构建与测试环境约束及各包验证方式

### 5.1 约束
| 约束 | 说明 |
|---|---|
| 提交内存 | 撰写本文时可用提交内存约 1.7 GB（总量 23.8 GB），`E:` 盘可用约 11 GB。`turso_core` 编译即使在 `opt-level=0` 下也需要超过 1.3 GB 提交内存；`cargo check` 约需 1 GB 以上，`cargo test` 需数 GB。内存不足时 rustc 以 `failed to mmap … paging file too small` 失败 |
| 本机工具链 | 默认 MSVC 工具链在本机无法链接；可用路径是 `stable-x86_64-pc-windows-gnullvm` 加 llvm-mingw，`cargo +stable-x86_64-pc-windows-gnullvm check --release --all-targets -j 1`，`CARGO_INCREMENTAL=0`；测试需 `CARGO_PROFILE_DEV_DEBUG=0`，且空闲提交内存不低于上限加 500 MB（测试至少约 3 GB）才启动 |
| 杀毒软件 | 360 Total Security 会间歇阻止刚生成的构建脚本可执行文件（`os error 5`），重试构建即可；不得改动其信任列表，该决定属于所有者 |
| Cargo 陷阱 | 仓库与临时工程绝不共用同一个 `CARGO_TARGET_DIR`；以 `cargo clean -p ladon-explorer --release` 后再构建确认结果 |
| CI 触发 | `release.yml` 在标签 `v*` 与分支 `fix/compile-errors` 的推送上运行；其他分支不会触发，除非所有者批准修改工作流。每次推送都会运行完整的内核测试、GUI 测试、发布构建、打包与 MSI，且推送前必须征得所有者同意 |
| CI 与 GUI | GUI 的编译只发生在 `release.yml` 的发布构建步骤（GUI 测试步骤使用 `--no-default-features --features kernel`），所以 GUI 使用的内核签名被破坏时测试步骤仍然通过；`tests/it_kernel_api_freeze.rs`（WP01）把这类破坏提前到内核测试步骤暴露 |
| `rust-ci.yml` | 先 `cargo generate-lockfile` 再 `--locked`，不强制已提交的锁文件；其 ubuntu 作业在第 14 行先执行 `cargo fmt --check`，其后才是测试（第 15 行）与 clippy（第 16 行），而 fmt 在既有代码上已失败（57 号文档第 15.1 节不重排既有文件），因此该作业的 clippy 步骤从不执行；`release.yml` 不含 clippy。它不是验收门禁。clippy 证据通道须由所有者择一建立：修改 `rust-ci.yml`（fmt 不阻断或置于最后，或新增独立的 clippy 作业，须所有者批准），或本机 gnullvm 的 `cargo clippy --all-targets -- -D warnings`（受提交内存限制）；建立之前，以 clippy 为证据的验收项（非 Windows 桩无警告、模块级 `deny` 属性）报告为“未验证”，stdout 守卫以 IT-062 与 IT-067 为准 |
| `main.rs` | `src/main.rs` 含 `watch` 的阻塞循环、`google_login`、`tag_folder` 与 shell 宿主分支，二进制层没有任何测试；若五个包各自改动它，每次都使整份文件进入严格高于 90% 的门禁而无法达标。默认方案是 WP00C（须所有者批准）把这些逻辑迁入带可注入依赖的库模块 `entry.rs` 并补特征化测试，`main.rs` 缩减为一次调用，此后没有包改动它；未获批准时 `main.rs` 的覆盖率按 WP00A 的度量报告为阻断，由所有者决定，不降低标准 |
| 质量门禁 | 项目整体行覆盖率严格高于 90% 的要求要把 `build.rs`、`native/shell_menu.cpp`、`ui-prototype/src/main.rs`、`scripts/*.py`、`server/vercel/**/*.ts` 等非 Rust 受管源文件计入，而 `cargo llvm-cov` 只报告 Rust 源，且 `excluded_sources` 必须保持为空；因此项目整体门禁无法通过，只报告结果，不降低标准 |
| 覆盖率度量 | 仓库 `.quality/` 下没有 `qguard.json`，全局配置的 `coverage_commands` 为空；钩子的 PATH 中没有 `cargo`；可用提交内存约 1.7 GB 时本机无法运行 LCOV 度量。度量方案（`cargo llvm-cov --lcov`）、`qguard.json`、CI 步骤与 `verify` 流程由 WP00A 交付，工作流变更与第三方工具的安装均须所有者批准，且先于任何生产源码包；在此之前不声称覆盖率达标 |
| 钩子行为 | Stop 钩子在生产源文件与 HEAD 不同时于每轮结束运行 `verify`，失败会阻止停止；每个工作轮次结束时不得留下未提交的生产源文件改动，除非 `verify` 通过；WP00A 落地之前没有包可以提交生产源码 |

### 5.2 验证通道
| 通道 | 做法 | 适用 |
|---|---|---|
| A 文档与目录 | Python 脚本检查 JSON、编号与映射，人工核对终态措辞 | WP00、各包的文档修订 |
| B 纯模块 | 仓库之外的临时工程，以 `#[path]` 引入纯模块与其依赖的 `diff.rs`、`crypto.rs`，使用独立的 `CARGO_TARGET_DIR`，只依赖 serde、serde_json、anyhow、sha2、hex、similar、regex、chrono、uuid、aes-gcm、rand 等轻量库，不编译 turso〔假设〕所需提交内存在 1 GB 量级，未度量 | WP02、WP03、WP09、WP10、WP12、WP15 的单元测试 || C 本机检查 | 第 5.1 节的 gnullvm `cargo check`（Job Object 内存上限保护），内存允许时运行 `cargo test` | 所有内核包的编译确认 |
| D CI | GitHub Actions `release.yml`：内核测试（`--locked --all-targets`）、GUI 测试、发布构建（含 `ladon-gui`）、打包、MSI；`#[cfg(windows)]` 测试只在此运行 | 全部包的权威验证 |
| E 所有者机器 | 真实 Hermes 与可见窗口的手工系统测试 ST-076～089 | WP16、WP17、WP18、WP19、WP20 |
| F 探针 | WP00B 的仓库外探针脚本：真实启动器链上的 `hermes mcp test` 与 D: 盘 Hermes 虚拟环境中的 `mcp` SDK 客户端，不启动聊天、不读取秘密文件 | WP00B |

### 5.3 各包验证
| 工作包 | 通道 | 证明完成的依据 |
|---|---|---|
| WP00 | A | 脚本输出无差异 |
| WP00A | A、D | `verify` 读到一份来自 CI 的真实 LCOV，基线覆盖率逐文件记录；所有者的流程决定在案 |
| WP00B | F | 探针结论与金标记录在案，金标通过 SDK 模型验证 |
| WP00C | C、D | 特征化测试与二进制冒烟在 CI 通过，`entry.rs` 覆盖率按 WP00A 的流程报告，`ladon-gui` 编译通过 |
| WP01 | C、D | CI 上 IT-047、IT-048、IT-068、IT-073、UT-162 通过且“Build release binaries”中 `ladon-gui` 编译通过；探针结果写入测试设计 |
| WP02、WP03 | B、D | UT-141～150 在两个通道通过 |
| WP04 | C、D | IT-054（引擎部分）通过，GUI 编译通过 |
| WP05 | C、D | IT-049、IT-050 在 CI 通过 |
| WP06、WP07、WP08 | D（C 编译） | IT-051～057 在 CI 通过；`tc108`、`tc109`、`it_05` 仍通过 |
| WP09、WP10 | B、D | UT-145、UT-151～154 通过 |
| WP11 | D | IT-060、IT-061、IT-066 在 CI 通过；二进制冒烟覆盖 `entry.rs` 接线 |
| WP12 | B、D | UT-155、UT-156 通过 |
| WP13 | D | UT-157、IT-062、IT-063、IT-067、IT-069、IT-072、IT-074 在 CI 通过 |
| WP14 | D | IT-058、IT-059 通过，既有 `restore` 测试不变，GUI 编译通过 |
| WP15 | B、D | UT-161 通过；非 Windows 桩的 clippy 结论按 5.1 报告（证据通道建立之前为“未验证”） |
| WP16 | D、E | UT-159、UT-160、IT-065、IT-071 在 CI 通过；ST-076、ST-079、ST-080、ST-083 在所有者机器上执行 |
| WP17 | D、E | UT-158、IT-064 在 CI 通过；ST-077、ST-078、ST-081、ST-082、ST-085、ST-086、ST-087 在所有者机器上执行 |
| WP18 | D、E | 参数断言在 CI 通过；ST-088 在所有者机器上执行 |
| WP19 | D、E | IT-070 在 CI 通过；覆盖率度量结果；ST-084 在所有者机器上执行 |
| WP20 | D、E | `tests/it_build_rc_tool.rs`、`tests/it_exe_icon.rs`、`fileicon.rs` 单元测试与既有 GUI 测试通过，“Build release binaries”中 `ladon-gui` 编译通过；`ladon-gui.exe` 的图标与文件列表图标由 ST-089 在所有者机器上确认 |
所有通道的首个失败都必须回到对应包修复，不得跳过测试或降低标准；失败的测试不以“环境问题”为由放过。

## 6. 风险
| 编号 | 风险 | 缓解 |
|---|---|---|
| RSK-21 | Hermes 集成路线尚未经端到端验证：覆盖层与指令文本是否经本机启动器链（两层 `.cmd` 加 `powershell -File`）透传、`-c … --query-file` 组合、MCP 子进程是否在发现窗口内就绪、Tool Search 是否确实关闭 | 路线是设置项（`overlay`、`profile`、`print`）；WP00B 的探针在 WP12 与 WP16 之前用真实启动器与 SDK 客户端检验这些前提并选定路线（`HERMES_MANAGED_DIR` 的每次启动设置超出 Hermes 文档的意图，Hermes 升级可能使其失效）；`hermes doctor` 以 `hermes mcp test ladon` 的退出码判定；完整的 ST-076 在 WP16 之后执行 |
| RSK-22 | 人工专属是纵深防御而不是安全边界：本地模式无认证，同一用户下的任意代码可改写数据库或伪造终端 | 工具表不含人工专属工具、服务层拒绝、`verify` 回放、`-t ladon,clarify` 去掉终端与文件工具；手册明确该边界 |
| RSK-23 | 外传：Hermes 把工具结果发给远端模型；门禁依赖规则正确性与新鲜度（30 秒窗口、嵌套规则文件、元数据） | 失败即关闭、`local_only` 默认、版本级门禁与 `AgentView` 统一投影、全局忽略路径显式传给 `mcp-serve`、`hermes resume` 的披露检查、首次同意提示、IT-054、IT-071、IT-074、ST-083 |
| RSK-24 | 跨进程数据库锁：Windows 上 turso 的独占且立即失败的锁使控制台、`mcp-serve`、`watch`、GUI 偶发冲突 | 即用即弃连接加有界重试；长时间扫描仍可能耗尽重试而得到 `E_STORE_BUSY` |
| RSK-25 | turso 0.8.3 行为（影响行数、`BEGIN IMMEDIATE`、DDL、CHECK、外键）部分未验证 | WP01 探针在依赖它们之前记录事实；设计不依赖 CHECK 与外键，CAS 有读回确认 |
| RSK-26 | 只能依赖 CI 构建：一次编译错误消耗一个 CI 周期；GUI 在最后一步才编译 | 小包、纯模块先在通道 B 验证、内核 API 冻结测试 |
| RSK-27 | 质量门禁负载：16 个新受管源文件各需设计提交；项目整体覆盖率门禁不可通过 | 每包设计提交模板化；合并过小的模块；如实报告项目门禁；度量与 `main.rs` 的处理见 RSK-37 |
| RSK-28 | Windows 控制台行为只能在真实控制台观察：Ctrl+C、`Terminate batch job (Y/N)?`、作业嵌套、代码页与控制台模式、输入法 | 手工 ST-078～082；作业加入失败只是警告；启动器可配置 |
| RSK-29 | 锚点稳定性：标题改名改变键，围栏代码内的 `#` 被当作标题，非 Markdown/NEMA 文件只有 `ROOT` 锚点 | `body_hash` 识别改名、`ROW` 条款、显式状态与人工处理、绝不静默重定基；限制写入手册 |
| RSK-30 | 单文件方案是有意的范围限制，多文件改动需拆成多个任务 | 手册说明；任务调度识别写入冲突 |
| RSK-31 | 批准疲劳：人可能不读差异就键入任务号 | 展示统一差异、哈希绑定与风险标志；无法强制理解，记录为残余风险 |
| RSK-32 | 文档文本中的提示注入无法检测，只能收容 | 去掉 Agent 的能力：只能创建受配额限制的提案、批注、带标注的证据与待人审阅的方案 |
| RSK-33 | 旧接口 `set_task_status` 无限制，仍可使状态与链脱节 | `doc verify` 报告 `STATUS_DRIFT`；新代码不调用 |
| RSK-34 | 仓库根目录存在若干零字节的散落文件（疑为 shell 重定向误操作产物），可能被 `scan` 或打包拾取 | 经所有者同意后在首次提交前清理，不纳入提交 |
| RSK-35 | 有探索记录称 Hermes 启动器脚本中存在明文凭据赋值（此处不复述） | 所有者应视为已暴露并轮换；LADON 从不读取、复制、记录或转发启动器脚本及其环境 |
| RSK-36 | 向 `fix/compile-errors` 推送即触发 MSI 构建与完整测试，成本高 | 合并小包后再推送，推送前征得所有者同意 |
| RSK-37 | 覆盖率度量与 `verify` 流程在 WP00A 之前不存在，`store.rs`、`engine.rs`、`main.rs` 的既有覆盖率未知，`main.rs` 的遗留逻辑无法在进程内测试 | WP00A 先行并给出基线；WP00C 迁移遗留逻辑并补特征化测试（须所有者批准）；既有未覆盖行的补测工时在度量之后追加；未达标如实报告为阻断，不降低标准 |
| RSK-38 | Hermes 的会话转录留存在 Hermes 中，恢复会话会把此前返回的内容再次发给模型；LADON 不读取也不删除转录 | 披露记录先写后返回、策略指纹与逐文件复核、不满足时开启新会话；手册说明转录由所有者自行管理 |
| RSK-39 | 迁移前备份可能无法验证（`-wal` 成对一致性、复制失败），而迁移不可逆地增加表 | 备份在迁移内成对复制并按行数核对；不可验证时 004 不执行，需所有者确认并记录；004 是纯增量，0.0.3 忽略新表 |
| RSK-40 | CI 不运行 clippy（`rust-ci.yml` 的 fmt 步骤先失败），依赖 clippy 的守卫（模块级 `deny`、非 Windows 桩无警告）没有执行位 | 所有者择一建立 clippy 证据通道；其间报告“未验证”，stdout 守卫以 IT-062 与 IT-067 为准 |

## 7. 没有真实 Hermes 模型或可见窗口时无法验证的项
| 项 | 原因 | 手工用例 |
|---|---|---|
| 覆盖层与指令文本经本机启动器链透传 | 需要真实启动器与 Hermes | ST-076 |
| `-c`、`--create-if-missing`、`--query-file`、`--in`、`--cli` 在同一次交互启动中的组合 | 需要真实交互会话 | ST-076、ST-077 |
| MCP 子进程在发现窗口内就绪、Tool Search 确实关闭、工具以 `mcp__ladon__doc_*` 出现 | 需要真实 Hermes | ST-076 |
| `untrusted` 逐次确认的显示（含中文） | 需要真实 Hermes 与终端 | ST-077 |
| Hermes 是否对 MCP 结果脱敏、模型对 `needs_human` 指示的遵循程度 | 需要真实模型 | ST-077、ST-084 |
| Ctrl+C 在提示符处与 Hermes 内的行为 | 需要真实控制台 | ST-078 |
| `taskkill /F` 后无孤儿进程（嵌套作业） | 需要真实进程树 | ST-079 |
| `Terminate batch job (Y/N)?` 是否出现 | 需要真实批处理层 | ST-080 |
| cmd、Windows Terminal、conhost 中的中文输入输出 | 需要可见终端与输入法 | ST-081 |
| Hermes 异常退出后的控制台模式与编码恢复 | 需要真实控制台 | ST-082 |
| 敏感文件内容确实不发送给模型 | 需要真实模型 | ST-083 |
| `ladon shell-menu` 菜单入口（0x7003）与图标显示 | 需要交互桌面会话 | ST-088、ST-089 |
| Hermes 恢复会话时是否把此前的转录再次发给模型（披露检查的前提） | 需要真实模型与会话 | ST-077 |
CI 中的假启动器、桩 MCP 客户端与记录型 `ProcessRunner` 只证明 LADON 一侧的行为，不能替代以上用例；未执行的用例保持 Not Run，不得对外宣称已通过。

## 8. 文档任务版本的发布标准
1. WP00～WP20（含 WP00A、WP00B、WP00C）的验收全部通过，或其未通过项已由所有者决定并记录。
2. 发布提交上 `release.yml` 全绿（内核测试、GUI 测试、`ladon` 与 `ladon-gui` 发布构建、打包、MSI）。
3. 每个变更源文件的行覆盖率结果按 WP00A 确立的流程记录在案，严格高于 90% 才视为通过；不能达标的文件（`ui-prototype/src/main.rs`、两个 `build.rs`，以及 WP00C 未获批准时的 `src/main.rs`）报告为“阻断，待所有者决定”，发布说明不得把它们表述为通过；项目整体门禁的结论如实报告，不降低标准。
4. UAT-01、UAT-05 的自动化对应用例通过；ST-076～089 已执行并记录，或明确标为 Not Run 且在发布说明中如实说明。
5. 57、58 号文档与 11、36、41、42、43、46、53 号文档、`CHANGELOG.md`、`design/*.json` 一致，且只描述终态。
6. `Cargo.toml`、`Cargo.lock`、`ui-prototype/Cargo.lock` 无改动。
7. `ladon.exe` 与 `ladon-gui.exe` 的 MSI 与 zip 打包流程不变；GUI 的唯一变化来自 WP20，两个可执行文件的资源含 LADON 图标。
8. 仓库根目录的散落文件已清理，`git status` 除受管文件外为空。
9. 用户手册写明已知限制：本地无认证与同用户代码的边界、非 Markdown/NEMA 文件只有 `ROOT` 锚点、方案限于单文件、Hermes 的逐次确认不是 LADON 批准、GUI 文档任务页保持示例内容。
10. 版本号、标签、推送与发布由所有者决定，未经同意不执行。
11. 本版本不是 1.0：57 号文档第 15.3 节列出的仍未完成的 P0 在发布说明中如实列出，P1/P2 不宣传为现成功能。

## 9. 需要同步更新的既有文档
| 文档 | 更新内容 | 时机 |
|---|---|---|
| `11_文档治理业务要件.md` | 迁移表、人工专属操作、链定义、基线与锚点规则（终态） | WP00、WP19 |
| `36_数据库事务与变更策略.md` | 迁移 004、新路径的单事务写入、连接重试 | WP00、WP19 |
| `33_并行与任务详细设计.md` | 写入单位为文件、调度输出与冲突组 | WP19 |
| `40_WindowsShell菜单与隔离.md` | 0x7003 打开控制台（经 `ladon shell-menu` 宿主的菜单；MSI 无右键谓词） | WP18 |
| `41_UT详细设计.md`、`42_IT详细设计.md`、`43_ST详细设计.md`、`46_测试执行记录与缺陷.md` | 新增用例与执行记录，ST 保持 Not Run 直至执行 | WP00、WP19 |
| `53_用户操作说明.md` | `doc` 命令、控制台、批准流程、Hermes 启动、`hermes doctor`、限制 | WP19 |
| `design/requirements.json`、`design/testcases.json`、`design/nfrs.json`、追踪工作簿 | 新增条目与状态 | WP00、WP19 |
