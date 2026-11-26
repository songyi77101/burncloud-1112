//! 基于 aria2 与 SQLite 持久化的下载管理。

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
