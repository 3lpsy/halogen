use super::{EmbeddedState, LocalProfile};
use halogen_apiclient::{DispatchFuture, LocalTransport, MediaPathFuture};
use halogen_local_runtime::{ApiRequest, Library};
use halogen_webui_platform::paths;
use std::sync::{Arc, OnceLock, RwLock};

fn slot() -> &'static tokio::sync::Mutex<Option<Arc<Library>>> {
    static SLOT: OnceLock<tokio::sync::Mutex<Option<Arc<Library>>>> = OnceLock::new();
    SLOT.get_or_init(|| tokio::sync::Mutex::new(None))
}
fn status() -> &'static RwLock<EmbeddedState> {
    static STATUS: OnceLock<RwLock<EmbeddedState>> = OnceLock::new();
    STATUS.get_or_init(|| RwLock::new(EmbeddedState::Stopped))
}
async fn library() -> Result<Arc<Library>, String> {
    let mut slot = slot().lock().await;
    if let Some(library) = slot.as_ref() {
        return Ok(library.clone());
    }
    *status().write().unwrap() = EmbeddedState::Starting;
    match Library::open(&paths::embedded_server_root()).await {
        Ok(library) => {
            *slot = Some(library.clone());
            *status().write().unwrap() = EmbeddedState::Running;
            Ok(library)
        }
        Err(error) => {
            *status().write().unwrap() = EmbeddedState::Failed {
                error: error.clone(),
            };
            Err(error)
        }
    }
}
struct ProfileTransport(i32);
impl LocalTransport for ProfileTransport {
    fn invoke(&self, request: ApiRequest) -> DispatchFuture<'_> {
        Box::pin(async move {
            library()
                .await?
                .session_by_id(self.0)
                .await?
                .invoke(request)
                .await
        })
    }
    fn media_path<'a>(&'a self, path: &'a str) -> MediaPathFuture<'a> {
        Box::pin(async move {
            library()
                .await?
                .session_by_id(self.0)
                .await?
                .media_path(path)
                .await
        })
    }
}
fn resolve(url: &url::Url) -> Option<Arc<dyn LocalTransport>> {
    if url.scheme() != "halogen-local" {
        return None;
    }
    let id = url.host_str()?.parse::<i32>().ok().filter(|id| *id > 0)?;
    Some(Arc::new(ProfileTransport(id)))
}
fn resolver(id: i32) -> Option<String> {
    (id > 0).then(|| format!("halogen-local://{id}"))
}
pub fn new_password() -> String {
    format!("{}-{}", rand::random::<u64>(), rand::random::<u64>())
}
pub fn available() -> bool {
    true
}
pub fn library_exists() -> bool {
    paths::embedded_server_root().join("halogen.db").is_file()
}
pub fn install() {
    halogen_apiclient::install_local_resolver(resolve);
    halogen_webui_config::set_embedded_url_resolver(resolver);
}
pub fn ensure_started() -> Result<String, String> {
    install();
    tokio::spawn(async {
        let _ = library().await;
    });
    Ok("halogen-local://library".into())
}
pub async fn wait_ready() -> Result<(), String> {
    library().await.map(|_| ())
}
pub fn state() -> EmbeddedState {
    status().read().unwrap().clone()
}
pub async fn profile(username: Option<&str>) -> Result<LocalProfile, String> {
    install();
    let session = library().await?.session(username).await?;
    Ok(LocalProfile {
        id: session.user_id(),
        username: session.username().into(),
        is_admin: session.is_admin(),
        base: format!("halogen-local://{}", session.user_id()),
    })
}
// The library stays open across account switches so downloads keep progressing.
pub async fn stop() {}
pub async fn destroy() -> Result<(), String> {
    let mut slot = slot().lock().await;
    if !paths::embedded_server_root().exists() {
        return Ok(());
    }
    if !library_exists() {
        return Err("path is not an existing local library".into());
    }
    if slot.is_none() {
        *slot = Some(Library::open(&paths::embedded_server_root()).await?);
    }
    let library = slot.as_ref().ok_or("Local library is unavailable")?;
    library.close().await?;
    let root = library.root().to_path_buf();
    *slot = None;
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    *status().write().unwrap() = EmbeddedState::Stopped;
    Ok(())
}
pub fn data_dir_display() -> Option<String> {
    Some(paths::embedded_server_root().display().to_string())
}
