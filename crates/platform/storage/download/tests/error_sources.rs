use burncloud_download::DownloadError;
use std::{error::Error, io};

#[test]
fn transport_preserves_the_original_reqwest_error() {
    let request = reqwest::Client::new().get("not-a-url").build();
    let Err(error) = request else {
        panic!("测试应生成请求构造错误");
    };
    let error = DownloadError::from(error);
    assert!(matches!(error, DownloadError::Transport(_)));
    assert!(error
        .source()
        .and_then(|source| source.downcast_ref::<reqwest::Error>())
        .is_some_and(reqwest::Error::is_builder));
}

#[test]
fn file_io_preserves_the_original_error_kind() {
    let error = DownloadError::from(io::Error::new(io::ErrorKind::PermissionDenied, "固定件"));
    assert!(matches!(error, DownloadError::Io(_)));
    assert_eq!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<io::Error>())
            .map(io::Error::kind),
        Some(io::ErrorKind::PermissionDenied),
    );
}
