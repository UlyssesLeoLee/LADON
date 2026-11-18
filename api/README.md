# LADON API 合同

- `ladon-agent-openapi.yaml`：46 资源路径，目标 HTTP 合同。**不包含可运行 HTTP Server**。
- `ladon-agent-capabilities.yaml`：已实现、CLI 部分实现、设计项明确分组。
- `src/agent_api.rs`：当前 Rust 本地只读目录/文件/历史/规则 API；需在可信应用内调用。
- `src/auth.rs`：纯 RBAC 判定，**不替代** OIDC/JWT 验证。
- 跨终端服务需追加真正的认证与租户隔离，不能直接开放现有 CLI。
