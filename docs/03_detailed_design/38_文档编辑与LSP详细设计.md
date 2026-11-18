> 设计基线候选 · 2026-11-18 · IPA 共通フレーム参照 · 尚待设计评审/签字。


# 38. Document Editor / LSP / Annotation 詳細設計

## 1. 文档视图
左侧 FileTree，中间文档（Markdown/.NEMA 原生渲染、PDF/Office 默认只读/兼容预览），右侧任务/批注/版本影响区。每条可编辑要件应有稳定 `section_key` 与 `base_version_id`，点击条款显示“创建批注/变更建议/查看下游测试”。

## 2. 注释数据流
UI 选中文档文本范围→`CreateAnnotation(file_ref,base_version,section,range,text)`→Turso 增量写入 annotation 和治理事件。文档移动/修改时按稳定锚点复核映射；如果锚点失效则保留旧指向并等待人工重定位，不丢失历史说明。

## 3. 编辑保存
外部编辑和内联编辑都经 File Runtime 捕获；文档保存必须采用 OS 推荐的安全写策略与竞争检查，提交后记录主事件并自动关联 Task/CR。不能让任意 AI 返回的代码块直接覆盖批准的要件文件。

## 4. LSP 池化
与 The Real Explorer 类似，通过后缀映射 LSP；首次打开对应文件时启动语言服务，项目内共享进程；实时诊断异步送 UI，文件关闭/闲置按策略退出。此模块为 P1 设计，未包含可运行 Rust LSP 客户端。

## 5. 用户文档格式兼容
PDF/Office 需要具备文件内结构差异解析器或明确标注“二进制版本，暂不能结构化 Diff”。不要把 ZIP 里文件 XML 的机械变化误判成真正要件语义修改。

## 6. 当前原型
Turso annotation/task 与 CLI create 已实现，Slint 仅演示目录与忽略按钮；没有完整富文本 Markdown/Office 内联编辑。关联 UI 的最终验收见 ST-023。
