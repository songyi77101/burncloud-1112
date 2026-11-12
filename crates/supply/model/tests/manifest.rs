use burncloud_supply_model::{load_default_manifests, parse_manifests, ModelManifest, Variant};

#[test]
fn manifest_round_trips_through_json() {
    let manifest = ModelManifest {
        model_name: "deepseek-r1".into(),
        description: "推理模型".into(),
        supported_backends: vec!["llama.cpp".into()],
        version: "1".into(),
        variants: vec![Variant {
            id: "deepseek-r1-7b-q4_k_m".into(),
            model_size: Some("7b".into()),
            quantization: "q4_k_m".into(),
            backend: "llama.cpp".into(),
            min_ram_bytes: 8 * 1024 * 1024 * 1024,
            min_vram_bytes: 6 * 1024 * 1024 * 1024,
            disk_size_bytes: 5 * 1024 * 1024 * 1024,
            checksum: "sha256:example".into(),
            artifact_uri: "https://example.invalid/deepseek-r1-7b-q4_k_m.gguf".into(),
        }],
    };

    let json = serde_json::to_string(&manifest).unwrap();
    let decoded: ModelManifest = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded, manifest);
}

#[test]
fn default_manifest_is_empty_after_switching_to_external_model_discovery() {
    let manifests = load_default_manifests().unwrap();
    assert!(manifests.is_empty());
}

#[test]
fn external_manifest_json_can_override_default_source() {
    let external_json = r#"[{
        "model_name": "external-model",
        "description": "外部清单",
        "supported_backends": ["llama.cpp"],
        "version": "external",
        "variants": [{
            "id": "external-q4",
            "model_size": "7b",
            "quantization": "Q4_K_M",
            "backend": "llama.cpp",
            "min_ram_bytes": 8,
            "min_vram_bytes": 4,
            "disk_size_bytes": 3,
            "checksum": "sha256:external",
            "artifact_uri": "https://models.example/external-q4.gguf"
        }]
    }]"#;

    let manifests = parse_manifests(external_json).unwrap();

    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].model_name, "external-model");
    assert_eq!(manifests[0].variants[0].id, "external-q4");
}
