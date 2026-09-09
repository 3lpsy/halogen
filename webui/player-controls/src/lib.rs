mod components {
    pub use halogen_webui_component_widgets::{Artwork, Marquee};
}
pub mod mini;
pub mod now_playing;
pub mod sleep_button;
pub mod up_next;

pub use mini::MiniPlayer;
pub use now_playing::NowPlayingScreen;
pub use up_next::UpNext;
