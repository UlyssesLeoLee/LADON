# 55. 運用手順書 — Google Drive / Vercel 最小后端部署

## 个人模式

1. Google Cloud Console 配置 Desktop OAuth Client 与 Google Drive API；按最低权限选择 `drive.appdata` (隐藏个人) 或 `drive.file` (用户授权普通目录)。
2. 本机使用 OAuth PKCE/系统浏览器取票，refresh token 写入 Windows Credential Manager；开发环境仅可临时用 `LADON_GOOGLE_ACCESS_TOKEN`，不可写配置文件。
3. 选择归档目录与本地待上传配额；立即执行小文件往返校验，未通过不要清理本地 CAS。
4. 定期检查 Drive 使用量、归档队列/缺口/身份认证失效。

## 企业团队模式

1. Vercel 部署 `server/vercel/`，设置 `GOOGLE_OIDC_CLIENT_ID`、TURSO_DATABASE_URL、TURSO_AUTH_TOKEN。
2. 建立**独立**的远程 Turso 控制面数据库，迁移 `sql/002_collaboration_shell_agent.sql` 与 `sql/003_drive_rules_control_plane.sql` 中经过审计的控制面表；不能盲目复制本机包含隐私的全量数据库。
3. 由管理员创建租户、工作空间、成员角色和设备密钥；用户自己在 Google Drive 获得共享目录访问权限。
4. 联调真正 OIDC 登录、成员撤销、跨租户拒绝、历史加密共享、团队消息、故障恢复，完成 ST 后方可正式启用。

## 无后端与最小后端是两个不同目标

“个人云盘历史归档无常驻后端”可行；**多租户权限和安全密钥分发不能免掉可信服务端**。Vercel Function 是按请求执行的最小控制面，不是“无需后端授权”。

## 目前状态

本交付未持有 Google/Vercel 凭据，没有执行真实云部署。OAuth PKCE、共享密钥、通知与生产权限需实现及验收。
