use halogen_config::Config;
use halogen_orm::user;
use halogen_polling::PollingHandle;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct Library {
    pub(crate) db: DatabaseConnection,
    pub(crate) config: Config,
    pub(crate) polling: PollingHandle,
    pub(crate) root: PathBuf,
    _lock: File,
    pub(crate) active: AtomicBool,
    pub(crate) activity: tokio::sync::RwLock<()>,
}

impl Library {
    /// Open the existing library in place, preserving user IDs and media paths.
    pub async fn open(root: &Path) -> Result<Arc<Self>, String> {
        if !root.is_absolute() {
            return Err("library path must be absolute".into());
        }
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("lock"))
            .map_err(|e| e.to_string())?;
        lock_library(&lock)?;
        let mut config = Config {
            db_path: root.join("halogen.db"),
            media_root: root.join("media"),
            subscription_no_sync_before: chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            ..Default::default()
        };
        config.load_and_apply_overrides(&root.join("overrides.toml"));
        #[cfg(debug_assertions)]
        if std::env::var("HALOGEN_LOCAL_TEST_MODE").as_deref() == Ok("1") {
            config.allow_private_network = true;
            config.dev_use_mock_download = true;
        }
        std::fs::create_dir_all(&config.media_root).map_err(|e| e.to_string())?;
        let db = halogen_migrations::connect_and_migrate_wal(&config.db_path, true)
            .await
            .map_err(|e| e.to_string())?;
        if user::Entity::find()
            .one(&db)
            .await
            .map_err(|e| e.to_string())?
            .is_none()
        {
            halogen_fixture::user::seed_admin_user(&db, "local", None)
                .await
                .map_err(|e| e.to_string())?;
        }
        halogen_fixture::playlist::seed_default_queue(&db)
            .await
            .map_err(|e| e.to_string())?;
        halogen_net::configure(config.allow_private_network);
        halogen_net::configure_user_agent(config.server_fetch_user_agent.clone());
        let polling = PollingHandle::from_config(db.clone(), &config);
        polling.start()?;
        Ok(Arc::new(Self {
            db,
            config,
            polling,
            root,
            _lock: lock,
            active: AtomicBool::new(true),
            activity: tokio::sync::RwLock::new(()),
        }))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Select a local profile; identity comes from the opened library, never a request body.
    pub async fn session(
        self: &Arc<Self>,
        username: Option<&str>,
    ) -> Result<crate::LocalSession, String> {
        let _guard = self.activity.read().await;
        self.ensure_active()?;
        let mut query = user::Entity::find().order_by_asc(user::Column::CreatedAt);
        if let Some(username) = username {
            if username.is_empty() || username.len() > 256 {
                return Err("invalid profile name".into());
            }
            query = query.filter(user::Column::Username.eq(username));
        }
        let user = query
            .one(&self.db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("local profile does not exist")?;
        Ok(crate::LocalSession::new(self.clone(), user))
    }

    pub async fn shutdown(&self) {
        let _guard = self.activity.write().await;
        self.active.store(false, Ordering::Release);
        self.polling.shutdown().await;
    }

    pub async fn session_by_id(self: &Arc<Self>, id: i32) -> Result<crate::LocalSession, String> {
        let _guard = self.activity.read().await;
        self.ensure_active()?;
        if id <= 0 {
            return Err("invalid profile id".into());
        }
        let user = halogen_orm::user::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("local profile not found")?;
        Ok(crate::LocalSession::new(self.clone(), user))
    }

    pub(crate) fn ensure_active(&self) -> Result<(), String> {
        if self.active.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err("local session is closed".into())
        }
    }

    pub async fn close(&self) -> Result<(), String> {
        self.shutdown().await;
        self.db.close_by_ref().await.map_err(|e| e.to_string())?;
        unlock_library(&self._lock)
    }
}

fn lock_library(file: &File) -> Result<(), String> {
    #[cfg(not(target_os = "android"))]
    {
        file.try_lock()
            .map_err(|e| format!("library is unavailable: {e}"))
    }
    #[cfg(target_os = "android")]
    {
        use std::os::fd::AsRawFd;
        // The descriptor stays owned by Library until all sessions are dropped.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            Ok(())
        } else {
            Err(format!(
                "library is unavailable: {}",
                std::io::Error::last_os_error()
            ))
        }
    }
}

fn unlock_library(file: &File) -> Result<(), String> {
    #[cfg(not(target_os = "android"))]
    {
        file.unlock().map_err(|e| e.to_string())
    }
    #[cfg(target_os = "android")]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        }
    }
}
