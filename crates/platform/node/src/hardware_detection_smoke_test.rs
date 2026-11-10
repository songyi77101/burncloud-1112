//! Smoke test that runs the real hardware detection on the current machine
//! and prints the detected profile once.
//!
//! Run with: `cargo test -p burncloud-node-runtime hardware_detection_smoke -- --nocapture`

use crate::hardware_detection::{
    CachedHardwareProfile, HardwareError, HardwareFactsProbe, RealHardwareFactsProbe,
};

#[tokio::test]
async fn hardware_detection_smoke_runs_and_prints_profile() -> Result<(), HardwareError> {
    // Detect once through the cached entry point and print the full result.
    let mut cached = CachedHardwareProfile::detect_once()?;
    let profile = cached.profile().clone();
    println!("hardware detection result:\n{profile:#?}");

    // Basic sanity checks on fields that every supported platform should report.
    assert!(!profile.os.name.is_empty(), "OS name should not be empty");
    assert_eq!(profile.os.arch, std::env::consts::ARCH);
    assert_eq!(profile.cpu.arch, std::env::consts::ARCH);
    assert!(
        profile.cpu.logical_cores.is_some_and(|n| n > 0),
        "logical core count should be detected"
    );
    assert!(
        !profile.disk.model_cache_path.is_empty(),
        "model cache path should not be empty"
    );
    if let (Some(total), Some(available)) = (profile.memory.total_mb, profile.memory.available_mb) {
        assert!(available <= total, "available RAM should not exceed total RAM");
    }
    if let (Some(total), Some(available)) = (profile.disk.total_mb, profile.disk.available_mb) {
        assert!(available <= total, "available disk should not exceed total disk");
    }

    // Refreshing dynamic fields must succeed and keep static facts unchanged.
    cached.refresh_dynamic()?;
    assert_eq!(cached.profile().os, profile.os);
    assert_eq!(cached.profile().cpu, profile.cpu);

    // The async probe wrapper should run the same detection without errors.
    let probed = RealHardwareFactsProbe.inspect_full().await?;
    assert_eq!(probed.os, profile.os);
    assert_eq!(probed.cpu.arch, profile.cpu.arch);

    Ok(())
}
