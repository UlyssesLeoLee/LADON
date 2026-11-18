# 44. 詳細設計 — Drive 历史对象规则与免常驻后端协议

## 1. 设计约束

`appDataFolder` 对每一 Google 用户的应用私有数据有效，但 **不能分享**、不能跨 space 移动，适合个人同步/加密索引。团队需普通 Drive 文件夹或 Google Shared Drive，且每名成员需独立授权。`drive.appdata` 与 `drive.file` 权限不等价，不可混淆。

## 2. 结构

`protocol/drive-layout-v1.yaml` 定义逻辑层级；Drive 物理层通过 `files.create/files.list/changes.list` 保存 opaque ID → Drive File ID 的索引。文件名/`appProperties` 不含原始用户路径、内容摘要或聊天明文；不上传 `.LADONignore` 原件。

- 内容：`LADON/v1/{opaque_tenant}/{opaque_workspace}/content/{hash}.ldn`
- 事件：`.../events/{opaque_device}/{sha256}.ldn`
- 索引页：`.../indexes/{sha256}.ldn`
- Checkpoint：`.../checkpoints/{sha256}.ldn`
- 聊天：`.../chats/{opaque_device}/{sha256}.ldn`

路径只是协议逻辑键，**Drive 并不是 POSIX 文件系统**。不要用 `os.path.join` 操作 Drive；Provider 需要把逻辑键映射到上传时返回的 file ID。可使用私有 `appProperties` 的不透明属性定位，注意其 API 配额与权限。

## 3. 写入时序

1. 构造事件段（设备 ID、first_seq、last_seq、prev_segment_hash、FileRef、版本/行为）并写入可靠本地 Outbox。
2. 评估资源 .LADONignore、成员授权和 DLP，不允许敏感路径/标签/事件原因外流。
3. 每段压缩 Zstd，封装 AES-256-GCM；加密密钥来自本地安全凭据，不随对象上传。
4. Drive resumable upload；遇 403 rateLimitExceeded / 429 指数退避 + 随机抖动。避免小事件逐个请求，默认目标归档段约 4MiB。
5. 通过 Drive file ID 下载校验；解密后验证 SHA256，再持久化 receipt/locator。
6. 最后发布不可变索引页；**没有全局 last-write-wins manifest**。未得到可恢复副本禁止清理本地源。

## 4. 读取

新设备使用 checkpoint + 变更游标（`changes.getStartPageToken` / `changes.list`）/缺口扫描；无 checkpoint 则列出对象索引页并重建；元信息允许分页/缓存，展开历史详情时才按需下载。

## 5. 一致性和限流

Drive 的文件列表可能分页，通知可丢失；通知仅是唤醒，不能作为无缺口证据。并发来自各设备的追加日志，检查序列及前驱链，冲突按 `45` 方案处理。不依赖 Drive 修订版本保留全部历史。

## 6. 测试与状态

`src/storage_rules.rs`、`src/cloud_journal.rs` 含可编译目标 Rust 代码与 UT；现有 `GoogleDriveProvider` 仍以 `folder_id`/access token 方式支持对象上传下载，但**尚未实现 appDataFolder 自动选择、按逻辑键索引页检索、OAuth PKCE 和完整后台同步调度**。API/Drive 真人授权测试未执行。

## 官方依据

- https://developers.google.com/workspace/drive/api/guides/appdata
- https://developers.google.com/workspace/drive/api/guides/api-specific-auth
- https://developers.google.com/workspace/drive/api/guides/manage-uploads
- https://developers.google.com/workspace/drive/api/guides/manage-changes
