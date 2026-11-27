use super::*;

#[test]
fn embedded_manifest_matches_the_four_fixed_runtime_resources() {
    let manifest = parse_manifest(MANIFEST_JSON)
        .unwrap_or_else(|error| panic!("validate embedded runtime manifest: {}", error.code()));

    assert_eq!(manifest.targets.len(), 4);
    for (target, filename, _, _) in TARGETS {
        let package = &manifest.targets[target].package;
        assert_eq!(package.filename, filename);
        assert!(package.size_bytes > 0);
        assert!(is_sha256(&package.sha256));
        assert!(is_safe_relative_path(&package.server_relative_path));
    }
}

#[test]
fn manifest_rejects_drifted_identity_backend_path_and_digest() {
    for field in ["commit", "filename", "sha256", "server_relative_path"] {
        let mut manifest = parse_manifest(MANIFEST_JSON)
            .unwrap_or_else(|error| panic!("validate embedded test fixture: {}", error.code()));
        match field {
            "commit" => manifest.commit = "master".to_string(),
            "filename" => {
                let target = manifest
                    .targets
                    .get_mut("x86_64-pc-windows-msvc")
                    .unwrap_or_else(|| panic!("locate Windows test target"));
                target.package.filename = "arbitrary.zip".to_string();
            }
            "sha256" => {
                let target = manifest
                    .targets
                    .get_mut("x86_64-pc-windows-msvc")
                    .unwrap_or_else(|| panic!("locate Windows test target"));
                target.package.sha256 = "not-a-sha".to_string();
            }
            "server_relative_path" => {
                let target = manifest
                    .targets
                    .get_mut("x86_64-pc-windows-msvc")
                    .unwrap_or_else(|| panic!("locate Windows test target"));
                target.package.server_relative_path = "../outside/llama-server.exe".to_string();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            validate_manifest(&manifest),
            Err(RuntimeManifestError::Invalid),
            "{field}"
        );
    }
    assert_eq!(parse_manifest("{}"), Err(RuntimeManifestError::Invalid));
}

#[test]
fn backend_support_is_validated_before_any_package_is_downloaded() {
    for (target, _, _, supported) in TARGETS {
        let backends = supported
            .iter()
            .map(|backend| (*backend).to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            validate_requested_backend(&backends, RequestedBackend::Auto),
            Ok(())
        );
        assert_eq!(
            validate_requested_backend(&backends, RequestedBackend::Cpu),
            Ok(())
        );
        assert_eq!(
            validate_requested_backend(&backends, RequestedBackend::Metal).is_ok(),
            target.contains("apple-darwin")
        );
    }
    assert_eq!(
        validate_requested_backend(&["cpu".to_string()], RequestedBackend::Metal),
        Err(RuntimeManifestError::UnsupportedBackend)
    );
}

#[test]
fn runtime_request_cannot_select_a_different_version() {
    let request = RuntimeRequest {
        runtime: "llama.cpp".to_string(),
        version: Some("master".to_string()),
    };
    assert_eq!(
        resolve_runtime_entry(&request, RequestedBackend::Auto),
        Err(RuntimeManifestError::ConfigurationInvalid)
    );
}

#[test]
fn runtime_request_resolves_to_a_server_owned_entry() {
    let request = RuntimeRequest {
        runtime: "llama.cpp".to_string(),
        version: None,
    };
    let resolved = resolve_runtime_entry(&request, RequestedBackend::Auto)
        .unwrap_or_else(|error| panic!("resolve current fixed runtime: {}", error.code()));

    assert_eq!(resolved.version, VERSION);
    assert_eq!(resolved.archive.target(), resolved.target);
    assert_eq!(resolved.requested_backend, RequestedBackend::Auto);
    assert_eq!(resolved.probe_entry().target, resolved.target);
}
