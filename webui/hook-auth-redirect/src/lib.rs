use dioxus::prelude::*;

/// Destination restored after authentication completes.
#[derive(Clone, Copy)]
pub struct PendingAuthRedirect(pub Signal<Option<String>>);
impl PendingAuthRedirect {
    pub fn take_or_home(&self) -> String {
        let mut inner = self.0;
        inner.take().unwrap_or_else(|| "/".to_owned())
    }
}
