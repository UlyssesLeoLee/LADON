# 48. 詳細設計 — StorageProvider 统一能力协商

## 1. 适配约束

必须通过 `capabilities()` 报告 provider 类型及：`immutable_put,range_read,resumable_upload,query_metadata,list_changes,conditional_put,shared_acl,quota_lookup`；没有能力时由统一层以加密索引/传输块替代，禁止假设 NAS/Google 等价。

## 2. 优先接口

```rust
pub trait ArchiveProviderV2 {
    // design signature only; concrete async trait adapter required
    // async fn put_immutable(&self, logical_key: &str, bytes: &[u8]) -> Result<Locator>;
    // async fn read_verified(&self, locator: &Locator, range: Option<Range>) -> Result<Vec<u8>>;
    // async fn list_by_index(&self, workspace: &str, cursor: Option<String>) -> Result<Page>;
    // async fn capabilities(&self) -> CapabilityFlags;
}
```

这份接口在 `api/` 中有目标契约，**`src/provider.rs` 仍是 V1 trait**，V2 适配器尚未完整实现。

## 3. 迁移顺序

登记目的存储→清点不可变对象/密文索引→并行传输→readback核对→刷新绑定及游标→连续一段时间双读→确认无失败后允许删除旧对象。迁移不得依赖 Drive 特有“移动到 appData”操作，因为它不支持。
