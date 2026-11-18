# Vercel Control Plane (最小后端)

- Google ID Token 由服务器通过 Google JWKS 验证 `iss/aud/exp/sub`。需要 `GOOGLE_OIDC_CLIENT_ID`。此处不负责原生客户端的 OAuth PKCE 登录 UX。
- Turso Cloud `@tursodatabase/serverless` 仅存身份、租户、成员、设备、授权与短期密文聊天中继记录；使用 `sql/002` + `sql/003` 的控制面表。部署生产环境前必须将核心本地历史表与控制面数据库严格分离。
- `GET /api/v1/auth/session`：验证身份；`GET /api/v1/workspaces`：服务器按身份授权枚举；`POST /api/v1/chat`：检查成员角色、会话租户范围，暂存短期客户端加密中继消息。
- 此工程未提供真实的用户自助注册 / 管理员审批 / 身份注销 / 设备密钥分发 / 实时推送 / 聊天附件 DLP / 限速防滥用；**禁止作为生产多租户认证服务直接上线**。
- 不存 Drive refresh token；用户自己在本机 OAuth 授权。Shared Drive 需要用户本身有 Google Drive 访问权；LADON 授权并不能使无权用户访问云盘。
- 安装/部署：`npm install && npm run typecheck && npm run build`，之后 `vercel deploy`。当前生成环境未联通 Vercel 凭据。

> V1.3 改进：`chat_delivery_queue` 7 天自动到期字段，仍需部署定期清理；完整聊天史以用户 Drive/NAS 加密事件段为权威源。`GET /api/v1/chat` 仅返回尚未过期的密文临时记录。

## 临时密文队列清理

`vercel.json` 声明每日执行 `/api/internal/prune`。必须设置 `CRON_SECRET`；否则该路径会拒绝。定时任务是否实际可用，取决于部署平台计划与具体环境，未在本交付中运行验证。长久聊天历史依然依赖客户端的 Drive/NAS 归档。
