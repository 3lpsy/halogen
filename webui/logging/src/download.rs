#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{JsCast, closure::Closure};

/// Trigger a download of the full device log. On wasm this is a browser file
/// download; on native it writes a file and returns its path (for a toast).
pub fn download_logs(text: &str) -> Option<String> {
    download_text("halogen-device-logs.txt", text)
}

/// Trigger a download of `text` as `filename` — [`download_bytes`] with the
/// historical best-effort `Option` contract its callers expect (`None` covers
/// both "browser handled it" and "couldn't").
pub fn download_text(filename: &str, text: &str) -> Option<String> {
    download_bytes(filename, text.as_bytes(), "text/plain")
        .ok()
        .flatten()
}

/// Trigger a download of `bytes` as `filename` (e.g. the gzipped DB export). On wasm: a browser download (Blob +
/// synthetic anchor click), `Ok(None)`, the browser owns the outcome from there. On native: writes the file under the
/// app data directory, `Ok(Some(path))` for the caller's toast. `Err` is a REAL failure (unwritable disk, missing DOM):
/// callers must not report success for it.
#[cfg(target_arch = "wasm32")]
pub fn download_bytes(filename: &str, bytes: &[u8], mime: &str) -> Result<Option<String>, String> {
    let fail = |what: &str| format!("Browser download failed ({what})");
    let win = web_sys::window().ok_or_else(|| fail("no window"))?;
    let doc = win.document().ok_or_else(|| fail("no document"))?;
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::of1(&array.into());
    let bag = web_sys::BlobPropertyBag::new();
    bag.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &bag)
        .map_err(|_| fail("blob"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| fail("object url"))?;
    let anchor = doc
        .create_element("a")
        .ok()
        .and_then(|el| el.dyn_into::<web_sys::HtmlAnchorElement>().ok())
        .ok_or_else(|| fail("anchor"))?;
    anchor.set_href(&url);
    anchor.set_download(filename);
    // Attach the anchor to the DOM before clicking and remove it after: some
    // browsers ignore a click on a detached anchor and yield an empty/aborted
    // download.
    let body = doc.body().ok_or_else(|| fail("no body"))?;
    let _ = body.append_child(&anchor);
    anchor.click();
    let _ = body.remove_child(&anchor);
    // Defer revoking the object URL to the next tick. Revoking synchronously right
    // after `click()` can abort the download before the browser has started reading
    // the blob. The one-shot closure is handed to the JS GC via `once_into_js`.
    let revoke = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 0);
    Ok(None)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn download_bytes(filename: &str, bytes: &[u8], _mime: &str) -> Result<Option<String>, String> {
    let path = halogen_webui_platform::paths::data_root().join(filename);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, bytes).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}
