> 设计基线候选 · 2026-11-24 · IPA 共通フレーム参照 · 尚待设计评审/签字。


# 57. 文档任务模块与 Hermes CLI 详细设计

## 1. 目的、范围与证据约定

### 1.1 目的
文档任务（文档即任务）是 LADON 的核心功能。本设计把它实现到可验收：用户在已批准要件的条款上建立批注与任务，任务经分析、方案、人工批准、受控写入、验证到完成，全过程写入防篡改审计链，并能从同一位置查看下游任务、Agent 变更、提交事件、测试证据与回滚链，导出 .NEMA 关系与审计报告（对应 `11_文档治理业务要件.md` 第 8 节完成判定）。

同时新增交互式命令行界面 `ladon console`。所有者在控制台内启动 Hermes Agent，Hermes 只通过 LADON 自带的 stdio MCP 服务器 `ladon mcp-serve` 操作文档任务。一次性命令 `ladon doc …`、控制台、MCP 服务器三个入口调用同一个服务层 `DocTaskService`，因此文档任务模块内不存在绕过审计与权限的写入路径（对应 REQ-031）。

### 1.2 范围内
| 编号 | 内容 |
|---|---|
| S1 | 已批准基线登记（按文件版本）、条款大纲、条款编号与选择器 |
| S2 | 批注与任务的数据模型、短编号（`T-12`、`N-5`）、存量数据兼容 |
| S3 | 任务状态机、人工批准规则、Agent 提案规则、防篡改治理事件链与校验 |
| S4 | 锚点、条款指纹、锚点迁移分类、人工确认与重定位、文件移动谱系确认 |
| S5 | 关联（设计/代码/测试）、证据、依赖、并行调度与写入目标冲突检测 |
| S6 | 方案提交、人工批准、人工应用（受控写入）、中断恢复、回滚链 |
| S7 | 审计报告（Markdown/JSON）与 .NEMA 导出 |
| S8 | 一次性命令行、MCP 服务器、交互控制台、Hermes 受控启动与自检 |
| S9 | Agent 外传门禁、敏感文件策略、环境变量与秘密保护 |
| S10 | 测试体系、需求—测试追溯 |

### 1.3 非目标
下列内容不属于本设计，在文档任务版本发布后另行设计与接线：左侧导航快速浏览、底部 AI 聊天栏、GUI 中的 Google Drive 链接、GUI 文档任务页接线（该页保持示例内容及其提示条）、聊天消息转任务（REQ-069）、涌现式建议落地（REQ-094、REQ-095）、Concept Guard 与 CR 对象、FileSpace 租约与多 Agent 运行时（内核只产出调度方案，不启动任何 Agent）、影响分析算法（只提供确定性的提及检索，影响判断由模型作出）、角色体系（`auth.rs` 不接入）、docx/pdf/xlsx 的内容改写、跨设备同步、跨多个文件的原子写入。文件图标与可执行文件图标不属于文档任务模块，其交付见 `58_文档任务与HermesCLI开发WBS.md` 的 WP20。

### 1.4 证据约定
| 标记 | 含义 |
|---|---|
| 〔事实 出处〕 | 在文件或命令输出中看到的内容，出处为 `文件:行` 或命令 |
| 〔假设〕 | 未经执行验证，由对应测试或手工检查确认 |
| 〔决定〕 | 本设计采用的规则 |

Hermes 一侧的源码出处以 `H:` 表示 `D:\hermes-multica\agent`（版本标签 v0.21.7）。未标注〔事实〕的 Hermes 行为按〔假设〕处理。

### 1.5 既有内核事实（设计前提）
| 事实 | 出处 |
|---|---|
| `Store::init` 只执行 001（每次）与 002（`schema_migrations` 无版本 2 时），版本行 2 最后写入；003 不被装入 | 〔事实 src/store.rs:31-51〕 |
| 迁移语句以 `;` 切分后逐条在同一连接上自动提交，无事务、无校验和；故新迁移文件不得含触发器、注释内分号或字符串内分号 | 〔事实 src/store.rs:34,44〕 |
| `Store::connect` 是私有异步函数；每次调用新建 turso `Database`，没有忙等待或重试 | 〔事实 src/store.rs:24-30〕 |
| `Engine::open` 不调用 `store.init`；GUI 同时调用 `Engine::open` 与 `Engine::init` | 〔事实 src/engine.rs:74-93；ui-prototype/src/app.rs:564,601〕 |
| 任务/批注 API 为 `add_annotation`、`create_task`、`set_task_status`、`list_tasks`、`governance_events`；状态只校验是否属于 7 个字符串，不校验迁移；记录与治理事件分两个连接写入；actor 固定为 `human` | 〔事实 src/store.rs:168-206〕 |
| `annotations` 表只有写入没有任何读取查询 | 〔事实 grep `FROM annotations` 在 src/ 无命中〕 |
| 每文件事件账本 `events` 的哈希元组固定为 12 项，只由 `capture`/`mark_deleted` 写入 | 〔事实 src/store.rs:92-129,223-226〕 |
| CLI 现有 `annotate`、`task` 两条创建命令，输出 `annotation id: <uuid>`、`task id: <uuid>`；基线版本取账本最新版本 | 〔事实 src/main.rs:145-154〕 |
| 章节键：Markdown `H<级>:<标题>#<n>`，NEMA `DOMAIN:<id>#<n>`，其余 `ROOT`；章节哈希包含标题行；围栏代码不识别 | 〔事实 src/diff.rs:9-44〕 |
| `.NEMA` 仅解析 `>` 有向图子集，节点标签被丢弃；`topo_batches` 以 `from` 为先行节点分层，环与悬空节点报错 | 〔事实 src/nema.rs:7-66〕 |
| 恢复路径含摘要核对、移开备份、不替换放置、回滚；`capture` 是版本的唯一入口 | 〔事实 src/engine.rs:9,31-69,134,252-336〕 |
| `content_is_excluded(hash)` 判定曾持有该内容的路径是否被规则排除；归档路径使用 `policy.allow_external` | 〔事实 src/engine.rs:200-207,214〕 |
| `AgentApi::allow_remote_read` 存在但没有生产调用方 | 〔事实 src/agent_api.rs:71；grep 在 src/ 无其他调用〕 |
| `google_oauth.rs` 已有手写 `extern "system"` 的 Windows FFI 及非 Windows 桩 | 〔事实 src/google_oauth.rs:415-519〕 |
| 依赖含 clap（derive、env）、serde_json、sha2、hex、uuid、chrono、similar、anyhow、tokio（rt-multi-thread、macros、time、fs、sync）、regex；无行编辑器、无 MCP 库 | 〔事实 Cargo.toml〕 |
| release 工作流在 windows-latest 上用 `--locked` 依次执行内核测试、GUI 测试、`ladon` 发布构建、`ladon-gui` 发布构建、打包与 MSI | 〔事实 .github/workflows/release.yml〕 |
| 全库没有 `ladon` 二进制层面的测试；`src/main.rs` 无测试 | 〔事实 grep CARGO_BIN_EXE 在 tests/ 无命中〕 |
| Hermes 启动链为两层 `.cmd` 与 `powershell -File rewrite-and-run.ps1` | 〔事实 D:\hermes-multica\home\bin\hermes.cmd、D:\hermes-multica\home\hermes.cmd〕 |
| `Engine::init` 调用 `Store::init`，`Engine::open` 不调用 | 〔事实 src/engine.rs:74-93〕 |
| `capture` 在内容哈希等于账本记录且未删除时返回状态 `unchanged` 与空 `version_id`；`ladon watch` 以执行者 `watcher` 捕获每次稳定保存，并在路径不是文件时调用 `mark_deleted` | 〔事实 src/engine.rs:139-141；src/main.rs:246-272〕 |
| 恢复路径以 `root.join(相对路径)` 构造目标且不先规范化，规范化与工作区断言发生在其后的 `capture` 内；暂存、备份文件名含随机 uuid | 〔事实 src/engine.rs:264,272〕 |
| 全局忽略规则文件只从环境变量 `LADON_GLOBAL_IGNORE` 读取，未设置即没有全局规则 | 〔事实 src/policy.rs:138-148〕 |
| `reload_policy` 以一次新的整库遍历替换规则集，`PendingPolicy::with` 等待遍历完成 | 〔事实 src/engine.rs:96-107,199-207；src/policy.rs:44-51〕 |
| 旧治理事件载荷：`annotation.created` 为批注全文，`task.created` 为标题，`task.state_changed` 为状态字符串 | 〔事实 src/store.rs:171,176,205〕 |
| 大纲与章节哈希以 `from_utf8_lossy` 读取字节、以 `lines()` 切行 | 〔事实 src/diff.rs:28,33〕 |
| turso 的预写日志文件为 `<库路径>-wal` | 〔事实 turso_core-0.8.3 database.rs:952〕 |
| `std::io::IsTerminal` 在 Windows 上除真实控制台外，对名称以 `msys-`、`cygwin-` 开头并以 `-pty` 结尾的设备也返回真 | 〔事实 本机 Rust 文档 std/io/trait.IsTerminal.html〕 |
| Hermes 只给 MCP 子进程过滤后的环境（基线键加服务器配置中的 `env`）；`isError` 结果被转成带 `error` 键的载荷并计一次熔断，JSON-RPC 错误作为异常同样计一次，无 `error` 键的普通结果使熔断计数清零 | 〔事实 H:tools/mcp_tool_config.py:144-204；H:tools/mcp_tool_handlers.py:150-155,537-538,598-599；H:tools/mcp_tool.py:525-535〕 |
| `HERMES_MANAGED_DIR` 在 Hermes 源码中标注为测试用的引导覆盖，其文档称该变量由管理员固定而不是由用户设置 | 〔事实 H:hermes_cli/managed_scope.py:48；H:website/docs/user-guide/managed-scope.md:45-62〕 |
| MSI 只登记开始菜单快捷方式与 `Installed` 注册表值，没有资源管理器右键谓词；“文档即任务”菜单项 `0x7003` 只出现在由 `ladon shell-menu <文件>` 经 `shell-host-internal` 宿主的菜单中 | 〔事实 packaging/windows/msi/LADON.wxs:29-68；src/main.rs:40,54-60；docs/03_detailed_design/40_WindowsShell菜单与隔离.md:10,46〕 |
| `rust-ci.yml` 的 ubuntu 作业先执行 `cargo fmt --check`（第 14 行），其后才是测试与 clippy（第 15-16 行）；`release.yml` 不含 clippy | 〔事实 .github/workflows/rust-ci.yml:14-16；.github/workflows/release.yml〕 |
| `src/main.rs` 含无法在进程内测试的遗留逻辑：`watch` 的阻塞循环、`google_login`、`tag_folder`、shell 宿主分支 | 〔事实 src/main.rs:184-244,246-280〕 |

## 2. 架构与模块布局

### 2.1 层次与原则
```
所有者 ── ladon console ──┐
所有者 ── ladon doc … ────┼─→ doctask_cli ─→ DocTaskService(Principal::Human) ─┐
Hermes ── ladon mcp-serve ─→ doctask_mcp ─→ DocTaskService(Principal::Agent) ──┤
                                                                               ├─→ DocStore（全部 SQL，一次变更一个事务）
                                                                               └─→ Engine（文件、CAS、策略、版本）
```
〔决定〕
1. `DocTaskService` 是文档任务数据的唯一读写入口；三个适配层不含业务规则，`main.rs` 只含对 `entry` 的一次调用，`entry.rs` 中每个新增子命令是一次库调用。
2. 主体（Principal）由入口决定，任何命令参数、工具参数都不能设置主体。
3. 纯逻辑模块（不引用 `store`、`engine`）与内核相关模块分离，纯模块不依赖 turso 即可编译与单元测试。
4. 全部新增代码零新增依赖：`Cargo.toml`、`Cargo.lock`、`ui-prototype/Cargo.lock` 均不改动。

### 2.2 模块表
| 模块（`src/` 下） | 类别 | 职责 | 工作包 |
|---|---|---|---|
| `doctask_model.rs` | 纯 | 状态、主体、迁移表与守卫输入、错误码 `DocError`、记录类型、句柄与选择器解析、哈希函数、限制与文本校验、`schedule_batches` | WP02 |
| `doctask_outline.rs` | 纯 | 条款大纲（键与 `diff::section_hashes` 一致）、条款编号、虚拟条款、锚点分类、章节文本与拼接 | WP03 |
| `doctask_render.rs` | 纯 | 中英文案目录、CJK 显示宽度、表格、各屏幕文本、错误渲染 | WP09 |
| `doctask_report.rs` | 纯 | 审计报告（Markdown/JSON）、.NEMA 写出与回读校验 | WP10 |
| `doctask_store.rs` | 内核 | `DocStore`：Immediate 事务、计数器、全部新表的 SQL、事件链追加与校验、存量回填 | WP05 |
| `doctask_service.rs` | 内核 | 服务核心：`open`、`attach`、授权表、外传门禁与 `AgentView` 投影、基线、批注、任务创建与编辑、读取视图、旧命令兼容路径；人工专属操作的服务签名带展示时的 `expected_*` | WP06 |
| `doctask_flow.rs` | 内核 | 迁移 T1～T11 与批准（含只分析方案的提交与批准）、提案、依赖、关联、证据、配额、收件箱、调度、`verify`；持有可选的 `EditPlanSupport` 组件，缺省时只接受只分析方案 | WP07 |
| `doctask_reconcile.rs` | 内核 | 锚点对账、确认、重定位、重定基、文件移动谱系 | WP08 |
| `doctask_plan.rs` | 内核 | 编辑方案：目标校验、字节级章节拼接、差异预览、应用、恢复；实现 `EditPlanSupport` 并在 `DocTaskService` 构造时注册 | WP14 |
| `entry.rs` | 内核 | 命令入口：`Cli` 与 `Command` 的 clap 定义、全部命令（遗留与新增）的分派、遗留逻辑（`watch`、`google_login`、`tag_folder`、shell 宿主分支）及其可注入的依赖；`main.rs` 只含对 `entry` 的一次调用 | WP00C，其后 WP11、WP13、WP16、WP17、WP18 增量修改 |
| `doctask_cli.rs` | 内核 | `ladon doc` 命令树、命令注册表（名称、中文别名、是否人工专属）、退出码、旧命令路由；只分析方案的提交、批准、驳回在 WP11，编辑方案选项、`apply`、`recover`、`--diff` 在 WP14 | WP11、WP14 |
| `mcp.rs` | 纯 | JSON-RPC 2.0 与 MCP 生命周期、`ToolHost` 特征、行读取与限长 | WP12 |
| `doctask_mcp.rs` | 内核 | 14 个工具的定义（WP13 交付 13 个，`doc_plan_submit` 由 WP14 追加）、参数解码、服务调用、`serve()`、`CatchPanic` 适配器、panic 钩子 | WP13、WP14 |
| `winconsole.rs` | 纯（含 FFI） | 控制台模式保存与恢复、VT 开启、Ctrl+C 门、kill-on-close 作业、真实控制台判定（`GetConsoleMode`）；非 Windows 为空桩 | WP15 |
| `hermes.rs` | 内核 | 设置、启动器解析、覆盖层与种子写出、环境、命令行、预检、`ProcessRunner`、doctor | WP16 |
| `console.rs` | 内核 | 控制台 REPL、分词、收件箱与批准流程、Hermes 交接 | WP17 |

既有文件的改动：`src/store.rs`（`ensure_schema` 与迁移前备份、可见性为 `pub(crate)` 且带有界重试与可注入打开器的 `connect`、`file_by_id`；`init` 不变）、`src/engine.rs`（`remote_read_allowed`、`remote_version_allowed`、`refresh_policy_if_stale`、`commit_text`）、`src/policy.rs`（`PendingPolicy` 的就绪构造与就绪查询、遍历计数）、`src/main.rs`（WP00C 缩减为对 `entry` 的一次调用，此后不改动）、`src/lib.rs`（`pub mod` 行）、新增 `sql/004_document_tasks.sql`。`diff.rs`、`nema.rs`、`workspace.rs`、`model.rs`、`agent_api.rs` 不改动。

### 2.3 GUI 不变量
〔决定〕不修改 `ui-prototype/` 下任何文件（文件图标包 WP20 除外，其改动范围见 58 号文档）。`ui-prototype/src/app.rs` 使用的内核签名保持原样：`list_files`、`mark_deleted`、`version`、`find_path`、`versions`、`verify_chain`、`events`、`capture`、`archive_version`、`restore`、`reload_policy`、`load_version`、`cas.has`、`Engine::open`、`Engine::init`，以及 `PendingVersion`、`CaptureResult` 的全部公开字段（二者在多处以结构体字面量构造，新增字段会破坏编译）。GUI 编译只发生在 release 工作流的最后构建步骤，测试步骤使用 `--no-default-features --features kernel`，因此以 `tests/it_kernel_api_freeze.rs`（只编译、按 GUI 的参数与返回类型调用上述每个方法）在内核测试步骤中提前暴露签名破坏。

### 2.4 进程模型
| 进程 | 说明 |
|---|---|
| 控制台进程 | 所有者交互。命令在进程内执行；每条命令通过 `Store` 即用即弃的连接访问数据库，不缓存 turso `Database`，因此启动 Hermes 时不持有数据库句柄 |
| Hermes 进程 | 由控制台以继承的标准输入输出启动，控制台阻塞等待其退出 |
| `ladon mcp-serve` 进程 | 由 Hermes 作为 stdio 子进程启动；主体固定为 Agent；首次工具调用时才打开 `Engine` |
| 第二终端 | 所有者可另开终端运行 `ladon console` 或 `ladon doc inbox --watch`、`ladon doc approve`，与 MCP 子进程并发写入 |

〔事实 turso_core-0.8.3 io/windows_lock.rs:281-310〕Windows 上打开数据库文件时取得独占且立即失败的跨进程锁，同一进程内的多次打开共享一个带引用计数的锁，因此锁错误只可能来自另一个进程；第二个进程得到 `Locking error: Failed locking file. File is locked by another process`（`LimboError::LockingError`，其 `Display` 前缀为 `Locking error:`）。该错误经 `turso` 映射为哪个 `turso::Error` 变体〔假设〕未验证，由 WP01 的跨进程探针记录；规则因此按文本与变体两路判定。〔决定〕`Store::connect` 对 `turso::Error::Busy` 与文本含 `locked by another process` 的错误做有界重试：最多 6 次，退避从 25 ms 倍增到 400 ms，每次退避加 0～50% 的随机抖动（使竞争的对端不会与重试者同步），最后一次失败时错误信息指明其他进程占用并映射为 `E_STORE_BUSY`（退出码 7）。其余错误不重试。`connect` 经可注入的 `Opener` 特征打开数据库（生产实现为 turso `Builder`），单元测试以假打开器重复返回锁错误文本来验证重试次数、退避与映射，只有跨进程并发测试（IT-069）使用子进程。

### 2.5 主体与身份
| 入口 | 主体 | 名称 |
|---|---|---|
| 一次性命令、控制台 | Human | `--actor`，否则环境变量 `LADON_ACTOR`，否则 `USERNAME`，否则 `owner`（自由文本，本地模式没有登录） |
| `ladon mcp-serve` | Agent | 固定为 `hermes`，会话为 `hermes:<launch_id>` |
| 对账、回填、迁移、过期方案的系统迁移（T11） | System | 固定为 `ladon`；对账、回填、迁移只在控制台与一次性命令入口落库，`ladon mcp-serve` 不执行 |

人工专属操作（第 4.2 节）除主体检查外还要求人在场证明（`HumanProof`）：标准输入与标准输出都是真实 Windows 控制台，判定为 `winconsole.rs` 中的 `GetConsoleMode` 对两个句柄都成功；`std::io::IsTerminal` 对 msys、cygwin 的伪终端管道也返回真（第 1.5 节），故只用于颜色判断，不用于人在场证明与预检。测试中以注入的 `Presence` 替身代替。〔事实 doc 41 第 2 节：本地模式无登录〕人在场证明是纵深防御而不是安全边界：同一 Windows 用户下的任意代码仍可改写数据库（第 13.5 节）。

## 3. 数据模型与迁移

### 3.1 迁移机制
〔决定〕
1. `Store::init` 保持原样：001 每次执行，002 在 `schema_migrations` 无版本 2 时执行并最后写入版本行；003 是控制面文件，不装入，故本地新增编号为 4。迁移 004 只由新增的 `Store::ensure_schema` 应用，`Engine::init`（`ladon init`、GUI 的磁盘初始化）与 `Engine::open` 因此从不应用 004，也就从不在没有迁移前备份的情况下改动数据库。已记录的版本永远不重复执行（`tc112` 语义保持），版本行 4 在 004 全部语句成功之后最后写入。
2. `Store::ensure_schema(policy)`：一次 `SELECT` 判断版本 4 是否已记录，已记录则立即返回；否则先经既有 `init` 路径确保版本 1、2，再执行迁移前备份（第 5 项），备份通过验证（或所有者已按第 5 项确认）后逐条执行 004。`DocTaskService::open` 与 `doc migrate` 调用它并随后执行回填。`Engine::open`、`Engine::init`、`Store::init` 的行为与签名不变，GUI 不受影响。
3. 004 只含 `CREATE TABLE IF NOT EXISTS` 与 `CREATE [UNIQUE] INDEX IF NOT EXISTS`，没有注释、触发器、`ALTER TABLE`、CHECK 约束、部分索引、公用表表达式与窗口函数；取值域由 Rust 校验，完整性由 `doc verify` 复核。
4. `ladon mcp-serve` 从不迁移、不回填、不补建 `meta`、不把对账结果落库：Schema 版本低于 4 时，`tools/call` 以普通结果 `E_SCHEMA_OUTDATED` 回复且进程不退出（第 10.1 节）。服务层因此有两个构造函数：`DocTaskService::open`（`ensure_schema` 加回填，供一次性命令与控制台）与 `DocTaskService::attach`（只检查 Schema 版本，供 `mcp-serve`；其对账与 `meta` 视图只在内存中计算；唯一的读时写入是第 13.1 节的披露记录，其余写入只来自第 10.4 节所列的 Agent 变更工具）。
5. 迁移前备份在 `ensure_schema` 内、第一条 004 语句之前执行且只执行一次（版本 4 未记录且库中已有数据行时）：在本进程持有数据库打开锁期间（此时没有其他进程可写）把 `meta.turso` 与其预写日志 `meta.turso-wal`（存在时）成对复制为 `.ladon/backup/meta.turso.pre-004` 与 `.ladon/backup/meta.turso.pre-004-wal`，再打开该副本核对 `resources`、`versions`、`doc_tasks`、`annotations`、`governance_events` 的行数与原库一致。核对通过才继续迁移。无法核对（复制失败、副本打不开、行数不符）时不执行 004：控制台与 `doc` 入口向所有者展示原因并要求确认；确认后迁移继续，并在迁移完成后的 `dt.migration.applied` 事件载荷中记录 `backup_verified:false` 与原因；一次性命令在非真实控制台上返回 `E_MIGRATION_BACKUP`，`doc migrate --accept-unverified-backup`（人工专属，需要真实控制台）是显式确认的非交互形式。新建的空库没有数据行，不需要备份。〔假设〕在本进程持有打开锁、没有其他写入者时复制所得的库文件与日志成对一致，由 IT-073 在真实临时库上验证。
6. 回滚到 0.0.3 二进制无害：它只读写旧表。此后用旧二进制写入的状态由 `doc verify` 以 `STATUS_DRIFT` 与 `LEGACY_NEW` 报告，`ladon doc migrate --import-legacy` 重新执行幂等回填。

〔假设〕turso 0.8.3 的 `execute` 返回受影响行数、`BEGIN IMMEDIATE` 可用、`INSERT OR IGNORE` 可用、唯一索引违反在插入时以约束错误失败；`AUTOINCREMENT` 序号在被回滚的插入之后是否仍连续、跨进程锁错误对应的 `turso::Error` 变体均未验证。WP01 的探针测试（IT-048）把这些观测结果记录到测试设计中。CAS 更新以影响行数为 1 为准，并在同一事务内读回状态作为第二重确认，因此不依赖影响行数这一行为的单独可靠性。DDL 在事务内的行为未验证，迁移保持逐条自动提交。

### 3.2 `sql/004_document_tasks.sql`
```sql
CREATE TABLE IF NOT EXISTS doc_counters (
 name TEXT PRIMARY KEY,
 value INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS doc_baselines (
 id TEXT PRIMARY KEY,
 file_id TEXT NOT NULL REFERENCES resources(id),
 version_id TEXT NOT NULL REFERENCES versions(id),
 version_no INTEGER NOT NULL,
 content_hash TEXT NOT NULL,
 approver TEXT NOT NULL,
 approver_kind TEXT NOT NULL,
 note TEXT NOT NULL DEFAULT '',
 created_at TEXT NOT NULL,
 revoked_at TEXT,
 revoked_by TEXT);
CREATE INDEX IF NOT EXISTS idx_doc_baselines_file ON doc_baselines(file_id, created_at);
CREATE INDEX IF NOT EXISTS idx_doc_baselines_version ON doc_baselines(file_id, version_id);
CREATE TABLE IF NOT EXISTS doc_anchors (
 id TEXT PRIMARY KEY,
 file_id TEXT NOT NULL REFERENCES resources(id),
 base_version_id TEXT NOT NULL,
 baseline_id TEXT NOT NULL DEFAULT '',
 section_key TEXT NOT NULL,
 clause_id TEXT NOT NULL DEFAULT '',
 title TEXT NOT NULL DEFAULT '',
 level INTEGER NOT NULL DEFAULT 0,
 line_start INTEGER NOT NULL DEFAULT 0,
 line_end INTEGER NOT NULL DEFAULT 0,
 full_hash TEXT NOT NULL DEFAULT '',
 body_hash TEXT NOT NULL DEFAULT '',
 state TEXT NOT NULL DEFAULT 'valid',
 current_version_id TEXT NOT NULL,
 current_key TEXT NOT NULL,
 current_hash TEXT NOT NULL DEFAULT '',
 reviewed_version_id TEXT NOT NULL DEFAULT '',
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL,
 UNIQUE (file_id, base_version_id, section_key));
CREATE INDEX IF NOT EXISTS idx_doc_anchors_file ON doc_anchors(file_id, state);
CREATE TABLE IF NOT EXISTS doc_task_meta (
 task_id TEXT PRIMARY KEY REFERENCES doc_tasks(id),
 task_no INTEGER NOT NULL UNIQUE,
 anchor_id TEXT NOT NULL REFERENCES doc_anchors(id),
 owner TEXT NOT NULL DEFAULT '',
 priority INTEGER NOT NULL DEFAULT 2,
 proposal_state TEXT NOT NULL DEFAULT 'accepted',
 proposal_decided_by TEXT NOT NULL DEFAULT '',
 proposal_decided_at TEXT,
 visibility TEXT NOT NULL DEFAULT 'shared',
 source_kind TEXT NOT NULL DEFAULT 'cli',
 source_ref TEXT NOT NULL DEFAULT '',
 created_by TEXT NOT NULL,
 created_by_kind TEXT NOT NULL,
 session TEXT NOT NULL DEFAULT '',
 last_approval_id TEXT NOT NULL DEFAULT '',
 cancel_reason TEXT NOT NULL DEFAULT '',
 updated_at TEXT NOT NULL,
 closed_at TEXT);
CREATE INDEX IF NOT EXISTS idx_doc_task_meta_anchor ON doc_task_meta(anchor_id);
CREATE INDEX IF NOT EXISTS idx_doc_task_meta_proposal ON doc_task_meta(proposal_state);
CREATE TABLE IF NOT EXISTS doc_annotation_meta (
 annotation_id TEXT PRIMARY KEY REFERENCES annotations(id),
 annotation_no INTEGER NOT NULL UNIQUE,
 anchor_id TEXT NOT NULL REFERENCES doc_anchors(id),
 author TEXT NOT NULL,
 author_kind TEXT NOT NULL,
 rationale TEXT NOT NULL DEFAULT '',
 visibility TEXT NOT NULL DEFAULT 'shared',
 quote TEXT NOT NULL DEFAULT '',
 line_start INTEGER NOT NULL DEFAULT 0,
 line_end INTEGER NOT NULL DEFAULT 0,
 session TEXT NOT NULL DEFAULT '',
 created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_doc_annotation_meta_anchor ON doc_annotation_meta(anchor_id);
CREATE TABLE IF NOT EXISTS doc_task_deps (
 task_id TEXT NOT NULL REFERENCES doc_tasks(id),
 depends_on TEXT NOT NULL REFERENCES doc_tasks(id),
 created_by TEXT NOT NULL,
 created_at TEXT NOT NULL,
 PRIMARY KEY (task_id, depends_on));
CREATE INDEX IF NOT EXISTS idx_doc_task_deps_rev ON doc_task_deps(depends_on);
CREATE TABLE IF NOT EXISTS doc_task_events (
 seq INTEGER PRIMARY KEY AUTOINCREMENT,
 id TEXT NOT NULL UNIQUE,
 task_id TEXT NOT NULL DEFAULT '',
 file_id TEXT NOT NULL DEFAULT '',
 subject_kind TEXT NOT NULL,
 subject_id TEXT NOT NULL DEFAULT '',
 kind TEXT NOT NULL,
 from_status TEXT NOT NULL DEFAULT '',
 to_status TEXT NOT NULL DEFAULT '',
 actor TEXT NOT NULL,
 actor_kind TEXT NOT NULL,
 session TEXT NOT NULL DEFAULT '',
 claimed_session TEXT NOT NULL DEFAULT '',
 payload_json TEXT NOT NULL,
 timestamp TEXT NOT NULL,
 prev_hash TEXT NOT NULL,
 hash TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_doc_task_events_task ON doc_task_events(task_id, seq);
CREATE INDEX IF NOT EXISTS idx_doc_task_events_file ON doc_task_events(file_id, seq);
CREATE UNIQUE INDEX IF NOT EXISTS idx_doc_task_events_prev ON doc_task_events(prev_hash);
CREATE TABLE IF NOT EXISTS doc_task_plans (
 id TEXT PRIMARY KEY,
 task_id TEXT NOT NULL REFERENCES doc_tasks(id),
 plan_no INTEGER NOT NULL,
 summary TEXT NOT NULL,
 rationale TEXT NOT NULL DEFAULT '',
 impact TEXT NOT NULL DEFAULT '',
 risk TEXT NOT NULL DEFAULT 'medium',
 options_json TEXT NOT NULL DEFAULT '[]',
 chosen_option TEXT NOT NULL DEFAULT '',
 edit_kind TEXT NOT NULL DEFAULT '',
 target_file_id TEXT NOT NULL DEFAULT '',
 target_section TEXT NOT NULL DEFAULT '',
 new_text TEXT NOT NULL DEFAULT '',
 reads_json TEXT NOT NULL DEFAULT '[]',
 expected_version_id TEXT NOT NULL DEFAULT '',
 expected_hash TEXT NOT NULL DEFAULT '',
 result_hash TEXT NOT NULL DEFAULT '',
 plan_hash TEXT NOT NULL,
 status TEXT NOT NULL,
 submitted_by TEXT NOT NULL,
 submitted_by_kind TEXT NOT NULL,
 session TEXT NOT NULL DEFAULT '',
 created_at TEXT NOT NULL,
 decided_at TEXT,
 applied_at TEXT,
 applied_version_id TEXT NOT NULL DEFAULT '',
 UNIQUE (task_id, plan_no));
CREATE TABLE IF NOT EXISTS doc_task_approvals (
 id TEXT PRIMARY KEY,
 task_id TEXT NOT NULL REFERENCES doc_tasks(id),
 kind TEXT NOT NULL,
 decision TEXT NOT NULL,
 approver TEXT NOT NULL,
 approver_kind TEXT NOT NULL,
 plan_id TEXT NOT NULL DEFAULT '',
 subject_hash TEXT NOT NULL,
 base_version_id TEXT NOT NULL,
 comment TEXT NOT NULL DEFAULT '',
 event_id TEXT NOT NULL,
 created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_doc_task_approvals_task ON doc_task_approvals(task_id, created_at);
CREATE TABLE IF NOT EXISTS doc_task_links (
 id TEXT PRIMARY KEY,
 task_id TEXT NOT NULL REFERENCES doc_tasks(id),
 link_kind TEXT NOT NULL,
 target_kind TEXT NOT NULL,
 target_file_id TEXT NOT NULL DEFAULT '',
 target_section TEXT NOT NULL DEFAULT '',
 target_ref TEXT NOT NULL DEFAULT '',
 label TEXT NOT NULL DEFAULT '',
 created_by TEXT NOT NULL,
 created_by_kind TEXT NOT NULL,
 session TEXT NOT NULL DEFAULT '',
 created_at TEXT NOT NULL,
 removed_at TEXT);
CREATE INDEX IF NOT EXISTS idx_doc_task_links_task ON doc_task_links(task_id);
CREATE TABLE IF NOT EXISTS doc_task_evidence (
 id TEXT PRIMARY KEY,
 task_id TEXT NOT NULL REFERENCES doc_tasks(id),
 kind TEXT NOT NULL,
 result TEXT NOT NULL,
 source TEXT NOT NULL,
 ref TEXT NOT NULL DEFAULT '',
 version_id TEXT NOT NULL DEFAULT '',
 content_hash TEXT NOT NULL DEFAULT '',
 summary TEXT NOT NULL,
 actor TEXT NOT NULL,
 actor_kind TEXT NOT NULL,
 session TEXT NOT NULL DEFAULT '',
 event_id TEXT NOT NULL,
 created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_doc_task_evidence_task ON doc_task_evidence(task_id, created_at);
CREATE TABLE IF NOT EXISTS doc_file_lineage (
 id TEXT PRIMARY KEY,
 old_file_id TEXT NOT NULL REFERENCES resources(id),
 new_file_id TEXT NOT NULL REFERENCES resources(id),
 evidence TEXT NOT NULL,
 score_permille INTEGER NOT NULL,
 state TEXT NOT NULL DEFAULT 'proposed',
 decided_by TEXT NOT NULL DEFAULT '',
 decided_at TEXT,
 created_at TEXT NOT NULL,
 UNIQUE (old_file_id, new_file_id));
CREATE TABLE IF NOT EXISTS doc_agent_sessions (
 launch_id TEXT PRIMARY KEY,
 hermes_slug TEXT NOT NULL,
 hermes_exe TEXT NOT NULL,
 focus_task_no INTEGER,
 started_by TEXT NOT NULL,
 started_at TEXT NOT NULL,
 ended_at TEXT,
 exit_code INTEGER,
 policy_fp TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS doc_agent_disclosures (
 launch_id TEXT NOT NULL REFERENCES doc_agent_sessions(launch_id),
 file_id TEXT NOT NULL REFERENCES resources(id),
 version_id TEXT NOT NULL DEFAULT '',
 content_hash TEXT NOT NULL DEFAULT '',
 first_at TEXT NOT NULL,
 PRIMARY KEY (launch_id, file_id, version_id));
CREATE INDEX IF NOT EXISTS idx_doc_tasks_file ON doc_tasks(file_id, status);
CREATE INDEX IF NOT EXISTS idx_doc_tasks_status ON doc_tasks(status);
CREATE INDEX IF NOT EXISTS idx_annotations_file ON annotations(file_id, section_key);
```
旧表 `doc_tasks`、`annotations`、`governance_events` 不修改：任务状态仍只存于 `doc_tasks.status`（7 个旧字符串），新表以 `task_id`、`annotation_id` 为键附加属性。`graph_edges`、`chat_task_links`、`embeddings` 不使用。

### 3.3 取值域（由 Rust 校验）
| 列 | 取值 |
|---|---|
| `doc_tasks.status` | `open`、`analysis`、`approval`、`working`、`verification`、`done`、`cancelled` |
| `actor_kind`、`*_kind` | `human`、`agent`、`system`、`unknown`（存量数据） |
| `proposal_state` | `accepted`、`proposed`、`dismissed` |
| `source_kind` | `cli`、`console`、`hermes`、`legacy` |
| `doc_anchors.state` | `valid`、`needs_review`、`relocated`、`ambiguous`、`orphaned`、`unknown`、`file_missing`、`unchecked` |
| `visibility`（批注与任务） | `shared`、`local_only` |
| 方案 `status` | `submitted`、`approved`、`applying`、`applied`、`conflicted`、`rejected`、`superseded` |
| 方案 `edit_kind` | 空串（只分析）、`replace_section`、`insert_after_section` |
| 方案 `risk` | `low`、`medium`、`high` |
| 批准 `kind` / `decision` | `plan`、`result` / `approved`、`rejected` |
| 证据 `kind` | `test_run`、`report`、`note`、`version`、`commit`、`agent_change`、`waiver` |
| 证据 `result` / `source` | `pass`、`fail`、`info` / `ledger`、`reported` |
| 关联 `link_kind` | `design`、`detail`、`code`、`ut`、`it`、`st`、`doc`、`target`、`other` |
| 关联 `target_kind` | `file`、`file_section`、`git_commit`、`url`、`task` |
| 谱系 `evidence` / `state` | `same_hash`、`section_overlap` / `proposed`、`confirmed`、`rejected` |
| 优先级 | 1、2、3（默认 2） |

### 3.4 编号与句柄
任务号 `task_no`、批注号 `annotation_no` 在工作区内单调递增、唯一，由 `doc_counters`（行 `task_no`、`annotation_no`）在分配所在事务内递增。句柄写作 `T-12`、`N-5`；人工主体的任务参数也接受 `12`、完整 UUID、至少 8 位且唯一的十六进制前缀，歧义时返回 `E_AMBIGUOUS` 并列出编号候选。Agent 主体只接受 `T-n`、`N-n` 与 `doc_file_find` 返回的 `file_ref`；Agent 的未找到、被门禁拒绝、`local_only` 一律返回同一个 `E_NOT_FOUND`，不附任何候选列表（第 13.1 节）。

### 3.5 存量数据回填
`DocStore::backfill_v4` 只由 `DocTaskService::open` 与 `doc migrate --import-legacy` 调用，在一个 Immediate 事务内执行，幂等：以 `NOT EXISTS` 判重，最后写入 `doc_counters` 行 `backfill_v4`；按 `(created_at, id)` 排序，结果确定。
| 步骤 | 内容 |
|---|---|
| 锚点 | 对 `doc_tasks.anchor_section` 与 `annotations.section_key` 的每个不同的 `(file_id, base_version_id, section)` 建 `doc_anchors`，状态 `unchecked`，指纹为空，`current_version_id` 取基线版本，`title`/`level` 从形如 `H<级>:<标题>#<n>` 的键解析（`ROOT`：级 0，标题“（全文）”） |
| 任务 | 无 meta 的 `doc_tasks` 行分配 `task_no`，`source_kind=legacy`，`created_by=human`，`created_by_kind=unknown`（旧 actor 固定为 `human` 且未经验证），`proposal_state=accepted`，`visibility=local_only`（失败即关闭：旧标题与说明可能是私人内容，所有者用 `doc edit T-n --share` 公开）；`doc_tasks.status` 不改 |
| 批注 | 无 meta 的 `annotations` 行分配 `annotation_no`，`author=human`、`author_kind=unknown`、`visibility=local_only`（失败即关闭：旧文本可能是私人内容，所有者用 `doc note-share` 公开） |
| 治理事件 | 未导入的 `governance_events` 行按 `seq` 顺序追加为 `legacy.<原 kind>` 链事件，事件 id 为 `legacy:<原事件 id>`（以此判重），`actor_kind=unknown`，原时间戳保留。载荷以白名单保留原文：只有 `task.state_changed` 且载荷恰为 7 个状态字符串之一时以 `{"legacy":true,"payload":"<状态>"}` 写入并同时写入该事件的 `to_status`；`annotation.created`、`task.created` 与任何清单外的种类（含载荷不是合法状态的 `task.state_changed`），载荷一律为 `{"legacy":true,"sha256":"<原内容的 SHA-256>","bytes":<字节数>}`，原文只留在 `annotations`、`doc_tasks` 行内并受 `visibility` 约束。事件链只增不改，写入的私人文本无法撤回，所以明文清单是白名单；未知历史标记为未知，不编造 |
| 任务状态 | 每个任务在其 legacy 事件之后追加一条 `dt.task.imported`（`actor_kind=unknown`，`to_status` 取 `doc_tasks.status`）；此后每次导入（`doc migrate --import-legacy`）若某任务最后一条带状态的事件的 `to_status` 不等于 `doc_tasks.status`（例如回滚到 0.0.3 使用后再升级），再追加一条。`dt.task.imported` 与 `legacy.task.state_changed` 是状态回放的重置点（第 4.9 节） |
旧存储 API 之后创建的行（测试、旧二进制）没有 meta：`DocTaskService::open` 在首次读取时按同样规则补建（`ensure_meta`，落库只发生在控制台与一次性命令入口），`attach` 不补建，无 meta 的行对 Agent 不可见；`doc verify` 以 `LEGACY_NEW` 提示。旧批注和任务的完整字节与章节表不回填；指纹由首次对账在字节可读时补上，字节为冷（已逐出本地）时保持 `unchecked`，不编造。

### 3.6 读取模型
`list`、`show`、`trace` 以 `doc_tasks` 连接 `doc_task_meta` 并附带锚点、最新方案、批准、证据、关联、依赖。任务视图包含：`T-n`、UUID、文件（路径）、章节、条款编号、基线版本、锚点状态、状态、创建者与类型、负责人、优先级、提案状态、时间、`base_stale`（文件最新版本不等于基线版本）、`file_state`（`live` 或 `missing`，取 `resources.deleted`）、阻塞者（未完成的先行任务）。面向 Agent 的视图一律经 `AgentView` 投影（第 13.1 节），门禁拒绝或 `local_only` 的任务整体省略，其在 `blocked_by` 中的引用只以 `hidden` 计数出现。

## 4. 任务状态机、角色与治理事件

### 4.1 状态与角色类
状态沿用 7 个旧字符串：`open`、`analysis`、`approval`、`working`、`verification`、`done`、`cancelled`。等待阶段：`open`（等待认领分析）与 `approval`（等待人工决定）；`verification` 等待人工验收。`done` 与 `cancelled` 是终态，只有人工专属的重开（T10）能离开。

角色类：**H** 人（需人在场证明）、**A** Agent（MCP，固定主体）、**S** 系统（LADON 对已验证事实的反应，例如对账）。

### 4.2 迁移表
所有迁移在一个 Immediate 事务内完成：读取当前状态、检查迁移表与守卫、`UPDATE doc_tasks SET status=新 WHERE id=? AND status=旧` 并要求影响 1 行（否则 `E_CONFLICT`）、更新 `doc_task_meta.updated_at`、写入附带行、追加链事件。表中未列出的迁移一律 `E_TRANSITION_DENIED`，错误信息列出调用者当前可做的迁移。
| 编号 | 迁移 | 执行者 | 守卫 | 附带写入 | 事件 |
|---|---|---|---|---|---|
| T1 | open → analysis | H、A | `proposal_state=accepted`；锚点状态不是 `orphaned`、`file_missing`、`ambiguous` | — | `dt.task.transitioned` |
| T2 | analysis → approval | H、A，仅随方案提交 | 同一事务内提交方案（可为只分析方案）；方案字段合法；编辑方案另须通过 `EditPlanSupport` 校验（第 8.1 节），未注册该组件时只接受只分析方案 | 方案行（`submitted`），旧的 `submitted` 方案变 `superseded` | `dt.plan.submitted`、`dt.task.transitioned` |
| T3 | approval → working | **H** | 见 4.4 | 批准行（`plan`/`approved`）；方案 `approved`；`last_approval_id` | `dt.approval.recorded`、`dt.task.transitioned` |
| T4 | approval → analysis（驳回） | **H** | 理由必填 | 批准行（`rejected`）；方案 `rejected` | `dt.approval.recorded`、`dt.task.transitioned` |
| T5 | working → verification | H、A | 方案若修改文件则必须是 `applied`；摘要非空 | 证据行（`note`，来源 `reported`） | `dt.task.transitioned` |
| T6 | verification → done | **H** | 见 4.5 | 批准行（`result`/`approved`）；`closed_at` | `dt.approval.recorded`、`dt.task.transitioned` |
| T7 | verification → working | H；A 仅当最新证据 `result=fail` | 理由必填 | 人工执行时写批准行（`result`/`rejected`，说明为理由） | `dt.task.transitioned` |
| T8 | working → analysis（重做方案） | H、A | 理由必填；`applying` 状态的方案存在时拒绝 | 已批准未应用的方案变 `superseded`；已应用的保持 `applied` | `dt.task.transitioned` |
| T9 | open、analysis、approval、working、verification → cancelled | **H** | 理由必填；存在 `applying` 方案时拒绝（先 `doc recover`） | 方案 `superseded`；`cancel_reason`、`closed_at` | `dt.task.transitioned` |
| T10 | done、cancelled → open | **H** | 理由必填 | 清 `closed_at` | `dt.task.reopened` |
| T11 | approval → analysis（方案过期） | **S** | 仅在 T3 守卫 4 判定方案过期时，与方案 `superseded` 同一事务 | 方案 `superseded` | `dt.plan.superseded`、`dt.task.transitioned`（`actor_kind=system`，原因 `stale_plan`） |
| P1 | 采纳提案（`proposed` → `accepted`） | **H** | 任务为 Agent 创建 | `proposal_decided_by/at` | `dt.proposal.accepted` |
| P2 | 忽略提案 | **H** | 任务为 `proposed` | 状态 → `cancelled`，`proposal_state=dismissed`，理由入 `cancel_reason` | `dt.proposal.dismissed`、`dt.task.transitioned` |
Agent 可调用的迁移只有 T1、T2（随方案提交）、T5、T7（最新证据为失败时）、T8；T11 只由系统在 T3 守卫 4 失败时执行；其余全部是人工专属。人工专属还包括：基线登记与撤销、批准与驳回、方案应用与恢复决定、任务属性（负责人、优先级、可见性）编辑、批注公开与取消公开、锚点确认、重定位与重定基、谱系确认与拒绝、导出报告到文件。人工专属操作的服务签名都带展示时的 `expected_*`（第 9.5 节）。

### 4.3 提案
Agent 创建的任务 `proposal_state=proposed`，在人工采纳之前不能离开 `open`，也不进入调度；每个启动（launch）最多保留 10 个未决提案，每 10 分钟最多 40 次变更调用，二者从 `doc_task_events` 按会话计数，进程重启后仍然有效，超限返回 `E_QUOTA`、`E_RATE_LIMIT`。人工忽略提案的理由作为负反馈保存在 `cancel_reason`。提案未被采纳时，Hermes 对该任务的 `doc_task_transition`、`doc_plan_submit` 得到普通结果 `E_TRANSITION_DENIED`，提示“请所有者采纳该提案”（`owner must adopt`）；因此从 Hermes 提出任务到提交方案需要两次交接：所有者先在控制台采纳，再让 Hermes 经 `doc_task_list` 在已采纳的任务上继续（第 11.4 节）。

### 4.4 批准（T3）守卫
1. 方案 `status=submitted` 且为该任务编号最大的方案；调用方传入的 `expected_subject_hash` 等于该方案的 `plan_hash`（控制台、命令行都以展示时的哈希绑定，展示后内容改变则 `E_STALE_SUBJECT`）。
2. 所有先行任务（依赖）都是 `done`；`cancelled` 的先行任务不满足依赖，所有者需移除依赖或重开。
3. 决策时对账（第 5.5 节）：锚点已绑定基线（`baseline_id` 非空），状态是 `valid`，或 `needs_review`、`relocated`、`unknown` 且 `reviewed_version_id` 等于当前版本（已经 `doc ack` 或 `doc relocate`）；`orphaned`、`ambiguous`、`file_missing` 一律拒绝。拒绝返回 `E_ANCHOR_BROKEN` 并提示 `ack`、`relocate`、`rebase`。`relocated` 与 `needs_review` 受同样约束，自动重定位不构成确认。
4. 方案修改文件时：目标文件账本最新版本等于 `expected_version_id`，且文件不超过 8 MiB 时其现存字节的摘要等于 `expected_hash`；否则在同一事务内提交方案 `superseded` 与 T11（任务回到 `analysis`，使新方案可以提交），并返回 `E_STALE_PLAN`。这是唯一一处返回错误但状态已改变的路径：事务提交而不是回滚，一次性命令以退出码 6 结束，控制台提示“方案已过期，任务已回到分析阶段”。
5. 批准人主体为 Human 且人在场证明成立；批准界面要求键入任务号。

### 4.5 验收（T6）守卫
1. 至少一条证据满足：`kind` 为 `test_run`、`report`、`note`，`result=pass`，且其后没有 `result=fail` 的证据，并且由人工主体写入（`actor_kind=human`）；Agent 写入的通过证据不满足该守卫。所有者也可以显式接受 Agent 证据：`doc close T-n --accept-agent-evidence <序号[,序号]>`（序号为 `doc evidence list T-n` 中的 `#k`），所接受证据的 id 写入批准载荷的 `accepted_agent_evidence`；或给出书面豁免（`--waive 理由`），写入 `waiver` 证据并把 `evidence_waived` 写入事件载荷。`version`、`commit`、`agent_change` 是变更证据，展示但不单独满足验收。
2. 所有先行任务 `done`。
3. 锚点状态可接受（同 4.4 第 3 点，`relocated` 须已确认）。
4. `expected_subject_hash` 等于 `evidence_hash`（方案哈希与排序后的证据 id 的 SHA-256），人读证据期间新增证据会迫使重新阅读。
5. 报告者标注：Agent 写入的证据在所有视图中标注“hermes 报告，未核验”；`version` 证据在写入时核对版本存在、属于该任务的文件且版本号大于基线版本（来源 `ledger`）。

### 4.6 与文件账本的关系
每文件事件账本 `events` 只由文件变化步骤写入：`capture`、`restore`，以及 `doc apply` 内的 `capture`。任务、批准、基线、锚点从不写入 `events`（`events.version_id` 为非空外键，哈希元组固定）。任务到版本的链接由系统证据行（`commit`、`agent_change`）与 `dt.commit.applied` 事件承担；版本到任务的可追溯性由两部分承担：`doc apply` 写入的 capture 原因前缀 `[T-12] 方案1 批准人 <名称>: <摘要，至多 120 字>`，以及权威链接——系统证据行 `commit` 与事件 `dt.commit.applied`（载荷含任务号、方案号、版本 id、账本事件 id）。`capture` 返回 `unchanged`（`ladon watch` 或 GUI 已先捕获同一字节）时没有新的 capture 原因，前缀缺失但权威链接仍成立（第 8.3 节）。

### 4.7 治理事件链
〔决定〕工作区一条链：`doc_task_events`。`hash` 为 `serde_json::to_vec` 序列化下列元组的 SHA-256，创世 `prev_hash` 为 64 个 `0`，`payload_json` 使用存储的文本原样参与，不重新序列化：
`(prev_hash, id, task_id, file_id, subject_kind, subject_id, kind, from_status, to_status, actor, actor_kind, session, claimed_session, payload_json, timestamp)`
元组一经发布冻结，由金标测试锁定。链尾在追加所在的同一 Immediate 事务内读取；`prev_hash` 上的唯一索引使两个事件承接同一链尾时后一个插入失败，写入方重读链尾并重试（至多 3 次）。`session` 是权威会话（`hermes:<launch_id>`），`claimed_session` 是模型自称的 Hermes 会话 id（来自 `--pass-session-id`），仅记录不信任。批注事件的载荷只含批注文本的 SHA-256 与长度，不含文本本身。链是无签名的防篡改证据：对整库的重写无法阻止（`31_事件与版本算法.md` 第 3 节）。

### 4.8 事件种类
| 种类 | 载荷要点 |
|---|---|
| `dt.baseline.registered`、`dt.baseline.revoked` | 版本号、内容哈希、说明、理由；同一版本在撤销后重新登记时，载荷带 `replaces_revoked`（被撤销的基线 id） |
| `dt.task.imported`、`dt.migration.applied` | 回填时的状态与来源；迁移版本、`backup_verified`、未验证的原因、各表行数 |
| `dt.annotation.created`、`dt.annotation.visibility_changed` | 批注号、锚点、可见性、文本哈希与长度 |
| `dt.task.created`、`dt.task.edited` | 任务号、锚点、标题的 SHA-256 与长度、来源、提案状态、可见性；变更字段只记新旧值的 SHA-256 与长度（任务文本可能是 `local_only`，链上不留明文） |
| `dt.task.transitioned`、`dt.task.reopened` | 理由、`evidence_waived`、`applied_version_id` |
| `dt.proposal.accepted`、`dt.proposal.dismissed` | 决定人、理由 |
| `dt.plan.submitted`、`dt.plan.superseded` | 方案号、`plan_hash`、`edit_kind`、目标 |
| `dt.approval.recorded` | 批准 id、种类、决定、`subject_hash`、说明 |
| `dt.dep.added`、`dt.dep.removed` | 先行任务 |
| `dt.link.added`、`dt.link.removed` | 关联种类与目标 |
| `dt.evidence.added` | 种类、结果、来源、引用 |
| `dt.commit.started`、`dt.commit.applied`、`dt.commit.conflicted`、`dt.commit.aborted` | 期望哈希、结果哈希、暂存与备份文件的路径（由方案 id 派生，见第 8.3 节）；`applied` 另含任务号、方案号、版本 id、账本事件 id，以及实际执行捕获的执行者与原因（`capture` 返回 `unchanged` 时为先行捕获者的值） |
| `dt.anchor.reconciled`、`dt.anchor.acknowledged`、`dt.anchor.relocated`、`dt.anchor.rebased` | 状态前后、键前后、方法 |
| `dt.lineage.proposed`、`dt.lineage.confirmed`、`dt.lineage.rejected` | 新旧文件、证据、分数 |
| `dt.session.started`、`dt.session.ended` | 启动 id、会话名、退出码 |
| `dt.export.generated` | 种类、路径、输出的 SHA-256 |
| `legacy.<种类>` | 回填的旧治理事件 |

### 4.9 校验（`doc verify`）
| 检查 | 级别 | 内容 |
|---|---|---|
| `CHAIN_BROKEN` | 错误 | 重算哈希与 `prev_hash` 链接不一致，指出首个失败的 `seq` |
| `SEQ_GAP` | 警告 | `seq` 不连续；只有 IT-048 的探针证明被回滚的插入不留空洞后才升为错误 |
| `BASELINE_DUPLICATE` | 错误 | 同一 `(file_id, version_id)` 存在两条未撤销的基线 |
| `HUMAN_ONLY_FORGED` | 错误 | 人工专属种类（批准、基线、采纳、应用、重开等）的 `actor_kind` 不是 `human`；T11 的 `dt.task.transitioned` 由 `system` 写入，不属于人工专属 |
| `ILLEGAL_TRANSITION` | 错误 | 按第 4.2 节迁移表回放，出现不合法迁移；回放对每个任务从其最近一个重置点（`dt.task.imported` 或 `legacy.task.state_changed`）起算，没有重置点时从 `open` 起算，T11 以 System 执行合法 |
| `STATUS_DRIFT` | 错误 | `doc_tasks.status` 与最后一条带状态事件（迁移、重开、导入重置点）的 `to_status` 不一致（捕获旧二进制或 `set_task_status` 的绕过） |
| `APPROVAL_ORPHAN` | 错误 | 批准行没有对应事件，或事件没有批准行 |
| `COMMIT_UNANCHORED` | 错误 | 应用证据的版本 id 不在 `versions`，或其账本事件不存在 |
| `PLAN_INCONSISTENT` | 错误 | 方案状态与事件不一致、`applying` 方案没有 `dt.commit.started` |
| `DEP_CYCLE` | 错误 | 存储的依赖存在环 |
| `LEGACY_NEW` | 警告 | 旧存储 API 在迁移后新增、尚无 meta 的行 |
| `LEGACY_UNAPPROVED` | 警告 | 存量任务处于 `approval`、`working`、`verification`、`done` 却没有批准行 |
| `ANCHOR_UNREVIEWED` | 警告 | 锚点处于 `needs_review`、`relocated`、`ambiguous`、`orphaned` 且未确认 |
| `SESSION_UNENDED` | 警告 | 启动记录没有结束记录（控制台被强制关闭） |
`ladon doc verify [T-n]` 打印 `OK` 或首个错误；存在错误级发现时退出码 9。

## 5. 锚点、条款指纹与锚点迁移

### 5.1 条款大纲
`doctask_outline.rs` 从字节与扩展名计算大纲，不修改 `diff.rs`。
| 项 | 规则 |
|---|---|
| 标题条款 | `md`、`markdown`、`nema`、`nemat`：键与 `diff::section_hashes` 完全一致（`H<级>:<标题>#<n>`、`DOMAIN:<id>#<n>`、`ROOT`），语料测试断言逐键相等 |
| 其他扩展名 | 单条 `ROOT` 条款，哈希为整份内容的章节哈希 |
| 条款编号 | 标题文本、表格行首单元格、列表项首词中，匹配 `^[A-Za-z][A-Za-z0-9]*-[0-9]+` 的第一个记号；标题末尾的 `{#编号}` 优先；选择器不区分大小写 |
| 虚拟条款 | 首单元格以条款编号开头的 Markdown 表格行，以及首词为条款编号的列表项，键为 `ROW:<编号>#<n>`（n 为同一编号的出现序号），行范围为该行，`full_hash` 与 `body_hash` 均为该行文本加换行的 SHA-256 |
| 指纹 | `full_hash` 等于 `diff::section_hashes` 的章节哈希（含标题行）；`body_hash` 为去掉标题行后其余行的 SHA-256（行以 `\n` 连接并以 `\n` 结尾，口径与 `diff.rs` 一致），标题改名可由 `body_hash` 识别 |
| 大纲条目 | `#序号`（文档顺序）、键、条款编号、标题、级别、起止行、两种哈希、批注数、任务数 |
已知限制（文档化并有测试固定）：围栏代码块内以 `# ` 开头的行被当作标题（与 `diff.rs` 一致）；Setext 标题不识别；CRLF 与 LF、缺失末尾换行不改变章节哈希；无效 UTF-8 在读取与大纲中按有损方式处理，但不能作为方案目标（第 8.1 节）；UTF-8 BOM 留在首行，首行标题因此不被识别为标题（与 `diff.rs` 一致）；修改标题文本会改变键；`ROW` 条款不可作为方案的编辑目标。

### 5.2 选择器
`#N`（大纲序号）、条款编号、`key:<精确键>`、标题文本（精确匹配，否则唯一子串匹配）。未指定时取 `ROOT`。无匹配返回 `E_SECTION_NOT_FOUND`，多个匹配返回 `E_AMBIGUOUS` 并附编号候选。

### 5.3 锚点创建
锚点以 `(file_id, base_version_id, section_key)` 为身份，由同一文件版本上的所有任务与批注共享。`doc new`、`doc note` 与 MCP 创建工具：
1. 文件已被跟踪且未删除（`E_FILE_MISSING`）。
2. 文件存在未撤销的基线，否则 `E_NO_BASELINE`，提示先执行 `doc baseline add`。
3. 在最新的未撤销基线版本的字节上解析选择器并写入指纹；该版本字节已不在本地（冷）时返回 `E_ANCHOR_BROKEN`，提示重新登记基线。
4. 同一锚点上标题相同且未终结的任务、同一锚点上文本哈希相同的批注，返回已有记录（`existing=true`），不重复创建。
旧命令 `ladon annotate`、`ladon task` 不要求基线：基线存在时绑定基线版本，否则绑定账本最新版本；章节串能在该版本字节中找到时写入指纹并标 `valid`，找不到或字节不可读时按旧行为接受并标 `unchecked`，不拒绝。无基线绑定（`baseline_id` 为空）的锚点在列表中带标志 `U`，批准前需经 `doc rebase` 绑定基线。

### 5.4 对账分类
`reconcile(anchor, 最新版本大纲)` 按顺序判定：
| 顺序 | 条件 | 结果状态 | `current_key` |
|---|---|---|---|
| 1 | 文件已删除且没有已确认的谱系 | `file_missing` | 不变 |
| 2 | 最新版本字节为冷（不在本地） | `unknown` | 不变 |
| 3 | 锚点指纹为空（`unchecked`）且基线版本字节可读 | 先以基线版本补全指纹，再继续 4–7 | — |
| 4 | 键存在且 `full_hash` 相同 | `valid`（`current_version_id` 前进） | 同键 |
| 5a | 键存在且 `full_hash` 不同，最新大纲中恰有一个其他键的条目 `full_hash` 与锚点相同（例如在前面插入同名章节，使原章节后移为 `#2`） | `relocated`（方法 `full_hash`） | 该条目键 |
| 5b | 键存在且 `full_hash` 不同，有多个其他键的条目 `full_hash` 相同 | `ambiguous` | 不变 |
| 5c | 键存在且 `full_hash` 不同，没有其他键的条目 `full_hash` 相同 | `needs_review`（绝不自动重定基） | 同键 |
| 6 | 键不存在，且最新大纲中恰有一个标题条款条目 `body_hash` 相同，并且该正文达到重定位下限（至少 2 个非空行，且去除空白后至少 40 个字符） | `relocated`（方法 `body_hash`） | 该条目键 |
| 7 | 键不存在，且多个条目 `body_hash` 相同，或唯一相同的条目正文未达重定位下限（如 `TBD`、单行表格行） | `ambiguous` | 不变 |
| 8 | 键不存在且没有匹配 | `orphaned` | 不变 |
`ROOT`-only 文件只有 4 与 5c 两种结果；`ROW` 虚拟条款不做 `body_hash` 重定位（其正文含条款编号）。自动重定位只是建议：`relocated`、`needs_review`、`unknown` 与 `ambiguous`、`orphaned` 一样，须由人工 `doc ack` 或 `doc relocate` 确认后才可批准、验收、应用（第 4.4、4.5 节）。每次状态或键变化追加 `dt.anchor.reconciled`（前后状态、前后键、方法 `same_hash`/`full_hash`/`body_hash`/`none`）。对账幂等：`current_version_id` 已等于最新版本时不读取字节，不产生重复事件。

### 5.5 对账时机
1. 控制台启动时对存在未终结锚点的文件对账并落库（System 主体）。
2. `list`、`show`、`trace`、`outline` 读取前，文件最新版本 id 不等于锚点 `current_version_id` 时对账（咨询性：失败只标 `unknown`）。控制台与一次性命令落库；`mcp-serve` 的对账只在内存中计算视图，不写 `doc_anchors` 与链事件，落库留给下一次控制台或一次性命令的读取。
3. 决策时必须对账：批准（T3）、验收（T6）、应用（第 8.3 节）；对账失败或结果不可接受时阻断，这是正确性的依据，不依赖任何捕获期钩子（本设计不修改 `Engine::capture`）。

### 5.6 人工处理
| 命令 | 作用 | 事件 |
|---|---|---|
| `doc ack T-n` | 把 `reviewed_version_id` 设为 `current_version_id`，使 `needs_review`、`relocated`、`unknown` 锚点恢复到可批准、可验收、可应用的状态（`relocated` 同样必须确认）；不改 `base_version_id`。服务签名带展示时的 `expected_version_id` 与章节 `expected_hash`，事务内复核，不符 `E_STALE_SUBJECT`；并比较现存文件摘要与 `resources.current_hash`，不符 `E_FILE_CHANGED`（提示先 `capture`） | `dt.anchor.acknowledged` |
| `doc relocate T-n 选择器` | `orphaned`、`ambiguous` 锚点人工指定最新版本中的条款，状态记为 `relocated` 并同时确认；服务签名带展示时的 `expected_version_id` 与所选条款的 `expected_hash`，不符 `E_STALE_SUBJECT` | `dt.anchor.relocated` |
| `doc rebase T-n` | 把任务重新钉到当前未撤销基线版本：该键必须存在于基线版本大纲；建立新锚点并更新 `doc_task_meta.anchor_id`；服务签名带展示时的基线版本 id 与哈希（`E_STALE_SUBJECT`），并比较现存文件摘要与 `resources.current_hash`（`E_FILE_CHANGED`） | `dt.anchor.rebased` |
新基线登记不改动任何既有锚点：旧批注与旧任务继续指向旧基线，新批注钉在新基线。`doc anchors [文件] --check` 输出锚点迁移报告：每个锚点对最新版本与当前基线的状态、键前后、方法。

### 5.7 文件移动谱系
文件身份是相对路径，外部移动表现为旧路径标记删除加新路径新建；任务与批注保持指向旧 `file_id`，内核不静默合并（`11_文档治理业务要件.md` 第 6 节）。`doc lineage scan` 是显式命令：
| 候选来源 | 条件 | `evidence` |
|---|---|---|
| 旧文件 | `resources.deleted=1` 且至少有一个锚点 | — |
| 新文件 | 未删除，当前内容哈希等于旧文件最后版本的内容哈希 | `same_hash` |
| 新文件 | 最新大纲与旧文件最后一个字节可读的版本，相同 `full_hash` 的条目占比不低于 800‰ | `section_overlap` |
候选写入 `doc_file_lineage`（`proposed`，事件 `dt.lineage.proposed`）。`confirm`（人工专属；服务签名带展示时的旧、新文件 id 与新文件当前内容哈希，不符 `E_STALE_SUBJECT`）后，列表与对账把旧 `file_id` 解析为新文件（链深度不超过 8，无环）；`reject` 后同一对不再被提议。`Store::mark_deleted` 不修改，`file_missing` 由读取时的 `resources.deleted` 推导。

## 6. 关联、证据、依赖、报告与 .NEMA 导出

### 6.1 关联
`doc link add` 把任务关联到设计、详细设计、代码、UT/IT/ST、文档、写入目标或其他对象：
| `target_kind` | 目标 | 校验 |
|---|---|---|
| `file`、`file_section` | 已跟踪文件（及章节选择器） | 文件存在；章节存在于最新版本 |
| `git_commit` | 提交哈希 | 十六进制 7–64 位 |
| `url` | 地址 | 仅 `http`、`https`，至多 300 字符 |
| `task` | 另一任务 | 任务存在 |
`link_kind=target` 表示任务打算修改的文件，用于方案提交前的写入冲突预判。Agent 可添加关联；移除关联（`removed_at`）是人工专属。关联取代未使用的 `graph_edges` 表。

### 6.2 证据
| `kind` | 写入者 | 来源 | 规则 |
|---|---|---|---|
| `test_run`、`report`、`note` | H、A | `reported` | 任务处于 `working` 或 `verification`；`result` 为 `pass`、`fail`、`info`；Agent 写入的在所有视图标注“hermes 报告，未核验”，其通过结果不满足 T6 守卫，除非所有者以 `--accept-agent-evidence` 显式接受（第 4.5 节） |
| `version` | H、A | `ledger` | 版本必须存在、属于任务文件、版本号大于基线版本，内容哈希取自账本 |
| `commit`、`agent_change` | 系统（`doc apply`） | `ledger` | 记录新版本 id 与账本事件 id；方案由 Agent 提交时另写 `agent_change` |
| `waiver` | H（验收） | `reported` | 书面豁免，理由写入事件载荷 |
证据只增不改。

### 6.3 依赖
`depends_on` 是先行任务，对应 `nema::topo_batches` 边的 `from`。添加依赖在同一事务内以全部存储依赖加新边运行 `topo_batches`：成环返回 `E_DEP_CYCLE`，目标不存在返回 `E_DEP_DANGLING`，自依赖返回 `E_INVALID_ARG`。Agent 只能在创建任务的同一调用里为它自己创建的任务声明至多 10 个先行任务；已有任务的依赖增删是人工专属。

### 6.4 .NEMA 导出
`doc export nema [--task T-n | --file 文件 --section 选择器 | --all]` 生成两个域：` \>1 1`（追溯域）与 ` \>2 1`（任务 DAG 域）；节点全局唯一。
| 项 | 规则 |
|---|---|
| 节点记号 | `C-<锚点 id 前 8 位十六进制>`、`T-<任务号>`、`F-<文件 id 前 8 位十六进制>`；前缀碰撞时延长到 12 位；记号只含 `[A-Za-z0-9-]`，不含空格、`<`、`>`、`=` |
| 节点行 | `记号 标签`；标签中制表符、U+3000、换行替换为空格，至多 80 字符，空标签写 `-` |
| 边行 | `A>E<n>>B 标签`，边 id 在文件内唯一 |
| 边标签 | 条款→任务 `Traces`；任务→设计、详细设计 `Refines`；任务→代码 `Implements`；任务→UT/IT/ST `VerifiedBy`；任务→写入目标、文档、其他 `Targets`；先行任务→依赖任务 `Blocks` |
| 校验 | 文本在写出前以 `nema::parse_graph_edges` 解析、以 `topo_batches` 分层；任一失败返回 `E_INTERNAL` 且不写文件 |
只使用受支持的 `>` 有向边。

### 6.5 审计报告
`doc export report [--task T-n | --file 文件 --section 选择器 | --all] [--format md|json] [--out 路径]`，内容与顺序：
1. 摘要与链校验结果；2. 条款与批注；3. 任务与状态时间线；4. 批准（批准人、类型、绑定哈希）；5. 方案与差异统计；6. 提交与账本事件（版本 id、账本事件 id、哈希）；7. 测试证据（带来源标注）；8. 关联；9. 依赖图；10. 回滚链（目标文件的版本序列与 `ladon restore <版本ID> --approve` 命令）；11. 异常（对账状态、未结束会话）；12. 来源未知的存量数据（单独成节并标注）。
规则：报告取一个明确的数据截止点 `cutoff_seq`，即链中除 `dt.export.generated` 之外的最大 `seq`；摘要中的校验结果、链头哈希（`cutoff_seq` 处事件的哈希）与全部事件时间线都只计算到该截止点，并在报告首部写明；`dt.export.generated` 事件不进入报告内容。因此数据不变时，同一 `--generated-at` 得到逐字节相同的输出，连续导出两次也相同；默认不包含 `local_only` 批注与任务的文本以及门禁拒绝文件的正文（`--include-local-only`、`--include-sensitive` 显式开启）；默认输出到 `<工作区>/.ladon/exports/<种类>-<yyyymmdd-hhmmss>.<扩展名>`（scan 与 watch 跳过 `.ladon`），`--out` 经临时文件改名写出，无 `--force` 不覆盖，拒绝 `.ladon` 下的其他路径；每次导出追加 `dt.export.generated`（路径与输出的 SHA-256）。

## 7. 并行运行规划与写入目标冲突检测

〔决定〕内核只产出调度方案，不启动任何 Agent（FileSpace 租约与多 Agent 运行时不在本设计内）。
1. 任务集合：状态为 `open`、`analysis`、`approval`、`working` 且 `proposal_state=accepted` 的任务。已 `done` 的先行任务的边丢弃；先行任务为 `cancelled`、`proposed` 或不在集合内时，该任务列入 `blocked` 并给出原因。
2. 分层：以任务 id 为节点、依赖为边调用 `nema::topo_batches`；环返回 `E_DEP_CYCLE`。
3. 写入目标：取任务最新的非 `superseded`、非 `rejected` 方案的目标文件；没有方案时取该任务 `target` 关联的文件；两者都没有则无已知目标，列入 `unknown_targets`。
4. 冲突：同一批次内写入目标相同的任务组成冲突组，按任务号串行，其余并行。独占写入的单位是文件：写同一文件不同章节的两个任务也串行。无依赖不保证无冲突，该保证只对已声明的写入目标成立。
5. 输出：`{batches:[["T-1","T-2"],["T-3"]], conflicts:[{file:"…", tasks:["T-1","T-2"]}], blocked:[{task,reason}], unknown_targets:["T-5"]}`，顺序确定。命令 `doc schedule`、MCP 工具 `doc_schedule_get` 同源；Agent 通道经 `AgentView` 投影（第 13.1 节）：门禁拒绝文件上的任务、`local_only` 任务不出现在 `batches`、`conflicts`、`blocked`、`unknown_targets` 中，被省略的数量以顶层 `hidden` 计数给出，冲突组也不暴露被拒文件的路径。
6. 运行期保护独立于调度：第二个写同一文件的方案在批准时因 `expected_hash` 与目标账本版本不符而失效（`E_STALE_PLAN`），应用时的摘要守卫再次拒绝（`E_CONFLICT`）且不写入；批准界面在同一文件另有已批准方案时给出提示。
UAT-05 的自动化对应用例：三个任务，两个互相独立，另一个与其中一个写同一文件，结果为同一批次内一个冲突组加一个并行任务。

## 8. 方案、批准与受控写入

### 8.1 方案
方案是一个文件的一处章节编辑（多文件工作拆成多个任务，各自一个目标文件）：
| 字段 | 说明 |
|---|---|
| `summary`（≤500）、`rationale`（≤4000）、`impact`（≤2000）、`risk` | 对应 `ChangeProposal` 的影响与风险 |
| `options_json`、`chosen_option` | 备选方案文本数组（至多 5 项）；批准人在批准时可选定一项 |
| `edit_kind` | 空串＝只分析方案；`replace_section`；`insert_after_section` |
| `target_file_id`、`target_section`、`new_text`（≤262144 字节） | 目标必须是 `md`、`markdown`、`nema`、`nemat` 文件的标题条款，大小不超过 1 MiB，字节须通过严格的 `std::str::from_utf8`；UTF-8 BOM 允许并原样保留；含 NUL、UTF-16（`FF FE`、`FE FF` 开头）、无效 UTF-8、`ROW` 条款与其他类型返回 `E_UNSUPPORTED_FILE`，超限返回 `E_PLAN_TOO_LARGE`；`new_text` 先把 `\r\n` 规范为 `\n`，其后仍含 `\r` 或其他控制字符（换行、制表符除外）返回 `E_INVALID_ARG`。目标文件不要求有基线（基线只约束任务所属条款） |
| `reads_json` | 已声明的读取文件及提交时的版本（来自 `read_refs` 参数，服务端解析） |
| `expected_version_id`、`expected_hash`、`result_hash` | 由服务端根据账本最新版本与现存文件计算：`expected_version_id` 是账本最新版本，`expected_hash` 取账本 `current_hash` 且须等于该版本字节的摘要，结果字节由对 `expected_version_id` 的字节拼接得到，`result_hash` 为其 SHA-256；绝不信任模型提供的哈希。`result_hash` 等于 `expected_hash`（方案不改变文件）的编辑方案在提交时返回 `E_INVALID_ARG` |
| `plan_hash` | `serde_json::to_vec` 序列化 `(task_id, plan_no, summary, edit_kind, target_file_id, target_section, new_text, expected_hash, result_hash)` 的 SHA-256 |
章节拼接在原始字节缓冲上进行，不经过 `String` 重组：以每行起始的字节偏移确定章节范围，范围之外的字节（含每一行自己的换行符、混合换行、BOM、缺失的末尾换行）逐字节保持不变；范围内用 `new_text` 替换，其换行统一为目标文件的主导换行（全文 CRLF 多于裸 LF 时为 CRLF，否则 LF），替换范围的末行带换行符时 `new_text` 也以换行结尾，末行无换行符（文件末尾）时不补。`insert_after_section` 在目标章节末行之后插入，该末行没有换行符时先为它补一个主导换行（预览差异中可见，是范围外唯一的字节变化）。方案提交后不可修改，修改即提交新方案（`plan_no` 递增，旧的 `submitted` 方案变 `superseded`）。与最新 `submitted` 方案 `plan_hash` 相同的重复提交返回已有方案。方案由 Agent 提交时，目标文件须通过外传门禁（第 13.1 节）。

### 8.2 批准
批准（T3）绑定展示给人的 `plan_hash`，守卫见第 4.4 节。批准只改变任务与方案状态，不写文件。批准界面显示：任务与条款、目标文件与章节、统一差异（由 `expected_version_id` 的字节与结果字节以 `similar` 计算；`expected_version_id` 的字节须已核对其摘要等于 `expected_hash`，该版本字节已不在本地时无法预览，批准以 `E_STALE_PLAN` 拒绝并提示重新提交方案）、`expected_hash` 与 `result_hash` 的短哈希、风险与影响、同一文件上的其他已批准方案、依赖与锚点状态。目标章节与基线是否一致单独显示：目标文件没有基线时标明“无基线”，否则比较基线版本与 `expected_version_id` 中该章节的 `full_hash`。因此人看到的差异正是 `doc apply` 将替换的字节与将写入的字节；WP14 的测试把所显示的补丁应用于 `expected_version_id` 的字节，断言结果与结果字节、与 `commit_text` 实际写入的字节逐字节相等。

### 8.3 应用（`doc apply T-n`，人工专属）
前置：任务 `working`，方案 `approved` 且为最新，人在场证明成立，键入任务号。
| 步骤 | 内容 |
|---|---|
| A1 事务 | 决策时对账目标锚点（`relocated` 等须已确认）；方案 `approved` → `applying`；事件 `dt.commit.started`，载荷记录暂存与备份文件的路径：`.ladon/stage/<plan_id>.stage` 与 `.ladon/stage/<plan_id>.backup`（由方案 id 派生，不含随机部分，恢复时按名称定位） |
| A2 复核 | 以 `expected_version_id` 的字节重算拼接结果，其 SHA-256 必须等于 `result_hash`（这些字节也是批准界面差异的左侧）；现存文件摘要必须等于 `expected_hash`；不符则方案 `conflicted`，事件 `dt.commit.conflicted`，返回 `E_CONFLICT` |
| A3 写入 | `Engine::commit_text(相对路径, expected_hash, 结果字节, "doctask-apply", 原因, 暂存路径, 备份路径)`。先规范化目标：对目标的父目录取规范路径并断言位于工作区根之下，并断言从根到目标的每个路径分量都不是重解析点（junction、符号链接），否则拒绝，不做任何移动；其后摘要守卫、在 `.ladon/stage` 暂存（scan、watch 跳过 `.ladon`）、把原文件移开并复核备份摘要、以不替换既有文件的方式放置、调用 `capture`；放置或记录失败时按恢复路径同样的方式回滚。`restore` 的现有代码与测试保持不变，`commit_text` 与其共用 `digest_file`、`place_without_replacing` 等私有辅助函数，规范化与重解析点检查是 `commit_text` 自己的前置步骤。原文件移开期间目标路径短暂不存在，`ladon watch` 可能把它记为已删除并在放置后抢先捕获；`capture` 返回 `unchanged` 时，A4 在账本中查找 `content_hash` 等于 `result_hash`、版本号大于 `expected_version_id` 且属于目标文件的最新版本，以它作为 `applied_version_id`，并把该版本实际的捕获执行者与原因记入 `dt.commit.applied` 载荷；找不到这样的版本则按 `E_INTERNAL` 失败，方案保持 `applying` 由 `doc recover` 收敛 |
| A4 事务 | 方案 `applying` → `applied`，写 `applied_version_id`；证据行 `commit`（及 `agent_change`）；事件 `dt.commit.applied` |
原因格式：`[T-12] 方案1 批准人 <名称>: <摘要，至多 120 字>`；`capture` 因竞争而没有产生新的原因时，任务号与方案号仍记录在 `dt.commit.applied` 载荷与 `commit` 证据中。冲突后任务保持 `working`，需要 T8 回到 `analysis` 重做方案。应用成功后任务仍在 `working`，由 T5 推进（要求方案 `applied`）。

### 8.4 恢复（`doc recover`，人工专属）
处理状态为 `applying` 的方案（A1 之后任一步骤被中断）：
| 现存状态（目标路径、`dt.commit.started` 记录的备份路径、账本） | 处理 | 事件 |
|---|---|---|
| 目标摘要等于 `expected_hash`（尚未写入） | 方案回到 `approved`，清理 `.ladon/stage` 中本方案的暂存与备份 | `dt.commit.aborted` |
| 目标不存在，备份存在且摘要等于 `expected_hash`（移开原文件后中断） | 以不替换既有文件的方式把备份放回目标；若 `ladon watch` 已把文件记为已删除，随后以执行者 `doctask-recover` 执行 `capture` 使账本与文件一致；方案回到 `approved` | `dt.commit.aborted` |
| 目标摘要等于 `result_hash`，账本已有 `content_hash` 等于 `result_hash` 的新版本（捕获已完成，A4 未提交） | 不再 `capture`，只完成 A4 | `dt.commit.applied` |
| 目标摘要等于 `result_hash`，账本没有对应新版本 | 执行 `capture` 并完成 A4 | `dt.commit.applied` |
| 其他（含目标不存在且备份缺失或摘要不符、目标摘要与两个哈希都不等） | 方案 `conflicted`，保留现场，由所有者决定处理方式（重做方案或手工处理） | `dt.commit.conflicted` |
文件因此只可能是旧内容、新内容或保留现场的第三种内容，不会出现混合内容；账本与任务链在恢复后一致。在 A1 至 A4 每两步之间中断（含并发的 `capture` 与 `mark_deleted`）由注入点驱动的测试覆盖（IT-059）。控制台启动与 Hermes 启动预检都会提示存在 `applying` 方案。

### 8.5 回滚链
每个已应用方案在 `trace` 与报告中给出目标文件的版本序列、对应账本事件，以及回到应用前版本的命令 `ladon restore <应用前版本ID> --approve`。`restore` 保持现有的 CLI 人工批准语义，不进入 MCP。

## 9. 一次性 CLI 参考

### 9.1 通用约定
| 项 | 规则 |
|---|---|
| 全局选项 | 既有 `--root`（默认 `.`）；新增 `--actor 名称`、`--json`、`--no-color`、`--lang zh\|en`（默认环境变量 `LADON_LANG`，否则 `zh`） |
| 文件参数 | 相对路径、绝对路径，或文件 id 前缀（至少 8 位十六进制） |
| 任务参数 | `T-12`、`12`、完整 UUID、唯一的十六进制前缀（至少 8 位） |
| 文本输出 | ASCII 列在前、自由文本在后（CJK 双宽字符后不补空格）；成功命令末尾输出一行 `下一步: …` |
| JSON 输出 | 一个 JSON 对象，stdout 上不含其他内容：`{"ok":true,"data":…}` 或 `{"ok":false,"code":"E_…","message":"…","hint":"…"}`，键稳定 |
| 人工专属命令（H） | 要求标准输入与标准输出都是真实 Windows 控制台（`GetConsoleMode` 判定，第 2.5 节），否则 `E_NOT_INTERACTIVE`（退出码 5），没有绕过开关；确认方式见第 9.5 节 |

### 9.2 命令表
| 命令 | 说明 | H | 控制台别名 |
|---|---|---|---|
| `doc inbox [--watch]` | 等待人处理的事项：待批准方案、待验收任务、待采纳提案、需确认锚点、谱系候选、中断的应用；`--watch` 每 2 秒刷新，供第二终端使用 | | 待办 |
| `doc baseline add 文件 [--version N] [--note 文本]` | 把文件的某个版本（默认最新）登记为已批准要件基线 | H | 基线 |
| `doc baseline list [文件]`、`doc baseline revoke ID --reason 理由` | 列出基线；撤销基线 | 撤销为 H | |
| `doc outline 文件 [--version N]` | 条款大纲：序号、条款编号、标题、行范围、批注数、任务数、与基线是否一致 | | 大纲 |
| `doc clause 文件 选择器 [--version N]` | 输出条款文本（人工主体不脱敏） | | 条款 |
| `doc note 文件 选择器 文本 [--private] [--rationale 理由] [--quote 引文]` | 在基线锚点上创建批注 `N-n`；`--private` 即 `local_only` | | 批注 |
| `doc notes [文件] [--section 选择器]`、`doc note-share N-n`、`doc note-unshare N-n` | 列出批注；公开或取消公开 | 后两者为 H | |
| `doc new 文件 选择器 标题 [--description 文本] [--owner 名称] [--priority 1\|2\|3] [--depends T-1,T-2] [--private]` | 创建任务；`--private` 即 `local_only`，Agent 看不到该任务（默认 `shared`） | | 新建 |
| `doc edit T-n [--title …] [--description …] [--owner …] [--priority …] [--private \| --share]` | 编辑任务属性与可见性 | H | 编辑 |
| `doc list [--status 状态[,状态]] [--file 文件] [--owner 名称] [--proposed] [--all] [--limit N]` | 任务列表；默认只列未终结且已采纳的任务，按优先级再按编号排序 | | 列表 |
| `doc show T-n` | 任务卡：锚点与条款文本、状态、依赖、批注、方案及决定、提交、证据、关联、最近 10 条事件、锚点状态 | | 查看 |
| `doc trace T-n` 或 `doc trace 文件 选择器` | 条款追踪屏（完成判定）：批注、下游任务、关联的设计与测试、Agent 变更与提交、测试证据、回滚链、最近事件 | | 追踪 |
| `doc history [T-n \| --file 文件] [--limit N]` | 链事件：序号、时间、种类、执行者与类型、状态变化、短哈希 | | 历史 |
| `doc status T-n 目标 [--reason 理由 \| --summary 摘要]` | 非权限迁移：`analysis`（T1、T8）、`working`（T7）、`verification`（T5，`--summary` 为工作摘要） | | 状态 |
| `doc plan new T-n --summary … [--rationale …] [--impact …] [--risk …] [--option …]… [--file 文件 --section 选择器 --kind replace\|insert --text-file 路径]` | 人工提交方案（T2）；不带 `--file` 为只分析方案。编辑选项由 `EditPlanSupport` 提供；`--text-file` 须指向普通文件（保留设备名 `CON`、`NUL`、`COMn`、`LPTn`、`\\.\` 前缀与目录被拒绝） | | 方案 |
| `doc plan show T-n [N] [--diff]` | 方案字段；`--diff` 输出统一差异（第 8.2 节的口径） | | |
| `doc approve T-n [--option N] [--comment 文本]`、`doc reject T-n --comment 文本` | 批准（T3）或驳回（T4）；只分析方案的批准与驳回不依赖 `EditPlanSupport` | H | 审批、驳回 |
| `doc apply T-n`、`doc recover` | 应用已批准方案（第 8.3 节）；恢复被中断的应用（第 8.4 节） | H | 应用、恢复 |
| `doc close T-n [--waive 理由] [--accept-agent-evidence 序号,…] [--comment 文本]` | 验收完成（T6）；`--accept-agent-evidence` 接受 Agent 写入的通过证据（第 4.5 节） | H | 验收 |
| `doc cancel T-n --reason 理由`、`doc reopen T-n --reason 理由` | 取消（T9）；重开（T10） | H | 取消、重开 |
| `doc adopt T-n`、`doc dismiss T-n --reason 理由` | 采纳或忽略 Agent 提案 | H | 采纳、忽略 |
| `doc dep add T-b T-a`、`doc dep rm T-b T-a`、`doc dep list T-n` | `T-b` 依赖先行任务 `T-a`；移除为 H；添加后输出新的批次 | rm 为 H | 依赖 |
| `doc link add T-n --kind … (--file 文件 [--section 选择器] \| --commit 哈希 \| --url 地址 \| --task T-m) [--label 文本]`、`doc link rm ID`、`doc link list T-n` | 关联 | rm 为 H | 关联 |
| `doc evidence add T-n --kind test_run\|report\|note\|version --summary … [--result pass\|fail\|info] [--ref …] [--version 版本ID]`、`doc evidence list T-n` | 证据；列表中的 `#k` 是按时间排序的序号（派生，不存储），供 `doc close --accept-agent-evidence` 引用 | | 证据 |
| `doc anchors [文件] [--check]`、`doc ack T-n`、`doc relocate T-n 选择器`、`doc rebase T-n` | 锚点迁移报告与人工处理 | 后三者为 H | 条款检查、确认条款、重定位、重定基 |
| `doc lineage scan\|list\|confirm ID\|reject ID` | 文件移动谱系 | confirm、reject 为 H | 谱系 |
| `doc schedule` | 并行调度方案（第 7 节） | | 调度 |
| `doc verify [T-n]` | 链与不变量校验（第 4.9 节） | | 校验 |
| `doc export nema\|report [范围] [--format md\|json] [--out 路径] [--force] [--include-local-only] [--include-sensitive] [--generated-at 时间]` | .NEMA 与审计报告导出；`--out` 拒绝设备名与既存的非普通文件 | `--out` 写到 `.ladon/exports` 之外时为 H | 导出 |
| `doc migrate [--import-legacy] [--accept-unverified-backup]` | 显式升级 Schema（含迁移前备份与验证，第 3.1 节）并重新执行幂等回填，输出备份路径与回填计数；`--accept-unverified-backup` 在备份无法验证时确认继续 | 后一选项为 H | |
| `ladon console [--root R] [--lang zh\|en] [--hermes 路径] [--open 文件]` | 交互控制台（第 11 节） | | |
| `ladon mcp-serve --root R --launch ID` | 供 Hermes 启动的 MCP 服务器（第 10 节） | | |
| `ladon hermes doctor`、`ladon hermes print-config`、`ladon hermes status` | Hermes 自检、打印回退配置、最近启动记录（第 12 节） | | |

### 9.3 退出码
| 码 | 含义 | 对应错误码 |
|---|---|---|
| 0 | 成功 | — |
| 1 | 意外失败（`Error: …`，同既有行为） | `E_INTERNAL` |
| 2 | 用法错误（clap 默认） | — |
| 3 | 未找到或有歧义 | `E_NOT_FOUND`、`E_AMBIGUOUS`、`E_SECTION_NOT_FOUND` |
| 4 | 规则拒绝 | `E_NO_BASELINE`、`E_TRANSITION_DENIED`、`E_GUARD_FAILED`、`E_DEP_CYCLE`、`E_DEP_DANGLING`、`E_ANCHOR_BROKEN`、`E_UNSUPPORTED_FILE`、`E_PLAN_TOO_LARGE`、`E_INVALID_ARG`、`E_QUOTA`、`E_RATE_LIMIT`、`E_FILE_MISSING`、`E_SCHEMA_OUTDATED`、`E_MIGRATION_BACKUP` |
| 5 | 人工专属或非交互 | `E_HUMAN_ONLY`、`E_NOT_INTERACTIVE` |
| 6 | 冲突或过期 | `E_CONFLICT`、`E_STALE_PLAN`、`E_STALE_SUBJECT`、`E_FILE_CHANGED` |
| 7 | 数据库被占用 | `E_STORE_BUSY` |
| 8 | 策略拒绝 | `E_POLICY_DENIED` |
| 9 | 完整性校验失败 | `E_INTEGRITY` |
`doctask_cli` 返回退出码，`entry::main_entry` 只在码非 0 时调用 `std::process::exit`（`main.rs` 只含对它的一次调用）。错误以 `错误[CODE]: 消息` 加 `提示: …` 输出到 stderr。

### 9.4 旧命令契约
`ladon annotate 文件 [--section 章节] 文本` 与 `ladon task 文件 [--section 章节] 标题 [--description 说明]` 保持位置参数语法，首行输出仍是 `annotation id: <uuid>`、`task id: <uuid>`；新增可选 `--actor`；创建经服务层（Human 主体、`source_kind=legacy`、锚点按第 5.3 节规则），新的短号以 `任务编号 T-12，查看: ladon doc show T-12` 输出到 stderr。`Store::add_annotation`、`create_task`、`set_task_status`、`list_tasks`、`governance_events` 的签名、键与错误文本不变（`tc108`、`tc109`、`it_05` 继续成立）；`set_task_status` 保持无限制的旧接口，新代码从不调用，`doc verify` 以 `STATUS_DRIFT` 检出绕过。

### 9.5 人工专属操作的确认
| 操作 | 确认方式 |
|---|---|
| 批准、应用、验收 | 展示将要作用的内容与绑定哈希，要求键入任务号（如 `T-12`），不能由 `--yes` 跳过 |
| 驳回、取消、重开、忽略 | 理由必填（非空），再以 `y/N` 确认 |
| 登记与撤销基线、采纳提案、确认锚点（`ack`）、重定位、重定基、谱系确认 | 展示对象（含版本号与内容哈希前 8 位）后 `y/N` 确认。`--yes` 只在同时给出 `--expect-hash <十六进制前缀，至少 8 位>`（取自 `doc anchors`、`doc show`、`doc baseline list` 或 `doc lineage list` 显示的主体哈希）时有效，否则 `E_INVALID_ARG`；它仍要求真实控制台 |
任何非精确输入都视为放弃，不产生写入。批准、验收把展示时的哈希作为 `expected_subject_hash` 传给服务层；其余人工专属操作的服务签名带展示时的主体：基线登记带 `expected_version_id` 与 `expected_hash`，`ack`、`relocate`、`rebase` 带锚点的 `expected_version_id` 与章节哈希（第 5.6 节），采纳带任务内容哈希（任务标题、说明、锚点、优先级、依赖的 SHA-256），谱系确认带新旧文件 id 与新文件内容哈希。服务层在 Immediate 事务内复核，展示后主体变化得到 `E_STALE_SUBJECT` 并重新展示。基线登记、`ack`、`rebase` 还要比较现存文件摘要与 `resources.current_hash`，不符则 `E_FILE_CHANGED`，提示先 `capture`，使所有者批准的永远是账本里已有的版本。

## 10. MCP 服务器参考

### 10.1 传输与协议
`ladon mcp-serve --root <绝对路径，不含 \\?\ 前缀> --launch <launch_id>`，手写实现，只依赖 `serde_json`，在标准输入输出上交换以换行分隔的 UTF-8 JSON-RPC，每行一个紧凑 JSON 对象（`to_string`，不用 pretty），每次回复后 flush。
| 项 | 规则 |
|---|---|
| stdout | 只输出协议行。`mcp.rs` 与 `doctask_mcp.rs` 模块级 `#![deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]`；唯一的 stderr 日志器只记录方法名、工具名、耗时、错误码，不记录参数与内容；`serve()` 在启动时安装自己的 panic 钩子，只写源码位置与 `E_INTERNAL`，不写 panic 消息（默认钩子把消息写到 stderr，消息可能带有行或路径中的文本，而 Hermes 会把 stderr 尾部引用到连接失败信息中）〔事实 既有库模块在 src/ 中没有 `println!`、`eprintln!`、`dbg!`，仅 `main.rs` 使用〕。模块级 `deny` 只在运行 clippy 时生效，而 CI 不运行 clippy（第 15.1 节），因此实际的守卫是 IT-062（stdout 每行恰为一个 JSON 对象）与 IT-067（词元扫描） |
| 读取 | `read_until(b'\n')` 读字节，单行上限 4 MiB，再 `serde_json::from_slice`；坏行得到 -32700 或 -32600（`id` 为 null）并继续循环 |
| `initialize` | `params.protocolVersion` 必填，否则 -32602；客户端版本属于 `2024-11-05`、`2025-03-26`、`2025-06-18`、`2025-11-25` 时原样回复，否则回复 `2025-11-25`，从不回复 `2026-07-28`；`capabilities` 为 `{"tools":{}}`（不含 `listChanged`、`resources`、`prompts`）；`serverInfo` 为 `{name:"ladon",version:CARGO_PKG_VERSION}` |
| 其他方法 | 通知（含 `notifications/initialized`、`notifications/cancelled`）静默；`ping` 回 `{}`；`tools/list` 单页无游标；`tools/call`；其余方法 -32601，`id` 按原类型（数字或字符串）回显 |
| 启动开销 | `initialize` 与 `tools/list` 不打开数据库与 `Engine`；首次 `tools/call` 才打开并在进程内保留，打开后立即检查 Schema 版本，低于 4 时该调用及其后的调用都以普通结果 `E_SCHEMA_OUTDATED` 回复（提示所有者在控制台执行 `doc migrate`），进程不退出（Hermes 把子进程退出视为死亡并重启）；启动时只检查文件 `<root>/.ladon/meta.turso` 是否存在（不打开数据库），不存在则输出一行错误并以码 1 退出（Hermes 会引用 stderr 尾部）。`--global-ignore <路径>`（可选）在 `Engine` 打开之前生效（第 12.3 节） |
| 退出 | 标准输入 EOF 立即以码 0 退出（Hermes 关闭 stdin 后 2 秒强制终止） |
| 顺序 | 严格串行，同一时刻只处理一个请求 |
| 模式 | 不使用 `outputSchema` 与 `structuredContent`；工具 `inputSchema` 为平铺对象（`type:object`、`required`、`enum`、`additionalProperties:false`，无 `$ref`、`oneOf`、可空联合） |
〔事实 H:tools/mcp_tool_transport.py:157-167〕Hermes 只在 `initialize` 结果带 `capabilities.tools` 时调用 `tools/list`。〔事实 H:tools/mcp_tool_handlers.py:52-85〕只有 `annotations.readOnlyHint=true` 的工具被视为只读，其余在 `trust: untrusted` 下每次调用都需所有者确认。〔事实 mcp 2.0.0 SDK mcp/client/session.py:610-632〕客户端只接受上述四个握手版本。〔事实 H:tools/mcp_tool.py:509,525-535；H:tools/mcp_tool_handlers.py:150-155,537-538,598-599〕连续 3 次失败（含 `isError` 结果与 JSON-RPC 错误）使该服务器 60 秒内全部调用被暂停；故业务拒绝、参数不合法与数据库暂忙都以无 `error` 键的普通结果返回，只有内部故障使用 `isError`。其余 Hermes 客户端行为（单行 JSON 分帧、2 秒终止、每次只执行一个调用）出自源码阅读记录，未逐项复核，按〔假设〕处理并由桩客户端测试覆盖。

### 10.2 工具表
工具名为 `doc_` 前缀的 snake_case，在 Hermes 中显示为 `mcp__ladon__doc_*`。〔假设〕工具名加前缀后不超过 64 字符（`mcp_tool_schema.py` 的上限），由测试断言。每个工具的描述只有一句话，因为全部模式在每次模型调用中发送。

**只读工具（`readOnlyHint=true`，共 8 个）**
| 工具 | 参数 | 返回 |
|---|---|---|
| `doc_file_find` | `query?`（≤100）、`has_tasks?`、`limit?`（1–50，默认 20） | `[{file_ref, path, version_no, baseline_version_no, open_tasks}]`；仅已跟踪、未删除且门禁允许的文件，被拒绝者直接省略 |
| `doc_clause_list` | `file_ref`、`version?` | 条款条目：序号、条款编号、标题、级别、行范围、批注数、任务数；非大纲类文件为一条 `ROOT`；所读版本须通过版本门禁（第 13.1 节） |
| `doc_clause_read` | `file_ref`、`section`、`version?`、`offset?`、`max_chars?`（200–6000，默认 3000） | `{text, version_no, hash, truncated, next_offset}`；文件或所读版本被门禁拒绝、文件不存在一律 `E_NOT_FOUND`（不区分原因），规则集不可用时 `E_POLICY_DENIED`；二进制文件 `E_UNSUPPORTED_FILE` |
| `doc_mentions_find` | `token`（2–80）、`limit?`（1–30，默认 10） | `[{file_ref, path, section, line}]`；对已跟踪的 `md`、`nema`、`txt` 文件（≤1 MiB、最多扫描 2000 个，超出带 `truncated`）的最新可读版本做字面子串检索，文件与被扫描的版本逐一通过门禁，被拒者不扫描也不出现；是影响分析的确定性输入，内核不实现其他影响算法 |
| `doc_task_list` | `status?`（枚举数组）、`file_ref?`、`owner?`、`proposed_only?`、`limit?`（1–50，默认 20）、`cursor?` | `{tasks:[{task, id, title, status, priority, owner, file_ref, section, anchor_state, proposal_state, blocked_by, updated_at}], hidden}`；门禁拒绝或 `local_only` 的任务省略，`hidden` 为省略数量 |
| `doc_task_get` | `task`、`events?`（0–30，默认 10） | 任务卡；仅含 `shared` 批注；条款文本取自基线版本字节并通过版本门禁；方案含 `last_decision {decision, comment, by, at}`，Hermes 据此得知批准或驳回；事件只含种类、状态变化、执行者与时间，不含载荷 |
| `doc_trace_get` | `task?`，或 `file_ref` 与 `section` | 条款追踪数据（任务、按层关联、证据、已应用提交与回滚链），仅含 `shared` 批注；下游任务、关联目标、依赖经 `AgentView` 投影，被省略的以 `hidden` 计数 |
| `doc_schedule_get` | 无 | 第 7 节的调度 JSON |

**变更工具（无 `readOnlyHint`，共 6 个，全部只产生提案与记录，不授予任何权限；`doc_plan_submit` 由 WP14 追加，WP13 交付时共 13 个工具）**
| 工具 | 参数 | 行为 |
|---|---|---|
| `doc_task_create` | `file_ref`、`section`、`title`（1–200）、`description?`（≤4000）、`priority?`（1–3）、`depends_on?`（≤10）、`rationale?`（≤1000）、`hermes_session?` | 创建 `proposed` 任务（`visibility=shared`）；无基线返回 `E_NO_BASELINE`；计入提案配额；在所有者采纳之前不能迁移，返回的 `next_zh` 提示所有者先在控制台采纳 |
| `doc_annotation_add` | `file_ref`、`section`、`text`（1–4000）、`rationale?`、`quote?`（≤500）、`hermes_session?` | 创建 `shared` 批注，`author_kind=agent` |
| `doc_plan_submit` | `task`、`summary`（1–500）、`rationale?`、`impact?`、`risk?`、`options?`（≤5）、`edit_kind?`、`edit_file_ref?`、`edit_section?`、`edit_text?`（≤262144 字节）、`read_refs?`（≤10）、`hermes_session?` | 一个事务内写入方案并执行 T2；任务须为 `analysis`；返回 `{plan_no, plan_hash, task_status:"approval", needs_human:true, next_zh:"请所有者退出 Hermes 回到 LADON 控制台，执行 审批 T-12"}` |
| `doc_task_transition` | `task`、`to`（`analysis`、`verification`、`working`）、`reason?`（≤500；`to=verification` 时为工作摘要，必填，≤4000）、`hermes_session?` | 只执行 T1、T5、T7（最新证据为失败时）、T8；其余返回 `E_TRANSITION_DENIED` 并列出可做的迁移 |
| `doc_evidence_add` | `task`、`kind`（`test_run`、`report`、`note`、`version`）、`result?`、`summary`（1–1000）、`ref?`、`version?`、`hermes_session?` | 写入证据，标注来源，不会使任务完成 |
| `doc_link_add` | `task`、`kind`、`file_ref?`、`section?`、`commit?`、`url?`、`label?`（≤120）、`hermes_session?` | 添加关联；`file_ref`、`commit`、`url` 恰好给出一个 |
`hermes_session` 是模型自称的 Hermes 会话 id（来自 `--pass-session-id`），记入 `claimed_session`，不被信任。

**模式规则**：上表的参数记法映射为 JSON Schema 的方式固定如下，`tools/list` 快照测试（UT-157）逐项断言。`inputSchema` 根为 `{"type":"object","additionalProperties":false,"properties":{…},"required":[…]}`；无 `?` 的参数进入 `required`；字符串长度范围写作 `minLength`、`maxLength`（`≤N` 只有 `maxLength`，`（1–N）` 同时有 `minLength:1`）；整数范围写作 `minimum`、`maximum`；`default` 只用于描述，服务端按表中默认值补全；枚举写作 `enum`；数组写作 `{"type":"array","items":{…},"maxItems":N}`；属性只出现 `type`、`description`、`enum`、`minLength`、`maxLength`、`minimum`、`maximum`、`items`、`maxItems`、`default`，不出现 `$ref`、`oneOf`、`anyOf`、`nullable`；每个属性的 `description` 至多一句。`annotations` 只含 `readOnlyHint`，只读工具为 `true`，变更工具不带该键。示例（`doc_plan_submit`）：
```json
{"name":"doc_plan_submit","description":"提交任务的分析与方案（可含一处章节编辑），任务进入待批准。","inputSchema":{"type":"object","additionalProperties":false,"properties":{"task":{"type":"string","description":"任务号，如 T-12"},"summary":{"type":"string","minLength":1,"maxLength":500},"rationale":{"type":"string","maxLength":4000},"impact":{"type":"string","maxLength":2000},"risk":{"type":"string","enum":["low","medium","high"]},"options":{"type":"array","items":{"type":"string"},"maxItems":5},"edit_kind":{"type":"string","enum":["replace_section","insert_after_section"]},"edit_file_ref":{"type":"string"},"edit_section":{"type":"string"},"edit_text":{"type":"string","maxLength":262144},"read_refs":{"type":"array","items":{"type":"string"},"maxItems":10},"hermes_session":{"type":"string","maxLength":64}},"required":["task","summary"]}}
```
`edit_kind`、`edit_file_ref`、`edit_section`、`edit_text` 要么同时给出，要么都不给（只分析方案），不满足时返回 `E_INVALID_ARG`；这类跨属性约束不用 `oneOf` 表达，由服务端校验。

### 10.3 结果与错误
| 情形 | 形态 |
|---|---|
| 成功 | `content` 为一个文本块，内容是紧凑 JSON：`{"ok":true,"data":{…},"summary_zh":"…","next_zh":"…"}` |
| 业务拒绝（未找到、守卫失败、策略不可用、配额）、已知工具的参数不合法（`E_INVALID_ARG`：长度、枚举、类型、缺失字段、未知属性、控制字符）、暂时性状态（数据库被占用 `E_STORE_BUSY`，提示约 2 秒后重试；`E_SCHEMA_OUTDATED`） | 同样是普通结果（不带 `isError`）：`{"ok":false,"code":"E_…","message":"…","hint":"…"}`，顶层没有名为 `error` 的键；不写任何行，不使熔断计数增加 |
| 真实内部故障（`E_INTERNAL`，含被 `CatchPanic` 捕获的 panic） | `isError:true`，消息简短 |
| 未知工具名 | JSON-RPC -32602，不写任何行（Hermes 只调用 `tools/list` 中列出的工具，该情形只来自桩客户端与手工探测） |
结果文本不含 `token=`、`key=`、`password=`、`auth=`、`Bearer` 样式的文本，也不含名为 `token`、`password`、`secret`、`api_key` 的 JSON 字段（Hermes 对错误文本做正则脱敏，MCP 结果是否被脱敏未验证〔假设〕）。单个结果不超过 32 KiB（UTF-8 字节），列表分页；写入文本中的控制字符（除换行与制表符）在输入时被拒绝，输出前再次净化。

panic 处理〔决定〕：每个 `tools/call` 的处理 future 由 `CatchPanic` 适配器包裹（约 15 行，把 future 装入 `Pin<Box<dyn Future>>`，每次 `poll` 在 `std::panic::catch_unwind(AssertUnwindSafe(..))` 内执行，panic 转为 `Err`，再映射为 `E_INTERNAL` 的 `isError` 结果，循环继续）。这个机制不要求 future 为 `Send`（`Engine`、turso 连接的 future 是否 `Send` 未经验证），不使用 `tokio::spawn`，不需要 `futures` 依赖，也不在运行时内调用 `block_on`。测试以故意 panic 的 `ToolHost` 断言服务器不退出、返回 `E_INTERNAL`，且 panic 消息中的标记文本不出现在 stderr（panic 钩子只写位置，第 10.1 节）；测试串行化并在结束时恢复原钩子。

### 10.4 每次调用的检查顺序
1. 解析并按工具模式校验参数（未知属性、类型、缺失字段、长度、枚举、控制字符 → 普通结果 `E_INVALID_ARG`；只有未知工具名返回 -32602）。
2. 主体固定为 Agent（名称 `hermes`，会话 `hermes:<launch_id>`），授权表允许该工具；表外的工具不存在。
3. 解析任务与文件（Agent 只接受 `T-n`、`N-n`、`file_ref`；未找到、被拒绝、`local_only` 一律同一个 `E_NOT_FOUND`，无候选）。
4. 外传门禁：涉及的每个文件调用 `Engine::remote_read_allowed`，每个将被读取或检索字节的版本调用 `Engine::remote_version_allowed`（第 13.1 节），结果经 `AgentView` 投影。
5. 状态与提案守卫。
6. 配额（第 4.3 节）。
7. 自然键幂等（第 5.3、8.1 节）：重复创建返回已有记录。
8. 一个 Immediate 事务写入记录与链事件，`actor=hermes`、`actor_kind=agent`。只读工具不写事件链、不写 `meta`、不改锚点；它们唯一的写入是在返回内容之前追加披露记录 `doc_agent_disclosures`（第 13.1 节），记录写入失败时不返回内容。
9. 结果整形：净化、32 KiB 上限、分页。

### 10.5 Hermes 永远做不到的事
批准、驳回、验收、取消、重开、采纳、忽略、应用、恢复、登记或撤销基线、编辑任务属性、公开批注、确认或重定位锚点、确认谱系、移除依赖或关联、导出到文件、校验、恢复版本（`restore`）、控制 Hermes 自身。这些操作在 MCP 工具表中不存在（调用得到 -32602，不写任何行），服务层对 Agent 主体再次拒绝（`E_HUMAN_ONLY`）；Hermes 以 `-t ladon,clarify` 启动，没有终端、文件、浏览器、网络或代码工具，只能通过 LADON 工具改变状态；经由 LADON 工具，Hermes 看不到门禁拒绝文件（含被拒绝的历史版本）、`local_only` 批注与 `local_only` 任务的内容，也不能设置主体；已经返回给模型的内容留在 Hermes 自己的会话记录中，由恢复前的披露检查约束（第 12.10 节）。

## 11. 控制台参考

### 11.1 入口与会话
`ladon console [--root R] [--lang zh|en] [--hermes 路径] [--open 文件]`。`Console<R: BufRead, W: Write>` 以注入的读写器、`Presence`、`ProcessRunner` 构造，测试使用 `Cursor` 缓冲；`entry.rs` 只构造真实对象。〔决定〕在调用线程上以 `read_line` 阻塞读取，不建立后台读线程（会抢走交给 Hermes 的按键），不使用 `spawn_blocking`，也不在运行时内调用 `Runtime::block_on`（会 panic）。Windows 熟模式输入提供行编辑、历史与中文输入法；〔假设〕std 的控制台读取经 UTF-16 转换，中文输入输出无需修改代码页，由 ST-081 在 cmd、Windows Terminal、conhost 中确认。

启动顺序：`Engine::open`（不用 `init`）→ `DocTaskService::open`（`ensure_schema`、回填）→ 对存在未终结锚点的文件对账 → 提示存在 `applying` 方案 → 输出启动屏。目标：热工作区启动不超过 3 秒（NFR-044）。控制台在命令之间不持有数据库句柄。

启动屏内容：工作区根与跟踪文件数；操作人与类型；链校验快速结果；“待你处理”分组——待批准方案、待验收任务、待采纳提案、需确认锚点、谱系候选、中断的应用，每项给出下一条命令。提示符 `ladon[工作区名]›`，任务获得焦点时为 `ladon[工作区名·T-12]›`。

### 11.2 命令
命令是 `doc` 子命令去掉 `doc` 前缀，名称为英文或表中的中文别名，可带前导 `/`；每行由约 30 行的分词器（空格、`"`、`'`、中文引号 “ ” ‘ ’、反斜杠转义，无新增依赖）切分，交给与一次性 CLI 同一个 clap `DocCommand` 解析器（`try_parse_from`），因此别名、用法与帮助文本来自 `doctask_cli` 的命令注册表这一唯一来源，新增子命令自动获得控制台支持。clap 错误以中文提示加用法行显示，不退出控制台；输入有误的命令按编辑距离给出建议。
控制台独有命令：
| 命令 | 作用 |
|---|---|
| `help [命令]`、`帮助` | 按所有者一天的顺序分组：准备（基线、大纲）、记录（批注、新建、编辑、依赖、关联）、查看（追踪、查看、列表、历史、调度）、让 Hermes 帮忙（hermes）、决定（审批、驳回、应用、验收、取消、重开、采纳、忽略、确认条款、重定位、重定基、谱系）、收尾（证据、校验、导出） |
| `use 文件`、`焦点 T-n` | 设置当前文件（之后选择器可单独给出）与当前任务 |
| `hermes [T-n \| resume \| doctor \| config \| status]` | 见第 11.5 节 |
| `set 键 值`、`设置` | 仅白名单键：`hermes_exe`、`hermes_route`、`hermes_trust`、`hermes_env_passthrough`、`actor`；原子写入控制台设置文件，不写入任何秘密 |
| `cls`、`exit`（`quit`、EOF） | 清屏；退出，退出码 0 |

### 11.3 屏幕
下列文本为示例，样例条款编号 `SPEC-12` 仅用于说明。
**大纲屏**（`大纲 要件定义.md`）
```
要件定义.md   已批准 v12（2026-11-20 10:21，owner）  当前 v12  与基线一致
 #  条款     标题                  行范围   批注  任务
 1  SPEC-11  同步流程              3-14      0     0
 2  SPEC-12  复测流程              15-30     2     3
下一步: 批注 要件定义.md #2 "文字"   或   新建 要件定义.md #2 "标题"
```
**追踪屏**（`追踪 要件定义.md SPEC-12`，对应完成判定）：依次为条款与基线状态；批注（含共享/仅本地标志）；下游任务（状态、依赖、方案状态）；关联（设计、测试）；Agent 变更与提交（版本 id、账本事件、批准人、回滚命令）；测试证据（带来源标注）；最近事件；末行下一步提示。
**批准屏**（`审批 T-12`）：任务与条款；提交者与会话；方案摘要、风险、影响；统一差异预览（`expected_version_id` 的字节到结果字节，第 8.2 节）；`expected_hash` 与 `result_hash` 的短哈希；目标章节与基线是否一致（单独一行，目标文件无基线时标明）；同一文件上的其他已批准方案；依赖与锚点状态；最后一行“批准请输入任务号 T-12 并回车；驳回请输入 n 再说明原因；直接回车跳过”。键入任务号以外的任何内容都不批准；方案已过期则在提示前以 `E_STALE_PLAN` 拒绝。
所有表格用手写的显示宽度函数对齐（宽字符区间 U+1100–115F、2E80–A4CF、AC00–D7A3、F900–FAFF、FE30–FE6F、FF00–FF60、FFE0–FFE6 计 2 列）。来自数据库或 Agent 的文本在渲染时净化（ESC 与其他控制字符、双向控制符、零宽字符替换为可见占位）。颜色仅在 stdout 为真实控制台、`NO_COLOR` 未设置且 VT 能被开启时使用，否则纯文本。

### 11.4 批准如何在 Hermes 占用终端时到达所有者
两次交接：Hermes 创建的任务是 `proposed` 提案，所有者采纳之前 Hermes 不能让它离开 `open`（第 4.3 节）。完整往返因此有两次交接：第一次，Hermes 用 `doc_task_create` 提出任务并告知所有者；所有者退出 Hermes，在控制台（或第二终端）执行 `采纳 T-n`（收件箱 `[p]`）；第二次，所有者再进入 Hermes（`hermes T-n` 或 `hermes resume`，后者先做第 12.10 节的披露检查），Hermes 经 `doc_task_list` 看到已采纳的任务，继续 `doc_task_transition` 到 `analysis` 并 `doc_plan_submit`；其后才是下面的方案批准往返。

1. 批准类命令不存在于 MCP 服务器，Hermes 只能提交方案，任务随之进入 `approval`，工具结果返回 `needs_human` 与中文指示。
2. Hermes 运行期间控制台进程阻塞在子进程等待上，不持有数据库句柄，Hermes 的 MCP 子进程可以写入。
3. 所有者在 Hermes 中 `/exit`（或 Ctrl+D）后，控制台恢复控制台模式，重新读取数据库并立即显示“Hermes 已退出（代码 n）。本次会话：新提议 a · 方案 b · 批注 c · 证据 d。有 k 项等你处理 → 审批 T-12”，随后进入收件箱。
4. 所有者也可在第二个终端运行 `ladon console` 或 `ladon doc inbox --watch`，在 Hermes 仍打开时批准；这是与 MCP 子进程并发的写入，由有界连接重试保证安全。
5. 收件箱逐项展示：任务、条款文本前 20 行（`m` 展开）、分析与方案或工作摘要、证据及其来源标注、绑定哈希；操作键 `[a]批准 [r]驳回 [s]跳过 [m]更多 [p]采纳提案 [e]接受 Agent 证据后验收 [w]书面豁免后验收 [q]退出`（验收项在只有 Agent 通过证据时，只有 `[e]` 与 `[w]` 可用，并显示哪些证据由 Agent 写入）；批准以展示时的哈希绑定，内容变化得到 `E_STALE_SUBJECT` 后重新展示。
6. Hermes 自身在 `trust: untrusted` 下弹出的“是否允许调用某工具”只授权“记录一条提案”，不是 LADON 批准，也不会被记录为批准；控制台启动横幅与用户手册都明确说明。
7. Hermes 通过 `doc_task_get` 的 `last_decision` 得知批准或驳回的结果与说明。

### 11.5 Hermes 命令
`hermes`（新会话）、`hermes T-12`（以任务为焦点的新会话，种子文本包含任务号）、`hermes resume`（恢复最近一次会话，先做第 12.10 节的披露检查，不满足时开启新会话并说明原因）、`hermes doctor`、`hermes config`（打印回退配置）、`hermes status`（最近启动：会话名、起止、退出码、计数，及“启动后未结束”的记录）。启动流程见第 12 节；退出后控制台恢复控制台模式，输出本次会话摘要与待处理队列。

### 11.6 脚本模式
标准输入或标准输出不是真实控制台时，控制台把每行当作一条命令执行，错误不中止，退出码取最差的一个；人工专属命令与 `hermes` 启动以 `E_NOT_INTERACTIVE` 拒绝。测试以注入的 `Presence` 替身与脚本输入驱动同一代码路径。

### 11.7 中断与结束
提示符处 Ctrl+C 放弃当前输入行重新提示（`ReadConsoleW` 在处理例程返回 TRUE 时的返回方式为〔假设〕，循环同时接受 `Ok(0)` 带中断标志、`Ok(0)` 不带标志（视为 EOF）与错误三种情形，由 ST-078 确认）；Hermes 运行期间 LADON 只吸收 Ctrl+C。

## 12. Hermes 启动方案与检查

### 12.1 路线
设置 `hermes_route` 取 `overlay`（默认）、`profile`、`print` 之一，由设置与 `hermes doctor` 的结果决定，代码不做猜测。
| 路线 | 做法 |
|---|---|
| `overlay` | LADON 写出自己拥有的托管覆盖目录并以环境变量 `HERMES_MANAGED_DIR` 指向它启动 Hermes。〔事实 H:hermes_cli/managed_scope.py:45-59〕该变量非空且目录存在时被采用；〔事实 H:hermes_cli/config.py:2373-2376〕覆盖内容在 `load_config`（MCP 发现所用）中合并到用户配置之上；〔事实 H:hermes_cli/managed_scope.py:69-96〕覆盖文件解析失败时被忽略并只记警告（失败即放行），因此 LADON 写出后必须回读比对，`hermes doctor` 还要用 `hermes mcp test ladon` 确认。所有者自己的 `config.yaml`、`.env` 与模型凭据不被触碰 |
| `profile` | 所有者自行创建名为 `ladon` 的 Hermes 档案并把 `ladon hermes print-config` 的输出粘贴进去（LADON 从不创建、复制或修改 Hermes 档案）；LADON 以 `-p ladon` 启动且不写覆盖层 |
| `print` | LADON 只打印配置片段与应执行的命令，由所有者手工完成 |
LADON 从不传递：`--safe-mode`（〔事实 H:tools/mcp_tool_config.py:543-549；H:hermes_cli/main.py:3023-3028〕会禁用全部 MCP 服务器）、`--yolo`、`--ignore-user-config`、`--ignore-rules`、`--source tool`，也从不设置 `HERMES_HOME`。〔事实 D:\hermes-multica\home\hermes.cmd〕本机启动器经 `powershell -File rewrite-and-run.ps1` 运行；该脚本是否转发 `HERMES_MANAGED_DIR` 与 `HERMES_EPHEMERAL_SYSTEM_PROMPT`、是否保持真实控制台，属〔假设〕，由 `hermes doctor` 与 ST-076 决定。〔事实 H:hermes_cli/managed_scope.py:48；H:website/docs/user-guide/managed-scope.md:45-62〕Hermes 把 `HERMES_MANAGED_DIR` 标注为引导覆盖并要求由管理员固定，LADON 每次启动都设置它超出了该文档的意图，是对 Hermes v0.21.7 行为的依赖〔假设〕，在 Hermes 升级后可能失效：该依赖由 WP00B 的探针在真实启动器上确认并记录 Hermes 版本，WP12 与 WP16 在探针结论之后开始；`hermes_route` 因此是设置项，`profile`、`print` 是备用路线，`hermes doctor` 在每次 Hermes 升级后复核。

### 12.2 启动器解析
顺序：`--hermes` 或设置 `hermes_exe`；环境变量 `LADON_HERMES_EXE`；都没有时按 PATH × PATHEXT 逐目录列出候选（绝对路径、带扩展名）：恰有一个则使用并显示，多个时由所有者按编号选一次，确认后保存到控制台设置文件。不会静默回退到 PATH 搜索，不使用 `Command::new("hermes")`（〔假设〕std 对裸名只补 `.exe`）。存活探测使用 `hermes chat --help`（退出码 0），不使用 `--version`（垫片可能自行应答〔假设〕）。启动器是 `.cmd` 时，每个参数与路径必须不含 `% ^ & | < > "` 与非 ASCII 字符（〔事实 hermes.cmd 以 `%*` 转发〕，参数会被 cmd 再解析一次、PowerShell 再解析一次），不满足时提示把 `LADON_SETTINGS_DIR` 设为纯 ASCII 路径。

### 12.3 LADON 写出的文件
目录 `<设置目录>\hermes\<ws8>\<launch_id>\`：设置目录取 `LADON_SETTINGS_DIR`，否则 `%LOCALAPPDATA%\LADON`〔事实 src/google_oauth.rs:366-371〕；`ws8` 为工作区规范路径（小写、去掉 `\\?\` 前缀）的 SHA-256 前 8 位十六进制；`launch_id` 为 `yyyymmdd-hhmmss-xx`（UTC 加 2 位随机十六进制，纯 ASCII）。每次启动一个目录，使两个控制台不会互相覆盖会话号；超过 7 天的目录在启动时清理，退出后本次目录尽力删除。文件均为 UTF-8、经临时文件改名原子写出、不含秘密：
| 文件 | 内容 |
|---|---|
| `managed\config.yaml` | 覆盖配置（下方 JSON；JSON 是合法 YAML〔假设〕，以 `serde_json` 输出，所有字符串带引号、无重复键） |
| `seed.md` | 首条用户消息，固定文本：`请先调用 doc_task_list 汇报待办；如指定了任务号 T-n，请聚焦该任务。` 不含任务标题、文档文本、文件名，不超过 4 KiB；`--no-seed` 时不写 |
| `cwd\` | 空的工作目录，位于工作区与所有 git 仓库之外，使 Hermes 不会加载工作区或 LADON 仓库中的 `AGENTS.md`、`CLAUDE.md`、`.hermes.md` |
```json
{"mcp_servers":{"ladon":{"command":"<ladon.exe 绝对路径，正斜杠>","args":["--root","<工作区根>","mcp-serve","--launch","<launch_id>","--global-ignore","<全局忽略文件绝对路径，可选>"],"enabled":true,"timeout":120,"connect_timeout":60,"protocol":"legacy","trust":"untrusted","sampling":{"enabled":false},"elicitation":{"enabled":false},"tools":{"resources":false,"prompts":false}}},"tools":{"tool_search":{"enabled":"off"}},"mcp_discovery_timeout":20,"mcp_single_query_discovery_timeout":30}
```
`trust` 取设置 `hermes_trust`（`untrusted` 默认，`full` 由所有者显式设置；自动化测试用 `full`，因为非交互运行会拒绝所有确认提示）。`--global-ignore` 仅在控制台自身环境中已设置 `LADON_GLOBAL_IGNORE`（一个路径，不是秘密）时出现，其值为该文件的绝对路径；`mcp-serve` 在 `Engine` 打开之前把它设为本进程的 `LADON_GLOBAL_IGNORE`，从而使 MCP 进程的外传门禁与控制台使用同一份全局规则（`LADON_*` 变量不会转发给 Hermes 子进程，第 12.4 节）。该文件不可读时 `Engine::open` 失败，门禁对每个含文件的调用一律返回 `E_POLICY_DENIED`（失败即关闭），`initialize` 与 `tools/list` 仍照常回复。选择理由：根路径、启动 id 与全局忽略路径放在 `args` 而不是环境变量，因为〔事实 H:tools/mcp_tool_config.py:182-194〕Hermes 只给 MCP 子进程过滤后的环境；`tool_search` 必须为 `off`，因为〔事实 H:tools/tool_search.py:149-161,196-205〕默认会把全部 `mcp-*` 工具藏到桥接工具之后；发现超时要抬高，因为〔事实 H:hermes_cli/config_defaults.py:534,539〕默认值是 1.5 秒与 15 秒，而 Hermes 自身启动在本机需要 6–25 秒〔探索记录〕；采样与征询关闭，使服务器无法反向询问 Hermes 的模型或用户。

### 12.4 环境变量
子进程环境由 `env_clear()` 加白名单构成，再加设置 `hermes_env_passthrough` 中列出的变量名（只存名称，不存值，默认空）。名称按 ASCII 不区分大小写匹配：比较时转为大写，设置子进程环境变量时保持原有大小写（`std::env::vars_os` 在 Windows 上返回存储的大小写，如 `Path`、`SystemRoot`、`ProgramFiles(x86)`；区分大小写的比较会丢掉 `Path`，使 `cmd.exe` 找不到 `powershell.exe`）；下文的禁转发前缀与名称同样按此规则匹配，`Ladon_Archive_Key_Hex` 与 `LADON_ARCHIVE_KEY_HEX` 一样被拒绝。白名单：
`PATH`、`PATHEXT`、`SYSTEMROOT`、`SYSTEMDRIVE`、`WINDIR`、`COMSPEC`、`TEMP`、`TMP`、`USERPROFILE`、`HOMEDRIVE`、`HOMEPATH`、`APPDATA`、`LOCALAPPDATA`、`PROGRAMFILES`、`PROGRAMFILES(X86)`、`PROGRAMDATA`、`USERNAME`、`USERDOMAIN`、`COMPUTERNAME`、`OS`、`PROCESSOR_ARCHITECTURE`、`NUMBER_OF_PROCESSORS`、`TERM`、`WT_SESSION`、`WT_PROFILE_ID`、`LANG`、`LC_ALL`、`HERMES_HOME`（所有者环境中已设置时原样转发，LADON 从不设置）。
LADON 再设置 `HERMES_MANAGED_DIR`（本次覆盖目录）与 `HERMES_EPHEMERAL_SYSTEM_PROMPT`（第 12.6 节的指令文本，〔事实 H:hermes_cli/cli_init_mixin.py:255〕在 CLI 初始化时读取；其不随会话保存，故每次启动与恢复都要重新提供）。
永不转发，即使列在 passthrough 中：以 `LADON_`、`LIMBO_` 开头的任何变量（`LADON_ARCHIVE_KEY_HEX`、`LADON_AI_API_KEY`、`LADON_GOOGLE_*` 等），以及 `HERMES_YOLO_MODE`、`HERMES_SAFE_MODE`、`HERMES_IGNORE_RULES`、`HERMES_IGNORE_USER_CONFIG`、`HERMES_GUEST_ONBOARDING`、`HERMES_MODEL`、`HERMES_INFERENCE_MODEL`。启动前环境中已存在 `HERMES_MANAGED_DIR` 时拒绝启动（可能是真实的管理员策略）。LADON 从不读取、打印或复制 Hermes 的配置、`.env`、认证、令牌、保险库文件与启动器脚本的环境。所有者的模型凭据如果依赖环境变量，把变量名加入 `hermes_env_passthrough`；Hermes 自己的首次运行检查会提示缺失。

### 12.5 命令行
参数逐项分离、仅含 ASCII 与绝对路径，不含任何用户文本：
`<hermes> chat --cli -t ladon,clarify --in <cwd 目录> -c ladon-doc-<ws8>-<launch_id> --create-if-missing --pass-session-id --query-file <seed.md>`
`profile` 路线在最前加 `-p ladon`；`hermes resume` 把 `-c` 的会话名换成最近一次启动记录的会话名。事实依据：〔事实 H:hermes_cli/mcp_startup.py:29-47、H:tools/mcp_tool_discovery.py:595-616〕`-t` 的值同时是工具集选择与 MCP 服务器启动白名单，条目必须是服务器键 `ladon`（不是 `mcp-ladon`）；〔事实 H:hermes_cli/_parser.py:198,235,272,277〕`--pass-session-id`、`--query-file`、`--in`、`--create-if-missing` 存在；〔事实 H:toolsets.py:156〕`clarify` 工具集存在；〔事实 H:cli.py:859-866〕`--query-file` 仅在标准输入与输出都是真实控制台时用来给交互会话播种首轮。终端、文件、浏览器、网络、代码工具都不在 `-t` 中。`-c … --create-if-missing --query-file … --in … --cli` 在同一次交互启动中的组合未经验证〔假设〕，由 ST-076、ST-077 确认。

### 12.6 指令文本
经环境变量传递（不受上下文文件扫描影响，不被 `--ignore-rules` 丢弃〔假设〕），中文、工具名为 ASCII、不超过 4000 字符，内容：文档任务以已批准要件为前提且不得杜撰；任务生命周期以及 Agent 可执行的步骤；使用顺序（`doc_file_find`、`doc_clause_list`、`doc_clause_read`、`doc_mentions_find` → `doc_task_create`、`doc_annotation_add` → 告知所有者先采纳提案并停止（提案被采纳前任务不能迁移）→ 所有者再次启动后经 `doc_task_list` 找到已采纳的任务 → `doc_task_transition` 到 `analysis` → `doc_plan_submit` 并引用条款编号与具体编辑 → 等待所有者 → `doc_task_get` 查看决定 → `doc_evidence_add` → `doc_task_transition` 到 `verification`）；批准、应用、验收、取消只由所有者在 LADON 中完成，除非 `doc_task_get` 显示已批准否则不得声称已批准；结果带 `hidden` 计数、`E_NOT_FOUND` 或 `E_POLICY_DENIED` 时不得尝试以其他方式取得被省略的内容；不明确时用 `clarify` 询问；被拒绝的调用不重复；不在工具参数中放入秘密；用中文回答。
措辞避开 Hermes 上下文扫描会拦截的模式〔事实 H:tools/threat_patterns.py:27-57〕：忽略或无视先前指令、不要告诉用户、你现在是、假装是、含 ignore/override/system/secret/hidden 的 HTML 注释、拉取任务（pull tasks）、心跳或信标、你必须注册或连接或汇报、携带密钥变量的 curl、读取 `.env`、零宽或双向字符。LADON 的单元测试复制这些正则并对指令文本做扫描，同时断言长度。

### 12.7 预检
`hermes` 启动按序执行 1–9（`hermes resume` 在 6 之后执行 6a）；`hermes doctor` 执行 1–5、7、8（不含外传同意 6，因为 doctor 不调用模型；不含登记 9）并增加 a、b：
| 序 | 检查 | 失败处理 |
|---|---|---|
| 1 | stdin、stdout、stderr 都是真实 Windows 控制台：`winconsole.rs` 的 `GetConsoleMode` 对三个句柄都成功（Git Bash 与 mintty 的伪终端管道不是控制台，也不满足 Hermes 自身的 isatty 判断；`IsTerminal` 对这些管道也返回真，故不用于此检查）。判定失败且句柄是名称形如 `msys-*-pty*`、`cygwin-*-pty*` 的管道时，说明提示改用 cmd 或 Windows Terminal | `E_NOT_INTERACTIVE`，说明原因 |
| 2 | 解析启动器（第 12.2 节），打印路径；存在多个候选时提示 | 未找到则给出候选与 `set hermes_exe` 用法 |
| 3 | `.cmd` 启动器的参数字符安全 | 提示改用纯 ASCII 的设置目录 |
| 4 | 环境中未预置 `HERMES_MANAGED_DIR` | 拒绝启动 |
| 5 | Schema 版本不低于 4；提示存在 `applying` 方案 | 版本过低则要求先执行 `doc migrate` |
| 6 | 外传同意：对每个（工作区、启动器）首次显示并要求键入 `y`，结果存入控制台设置；`--yes` 跳过。文案：工具返回的文档条款、批注与任务文本（含任务标题、说明与方案）会发送到 Hermes 中配置的模型；`local_only` 的批注与任务、命中 `.LADONignore` 或标记为敏感的文件不会返回；已返回的内容留在 Hermes 自己的会话记录中，LADON 从不删除该记录 | 拒绝则不启动 |
| 6a | 仅 `hermes resume`：披露检查（第 12.10 节） | 不满足时开启新会话并说明原因 |
| 7 | `mcp-serve` 自检：在进程内启动子进程，执行 `initialize`、`tools/list`、一次只读调用、EOF，要求 2 秒内干净退出。子进程以 `env_clear()` 加 Hermes 实际会给 MCP 子进程的基线环境（〔事实 H:tools/mcp_tool_config.py:144-204〕：`PATH`、`HOME`、`USER`、`LANG`、`LC_ALL`、`TERM`、`SHELL`、`TMPDIR` 与不区分大小写的 Windows 进程变量集，不含 `LADON_*`）启动，`args` 与覆盖层逐项相同（含 `--global-ignore`），使自检看到的规则与环境和真实运行一致；输出中显示生效的全局忽略路径或“无” | 报告失败，不启动 Hermes |
| 8 | 写出覆盖层、种子、`cwd`，回读并比对 `mcp_servers.ladon` | 失败则中止 |
| 9 | 登记 `doc_agent_sessions`（含启动时的 `policy_fp`）与 `dt.session.started`；释放数据库句柄（控制台本就不持有） | — |
| a | 仅 doctor：`hermes chat --help` 退出码 0（超时 90 秒；Hermes 启动 6–25 秒） | 报告 Hermes Python 栈无法启动 |
| b | 仅 doctor：带覆盖环境运行 `hermes mcp test ladon`，退出码 0 为已连接并显示工具数，1 为失败，3 为配置中无该服务器（覆盖层未生效，应改用 `profile` 路线）〔假设〕 | 报告并给出下一步 |
模型提供方是否可用不探测（判断依赖 `.env` 与认证文件的内容）；Hermes 自己的首次运行提示与退出码原样显示。

### 12.8 进程处理
`winconsole.rs`（手写 FFI，风格同 `src/google_oauth.rs:415-519`，非 Windows 为空桩；零新依赖）：
1. 首次启动子进程之前，把 LADON 自己加入一个只设置 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`、不含任何脱离（breakaway）标志的作业对象，子进程继承后关闭窗口会终止整棵进程树；〔假设〕Hermes 自己的作业允许静默脱离（出自探索记录），因此作业必须加在 LADON 上而不是子进程上；加入失败只是警告（CI 运行器可能已在作业中）。
2. Ctrl+C 门：`SetConsoleCtrlHandler` 注册处理例程，对 `CTRL_C_EVENT`、`CTRL_BREAK_EVENT` 置标志并返回 TRUE，对关闭、注销、关机返回 FALSE；绝不调用 `SetConsoleCtrlHandler(NULL, TRUE)`（该属性会被子进程继承，Hermes 将收不到 Ctrl+C）。
3. 启动子进程前保存标准输入与输出的控制台模式，退出后恢复；子进程以继承的标准输入输出运行，不使用 `CREATE_NEW_PROCESS_GROUP`。
4. 启动前打印“正在启动 Hermes（可能需要 10–30 秒）…”；子进程的任何退出码都只显示，不当作成功或失败（交互会话的退出码没有规定）。
5. 退出后：恢复控制台模式、重新确认 UTF-8 输出、更新 `doc_agent_sessions`（`ended_at`、`exit_code`、`policy_fp`）与 `dt.session.ended`、删除本次启动目录、输出本次会话摘要与待处理队列。
6. 全部函数返回布尔值或 `io::Result`，标准输出是管道时不 panic（CI 场景）。64 位作业结构大小以 `size_of` 测试断言。
7. 控制台判定：`is_real_console(handle)` 为 `GetConsoleMode` 成功（人在场证明与预检 1 的唯一依据）；失败时 `console_kind(handle)` 区分 `MsysPty`（经 `GetFileInformationByHandleEx(FileNameInfo)` 取得的管道名形如 `\msys-*-pty*-{from,to}-master`、`\cygwin-*-pty*`）、`Pipe`、`File`、`Other`，只用于错误说明；管道名匹配是纯函数，有单元测试。ConPTY（Windows Terminal、VS Code 终端）提供真实控制台句柄，按真实控制台处理。`std::io::IsTerminal` 只用于颜色判断。

### 12.9 `ladon hermes` 子命令
| 子命令 | 作用 |
|---|---|
| `doctor` | 预检 1–5、7、8 与 a、b，逐项输出结果，全部通过退出码 0，否则 4 |
| `print-config` | 打印覆盖配置片段（占位符已解析）与等价的手工命令，从不写入任何 Hermes 文件 |
| `status` | 最近的启动记录：会话名、起止时间、退出码、本次产生的提案/方案/批注/证据数量，以及“启动后未结束” |

### 12.10 会话
每次 `hermes` 是一个新的 Hermes 会话（会话名含启动 id）；`hermes resume` 复用最近一次的会话名，覆盖层与指令文本重新生成。LADON 从不读取 Hermes 的 `state.db`，不删除 Hermes 会话与转录，用户手册写明这一点。

披露检查〔决定〕：外传门禁逐次调用判定，而〔假设〕Hermes 恢复会话时会把此前的转录（含此前工具返回的条款与任务文本）再次发送给模型，因此会话之后新增的规则或敏感标记不能收回已经返回的内容。为此：(1) 每个返回内容的只读调用在返回之前向 `doc_agent_disclosures` 追加 `(launch_id, file_id, version_id, content_hash)`，写入失败时不返回内容（普通结果 `E_STORE_BUSY`）；这是只读工具唯一的写入，不进入事件链，不计入配额。(2) 控制台在启动前与 Hermes 退出后计算策略指纹——根 `.LADONignore` 的字节、全局忽略文件的字节与全部 `sensitive` 文件的 `(file_id, sensitive)` 清单的 SHA-256——存入 `doc_agent_sessions.policy_fp`（启动时写入，退出后更新）。(3) `hermes resume`（预检 6a）对该会话名下的全部启动要求：每个启动都有结束记录；当前策略指纹等于最近一次记录的指纹；其披露记录中的每个 `(file_id, version_id)` 重新通过 `remote_read_allowed` 与 `remote_version_allowed`（涵盖内容副本被放入排除路径、嵌套规则文件变化等情形）。任一不满足时不恢复，而是开启新的会话（新启动 id），并提示“规则或敏感标记在上次会话后已变化（或上次会话未正常结束），此前返回给模型的内容仍在 Hermes 的会话记录中，LADON 不会删除该记录”。没有披露记录的启动（模型没有读取任何内容）不构成阻碍。

### 12.11 需要真实 Hermes 与可见窗口才能验证的项
覆盖层与指令文本经本机启动器链透传；`-c`、`--create-if-missing`、`--query-file`、`--in`、`--cli` 的组合；MCP 子进程在发现窗口内就绪；Tool Search 实际关闭；`untrusted` 提示含中文时的显示；`Terminate batch job (Y/N)?` 是否出现；交互会话的退出码；Hermes 是否对 MCP 结果做脱敏。这些只能在所有者机器上执行 ST-076 至 ST-082，结果记入 `46_测试执行记录与缺陷.md`，未执行的保持 Not Run。

## 13. 安全与策略

### 13.1 外传门禁
Hermes 把工具结果发送到它所配置的远端模型，因此 LADON 的 MCP 表面是唯一的外传口，必须失败即关闭。`engine.rs` 增加两个方法，不改变 `capture`、`PendingVersion`、`CaptureResult`：`Engine::remote_read_allowed(&FileRecord) -> Result<bool>`（文件级）与 `Engine::remote_version_allowed(&VersionRecord) -> Result<bool>`（版本级，与 `archive_version` 对版本的判定同形：〔事实 src/engine.rs:208-214〕`version.sensitive` 或 `content_is_excluded(version.content_hash)`）：
| 判定 | 条件 | 结果 |
|---|---|---|
| 文件级 | 规则集无效（`policy_valid` 为 false 或加载失败，含全局规则文件不可读） | 拒绝 |
| 文件级 | `resources.sensitive` 为真 | 拒绝 |
| 文件级 | 缓存规则集 `allow_external(相对路径)` 返回错误（路径被排除） | 拒绝 |
| 文件级 | `content_is_excluded(当前内容哈希)` 为真（排除路径的副本或移动） | 拒绝 |
| 版本级 | 被读取、检索或用来派生文本的那个版本的 `versions.sensitive` 为真，或 `content_is_excluded(该版本内容哈希)` 为真 | 拒绝 |
| — | 其余 | 允许 |
每个返回字节或由字节派生文本的工具对文件调用文件级判定，对实际读取的每个版本（`version?` 指定的版本、任务卡条款文本所取的基线版本、`doc_mentions_find` 扫描的版本、追踪所用版本）调用版本级判定；一个文件当前可外传并不使它的历史版本可外传（历史版本可能含后来删除的秘密，或与某个被排除文件字节相同）。
新鲜度：每次内容相关调用先比较根 `.LADONignore` 与全局忽略文件的修改时间，变化时重载并等待新规则就绪（所有者刚改规则后的调用必须看到新规则，成本是一次遍历）；规则文件未变时，嵌套规则文件的变化由后台刷新发现：距上次刷新超过 30 秒的内容相关调用在后台线程启动一次新的规则遍历且不等待，新规则就绪后原子替换，其间沿用上一份有效规则（已记录的残余窗口为 30 秒加一次遍历耗时）。因此没有任何调用为等待嵌套规则遍历而阻塞。`Engine` 记录遍历开始次数，测试以该计数断言：规则未变时连续调用不启动遍历，一次规则编辑恰好启动一次。该机制由 `refresh_policy_if_stale` 实现，`reload_policy` 的既有语义不变。
使用方式：所有面向 Agent 的记录一律经唯一的 `AgentView` 投影，每次调用对每个（文件、版本）解析一次门禁结果并在该次调用内缓存。门禁拒绝的文件上的任务、批注、锚点、标题、章节键、路径、关联目标与依赖，以及 `local_only` 的批注与任务，整体从列表、调度、关联、追踪、阻塞者中省略，被省略的数量只以 `hidden` 计数出现，不出现名称；`doc_file_find` 直接省略。直接引用被拒绝或不存在的文件、任务、批注一律返回同一个 `E_NOT_FOUND`（消息与提示相同，不附候选列表），使 Agent 无法借错误码或候选探测敏感文件的存在与名称；规则集不可用时所有含文件的调用返回 `E_POLICY_DENIED`（与具体文件无关）。事件在 Agent 视图中只含种类、状态变化、执行者与时间，从不返回载荷。人工视图不受限制。该门禁不复用 `AgentApi` 的读取路径。

### 13.2 导出
`doc export` 默认不包含 `local_only` 批注与任务的文本以及被门禁拒绝文件的正文，显式 `--include-local-only`、`--include-sensitive` 才包含；审计事件载荷不含批注文本与任务标题，只含哈希与长度（回填的旧事件同样，第 3.5 节），使审计导出、`doc history`、报告与 Agent 的 `doc_task_get` 都不会从事件链泄露私人文本。

### 13.3 秘密
LADON 不打开 Hermes 的 `.env`、认证、令牌、保险库文件，不运行显示或修改 Hermes 配置与秘密的命令；子进程环境为白名单（第 12.4 节），`LADON_*` 永不转发；覆盖层、种子、指令文本、命令行、日志、数据库行中不出现任何秘密；工具结果不含环境变量值、Hermes 配置或工作区之外的路径；stderr 日志只含方法名、工具名、耗时、错误码。

### 13.4 批准的三层保证
| 层 | 机制 | 证明 |
|---|---|---|
| 工具表 | 人工专属操作不在 MCP 工具表中 | `tests/it_arch_guard.rs` 对源码做词元扫描（跳过字符串字面量、字符字面量与注释，标识符整词匹配）：`mcp.rs`、`doctask_mcp.rs` 不含标识符 `HumanProof`、`Presence`、`HumanOps`、`approve`、`reject`、`apply`、`recover`、`close_task`、`cancel`、`reopen`、`adopt`、`dismiss`、`relocate`、`rebase`、`acknowledge`、`confirm`、`revoke`、`baseline_add`、`restore`、`export_to_file`（工具输出字段名如 `baseline_version_no` 与错误码 `E_NO_BASELINE` 是字符串字面量或不同的标识符，不触发）；纯模块不含路径 `crate::store`、`crate::engine`；`entry.rs` 与 `main.rs` 不含标识符 `DocStore`、`execute`、`query`、`params`（即没有 SQL 调用），分支是否精简由评审判断而不由扫描判断 |
| 服务层 | `authorize(主体, Operation)` 表与人在场证明参数；Agent 主体对人工专属操作返回 `E_HUMAN_ONLY` | 服务层测试对每个人工专属操作断言 |
| 回放 | `doc verify` 回放事件链，人工专属种类的 `actor_kind` 不是 `human` 即 `HUMAN_ONLY_FORGED` | 篡改测试 |

### 13.5 边界声明
本地模式没有登录，批准人名称是自由文本；人在场证明是纵深防御，不是安全边界。同一 Windows 用户下能执行任意代码的进程仍可改写整个数据库、读取配置、调用命令行（管道与 msys、cygwin 伪终端上的人工专属命令会被拒绝，因为人在场证明要求 `GetConsoleMode` 成功；但 ConPTY 一类自建的伪控制台提供真实的控制台句柄，无法与人区分，属于残余风险）。事件链无签名，只防部分篡改，不防整库重写。Hermes 的工具集由 `-t ladon,clarify` 限定，只要所有者不在自己的 Hermes 配置里让终端类工具绕过该白名单〔假设〕，Hermes 无法执行 shell；`hermes doctor` 输出实际使用的 `-t` 值。Hermes 在 `untrusted` 下的逐次确认不是 LADON 批准，LADON 的批准记录只能由控制台或一次性命令在真实控制台上产生。

### 13.6 输入输出卫生
写入前拒绝控制字符（换行与制表符除外）、NUL，并按各字段长度限制校验；渲染前把 ESC、其他控制字符、双向控制符、零宽字符替换为可见占位，行宽按显示宽度截断到 400 字符。所有绑定值经参数绑定，不拼接 SQL；绑定值限定为 `&str` 与 `i64`，`String` 经 `.as_str()` 传入，布尔值存 0/1，时间为 RFC 3339 字符串（同 `36_数据库事务与变更策略.md` 第 7 节）。

## 14. 错误处理

### 14.1 错误码
CLI、控制台与 MCP 共用同一组稳定错误码（`DocError{code, message_zh, hint_zh, detail}`）。
| 错误码 | 含义 | 退出码 |
|---|---|---|
| `E_INVALID_ARG` | 参数缺失、越界、含控制字符 | 4 |
| `E_NOT_FOUND` | 任务、文件、批注、方案、版本不存在；对 Agent 还包括被门禁拒绝与 `local_only`，不区分原因，不附候选 | 3 |
| `E_AMBIGUOUS` | 人工主体的句柄或选择器匹配多个（附编号候选） | 3 |
| `E_SECTION_NOT_FOUND` | 选择器在版本中无匹配 | 3 |
| `E_NO_BASELINE` | 文件没有未撤销的基线 | 4 |
| `E_FILE_MISSING` | 文件已删除或不再跟踪 | 4 |
| `E_TRANSITION_DENIED` | 迁移不在迁移表内（附可做的迁移） | 4 |
| `E_GUARD_FAILED` | 守卫不满足（依赖、证据、方案未应用、存在 `applying` 方案等） | 4 |
| `E_HUMAN_ONLY` | 该操作只允许人工主体 | 5 |
| `E_NOT_INTERACTIVE` | 需要真实控制台 | 5 |
| `E_POLICY_DENIED` | 外传门禁不可用（规则集无效或全局规则文件不可读），Agent 通道的所有含文件调用失败即关闭；与具体文件无关，人工视图没有门禁，因此只出现在 Agent 通道 | 8 |
| `E_STALE_PLAN` | 方案的目标版本或哈希已过期（方案 `superseded`，任务回到 `analysis`，状态已提交） | 6 |
| `E_STALE_SUBJECT` | 展示后被批准、验收、采纳、确认、重定位、重定基或登记的主体已变化 | 6 |
| `E_CONFLICT` | CAS 失败、应用时摘要不符 | 6 |
| `E_FILE_CHANGED` | 现存文件与账本不一致（需要先 `capture`） | 6 |
| `E_DEP_CYCLE`、`E_DEP_DANGLING` | 依赖成环、目标不存在 | 4 |
| `E_ANCHOR_BROKEN` | 锚点状态不允许当前操作（附 `ack`、`relocate`、`rebase` 提示） | 4 |
| `E_UNSUPPORTED_FILE` | 文件类型不支持该操作 | 4 |
| `E_PLAN_TOO_LARGE` | 方案文本或目标文件超限 | 4 |
| `E_QUOTA`、`E_RATE_LIMIT` | 提案配额、调用频率 | 4 |
| `E_STORE_BUSY` | 重试 6 次后数据库仍被占用（MCP 中为普通结果，提示稍后重试） | 7 |
| `E_SCHEMA_OUTDATED` | Schema 版本低于 4（MCP 中为普通结果，提示所有者执行 `doc migrate`） | 4 |
| `E_MIGRATION_BACKUP` | 迁移前备份无法验证且没有所有者确认 | 4 |
| `E_INTEGRITY` | `doc verify` 发现错误级问题 | 9 |
| `E_INTERNAL` | 意外失败 | 1 |

### 14.2 规则
1. 每个变更与其链事件在同一个 Immediate 事务内提交；任一步失败则整体回滚，不留下没有事件的记录（新路径的写入是单事务的，旧接口的两连接写入不变）。唯一的例外是 T3 守卫 4 的过期处理：方案 `superseded` 与 T11 作为一个事务提交，随后返回 `E_STALE_PLAN`。
2. `Rows` 在同一连接的下一条语句之前释放；`params!` 只绑定 `&str` 与 `i64`。
3. 业务拒绝、参数不合法、数据库暂忙在 MCP 中都是普通结果，真实故障才是 `isError`；panic 经 `CatchPanic` 转为 `E_INTERNAL`，不使服务器退出（第 10.3 节）。
4. 过期与冲突不做部分成功：方案过期或应用冲突时不写文件，记录事件并告知下一步。
5. 中断可恢复：`applying` 方案由 `doc recover` 收敛（第 8.4 节）；启动后未结束的 Hermes 会话由 `hermes status` 与 `doc verify` 提示。
6. 所有错误文案有中文与英文两版，错误码与命令名不翻译。

## 15. 测试策略与需求—测试追溯

### 15.1 总则
1. 首次完整运行以 GitHub Actions（windows-latest、MSVC、`cargo test --locked --all-targets`）为准；本机在内存紧张时无法构建，纯模块可在仓库之外的临时工程（以 `#[path]` 引入、独立 target 目录、不引用 turso）中先行编译与测试，验证方式见 `58_文档任务与HermesCLI开发WBS.md` 第 5 节。
2. 每个工作包先做仅含设计的提交（`.quality/docs`、`.quality/test-design`、封印的计划文件），再写代码；每个被改动的源文件行覆盖率必须严格高于 90%，度量方法与验证流程由 WP00A 确立，在其落地之前不声称达标。`main.rs` 由 WP00C 缩减为对 `entry` 的一次调用；`entry.rs` 的全部逻辑可在进程内测试，遗留命令（`watch`、`google_login`、`tag_folder`、shell 宿主）经注入接缝由特征化测试覆盖。
3. 零新增依赖，`Cargo.toml` 与两份 `Cargo.lock` 不变；新代码按 rustfmt 格式书写，既有稠密文件不重排。非 Windows 桩的 clippy 通过与模块级 `deny` 属性只有在运行 clippy 时才被验证：`rust-ci.yml` 的 ubuntu 作业在 `cargo fmt --check` 处先于测试与 clippy，且 fmt 在既有文件上已失败，其 clippy 步骤从不执行；`release.yml` 不含 clippy。证据通道是所有者批准的 CI 调整（fmt 不阻断或置于最后，或独立的 clippy 作业）或本机 gnullvm 的 `cargo clippy --all-targets -- -D warnings` 通道；二者之一建立之前，该项报告为“未验证”，stdout 守卫以 IT-062 与 IT-067 为准。
4. 编号：已使用的最大编号为 REQ-100、NFR-043、UT-140、IT-046、ST-075、TC-818、RSK-20，本设计顺延分配 REQ-101～、NFR-044～、UT-141～、IT-047～、ST-076～、TC-819～、RSK-21～；UAT 复用 UAT-01 与 UAT-05。
5. 每个工作包的测试设计含“验收条目—TC 编号—输入—断言”对照表，每条验收对应至少一个可被证明会失败的 TC，在该包的设计提交之前由 WP00 的核对脚本（只读，不入库）检查条目与 TC 区段一一对应。性能类要求给出固定夹具：NFR-044 的夹具为 200 个跟踪文件、100 个任务、1000 条链事件、30 个带未终结任务的锚点；CI 上只断言不超过 10 秒（作为回归报警，并记录实测值），3 秒目标由所有者机器上以同一夹具生成的热工作区在 ST-077 中手工度量。

### 15.2 测试层次
| 层 | 内容 | 位置 |
|---|---|---|
| L1 纯单元 | 迁移表穷举、选择器、哈希金标、大纲与分类矩阵、拼接、渲染与对齐、报告与 .NEMA 往返、MCP 行处理、控制台分词、hermes 构造器、winconsole | 各模块内 `#[cfg(test)]` |
| L2 存储与服务集成 | 真实临时 turso 文件（`tests/it_store.rs` 的夹具风格）：迁移、CAS 竞争、链篡改、回填、全生命周期、门禁、对账、谱系、方案与恢复 | `tests/it_doctask_*.rs` |
| L3 桩 MCP 客户端 | 辅助程序以 `env_clear()` 加 Hermes 基线环境（第 12.7 节第 7 项）启动 `CARGO_BIN_EXE_ladon mcp-serve`，重放 WP00B 从 Hermes 所用的 `mcp` SDK 客户端录得的字节序列金标（`tests/fixtures/mcp_client/*.jsonl`：`initialize` 2025-11-25 带采样与征询能力、`notifications/initialized`、`tools/list`、调用、`ping`、EOF）；LADON 的回复同样在 WP00B 中以 SDK 的 `mcp.types` 模型验证过，因此桩不会比真实客户端宽松。断言：每行一个 JSON 对象、含 `tools` 能力、无 `outputSchema`、业务错误无 `error` 键、已知工具的非法参数得到不带 `isError` 的 `ok:false` 结果（不触发熔断）、数据库被占用时 `initialize` 仍在 200 ms 内回复且工具调用得到普通结果 `E_STORE_BUSY`、Schema 过低得到普通结果 `E_SCHEMA_OUTDATED`、仅在全局忽略文件中被排除的文件在 MCP 进程内被遮蔽、EOF 后 2 秒内退出、中文往返、完整代理生命周期 | `tests/support/mcp_stub.rs`、`tests/it_mcp_stdio.rs` |
| L4 假 Hermes | 测试时在临时目录生成与真实启动器同形的两层链：`hermes.cmd` 调用 `powershell -NoProfile -NonInteractive -File fake.ps1 %*`；`fake.ps1` 记录 `$args`（逐元素）与环境变量名（只记名称不记值），记录 `HERMES_EPHEMERAL_SYSTEM_PROMPT` 的 SHA-256 与长度（不记值，使 4000 字符的中文指令是否被截断可被断言），复制覆盖层，按测试指定的码退出；校验 argv（含 `ladon,clarify` 作为单个元素到达、空格与 `-c` 的处理）、`LADON_*` 不可见、`HERMES_MANAGED_DIR` 与 `HERMES_EPHEMERAL_SYSTEM_PROMPT` 存在且哈希与长度正确、`env_clear` 之后 PowerShell 仍能启动、退出码映射、目录创建与清除、预置变量拒绝、`hermes resume` 的披露检查；该路径覆盖批处理与 PowerShell 的二次参数解析（生产风险所在） | `tests/it_hermes_launch.rs`（Windows 限定） |
| L5 二进制冒烟 | `CARGO_BIN_EXE_ladon` 加 `std::process::Command`，无新增开发依赖：`doc` 全流程、`--json` 可解析、人工专属命令在管道上退出 5、旧命令输出行、控制台脚本模式 | `tests/it_doctask_cli_binary.rs` |
| L6 手工系统测试 | 真实 Hermes 与可见窗口：doctor、交互会话、Ctrl+C、`taskkill` 无孤儿、`Terminate batch job`、中文输入输出、控制台模式恢复、敏感文件不外传 | ST-076 起 |
另有：架构守卫（`tests/it_arch_guard.rs`）、内核 API 冻结（`tests/it_kernel_api_freeze.rs`，只编译）、跨进程并发测试（子进程与测试进程同时读写同一工作区，两侧每次存储调用之间各停顿 10～30 ms 的随机时间，使竞争者不会长期饿死重试者；全部成功，链校验通过，并记录第二个进程得到的锁错误对应的 `turso::Error` 变体与文本）。

### 15.3 需求目录
| 编号 | 名称 | 优先级 |
|---|---|---|
| REQ-101 | 任务生命周期与人工批准规则 | P0 |
| REQ-102 | 已批准基线登记 | P0 |
| REQ-103 | 条款锚点、指纹与迁移 | P0 |
| REQ-104 | 任务审计链与完整性校验 | P0 |
| REQ-105 | 存量数据兼容与 Schema 迁移 | P0 |
| REQ-106 | 关联、证据、依赖与并行调度 | P0 |
| REQ-107 | 方案、批准与受控写入 | P0 |
| REQ-108 | 审计报告与 .NEMA 导出 | P0 |
| REQ-109 | 一次性命令行 `ladon doc` | P0 |
| REQ-110 | MCP 文档任务服务器 | P0 |
| REQ-111 | 交互控制台 | P0 |
| REQ-112 | Hermes 受控启动 | P0 |
| REQ-113 | Agent 外传门禁 | P0 |
| REQ-114 | 文件移动谱系确认 | P0 |
| REQ-115 | 文件图标与可执行文件图标（交付范围与设计见 58 号文档 WP20） | P0 |
| NFR-044 | 控制台热工作区启动不超过 3 秒 | — |
| NFR-045 | `mcp-serve` 的 `initialize` 在数据库被占用时 200 ms 内回复，EOF 后 2 秒内退出 | — |
| NFR-046 | 跨进程数据库竞争以有界重试（6 次，约 0.8 秒窗口）消化，超出时给出明确错误 | — |
| NFR-047 | 零新增依赖，`Cargo.toml` 与两份 `Cargo.lock` 不变 | — |
既有文档任务相关需求的处置（目标状态以自动化测试通过为 Implemented，对应的 ST 保持 Not Run 时不标记 Tested）：
| 需求 | 现状 | 本版本目标 | 对应或延后原因 |
|---|---|---|---|
| REQ-023 已批准要件锚点 | P0 Design | Partial | 文件版本级基线登记与锚点增量对账（REQ-102、REQ-103；WP06、WP08；IT-051、IT-055）；“仅用相关批准 REQ 筛选并提示 Agent”由 `doc_clause_list`、`doc_mentions_find` 提供确定性输入，筛选建议由模型作出，ST-010 保持 Not Run |
| REQ-024 文档即任务 | P0 Partial | Implemented | REQ-101～103、107；IT-005（WP06 加强）、IT-052；ST-084 |
| REQ-025 条款关联图谱 | P0 Partial | Partial | 任务到设计、代码、UT/IT/ST 的关联与条款追踪屏，任务与条款双向可查（WP07、WP10；IT-057、IT-066、UT-153）；独立于任务的条款间影响关系延后，本设计的关联取代未使用的 `graph_edges` 表 |
| REQ-026 新需求冲突守卫 | P0 Design | Deferred | Hook 与 Concept Guard 不在本设计内（第 1.3 节） |
| REQ-027 .NEMA格式支持 | P0 Partial | Partial | 只使用 `>` 有向图子集的导出与回读（UT-153）；Table、5W1H 语法的解析不在本设计内，`nema.rs` 不改动 |
| REQ-028 关系图并行调度 | P0 Partial | Implemented（任务 DAG 范围） | `topo_batches` 分层、冲突组与环阻断（UT-146、IT-057、UAT-05） |
| REQ-029 FileSpace隔离 | P0 Partial | Deferred | FileSpace 租约与多 Agent 运行时不在本设计内 |
| REQ-030 Agent写入协调 | P0 Design | Partial | 单文件章节写入的版本戳校验、批准后提交与冲突保护（REQ-107；IT-058、IT-059）；ST-012 的“两个 Agent 并发”依赖 FileSpace，保持 Not Run |
| REQ-031 CLI/MCP/Skills命令 | P1 Partial | Partial | 文档任务范围内 CLI、控制台、MCP 共用 `DocTaskService`（IT-060、IT-062、IT-067）；GUI 与 Skills 入口不在范围内，ST-013 依赖 GUI，保持 Not Run |
| REQ-042 原生文档内联编辑 | P0 Partial | Partial | 批注、统一差异、修改原因关联任务与版本事件（REQ-107；ST-023 由 ST-084 覆盖）；GUI 内联编辑界面延后 |
| REQ-043 文件附属知识笔记 | P1 Partial | Partial | 批注以 `file_id` 存于 Turso，移动经谱系确认后保持（REQ-114；IT-056；ST-024 由 ST-087 覆盖） |
| REQ-044 统一操作审计 | P0 Partial | Implemented（文档任务范围） | 每个变更写入事件链（REQ-104；IT-049、IT-052）；GUI 路径尚无文档任务写入 |
| REQ-056 AI／团队会话切换 | P0 Design | Deferred | 底部 AI 聊天栏在文档任务版本发布之后另行设计 |
| REQ-060 Agent 文件夹操作协议 | P0 Partial | Partial | 新增 `doc_*` 工具（REQ-110）；目录枚举、Grep、指纹、订阅等 `AgentApi` 能力不变 |
| REQ-061 Agent 安全写入与恢复接口 | P0 Design | Implemented（单文件章节写入） | 计划、版本戳校验、人工批准、受控提交与恢复（REQ-107；IT-058、IT-059）；ST-058 的“无审批、旧哈希被拒”由 IT-058 对应并在 ST-084 中人工确认 |
| REQ-062 Agent 事件订阅与增量快照 | P0 Design | Deferred | `subscribe`、游标、ack 不在本设计内；UC-11 的“通知 Agent”以拉取方式交付：Hermes 启动时经 `doc_task_list` 取得待办，所有者用 `doc inbox --watch` |
| REQ-067 成员角色与分享 | P0 Partial | Deferred | `auth.rs` 不接入，本设计只有 Human、Agent、System 三类主体 |
| REQ-069 聊天到文档任务转换 | P0 Design | Deferred | 依赖底部聊天栏（第 1.3 节）；`source_ref` 列为后续接入保留 |
| REQ-070 完整历史外部审计 | P0 Partial | Partial | 任务审计链校验、回滚链与报告（REQ-104、REQ-108；IT-049、IT-066）；文件历史与云盘状态的审计不变 |
| REQ-072 文档即任务实时侧栏 | P0 Design | Deferred | GUI 接线在发布之后；控制台的追踪屏提供同一数据的文本形式 |
| REQ-094 多信号涌现式建议 | P0 Partial | Deferred | 涌现式建议落地不在本设计内（第 1.3 节） |
| REQ-095 执行与审批强分离 | P1 Design | Partial | Agent 只能产生提案、方案与证据，应用只能由人在真实控制台执行（REQ-101、REQ-107）；涌现建议部分延后 |
| REQ-096 云端目录信息安全 | P1 Design | Deferred | Drive 链接不在本设计内 |
| REQ-097 Agent 文件夹能力租户限制 | P1 Design | Partial | 文档任务工具的工作区范围、敏感策略、幂等与审计（REQ-113）；身份授权与多租户不在范围内 |
| REQ-098 个人本地模式独立运行 | P1 Design | Implemented（文档任务范围） | 文档任务功能不依赖网络与登录（Hermes 自身的模型访问不在范围内） |
| REQ-099 双向文件/任务历史关联 | P1 Design | Deferred | 聊天引用与聊天源证据依赖底部聊天栏 |
本版本不是 1.0。依据 `14_验收判定标准.md` 第 3 节（P0 全部 Implemented + Tested，P1/P2 不得在 1.0 宣传为现成功能），发布后仍未完成的 P0 为：Deferred 的 REQ-026、029、056、062、067、069、072、094，以及 Partial 的 REQ-023、025、027、030、042、060、070；发布说明按此如实列出。

### 15.4 追溯表
| 需求 | 设计章节 | UT | IT | ST / UAT | 工作包 |
|---|---|---|---|---|---|
| REQ-101 | 4 | UT-141、UT-142 | IT-052、IT-053 | ST-084 | WP02、WP07、WP11 |
| REQ-102 | 5.3 | — | IT-051、IT-070 | ST-084 | WP06 |
| REQ-103 | 5 | UT-147～150 | IT-055 | ST-086 | WP03、WP08 |
| REQ-104 | 4.7～4.9 | UT-144 | IT-049、IT-069 | — | WP05、WP07 |
| REQ-105 | 3 | UT-162 | IT-047、IT-048、IT-050、IT-061、IT-073 | — | WP01、WP05 |
| REQ-106 | 6.1～6.3、7 | UT-146 | IT-057 | UAT-05 | WP07 |
| REQ-107 | 8 | UT-150 | IT-058、IT-059 | ST-084 | WP14 |
| REQ-108 | 6.4、6.5 | UT-153、UT-154 | IT-066 | UAT-01 | WP10 |
| REQ-109 | 9 | UT-143 | IT-060、IT-061 | — | WP11 |
| REQ-110 | 10 | UT-155～157 | IT-062、IT-063、IT-067、IT-072、IT-074 | ST-076 | WP00B、WP12、WP13 |
| REQ-111 | 11 | UT-145、UT-151、UT-152、UT-158 | IT-064、IT-070 | ST-077、ST-078、ST-081、ST-082、ST-085、ST-088 | WP09、WP17、WP18 |
| REQ-112 | 12 | UT-159～161 | IT-065、IT-071 | ST-076、ST-078～ST-080、ST-082 | WP00B、WP15、WP16 |
| REQ-113 | 13.1 | — | IT-054、IT-063、IT-071、IT-074 | ST-083 | WP04、WP06、WP13、WP16 |
| REQ-114 | 5.7 | — | IT-056 | ST-087 | WP08 |
| REQ-115 | 见 58 号文档 WP20 | TC-1000～1009 中的 `fileicon` 与 `rc_tool` 单元测试 | TC-1000～1009 中的 `it_build_rc_tool`、`it_exe_icon` | ST-089 | WP20 |
| NFR-044 | 11.1 | — | IT-064（CI 上 ≤10 秒的回归报警） | ST-077（3 秒目标的手工度量） | WP17 |
| NFR-045 | 10.1 | — | IT-062 | — | WP13 |
| NFR-046 | 2.4 | UT-162 | IT-069 | — | WP01 |
| NFR-047 | 2.1 | — | IT-068 | 每包的 `git diff` 检查 | 全部 |
UAT-01（管理已批准要件）由 IT-070 自动化对应并在 ST-084 中人工执行；UAT-05（多 FileSpace 并行）由 IT-057 自动化对应。

### 15.5 测试用例清单
**UT-141～162**
| 编号 | 内容 | 模块 |
|---|---|---|
| UT-141 | 状态迁移矩阵穷举（7×7×3 类主体）与终态 | `doctask_model` |
| UT-142 | 人工专属操作对 Agent、System 拒绝（操作表） | `doctask_model` |
| UT-143 | 句柄、选择器、限制、文本校验 | `doctask_model` |
| UT-144 | 事件哈希金标与字段敏感性；方案哈希、证据哈希稳定 | `doctask_model` |
| UT-145 | 控制字符与双向字符的拒绝与渲染净化 | `doctask_model`、`doctask_render` |
| UT-146 | 调度分批、冲突组、环拒绝 | `doctask_model` |
| UT-147 | 大纲键与 `diff::section_hashes` 逐键一致（含重复标题、NEMA 域） | `doctask_outline` |
| UT-148 | 条款编号提取与 `ROW` 虚拟条款 | `doctask_outline` |
| UT-149 | 锚点分类矩阵：改名重定位、级别变化、CRLF、围栏 `#`、`ROOT`-only、冷版本；前置重复标题（插入同名章节使原章节后移为 `#2`：全文相同为 `relocated` 方法 `full_hash`、原章节同时被改为 `needs_review`、多个相同为 `ambiguous`）；`body_hash` 重定位下限边界（1 行、2 行、39 与 40 个字符）与 `TBD` 类短正文为 `ambiguous`；`relocated` 在 `ack` 之前不可批准 | `doctask_outline` |
| UT-150 | 字节级章节拼接：CRLF、混合换行、BOM、缺失末尾换行时范围外字节逐字节不变，插入与替换，`new_text` 的 `\r` 处理；无效 UTF-8、NUL、UTF-16 目标被拒绝 | `doctask_outline` |
| UT-151 | CJK 显示宽度与表格对齐 | `doctask_render` |
| UT-152 | 各屏幕 golden；中英文案键集一致；纯文本无 ANSI | `doctask_render` |
| UT-153 | .NEMA 导出经 `parse_graph_edges`、`topo_batches` 往返；记号与标签净化 | `doctask_report` |
| UT-154 | 报告确定性（含连续导出两次逐字节相同，`dt.export.generated` 不进入内容）；排除 `local_only` 批注与任务；回滚链 | `doctask_report` |
| UT-155 | MCP 握手版本协商 | `mcp` |
| UT-156 | MCP 通知、ping、未知方法 id 回显、坏 JSON、无效 UTF-8、超长行、EOF | `mcp` |
| UT-157 | 工具列表快照：WP13 时 13 个工具，WP14 追加 `doc_plan_submit` 后 14 个；`readOnlyHint` 恰在 8 个只读工具、无 `outputSchema`、平铺模式、名称长度；`CatchPanic`：故意 panic 的 `ToolHost` 得到 `E_INTERNAL` 且循环继续，panic 钩子只写位置（测试串行化并恢复原钩子） | `doctask_mcp` |
| UT-158 | 控制台分词、中文引号、别名、建议、命令解析 | `console` |
| UT-159 | 覆盖层快照（含 `--global-ignore`）、argv、环境白名单与禁转发（夹具含 `Path`、`SystemRoot`、`ProgramFiles(x86)` 与混合大小写的 `Ladon_Archive_Key_Hex`：前三者保留原大小写，后者被拒绝）、预置变量拒绝、`.cmd` 参数安全、路径前缀剥离 | `hermes` |
| UT-160 | 指令文本威胁模式扫描与长度 | `hermes` |
| UT-161 | `winconsole` 管道安全、作业结构大小、Ctrl+C 门返回值、非 Windows 桩、`is_real_console` 以管道句柄为假、msys 与 cygwin 管道名匹配（纯函数） | `winconsole` |
| UT-162 | 连接重试（可注入打开器）：假打开器返回锁错误文本 N 次后成功、退避序列与抖动范围、耗尽后给出明确信息并映射 `E_STORE_BUSY`、非锁错误不重试 | `store` |
**IT-047～074**
| 编号 | 内容 |
|---|---|
| IT-047 | 迁移 004：全新库、从 0.0.3 形态升级、已记录版本不重复执行、`ensure_schema` 幂等；`Store::init`、`Engine::init` 不应用 004 |
| IT-048 | turso 行为探针：影响行数、`BEGIN IMMEDIATE`、CHECK 与外键的观测记录；唯一索引违反的错误变体；`AUTOINCREMENT` 序号在被回滚的插入之后是否连续（决定 `SEQ_GAP` 的级别）；第二个进程打开时得到的 `turso::Error` 变体与文本 |
| IT-049 | `DocStore`：事务回滚无残留、CAS 单赢家、链线性追加、两个连接同时追加时唯一索引使后者重读链尾重试、链篡改与删除检出、`STATUS_DRIFT` |
| IT-050 | 存量回填：编号顺序、`unchecked`/`unknown` 标注、批注与任务 `local_only`、事件导入、幂等；任何链载荷都不含批注文本或任务标题（只含哈希与长度），`task.state_changed` 的合法状态以明文导入；`doc_task_events`、`doc history`、Agent 的 `doc_task_get`、默认报告都不出现旧批注文本；每个非 `open` 的旧任务有 `dt.task.imported`，`doc verify` 无错误级发现；回滚到 0.0.3 使用后再升级并 `doc migrate --import-legacy`，`doc verify` 仍无错误级发现 |
| IT-051 | 服务：基线登记（人工）、批注、任务创建、`E_NO_BASELINE`、锚点钉基线版本；人工专属操作的 `expected_*` 在展示后主体变化时得到 `E_STALE_SUBJECT`、现存文件未捕获时基线登记、`ack`、`rebase` 得到 `E_FILE_CHANGED`；`--yes` 无 `--expect-hash` 被拒；基线撤销后同一版本可重新登记且 `doc verify` 无 `BASELINE_DUPLICATE` |
| IT-052 | 服务：open→done 全生命周期、全部非法迁移、驳回回路、重开；T3 守卫 4 失败时方案 `superseded`、任务回到 `analysis`、状态已提交且返回 `E_STALE_PLAN`；只有 Agent 通过证据时 T6 得到 `E_GUARD_FAILED`，`--accept-agent-evidence` 与书面豁免各自通过且 id 写入批准载荷；`relocated` 未确认时 T3、T6 被拒 |
| IT-053 | 服务：Agent 对全部人工专属操作被拒、提案采纳与忽略、配额；未采纳的提案上 `doc_task_transition` 与 `doc_plan_submit` 得到普通结果 `E_TRANSITION_DENIED` 且提示所有者采纳 |
| IT-054 | 外传门禁：敏感文件脱敏、排除内容的副本、策略失败即关闭、`local_only` 批注与任务隐藏、人工视图完整；历史版本：当前未标记的文件的旧版本曾被标记敏感、旧版本字节与某个被排除文件相同时，`doc_clause_read`、`doc_clause_list`、任务卡、追踪、`doc_mentions_find` 都不返回其文本；调度、关联、追踪、阻塞者经 `AgentView` 省略并以 `hidden` 计数；被拒绝、`local_only` 与不存在的引用对 Agent 得到完全相同的 `E_NOT_FOUND` 且无候选；Agent 事件视图不含载荷 |
| IT-055 | 锚点对账：改名重定位、正文改动 `needs_review` 与确认、删除、冷字节、幂等；前置重复标题得到 `relocated`（`full_hash`）；`relocated` 须 `ack` 或 `relocate` 后才可批准、验收、应用；短正文不自动重定位 |
| IT-056 | 谱系：提议、确认（人工）、拒绝不再提议、列表跟随 |
| IT-057 | 依赖、环与悬空、调度批次与冲突组（UAT-05 对应） |
| IT-058 | 方案：提交、批准、应用、过期方案、外部编辑冲突、账本原因前缀与证据、回滚链；`result_hash` 等于 `expected_hash` 的方案被拒；基线之后又被编辑的目标文件：批准界面的差异应用于 `expected_version_id` 的字节后，与结果字节及 `commit_text` 写入的字节逐字节相等；`commit_text` 在 `capture` 返回 `unchanged`（`watch` 抢先捕获）时 A4 仍链接正确的版本，竞争发生在放置与 A4 之间，`doc recover` 同样；`cfg(windows)` 的 junction 目录组件使应用被拒且不移动任何文件；两个方案写同一文件时后者 `E_STALE_PLAN` |
| IT-059 | 恢复：`applying` 的各种状态（目标为旧内容、目标缺失而备份在、目标为新内容且账本已有或没有新版本、第三种内容、目标缺失且备份缺失）收敛为规定结果；A1 至 A4 每两步之间的中断（注入点）以及注入的并发 `capture` 与 `mark_deleted`，文件最终只可能是旧内容、新内容或保留现场的内容，账本与事件链一致 |
| IT-060 | 二进制冒烟：`doc` 全流程、`--json`、退出码、管道上人工专属命令退出 5 |
| IT-061 | 旧命令契约：`annotate`/`task` 输出行与位置参数、未知章节记 `unchecked` |
| IT-062 | MCP 桩客户端：握手字节序列（WP00B 金标）、工具集、业务错误无 `error` 键、EOF 2 秒内退出、数据库被占用时 `initialize` 仍回复、中文往返；每行 stdout 恰为一个 JSON 对象 |
| IT-063 | MCP 门禁与配额、重复创建；未知工具名与人工专属工具名得到 -32602 且不写任何行；已知工具的非法参数（越长、枚举外、类型错误、缺字段、控制字符、未知属性）得到不带 `isError` 的 `E_INVALID_ARG` 普通结果且不使熔断计数增加 |
| IT-064 | 控制台脚本会话：收件箱批准、键入任务号、脚本模式拒绝、启动时间（NFR-044 夹具，CI 上 ≤10 秒的回归报警）；收件箱对只有 Agent 证据的验收只提供接受证据与书面豁免 |
| IT-065 | Hermes 假启动器（两层 `.cmd` 与 `powershell -File` 链）：argv、环境名、指令文本的 SHA-256 与长度、覆盖层、退出码映射、预置拒绝、启动目录创建与清除 |
| IT-066 | 报告与导出集成：完成判定端到端、.NEMA 回读 |
| IT-067 | 架构守卫（词元扫描，第 13.4 节的标识符清单） |
| IT-068 | 内核 API 冻结（只编译） |
| IT-069 | 跨进程并发（占空比见第 15.2 节） |
| IT-070 | 所有者一天端到端（UAT-01 对应自动化用例），含 Hermes 提出提案、所有者采纳、Hermes 在已采纳任务上提交方案的两次交接 |
| IT-071 | 恢复披露检查（假启动器与假 `mcp-serve` 记录）：规则文件改变、敏感标记改变、已披露内容的副本被放入排除路径、上次会话没有结束记录，各自使 `hermes resume` 开启新会话并给出说明；一切不变时恢复；披露记录在内容返回之前写入，写入失败时不返回内容 |
| IT-072 | `mcp-serve` 的只读不变量与错误形态：一系列 Agent 读调用之后 `doc_task_events`、`doc_anchors`、`doc_task_meta` 逐字节不变，只有 `doc_agent_disclosures` 增加；无 `meta` 的旧行对 Agent 不可见；`E_STORE_BUSY`、`E_SCHEMA_OUTDATED`、`E_INVALID_ARG` 都是带 `ok:false` 的普通结果；panic 携带的标记文本不出现在 stderr |
| IT-073 | 迁移前备份：`meta.turso` 与 `-wal` 成对复制并以行数核对；复制失败、副本打不开、行数不符时 004 不执行并要求确认，确认后写入 `backup_verified:false`；空库不备份 |
| IT-074 | 全局忽略规则在 MCP 进程内生效：仅存在于全局忽略文件中的规则，经 `env_clear()` 加 Hermes 基线环境启动的 `mcp-serve`（带 `--global-ignore`）仍遮蔽对应文件；全局文件不可读时所有含文件的调用得到 `E_POLICY_DENIED` |
**ST-076～089（手工，未执行时为 Not Run）**
| 编号 | 内容 |
|---|---|
| ST-076 | `hermes doctor` 与覆盖层透传、`-c … --query-file` 组合、工具以 `mcp__ladon__doc_*` 出现 |
| ST-077 | 交互会话：Hermes 创建提案，退出后在收件箱批准；控制台启动时间 |
| ST-078 | Ctrl+C：提示符处与 Hermes 内 |
| ST-079 | `taskkill /F` 终止 `ladon.exe` 后无遗留 Hermes、python、powershell、`mcp-serve` 进程 |
| ST-080 | 经两层 `.cmd` 时 `Terminate batch job (Y/N)?` 是否出现 |
| ST-081 | cmd、Windows Terminal、conhost 中的中文输入输出 |
| ST-082 | Hermes 异常退出后控制台模式与输出编码恢复 |
| ST-083 | 敏感文件内容不发送给模型 |
| ST-084 | 全流程：条款→批注→任务→Hermes 提案→所有者采纳→Hermes 在已采纳任务上提交方案→批准→应用→证据→验收→报告与 .NEMA 导出（UAT-01） |
| ST-085 | Hermes 打开时在第二终端批准 |
| ST-086 | 外部编辑条款后的启动屏提示与确认条款 |
| ST-087 | 文件移动后的谱系确认 |
| ST-088 | 运行 `ladon shell-menu <文件>`，在其菜单中选择“文档即任务”（`0x7003`）打开新的控制台窗口；文件名含 `&`、`^`、`%` 时同样成功（MSI 没有登记资源管理器右键谓词，该入口只存在于此菜单） |
| ST-089 | 文件图标与可执行文件图标显示（WP20） |

### 15.6 TC 编号分段
| 区段 | 工作包 | 区段 | 工作包 |
|---|---|---|---|
| TC-820～829 | WP01 迁移与连接 | TC-910～919 | WP09 渲染 |
| TC-830～839 | WP02 模型 | TC-920～929 | WP10 报告与导出 |
| TC-840～849 | WP03 大纲与分类 | TC-930～939 | WP11 CLI |
| TC-850～859 | WP04 外传门禁 | TC-940～949 | WP12 MCP 协议 |
| TC-860～869 | WP05 `DocStore` | TC-950～959 | WP13 MCP 工具 |
| TC-870～879 | WP06 服务核心 | TC-960～969 | WP14 方案与应用 |
| TC-880～899 | WP07 服务流程 | TC-970～974 | WP15 `winconsole` |
| TC-900～909 | WP08 对账与谱系 | TC-975～984 | WP16 Hermes 启动 |
| TC-985～994 | WP17 控制台 | TC-995～996 | WP18 菜单入口 |
| TC-997～999 | WP19 端到端 | TC-1000～1009 | WP20 图标 |
| TC-1010～1014 | WP00A 质量门禁与覆盖率度量 | TC-1015～1019 | WP00B Hermes 集成探针 |
| TC-1020～1029 | WP00C 命令入口库化 | | |
