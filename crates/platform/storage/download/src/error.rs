use burncloud_database::DatabaseError;
use burncloud_download_aria2::Aria2Error;
use thiserror::Error;

/// 下载管理和任务执行错误。
#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("数据库错误：{0}")]
    Database(#[from] DatabaseError),
    #[error("aria2 错误：{0}")]
    Aria2(#[from] Aria2Error),
    /// 原生 HTTP 传输失败；通过错误链保留请求库的原始错误。
    ///
    /// # 注意
    /// 对外报告时应使用固定错误信息，不能展示可能含签名查询参数的 URL。
    #[error("HTTP 下载传输错误：{0}")]
    Transport(#[from] reqwest::Error),
    /// 下载文件或断点侧车的 I/O 操作失败。
    ///
    /// # Examples
    /// ```
    /// use burncloud_download::DownloadError;
    /// use std::error::Error;
    ///
    /// let error = DownloadError::from(std::io::Error::other("磁盘写入失败"));
    /// assert!(error.source().is_some());
    /// ```
    #[error("下载文件 I/O 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("无效的下载 URL：{0}")]
    InvalidUrl(String),
    #[error("无效的下载目录：{0}")]
    InvalidDirectory(String),
    #[error("无法从 URL 提取有效文件名")]
    InvalidFilename,
    #[error("下载任务失败：{0}")]
    Failed(String),
    #[error("下载任务通知通道已关闭")]
    NotificationClosed,
    #[error("下载任务不存在")]
    TaskNotFound,
    #[error("下载任务不可取消：{0}")]
    TaskNotCancellable(String),
    #[error("操作失败：{operation}；清理任务也失败：{cleanup}")]
    Compensation { operation: String, cleanup: String },
    #[error("恢复数据库任务失败：{0}")]
    InvalidRecord(String),
}

pub(super) type Result<T> = std::result::Result<T, DownloadError>;
