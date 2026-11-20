> 设计基线候选 · 2026-11-18 · IPA 共通フレーム参照 · 尚待设计评审/签字。


# 34. .LADONignore 安全策略详细设计

## 1. 命名与规则
用户约定文件名为 `.LADONignore`（大小写严格遵守项目规范）；界面文字“LADON Ignore”。解析采用 `ignore::gitignore::GitignoreBuilder` 风格，支持 `*`, `**`, 目录、`!` 例外、`#` 注释。第一版加载全局变量 `LADON_GLOBAL_IGNORE` 指定的文件（指向不存在的文件时忽略），以及工作区内所有名为 `.LADONignore` 的文件（遍历不跟随链接，跳过 `.ladon` 与 `.git`）；规则文件无法解析时加载失败，不静默丢弃规则；工作区目录不存在时加载失败，错误文本为 `workspace not found`。各规则文件合并为以工作区根为基准的同一个匹配器：子目录中的 `.LADONignore` 的规则同样相对于工作区根解释，不以其所在目录为作用域（按目录限定作用域属于后续设计）。工作区内不存在任何 `.LADONignore`，且 `LADON_GLOBAL_IGNORE` 未指向存在的文件时，规则集为空，`LadonPolicy::load` 仍成功返回（`Engine::init` 在工作区根目录不存在 `.LADONignore` 文件时（`src/engine.rs` 中的 `if !rule.exists()`）写入示例规则文件，`Engine::open` 不写入）。

`add_sensitive`（右键“添加为敏感文件”与 `ladon sensitive-add`）向根 `.LADONignore` 追加以 `/` 起始的根锚定规则：路径分隔符（含 Windows 的 `\`）先统一规范化为 `/`，再将路径中的 `* ? [ ] ! # \` 以反斜杠转义，目录规则以 `/` 结尾，同一规则不重复写入，并写入注释行 `# LADON sensitive: exclude remote transfers`。目标必须已存在；工作区根目录本身、`.ladon/` 下内容和根目录的 `.LADONignore` 不可作为目标（返回 `invalid ignore target`），目标位于工作区之外时返回 `cannot exclude resource outside workspace`；规则只约束此后的外传，不撤回已上传的数据。

## 2. 命中语义
`.gitignore` 仅控制 Git 追踪，本规则控制**外部数据流**。匹配文件仍可本地查询、建立历史、编辑/恢复，默认禁止 Google Drive/NAS 外存、远端 AI 请求、远端 Embedding/日志同步等任何外传。`.LADONignore` 本身、`.ladon` 应用目录及 `.git` 内部数据强制保护，不接受 `!` 例外覆盖；这些路径无需任何规则即判定为排除。

`excluded` 对相对路径按工作区根解析；绝对路径必须位于工作区的规范化根之下（Windows 下为 `\\?\` 扩展长度形式），否则返回 `outside workspace` 错误。`ladon ignore-check` 把参数原样交给 `excluded`、不先规范化，因此在 Windows 上应传相对路径或规范化后的绝对路径。`allow_external` 在路径被排除时返回 `LADON policy denies external data transfer: <路径>` 错误。

## 3. 多级优先级
强制保护集合（内部 DB/证书/密钥） > 全局管理员拒绝 > 工作区拒绝 > 子目录拒绝 > 用户明确的分类例外。生产版要求“某个子目录 `!` 例外不能取消更高层强制保护”，不得简单套用 Gitignore 后返回允许；当前代码实现的是 gitignore 匹配与内部目录强制排除，不包含管理员规则优先级升级。具体而言，同一路径上后加入匹配器的规则优先；路径自身命中的 `!` 例外先于父目录的排除规则判定，因此被排除目录中的文件可以被后加入的 `!` 例外重新包含，与 Git 行为不同（Git 不允许重新包含已排除目录中的文件）。

## 4. 重评估与净化
规则发生变化后扫描已归档记录，发现过去泄露敏感资源时生成 `security.reclassification` 事件，显示供应商删除范围并由用户审批远端删除。不能保证平台内部备份立即物理擦除；应提示可见可控的最终状态。当前实现不做重评估与 `security.reclassification` 事件，上述为设计要求。

## 5. 路径安全
先 OS 级 File ID/重解析点检查，之后 Canonical Path 作用域对照，杜绝 `..`、symlink、UNC/device 路径绕过。当前实现未检查 File ID 与重解析点，作用域对照由规范化路径与根前缀比较完成（`archive.rs` 的 `assert_under_workspace`），`LadonPolicy::excluded` 仅做词法前缀比较；未来针对 Windows Reparse Point 与大小写不敏感路径逐项 ST。避免“检查时一个路径、上传时另一个路径”的 TOCTOU，通过已验证 FileRef/文件句柄访问。

## 6. 测试
规划的测试范围包括嵌套文件夹、例外、强制保护、目录改名、文件移动、忽略文件自身、远端 AI、云存储、向量索引、metadata 泄露、现有云端归档回查。当前 UT（`src/policy.rs`）覆盖常规模式、规则文件自身、工作区外路径与 `add_sensitive`：UT-010（`ut_policy_01_ignore_and_allow`）、UT-011（`ut_policy_02_rule_is_ignored`）的绝对路径取自规范化工作区根，UT-012（`ut_policy_03_outside_workspace`）使用工作区之外的固定路径 `/this/is/outside`，`ut_policy_04_add_sensitive` 覆盖 `add_sensitive`。集成测试 `tests/it_policy.rs` 包含 `tc505_add_sensitive_writes_anchored_rule_once`（根锚定与去重）、`tc506_add_sensitive_directory_rule`（目录规则）、`tc507_add_sensitive_escapes_glob_characters`（特殊字符转义）、`tc508_add_sensitive_rejects_invalid_targets`（非法目标）、`tc509_internal_directories_always_excluded`（`.git`/`.ladon` 强制排除）、`tc510_allow_external_follows_rules`（`allow_external`）、`tc511_invalid_rule_file_fails_load`（规则文件语法错误）、`tc512_load_requires_existing_workspace`（工作区不存在）；`tests/it_policy_global.rs` 的 `tc513_global_ignore_file_applies` 覆盖 `LADON_GLOBAL_IGNORE`（该测试独占进程，避免进程级环境变量竞争）；引擎侧的敏感内容哈希拦截与策略热加载见 `tests/it_engine.rs` 的 `tc314_archive_blocks_hash_shared_with_sensitive_file` 与 `tc308_reload_policy_applies_new_rules`。现有测试未覆盖 `!` 例外的行为。
