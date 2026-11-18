# 42. Agent 文件夹 API 详细设计

## 1. 目标与协议
REQ-060/061/062、REQ-029/030/031。**统一 Rust Core Service→ IPC/CLI→版本化 HTTP/MCP**；不允许 Agent 直接绕过命令总线调用 Windows 文件删除、远程备份或 Turso 数据库写入。当前 `src/agent_api.rs` 提供一部分实际本地方法，`api/ladon-agent-openapi.yaml` 描述目标 HTTP 契约，HTTP 服务端与 MCP 暂未实现。

## 2. 功能分组（目标 API 全集）
| 资源 | 操作 | 只读/变更 | 当前状态 |
|---|---|---|---|
| Folder | list/tree/stat/resolve/create/move/copy/delete | 混合 | list 已实现核心，其余设计 |
| File | read/range/stream/digest/mime/preview | 只读 | read 核心已实现，其余设计 |
| Query | grep/FTS/hybrid/semantic/filter | 只读 | Grep 已实现 |
| Version | history/detail/diff/content/restore-plan/commit | 混合 | history、load、恢复 CLI 已实现 |
| Task | annotations/tasks/anchor links/impact/approval | 混合 | 创建任务/批注 CLI 已实现 |
| Tag | preview/apply/list/lock/unlock/batch | 混合 | 简单批量标签 CLI 已实现 |
| Ignore | check/rules/add/remove/audit/cloud cleanups | 混合 | check、add 已实现 |
| FileSpace | allocate/snapshot/delta/release/plan/commit | 混合 | snapshot/乐观检查部分实现 |
| Event | subscribe/ack/cursor/replay/backpressure | 只读 | 设计 |
| Storage | providers/capabilities/archive/verify/restore/migrate | 混合 | Local/Drive 基本归档实现 |
| Share | tenants/workspaces/members/invites/permissions | 混合 | DB+RBAC 判定模块 |
| Chat | conversations/messages/attachments/mention/task | 混合 | DB Schema/设计 |
| Shell | native-menu/open-explorer/selected-item-actions | 混合 | 经典单项桥初版 |

## 3. 强制请求包络
```yaml
request:
  api_version: v1
  request_id: uuid
  idempotency_key: required_for_write
  authenticated_principal: server_verified
  tenant_id: derived_from_principal
  workspace_id: authorized_workspace
  resource:
    file_ref: stable_uuid
    expected_version_hash: sha256
  file_space_id: optional
  operation: files.read
  approval_token: optional
  data_classification: checked_before_transfer
  limits:
    max_items: 1000
    max_response_bytes: 8388608
    deadline_ms: 30000
```
- 资源引用优先 FileRef + 版本；绝对路径只允许本地权限检查后使用，防止重解析点/符号链接越界。
- 对分页：`next_cursor` 必须绑定租户、查询过滤条件、快照版本、有限 TTL，不能直接当 SQL OFFSET。
- 回应显式 `status, partial, changed, conflict, audit_event_id`，部分失败不能伪装完全成功。
- 写操作：`plan→preview→approval→commit(expected_hash,idempotency_key)`；不同 Agent 的同一文件写入使用 CAS 比对，版本冲突拒绝。
- 事件订阅：`subscribe(cursor)→batch→ack(high_watermark)`；超出保留窗返回 `CURSOR_EXPIRED`，重新 Snapshot+Delta。慢消费者背压而非无界积压。
- 远程 API 所有方法必须做授权、租户范围和 LADON Ignore，不能仅依赖客户端。

## 4. 统一错误码
| code | 说明 | Agent 后续行动 |
|---|---|---|
| NOT_FOUND | 资源不存在或无权访问（避免信息枚举） | 更新 FileRef |
| PERMISSION_DENIED | 操作已认证但权限不足 | 申请权限 |
| SENSITIVE_BLOCKED | .LADONignore 限制跨信任域外传 | 保持本地或取消 |
| CONFLICT | 基准版本已变化 | 重新获取 snapshot |
| APPROVAL_REQUIRED | 高风险动作未批准 | 请求人类审核 |
| CURSOR_EXPIRED | 事件游标过旧 | 完整重建后续增量 |
| RESOURCE_BUSY | 文件被占用、锁争用 | 有界退避 |
| RATE_LIMITED | 超过配额 | 等待并降低并发 |
| ARCHIVE_UNVERIFIED | 云端对象尚未读回验证 | 保留本地副本 |

## 5. Rust 模块职责
`src/agent_api.rs`：本地枚举、读取、Grep、历史、Ignore 判断及能力清单；`src/auth.rs`：身份判定纯函数；`src/engine.rs`：版本与归档；`src/shell.rs`：Shell 子进程；目标 IPC/HTTP/MCP Adapter 未来只能调用以上统一 Core Service，不准重写文件处理逻辑。
