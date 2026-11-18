> 设计基线候选 · 2026-11-18 · IPA 共通フレーム参照 · 尚待设计评审/签字。


# 35. .NEMA AST/依赖图/结构化 Diff 詳細設計

## 1. 规范基线
参考用户公开语法：https://github.com/UlyssesLeoLee/.NEMA/blob/main/website/grammar.html 。正式 .NEMA 主域：Table（` 空格\数字 版本`）、5W1H（` 空格\?数字 版本`）、Node–Edge（` 空格\>数字 版本`）；保留一个 ASCII U+0020 前导空格，不使用 Tab/全角空格；正式语法需以用户最新 Codec 验证。

## 2. 系统实体映射
Table 描述 FileEvent/Version/Tag/Task；5W1H 列序 `what why how who when where`，用于修改原因/责任；Node–Edge 用于稳定锚点关系与 Agent DAG。文件版本由 Domain ID/Revision 与原文件 FileRef 共同定位。Node ID 不等价于文件路径。

## 3. 解析器与兼容性
当前 `src/nema.rs` **只实现**以 `>` 表示的有向图节点和边提取、悬空边拒绝以及拓扑分批；支持 Node–Edge 中 `A>E1>B` 形式，但尚未实现全文语法中的 Table、5W1H 完整 AST、字典压缩、反向/无向边、转义、跨 Domain 语义关联。`diff.rs` 仅支持按 Domain 头部的章节指纹比较。

## 4. 生产版完整增量设计
增量 parser 为每个域输出 `Domain{type,id,rev,columns|records|edges,hash}`，上层再对 Record / Edge ID 做细粒度 Hash。变化事件存 `changed_domain_ids`, `changed_record_ids`, `added_edges`, `removed_edges`；只命中的下游锚点进行影响分析，避免整份文件向量化和全量 LLM 提示。

## 5. 并行判断
拓扑层序中互不依赖 Node 可并发分析，但写入同一 FileRef 或同一要件域的计划需串行提交；`Graph Cycle` 或 `Dangling Edge` 不得自动调度。`design/nema/parallel_tasks.nema` 可用 `ladon parallel` 试验。

## 6. 语法安全
原始反斜杠、`^`、空字段表示、字典压缩可能改变分词，必须在 Codec 层准确解析。对合规性最终验收需对照官方解释器及 139 个语法案例；当前内核的简单图解析不具备全部语言规范能力。
