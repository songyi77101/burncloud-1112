use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::Path,
};

/// 服务运行期间持有的本地数据库所有者锁。
pub(crate) struct ServerOwnerLock {
    _file: File,
}

impl ServerOwnerLock {
    /// 根据服务将使用的数据库身份取得唯一进程锁。
    pub(crate) fn acquire_current_database() -> anyhow::Result<Self> {
        let identity = match std::env::var("BURNCLOUD_DATABASE_URL") {
            Ok(value) if !value.trim().is_empty() => value,
            Ok(_) => anyhow::bail!("BURNCLOUD_DATABASE_URL 不能为空"),
            Err(std::env::VarError::NotPresent) => {
                let path = burncloud_database::get_default_database_path()?;
                let normalized = path.to_string_lossy().replace('\\', "/");
                burncloud_database::sqlite_url(&normalized, cfg!(windows), true)
            }
            Err(error) => anyhow::bail!("无法读取 BURNCLOUD_DATABASE_URL: {error}"),
        };
        let identity = normalize_database_identity(identity)?;
        let root = dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| anyhow::anyhow!("无法解析服务所有者锁目录"))?
            .join("BurnCloud")
            .join("locks");
        let file = acquire_owner_lock(&identity, &root).map_err(|error| {
            if is_lock_contended(&error) {
                anyhow::anyhow!("同一数据库已有 BurnCloud 服务实例运行")
            } else {
                anyhow::Error::from(error)
            }
        })?;
        Ok(Self { _file: file })
    }
}

fn normalize_database_identity(identity: String) -> anyhow::Result<String> {
    if !identity.starts_with("sqlite:") {
        return Ok(identity);
    }
    let options = identity
        .parse::<sqlx::sqlite::SqliteConnectOptions>()
        .map_err(|error| anyhow::anyhow!("BURNCLOUD_DATABASE_URL 无效: {error}"))?;
    let filename = options.get_filename();
    if filename == Path::new(":memory:") {
        anyhow::bail!("服务数据库不能使用每连接独立的 SQLite 内存数据库");
    }
    let normalized_path = if filename.is_absolute() {
        filename.to_path_buf()
    } else {
        std::env::current_dir()?.join(filename)
    };
    Ok(format!("sqlite: {}", normalized_path.to_string_lossy()))
}

fn is_lock_contended(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || (cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)))
}

fn acquire_owner_lock(identity: &str, root: &Path) -> io::Result<File> {
    fs::create_dir_all(root)?;
    let digest = Sha256::digest(identity.as_bytes());
    let path = root.join(format!("deployment-{:x}.lock", digest));
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock_exclusive()?;
    Ok(file)
}

#[cfg(test)]
#[path = "deployment_lifecycle_tests.rs"]
mod tests;
