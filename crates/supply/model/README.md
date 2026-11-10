# burncloud-supply-model

Supply-owned vertical Model crate.

This crate consolidates the former `burncloud-database-model` and
`burncloud-service-models` technical layers under one business Owner.

It owns:

- `model_capabilities` capability persistence defined by #621;
- Model manifest and variant structures;
- Model resolver interfaces and current resolver implementations;
- HuggingFace model-list discovery;
- model data-directory configuration.

It does not own canonical pricing truth (Commerce), Traffic routing, database
schema migrations, or runtime process lifecycle.

The Cargo package is `burncloud-supply-model`, and the Rust crate import is
`burncloud_supply_model`. The temporary `burncloud_service_models`
compatibility surface used during #778 was removed by #780.

## 阶段一到阶段三

- 阶段一：`ModelManifest`、`Variant` 描述模型、量化版本、Runtime 和工件资源要求。
- 阶段二：`RealModelResolver` 保存清单，按 RAM、最大单卡 VRAM 和 Runtime 过滤 Variant。
- 阶段三：`resolve_with_capabilities` 加入磁盘过滤，按清单顺序返回全部兼容候选，
  保留模型编号与 Variant 编号；无兼容候选时返回 `Unsupported`。

`RealModelResolver::new` 返回 `Result`，拒绝未在 Manifest 中声明的 Variant 后端。
`load_default_manifests` 加载内置清单，`parse_manifests` 解析外部 JSON；
解析后的清单由 `RealModelResolver::new` 校验后使用。
内置清单的 RAM/VRAM 数值是演示预算，不代表实测最低配置。

```sh
cargo run -p burncloud-supply-model --example phase_three -- --model qwen2.5:1.5b --ram-bytes 8589934592 --vram-bytes 4294967296 --disk-bytes 10737418240 --runtime llama.cpp
```

可通过 `--manifest <manifest.json>` 指定外部清单。该示例只筛选候选，不下载工件、
启动 Runtime、打分或选择推荐项。历史文件遍历、GGUF 筛选和下载 URL 构建接口及其
示例已移除；调用方直接提供 Manifest 中的 `artifact_uri` 和 `checksum`。

## Domain Crate 10 问

1. **Owner** — Supply. Business capability: local model capability truth, model manifest/variant
   resolution and HuggingFace discovery. The Cargo package is `burncloud-supply-model` and the Rust
   import is `burncloud_supply_model`. This crate consolidates the former
   `burncloud-database-model` and `burncloud-service-models` technical layers under one Owner; the
   temporary `burncloud_service_models` alias was removed by #780.

2. **Responsibility** — read/write the `model_capabilities` capability projection
   (`ModelCapabilityModel::{get, upsert}`); define the manifest and variant value types
   (`ModelManifest`, `Variant`) and the resolver contract with its implementations (`ModelResolver`,
   `RealModelResolver`, `FakeModelResolver`); filter variants by RAM/VRAM, runtime backend and disk;
   and provide HuggingFace model-list discovery plus data-directory configuration.
   **Not responsible for**: canonical pricing and cost calculation (Commerce — `input_price` /
   `output_price` are only a legacy projection in this mixed historical table, not a second pricing
   truth); the `sys_settings` rows behind the `huggingface` / `dir_data` keys (Platform setting);
   `model_capabilities` DDL and migrations (Platform storage database); HuggingFace repository
   metadata CRUD (the no-op facade was removed by #621); Traffic routing and router logs; artifact
   download execution and on-disk data layout; runtime process lifecycle and hardware probing
   (Platform node runtime).

3. **Data Truth** — one table, `model_capabilities`, created by the Platform versioned migrations:
   `id`, `model` (`NOT NULL UNIQUE`), `context_window`, `max_output_tokens`, `supports_vision`,
   `supports_function_calling`, `input_price`, `output_price`, `synced_at`. This crate reads it with
   `SELECT ... WHERE model = ?` and writes it only through `ModelCapabilityModel::upsert`
   (`INSERT ... ON CONFLICT(model) DO UPDATE`), which stamps `synced_at`. The two price columns are
   legacy USD `f64` projections; canonical pricing stays in Commerce `billing_prices` and the table
   is intentionally not rewritten (#621). Separately, the HuggingFace/data-dir helpers read and
   write the keys `huggingface` and `dir_data` in Platform's `sys_settings` through the setting
   service — Platform-owned configuration, not Supply Model truth. Schema and migration history are
   never owned or rewritten here.

4. **Public contract** — there is no `burncloud-supply-contracts` dependency for this capability;
   the boundary types are defined in this crate and exported from `src/lib.rs`:
   - persistence: `ModelCapability` (row), `ModelCapabilityInput` (write projection),
     `ModelCapabilityModel::{get, upsert}`, `current_timestamp`;
   - resolution: `ModelManifest`, `Variant`, `ModelResolutionRequest`, `ResolvedModel`,
     `ModelResolutionOutcome` (`Local` / `Unsupported`), `LocalModelUnsupported`,
     `LocalModelUnsupportedReason`, `ModelResolutionError`, the `ModelResolver` trait,
     `RealModelResolver`, `FakeModelResolver`, `ResolvedModelCandidate`,
     `CandidateResolutionOutcome`;
   - HuggingFace/configuration: `ModelService`, `HfApiModel`, `get_huggingface_host`,
     `get_data_dir`;
   - Manifest loading: `load_default_manifests` loads the static JSON from
     `crates/supply/model/data/model_manifests.json`; `parse_manifests` parses external JSON
     supplied through `phase_three --manifest`.
   `RealModelResolver` is the manifest-backed foundation and its `ModelResolver::resolve` currently
   returns `ModelResolutionError::ResolutionFailed(...)`; `FakeModelResolver` is the deterministic
   placeholder used by the Node skeleton. 阶段三候选列表中的 `model_name` 直接来自
   `ModelManifest.model_name`，`variant_id` 直接来自 `Variant.id`；两者用于区分模型和同一模型
   的不同量化或 Runtime Variant，不读取前端 marketplace 运营目录的展示名称。

5. **Allowed dependencies** — `burncloud-database`, `burncloud-service-setting` (`SettingService`),
   `burncloud-service-ip` (CN HuggingFace mirror selection), `burncloud-node-runtime`
   (`HardwareProfile`), plus `sqlx`, `tokio`, `serde`,
   `serde_json`, `reqwest`, `async-trait` and `thiserror`. No Commerce, Traffic, Identity or
   Interfaces package appears. `deny.toml` denies routing through `burncloud-common`, so new code
   must depend on the owning crate directly.

6. **Allowed consumers** — `deny.toml` has a `[[bans.deny]]` entry for `burncloud-supply-model`
   whose wrappers are `burncloud-node-runtime`, `burncloud-router` and `burncloud-server`, with a
   reason stating that new consumers require architecture review. Existing in-repo use:
   `burncloud-router` (`src/price_sync.rs` projects Commerce pricing into `ModelCapabilityInput` and
   calls `upsert`); `burncloud-server` (`node_orchestrator.rs`, `node_attachment.rs`, `node_test.rs`
   and their integration tests consume the resolver types); and `burncloud-node-runtime` as a
   **dev-dependency** in `tests/golden_path.rs`. `interfaces/service` has the dependency commented
   out, and `trust/inference/README.md` lists this crate although its `Cargo.toml` does not depend on
   it — that README line is stale, not a consumer.

7. **Internal structure** — flat business modules at the crate root, all SQL and row mapping in one
   module:

   ```text
   src/
   ├── lib.rs                 HuggingFace discovery, data-dir configuration, re-exports
   ├── model_capability.rs    ModelCapability/Input/Model, private row mapping, all SQL
   ├── manifest.rs            ModelManifest, Variant
   ├── resolver.rs            ModelResolver contract, DTOs, FakeModelResolver
   ├── real_model_resolver.rs RealModelResolver, hardware/runtime/disk filters
   └── common.rs              current_timestamp (re-exported)
   data/model_manifests.json  阶段三默认 Manifest 清单
   tests/    model_capability.rs (SQLite), manifest.rs, resolver.rs, real_model_resolver.rs
   examples/ fetch_huggingface.rs, phase_three.rs
   docs/     json.md, database-design.md, legacy owner READMEs
   ```

   There is no service/repository/factory shell layer.

8. **Errors / logs / security** — persistence failures propagate as the crate's re-exported
   `burncloud_database::DatabaseError`, so callers own presentation; resolution failures use
   `ModelResolutionError::ResolutionFailed`. The HuggingFace/settings helpers return
   `Result<_, Box<dyn Error>>`. The crate has no `tracing`/`log` dependency and no log statements —
   consumers log, e.g. `burncloud-router::price_sync` reports an upsert failure with
   `tracing::error!`. SQL values are always `.bind(...)`-ed; `adapt_sql` only rewrites `?`
   placeholders for PostgreSQL and never interpolates values (the only `format!` calls build
   HuggingFace URLs). No credential, API key or secret is read, stored or formatted here.

9. **Proof** — what actually executes:
   - `tests/model_capability.rs::capability_upsert_round_trips_through_real_table` opens a temporary
     `sqlite:///…` database through `create_database_with_url` (which runs the Platform migrations),
     so it round-trips every capability field against the real table, including `synced_at` and the
     legacy price projections. This is the only test touching the real table.
   - `tests/resolver.rs::fake_resolver_returns_runtime_and_artifact_choice`, plus tests in
     `tests/manifest.rs`, `tests/resolver.rs` and `tests/real_model_resolver.rs` (hardware, Runtime
     and disk filtering, with Manifest model and Variant identifiers preserved in candidates).
   - The shared default Manifest contains no machine-specific `file:///` URI; local records can
     be supplied through an external `--manifest` file.
   - Downstream contract proof: `crates/platform/node/tests/golden_path.rs`,
     `crates/interfaces/server/tests/node_framework_e2e.rs` and
     `node_route_visibility_lifecycle.rs`.
   - **PostgreSQL: no test in this crate opens a PostgreSQL URL.** The `db.kind() == "postgres"`
     branches in `src/model_capability.rs` are dialect selection only; the `model_capabilities`
     statements — including the `ON CONFLICT` upsert — are never executed against PostgreSQL here.
     SQLite is the only executed persistence proof, and that limitation is recorded rather than
     claimed as coverage.

10. **Correct / incorrect calls**

    ```text
    Correct:   Consumer        -> ModelResolver (ModelResolutionRequest -> ModelResolutionOutcome)
    Correct:   Consumer        -> RealModelResolver::resolve_with_capabilities -> CandidateResolutionOutcome
    Correct:   Traffic price sync -> ModelCapabilityModel::upsert(&db, &ModelCapabilityInput { .. })
    Correct:   Consumer        -> ModelService / get_data_dir
    Incorrect: Any crate       -> raw SELECT/INSERT/UPDATE on model_capabilities
    Incorrect: Any crate       -> treat model_capabilities.input_price/output_price as pricing truth
    Incorrect: Any crate       -> expect ModelService to persist HuggingFace metadata (removed by #621)
    Incorrect: New domain code -> depend on burncloud-supply-model without architecture review
    ```

## 不拥有

- Commerce canonical pricing (`billing_prices`, nanodollar amounts) and request cost calculation
- Platform `sys_settings` rows and the `huggingface` / `dir_data` configuration keys
- `model_capabilities` DDL and Platform migration history
- HuggingFace repository metadata CRUD (the former no-op facade)
- Traffic routing, classifier projection and `router_logs`
- Artifact download execution and the on-disk data-directory layout
- Runtime process lifecycle and hardware probing (`burncloud-node-runtime` owns `HardwareProfile`)

迁移不变量：

```text
Before behavior == After behavior
Before data == After data
Before public business contract == After public business contract
```
