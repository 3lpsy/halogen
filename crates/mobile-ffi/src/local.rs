use crate::CoreError;
use halogen_local_runtime::{ApiRequest, Library, LocalSession};
use std::{
    path::Path,
    sync::{Arc, OnceLock},
};
use tokio::sync::Mutex;

fn library_slot() -> &'static Mutex<Option<Arc<Library>>> {
    static SLOT: OnceLock<Mutex<Option<Arc<Library>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

#[derive(uniffi::Object)]
pub struct LocalCore {
    pub(crate) session: Arc<LocalSession>,
}

#[derive(uniffi::Record)]
pub struct InvokeResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

fn error(msg: String) -> CoreError {
    CoreError::Server { msg }
}

/// Open a local profile without a listener, credentials, or HTTP authentication.
#[uniffi::export(async_runtime = "tokio")]
pub async fn start_local(
    data_root: String,
    username: Option<String>,
) -> Result<Arc<LocalCore>, CoreError> {
    let mut slot = library_slot().lock().await;
    if let Some(library) = slot.as_ref() {
        let requested = Path::new(&data_root)
            .canonicalize()
            .map_err(|e| error(e.to_string()))?;
        if library.root() != requested {
            return Err(error("another library is open".into()));
        }
    } else {
        *slot = Some(Library::open(Path::new(&data_root)).await.map_err(error)?);
    }
    let library = slot
        .as_ref()
        .ok_or_else(|| error("library unavailable".into()))?;
    let session = library.session(username.as_deref()).await.map_err(error)?;
    Ok(Arc::new(LocalCore {
        session: Arc::new(session),
    }))
}

#[uniffi::export(async_runtime = "tokio")]
impl LocalCore {
    pub fn user_id(&self) -> i32 {
        self.session.user_id()
    }
    pub fn username(&self) -> String {
        self.session.username().into()
    }
    pub fn is_admin(&self) -> bool {
        self.session.is_admin()
    }

    pub async fn invoke(
        &self,
        method: String,
        path: String,
        query: Option<String>,
        body: Option<Vec<u8>>,
    ) -> Result<InvokeResponse, CoreError> {
        let result = self
            .session
            .invoke(ApiRequest {
                method,
                path,
                query,
                body,
            })
            .await
            .map_err(error)?;
        Ok(InvokeResponse {
            status: result.status,
            body: result.body,
        })
    }

    pub async fn audio_path(&self, episode_id: i32) -> Result<Option<String>, CoreError> {
        self.session.audio_path(episode_id).await.map_err(error)
    }

    pub async fn art_path(
        &self,
        id: i32,
        is_episode: bool,
        small: bool,
    ) -> Result<Option<String>, CoreError> {
        self.session
            .art_path(id, is_episode, small)
            .await
            .map_err(error)
    }
}

/// Close all sessions before removing the library selected by the host.
#[uniffi::export(async_runtime = "tokio")]
pub async fn destroy_local(data_root: String) -> Result<(), CoreError> {
    let mut slot = library_slot().lock().await;
    let requested = Path::new(&data_root);
    if !requested.is_absolute() {
        return Err(error("library path must be absolute".into()));
    }
    if !requested.exists() {
        return Ok(());
    }
    let root = requested.canonicalize().map_err(|e| error(e.to_string()))?;
    if root.parent().is_none() || !root.join("halogen.db").is_file() {
        return Err(error("path is not an existing local library".into()));
    }
    if slot.is_none() {
        *slot = Some(Library::open(&root).await.map_err(error)?);
    }
    let library = slot
        .as_ref()
        .ok_or_else(|| error("library is not open".into()))?;
    if library.root() != root {
        return Err(error("different library is open".into()));
    }
    library.close().await.map_err(error)?;
    *slot = None;
    std::fs::remove_dir_all(&root).map_err(|e| error(e.to_string()))?;
    Ok(())
}
