# fix(model-selection): 更新预设 GGUF 下载入口并移除 DeepSeek V4 Pro

## 背景与目标

- **背景**：当前模型选择器在 `crates/interfaces/server/src/model_selection/presets.rs` 中定义 17 个预设，统一绑定 ModelScope 的 `master` 分支，多个预设仍指向原始模型仓库。用户希望选择现有模型名称时，进入对应的 GGUF 仓库并沿用现有流程查询文件与生成下载地址，涉及使用模型选择器的用户。
- **目标**：删除 DeepSeek V4 Pro 预设；保留其余 17 个模型的展示名称和相对顺序，调整其 GGUF 下载链路入口。优先绑定已验证的 ModelScope 仓库，无同名仓库时再使用 Hugging Face。开发序号为 `001`。

## 需求列表

| # | 需求 | 是否必需 |
| --- | --- | --- |
| 1 | 删除 DeepSeek V4 Pro 的预设信息 | 是 |
| 2 | 将已明确的 16 个模型绑定到指定 GGUF 仓库，保持模型名称不变 | 是 |
| 3 | 寻找并绑定可用的 DeepSeek V4.1 Flash GGUF 入口 | 是 |
| 4 | 按已确认的来源优先级确定每个预设的下载入口 | 是 |

## 每项需求的期望行为与期望结果

| # | 需求 | 预期行为 | 期望结果 |
| --- | --- | --- | --- |
| 1 | 删除 DeepSeek V4 Pro 的预设信息 | 从预设目录中移除该模型及其预设绑定。 | 模型选择器及 `GET /api/models/presets` 不再返回 DeepSeek V4 Pro，预设总数由 18 个变为 17 个。 |
| 2 | 更新 16 个明确的 GGUF 入口 | 用户仍按原有展示名称选择模型；预设绑定、状态查询和后续文件解析使用下表对应的 GGUF 仓库。其余模型保持原有相对顺序。 | 16 个模型的仓库、来源和修订与下表一致；查询目标仓库可以发现 GGUF 文件，生成的文件下载地址对应所选目标仓库；模型展示名称不因仓库名称变化而改变。 |
| 3 | 寻找 DeepSeek V4.1 Flash 的可用入口 | 核验该模型的 GGUF 仓库，找到可访问且确实提供该模型 GGUF 文件的入口后更新绑定；展示名称仍为 DeepSeek V4.1 Flash。 | 最终明确可验证的站点、完整仓库名和修订号，并能够查询真实 GGUF 文件及对应下载地址。该项完成后才视为全部 17 个入口调整完成。 |
| 4 | 确定下载来源 | 为每个预设优先选择已验证的 ModelScope 同名仓库；没有同名仓库时，再核验并使用 Hugging Face。 | 最终绑定的仓库、来源、修订号相互一致；Kimi K3 使用已验证的 ModelScope 同名仓库；Hugging Face 入口按实际验证的修订号绑定。 |

已确认的 17 个目标入口如下。ModelScope 的 `master` 文件列表于 2026-11-12 核验可访问且包含 GGUF 文件；此记录不表示已下载全部文件或验证运行兼容性。展示名称沿用当前代码，包括 GLM 名称中的现有连接符。

| 模型展示名称 | GGUF 仓库文件入口 | 来源 | 修订号 |
| --- | --- | --- | --- |
| DeepSeek V4.1 Flash | [unsloth/DeepSeek-V4.1-Flash-1017-GGUF](https://modelscope.cn/models/unsloth/DeepSeek-V4.1-Flash-1017-GGUF/files?Revision=master) | ModelScope | master |
| GLM‑5.3 | [Scantist/GLM-5.3-SLIM-GGUF](https://modelscope.cn/models/Scantist/GLM-5.3-SLIM-GGUF/files?Revision=master) | ModelScope | master |
| GLM‑5.3‑Flash | [unsloth/GLM-5.3-Flash-GGUF](https://modelscope.cn/models/unsloth/GLM-5.3-Flash-GGUF/files?Revision=master) | ModelScope | master |
| Kimi K3 | [unsloth/Kimi-K3-GGUF](https://modelscope.cn/models/unsloth/Kimi-K3-GGUF/files?Revision=master) | ModelScope | master |
| MiniMax M3.1 Flash | [unsloth/MiniMax-M3.1-Flash-GGUF](https://modelscope.cn/models/unsloth/MiniMax-M3.1-Flash-GGUF/files?Revision=master) | ModelScope | master |
| Qwen3.8-27B | [unsloth/Qwen3.8-27B-GGUF](https://modelscope.cn/models/unsloth/Qwen3.8-27B-GGUF/files?Revision=master) | ModelScope | master |
| Qwen3-8B | [Qwen/Qwen3-8B-GGUF](https://modelscope.cn/models/Qwen/Qwen3-8B-GGUF/files?Revision=master) | ModelScope | master |
| Qwen3-Coder-Next | [unsloth/Qwen3-Coder-Next-GGUF](https://modelscope.cn/models/unsloth/Qwen3-Coder-Next-GGUF/files?Revision=master) | ModelScope | master |
| Qwen3.8-2.4T-A95B | [unsloth/Qwen3.8-2.4T-A95B-GGUF](https://modelscope.cn/models/unsloth/Qwen3.8-2.4T-A95B-GGUF/files?Revision=master) | ModelScope | master |
| Qwen3.8-Flash-Next | [unsloth/Qwen3.8-Flash-Next-GGUF](https://modelscope.cn/models/unsloth/Qwen3.8-Flash-Next-GGUF/files?Revision=master) | ModelScope | master |
| Qwen-Image-2512 | [unsloth/Qwen-Image-2512-GGUF](https://modelscope.cn/models/unsloth/Qwen-Image-2512-GGUF/files?Revision=master) | ModelScope | master |
| Wan2.2-TI2V-5B | [unsloth/Wan2.2-TI2V-5B-GGUF](https://modelscope.cn/models/unsloth/Wan2.2-TI2V-5B-GGUF/files?Revision=master) | ModelScope | master |
| MiMo-V2.6-Flash-RL | [ggml-org/MiMo-V2.6-Flash-RL-GGUF](https://modelscope.cn/models/ggml-org/MiMo-V2.6-Flash-RL-GGUF/files?Revision=master) | ModelScope | master |
| gpt-oss-20b | [unsloth/gpt-oss-20b-GGUF](https://modelscope.cn/models/unsloth/gpt-oss-20b-GGUF/files?Revision=master) | ModelScope | master |
| gpt-oss-120b | [unsloth/gpt-oss-120b-GGUF](https://modelscope.cn/models/unsloth/gpt-oss-120b-GGUF/files?Revision=master) | ModelScope | master |
| Gemma 4 E4B IT | [unsloth/gemma-4-E4B-it-GGUF](https://modelscope.cn/models/unsloth/gemma-4-E4B-it-GGUF/files?Revision=master) | ModelScope | master |
| Gemma 4 31B | [unsloth/gemma-4-31B-it-GGUF](https://modelscope.cn/models/unsloth/gemma-4-31B-it-GGUF/files?Revision=master) | ModelScope | master |

## 范围边界

- **做什么**：删除 DeepSeek V4 Pro 预设；更新上述 16 个预设的 GGUF 下载入口；寻找并绑定 DeepSeek V4.1 Flash 的可用 GGUF 入口；保持剩余模型的展示名称和相对顺序，按确认的来源优先级选择入口。
- **不做什么**：本次需求不包含此前讨论的 User-Agent/HTTP 403 修复、下载错误提示调整、GGUF 分片处理或运行时与部署能力扩展；更换入口不等同于承诺所有 GGUF 模型均可由现有运行时部署。

## 实施结果

- DeepSeek V4.1 Flash 已绑定 `unsloth/DeepSeek-V4.1-Flash-1017-GGUF`（ModelScope，`master`），已核验可发现 GGUF 文件。
- DeepSeek V4 Pro 已删除；其余 16 个模型已绑定 Issue 中列出的 GGUF 仓库。
- 预设 ID、展示名称和相对顺序保持兼容。
