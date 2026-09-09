use super::fetch;
use crate::{DispatchFuture, LocalTransport, MediaPathFuture};
use halogen_wire_meta::api::ApiRequest;

struct FileTransport(std::path::PathBuf);
impl Drop for FileTransport {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
impl LocalTransport for FileTransport {
    fn invoke(&self, _: ApiRequest) -> DispatchFuture<'_> {
        Box::pin(async { Err("not an API request".into()) })
    }
    fn media_path<'a>(&'a self, _: &'a str) -> MediaPathFuture<'a> {
        Box::pin(async { Ok(Some(self.0.to_string_lossy().into_owned())) })
    }
}

#[tokio::test]
async fn local_media_preserves_seek_offsets_and_rejects_out_of_bounds_ranges() {
    let path = std::env::temp_dir().join(format!("halogen-media-{}.mp3", std::process::id()));
    std::fs::write(&path, b"0123456789").unwrap();
    let transport = FileTransport(path);
    let response = fetch(&transport, "episodes/1/audio", Some("bytes=3-6"))
        .await
        .unwrap();
    assert_eq!(response.status, 206);
    assert_eq!(response.bytes, b"3456");
    assert_eq!(response.content_range.as_deref(), Some("bytes 3-6/10"));
    let rejected = fetch(&transport, "episodes/1/audio", Some("bytes=10-"))
        .await
        .unwrap();
    assert_eq!(rejected.status, 416);
    assert_eq!(rejected.content_range.as_deref(), Some("bytes */10"));
    assert!(rejected.bytes.is_empty());
}
