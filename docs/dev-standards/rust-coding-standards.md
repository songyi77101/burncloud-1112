# Rust 编码规范

> 本文件只保留项目当前最新的 Rust 编码约束；历史变更见 `docs/dev-standards/migration-info/`。
> 每条约束标注来源，项目级强制要求以根目录 `AGENTS.md` 和 `Cargo.toml` 为准。

## 一、命名与代码组织

- 遵循 Rust 社区命名、目录和模块组织约定（来源：现状归纳，`AGENTS.md` 第 1 节）。
- 公共 API 的可见性必须由接口契约、API 文档或公共 manifest 明确要求；新增函数、方法和结构体字段默认保持私有（来源：现状归纳，`AGENTS.md` 第 3 节）。
- 生产代码与测试代码分离；公共 API 必须配套单元测试，关键模块补充 `/// # Examples` 文档测试（来源：现状归纳，`AGENTS.md` 第 9 节）。

## 二、错误处理与安全

- 可恢复错误使用 `Result`，不可恢复错误才使用 `panic`；不得静默吞掉错误（来源：现状归纳，`AGENTS.md` 第 6 节）。
- 禁止使用 `unsafe`；确需使用时必须先完成显式安全评估和文档说明（来源：现状归纳，`AGENTS.md` 第 7 节）。
- 外部输入路径、网络响应和用户参数不得使用 `unwrap()` 或 `expect()`；公共接口执行黑盒参数校验，公共函数执行白盒参数校验（来源：现状归纳，`AGENTS.md` 第 3、4、7 节）。
- 复用借用优先于无必要的所有权转移和克隆；避免在热路径上制造不必要的堆分配（来源：现状归纳，`AGENTS.md` 第 8 节）。

## 三、文档注释

- 注释必须使用中文；公共 API 必须使用 `///` Markdown 文档注释，覆盖用途、示例和注意事项（来源：现状归纳，`AGENTS.md` 第 2 节）。
- 私有实现使用 `//` 注释解释设计原因，而不是重复描述代码表面行为（来源：现状归纳，`AGENTS.md` 第 2 节）。
- 修改公共 API 时同步更新 rustdoc、模块 README、根 README 和接口文档（来源：现状归纳，`AGENTS.md` 第 11 节）。

## 四、依赖与质量门禁

- 遵循根 `Cargo.toml` 的 workspace lint；生产代码不得引入 `unwrap_used`、`todo`、`unimplemented`、`dbg_macro` 等被禁止的逃生写法（来源：现状归纳，`Cargo.toml` workspace lints）。
- 不得为未来可能性提前增加过度抽象、扩展点或共享层；以最小满足当前需求的实现为准（来源：现状归纳，`AGENTS.md` 第 5 节）。
- 提交前后执行 `cargo fmt`、`cargo clippy` 和 `cargo test`；提交信息遵循 Conventional Commits（来源：现状归纳，`AGENTS.md` 第 10 节）。

## 五、变更边界

- 开发任务只能修改登记工作区和任务报告；未登记的目录视为只读（来源：实践补齐，`from-issue-to-pr` 流程约束）。
- 运行时资源、缓存和用户数据不得写入安装目录、项目目录或 `target`；路径必须由服务端配置和可信 manifest 控制（来源：实践补齐，方案二开发文档）。
