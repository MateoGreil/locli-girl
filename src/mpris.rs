use crate::app::AppState;
use anyhow::Result;
use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, PlatformConfig};
use std::sync::{atomic::Ordering, Arc};

pub fn start_mpris(state: Arc<AppState>) -> Result<MediaControls> {
    let config = PlatformConfig {
        dbus_name: "locli_girl",
        display_name: "locli-girl",
        hwnd: None,
    };
    let mut controls = MediaControls::new(config)?;

    controls.attach(move |event| match event {
        MediaControlEvent::Play => state.is_paused.store(false, Ordering::Relaxed),
        MediaControlEvent::Pause => state.is_paused.store(true, Ordering::Relaxed),
        MediaControlEvent::Toggle => {
            let p = state.is_paused.load(Ordering::Relaxed);
            state.is_paused.store(!p, Ordering::Relaxed);
        }
        _ => {}
    })?;

    controls.set_metadata(MediaMetadata {
        title: Some("Lofi Girl Radio"),
        artist: Some("locli-girl"),
        ..Default::default()
    })?;

    Ok(controls)
}
