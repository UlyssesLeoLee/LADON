# 43. Google Drive／NAS 存储适配与多租户完整历史

## 1. 不变量
- 每个已捕获稳定版本有永久 FileRef、版本 ID、内容 SHA256、审计事件、内容引用和明确归档状态。仅在确认**至少一个可恢复的已验证副本**后驱逐本地内容。
- Google Drive 的二进制修订保留限制不能满足每文件全历史；LADON 每个历史内容对象采用不可变 CAS Object 存储。
- 文件名、路径和历史差异可能包含敏感信息。`.LADONignore` 限制内容与相关详情、外传元数据，不只限制正文。
- 租户隔离的 Provider 绑定属于某个 `(tenant_id, workspace_id)`，不得根据客户端上传的 Provider ID 跨租户访问。

## 2. Provider 接口合同
```yaml
StorageProviderV1:
  capability_discovery: [range_read, resumable_upload, conditional_put, list_objects, max_object_size, quota, delete]
  methods: [connect, head, put_immutable, get, get_range, abort_upload, resume_upload, list, delete, health, quota]
  object_key: tenant_scoped_sha256
  metadata: [tenant_id, workspace_id, content_hash, key_id, encryption_version, size, verified_at]
  guarantees:
    - upload_idempotency
    - read_after_write_verified_before_local_evict
    - immutable_history_object
    - no_cross_tenant_dedup_without_explicit_agreement
    - safe_retry_after_process_crash
```
`src/provider.rs` 当前仅实现 put/get/delete 的最小接口；上面是后续 Provider v1 契约（未全部代码实现）。

## 3. 归档序列
`Capture → local CAS → local Turso event → encrypted outbox → provider upload → GET verify → Manifest transaction → optionally evict → streaming restore`. 一个大小为 N 的大文件应分块加密及上传，当前内存上限 128MiB，**大文件流式归档属于后续实施项**。

## 4. 存储后端
| 后端 | 当前代码 | 后续 |
|---|---|---|
| Local | FilesystemProvider 完整 put/get，演示删除 | 事务/校验多副本 |
| NAS | 挂载目录可用同一 FilesystemProvider | SMB/WebDAV 原生凭据/重试/锁 |
| Google Drive | 令牌输入、Resumable Upload、GET readback | OAuth PKCE/刷新、限流断点续传、异常重试 |
| S3 Compatible | 设计 | multipart + ETag/Hash 验证 |
| OneDrive | 设计 | Graph upload session/token refresh |

## 5. 事件云端可见性
历史列表使用 Turso 的分页**可检索元数据**；详细事件压缩/加密归档后，UI 按需加载。跨设备同步可见范围由服务端 ACL 与 Ignore/DLP 决定，不允许直接共享原本地数据库文件。网络不可用时未同步状态必须可见。

## 6. 安全清理与迁移
新 Ignore 排除规则出现→标记受影响对象为 `REVIEW_REQUIRED`→停止后续上传→用户批准后尝试远端清理；平台可能有保留期/备份，无法保证已下载副本被擦除。
存储迁移先复制、读回验明、更新双写 Manifest，再切换 Primary；旧端保留至迁移验收。不得自动清理唯一一份备份。

## 7. 官方文档
- https://developers.google.com/workspace/drive/api/guides/manage-uploads
- https://developers.google.com/workspace/drive/api/reference/rest/v3/files/create
