//! 保留模型选择器的十七个预置名称和顺序，并绑定可发现 GGUF 文件的仓库。

use serde::Serialize;

use super::sources::{ModelResource, Source};

// 预置目录只表示绑定关系，仓库是否可访问仍由状态接口判断。
const ENTRIES: [(&str, &str, &str); 17] = [
    (
        "DeepSeek V4.1 Flash",
        "deepseek-v4.1-flash",
        "unsloth/DeepSeek-V4.1-Flash-1017-GGUF",
    ),
    ("GLM‑5.3", "glm-5.3", "Scantist/GLM-5.3-SLIM-GGUF"),
    (
        "GLM‑5.3‑Flash",
        "glm-5.3-flash",
        "unsloth/GLM-5.3-Flash-GGUF",
    ),
    ("Kimi K3", "kimi-k3", "unsloth/Kimi-K3-GGUF"),
    (
        "MiniMax M3.1 Flash",
        "minimax-m3.1-flash",
        "unsloth/MiniMax-M3.1-Flash-GGUF",
    ),
    ("Qwen3.8-27B", "qwen3.8-27b", "unsloth/Qwen3.8-27B-GGUF"),
    ("Qwen3-8B", "qwen3-8b", "Qwen/Qwen3-8B-GGUF"),
    (
        "Qwen3-Coder-Next",
        "qwen3-coder-next",
        "unsloth/Qwen3-Coder-Next-GGUF",
    ),
    (
        "Qwen3.8-2.4T-A95B",
        "qwen3.8-2.4t-a95b",
        "unsloth/Qwen3.8-2.4T-A95B-GGUF",
    ),
    (
        "Qwen3.8-Flash-Next",
        "qwen3.8-flash-next",
        "unsloth/Qwen3.8-Flash-Next-GGUF",
    ),
    (
        "Qwen-Image-2512",
        "qwen-image-2512",
        "unsloth/Qwen-Image-2512-GGUF",
    ),
    (
        "Wan2.2-TI2V-5B",
        "wan2.2-ti2v-5b",
        "unsloth/Wan2.2-TI2V-5B-GGUF",
    ),
    (
        "MiMo-V2.6-Flash-RL",
        "mimo-v2.6-flash-rl",
        "ggml-org/MiMo-V2.6-Flash-RL-GGUF",
    ),
    ("gpt-oss-20b", "gpt-oss-20b", "unsloth/gpt-oss-20b-GGUF"),
    ("gpt-oss-120b", "gpt-oss-120b", "unsloth/gpt-oss-120b-GGUF"),
    (
        "Gemma 4 E4B IT",
        "gemma-4-e4b-it",
        "unsloth/gemma-4-E4B-it-GGUF",
    ),
    ("Gemma 4 31B", "gemma-4-31b", "unsloth/gemma-4-31B-it-GGUF"),
];

#[derive(Clone, Debug, Serialize)]
pub(super) struct Preset {
    id: String,
    name: &'static str,
    preset: bool,
    #[serde(flatten)]
    pub(super) resource: ModelResource,
}

/// 返回按原顺序排列的预置资源。
///
/// # Examples
/// 第一项为 `DeepSeek V4.1 Flash`。
///
/// # Notes
/// 不因上游故障替换为模拟目录。
pub(super) fn all() -> Vec<Preset> {
    ENTRIES
        .iter()
        .map(|(name, id, model)| Preset {
            id: (*id).to_string(),
            name,
            preset: true,
            resource: ModelResource {
                model: (*model).to_string(),
                source: Source::ModelScope,
                revision: "master".into(),
            },
        })
        .collect()
}

/// 按预置标识返回绑定资源。
///
/// # Examples
/// `qwen3-8b` 对应 `Qwen/Qwen3-8B`。
///
/// # Notes
/// 未知标识返回 `None`，不会触发搜索。
pub(super) fn find(id: &str) -> Option<Preset> {
    all().into_iter().find(|preset| preset.id == id)
}
