use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::error::{Aria2Error, Aria2Result};
use crate::process::{find_running_aria2, start_aria2_rpc};
use crate::rpc::Aria2RpcClient;
use crate::types::{Aria2Config, Aria2Instance, ChildTerminator, ProcessTerminator};
use tokio::sync::Mutex as AsyncMutex;

// ============================================================================
// 简单守护进程
// ============================================================================

/// 启动器返回的 future：一个已经启动的服务实例。
pub(crate) type InstanceFuture = Pin<Box<dyn Future<Output = Aria2Result<Aria2Instance>> + Send>>;

/// 启动**一个** aria2 服务实例。
///
/// 做成可注入的缝，是为了让重启监控能被**桩进程**监督（#904）：本增量要断言的行为是监控的判定 ——
/// 「服务真的没了才补一个，且只补一个」 —— 而它必须在不启动真实 aria2、不联网、不调用进程清扫的
/// 前提下可观测。
pub(crate) type Launcher = Arc<dyn Fn(&Aria2Config) -> InstanceFuture + Send + Sync>;

pub struct Aria2Daemon {
    instance: Arc<Mutex<Option<Aria2Instance>>>,
    attached_port: Arc<Mutex<Option<u16>>>,
    config: Aria2Config,
    is_running: Arc<AtomicBool>,
    add_uri_lock: Arc<AsyncMutex<()>>,
    launcher: Launcher,
    terminator: Arc<dyn ProcessTerminator>,
}

impl Aria2Daemon {
    // 输入：aria2 运行配置。
    // 功能：创建未启动的 aria2 统一管理对象。
    // 错误：无，构造过程不执行外部操作。
    pub fn new(config: Aria2Config) -> Self {
        Self {
            instance: Arc::new(Mutex::new(None)),
            attached_port: Arc::new(Mutex::new(None)),
            config,
            is_running: Arc::new(AtomicBool::new(false)),
            add_uri_lock: Arc::new(AsyncMutex::new(())),
            launcher: Arc::new(|config: &Aria2Config| -> InstanceFuture {
                let config = config.clone();
                Box::pin(async move { start_aria2_rpc(&config).await })
            }),
            terminator: Arc::new(ChildTerminator),
        }
    }

    /// 启动 aria2 子进程，或使用已在本机运行的 aria2 RPC 服务。
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn run() -> burncloud_download_aria2::Aria2Result<()> {
    /// use burncloud_download_aria2::{Aria2Config, Aria2Daemon};
    /// let mut daemon = Aria2Daemon::new(Aria2Config::default());
    /// daemon.start().await?;
    /// daemon.stop().await;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// 已启动时拒绝重复调用，创建子进程失败时返回启动错误。
    /// 已有服务不会被作为本进程创建的子进程管理。
    pub async fn start(&mut self) -> Aria2Result<()> {
        if self.is_running.load(Ordering::SeqCst) {
            return Err(Aria2Error::DaemonError("守护进程已在运行".to_string()));
        }

        let existing_port = find_running_aria2(&self.config.secret).await;
        self.start_after_discovery(existing_port).await
    }

    // 将真实端口探测与进程监管分开，监管只需要知道是否发现可复用的服务。
    async fn start_after_discovery(&mut self, existing_port: Option<u16>) -> Aria2Result<()> {
        if let Some(port) = existing_port {
            *self
                .attached_port
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(port);
            self.is_running.store(true, Ordering::SeqCst);
            println!("已接管运行中的 aria2 RPC 服务，端口: {port}");
            return Ok(());
        }

        let instance = (self.launcher)(&self.config).await?;
        println!("aria2 RPC 服务已启动在端口: {}", instance.port);
        *self
            .instance
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(instance);
        self.is_running.store(true, Ordering::SeqCst);

        // 启动监控任务
        let instance = Arc::clone(&self.instance);
        let is_running = Arc::clone(&self.is_running);
        let config = self.config.clone();
        let launcher = Arc::clone(&self.launcher);

        // 异步监测进程状态并在退出后尝试重启
        tokio::spawn(async move {
            while is_running.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(1000)).await;
                if !is_running.load(Ordering::SeqCst) {
                    break;
                }

                // 在互斥锁保护下检查当前进程状态
                let need_restart = {
                    let mut lock = instance.lock().unwrap_or_else(|e| e.into_inner());
                    match lock.as_mut() {
                        Some(inst) => !inst.is_running(), // 检查服务是否还在运行
                        None => true,
                    }
                };

                // 服务确实退出时创建并替换新的运行实例
                if need_restart {
                    println!("检测到aria2已退出，重启中...");
                    if let Ok(new_instance) = (launcher)(&config).await {
                        let new_port = new_instance.port;
                        *instance.lock().unwrap_or_else(|e| e.into_inner()) = Some(new_instance);
                        println!("aria2重启成功，端口: {}", new_port);
                    }
                }
            }
        });

        Ok(())
    }

    // 输入：可变的守护进程管理对象。
    // 功能：停止当前管理的 aria2 子进程并清理本地连接状态。
    // 错误：进程终止错误被忽略，不向调用方返回错误。
    pub async fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);

        if let Some(ref mut instance) = self
            .instance
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_mut()
        {
            drop(self.terminator.terminate(instance));
        }

        *self
            .instance
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        *self
            .attached_port
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        println!("aria2 守护进程已停止");
    }

    // 输入：守护进程管理对象。
    // 功能：根据当前实例或接管的端口创建 RPC 客户端。
    // 错误：守护进程尚未启动时返回 None。
    pub fn get_rpc_client(&self) -> Option<Aria2RpcClient> {
        let instance = self
            .instance
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let port = instance.as_ref().map(|value| value.port).or_else(|| {
            self.attached_port
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .to_owned()
        })?;
        Some(Aria2RpcClient::with_add_uri_lock(
            port,
            self.config.secret.clone(),
            Arc::clone(&self.add_uri_lock),
        ))
    }

    // 输入：守护进程管理对象。
    // 功能：读取守护进程是否可供 RPC 使用。
    // 错误：无。
    // 说明：这是**监管状态**（已启动且未停止），不是对服务的探测：服务本身是否还在，由
    // `Aria2Instance::is_running` 回答，监控循环用的也是后者。
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests;
