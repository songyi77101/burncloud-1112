#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "锁生命周期测试用 expect 明确报告临时目录和文件锁前提失败。"
)]

use super::acquire_owner_lock;
use tempfile::tempdir;

#[test]
fn owner_lock_serializes_instances_for_the_same_database_only() {
    let root = tempdir().expect("创建临时锁目录");
    let first = acquire_owner_lock("sqlite:///first.db", root.path()).expect("取得首个服务锁");

    let conflict = acquire_owner_lock("sqlite:///first.db", root.path())
        .expect_err("同一数据库的第二个服务实例必须被拒绝");
    assert!(conflict.raw_os_error().is_some() || conflict.kind() == std::io::ErrorKind::WouldBlock);

    let independent =
        acquire_owner_lock("sqlite:///second.db", root.path()).expect("不同数据库可以使用独立锁");
    drop(first);
    acquire_owner_lock("sqlite:///first.db", root.path()).expect("首个服务释放锁后应允许重启");
    drop(independent);
}
