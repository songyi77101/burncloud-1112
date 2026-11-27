//! 提供基于 aria2 与 SQLite 的常规下载管理，以及独立的原生 HTTP 运行时归档下载。

mod catalog;
mod error;
#[allow(dead_code, reason = "任务 003 的 HTTP 传输接入后移除此阶段标注")]
mod http;
mod manager;
mod monitor;
mod operations;
mod utils;

pub use catalog::{catalog_downloads, CatalogDownload};
pub use error::DownloadError;
pub use manager::{DownloadManager, DownloadTaskHandle, DownloadTaskSnapshot, DownloadTaskStatus};
mod runtime_http;
pub use runtime_http::{
    download_runtime_archive, RuntimeArchive, RuntimeDownloadError, RuntimeDownloadProgress,
    VerifiedArchive,
};
