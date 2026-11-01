//! Linux desktop controls, consumed on the UI thread so audio and saved state stay in sync.
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

pub struct Desktop {
    controls: MediaControls,
    pub events: Receiver<MediaControlEvent>,
    track: Option<PathBuf>,
    volume: Option<f32>,
}
impl Desktop {
    pub fn new() -> Result<Self, souvlaki::Error> {
        let mut controls = MediaControls::new(PlatformConfig {
            dbus_name: "cozyread",
            display_name: "cozyread",
            hwnd: None,
        })?;
        let (sender, events) = mpsc::channel();
        controls.attach(move |event| {
            let _ = sender.send(event);
        })?;
        Ok(Self {
            controls,
            events,
            track: None,
            volume: None,
        })
    }
    pub fn update(&mut self, app: &super::App) -> Result<(), souvlaki::Error> {
        let track = app.active.map(|i| app.files[i].clone());
        if track != self.track {
            let title = track
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy());
            let album = track
                .as_ref()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy());
            self.controls.set_metadata(MediaMetadata {
                title: title.as_deref(),
                album: album.as_deref(),
                duration: app
                    .audio
                    .as_ref()
                    .filter(|_| track.is_some())
                    .and_then(|a| a.duration),
                ..Default::default()
            })?;
            self.track = track;
        }
        let playback = match app.audio.as_ref().filter(|_| app.active.is_some()) {
            Some(audio) => {
                let progress = Some(MediaPosition(Duration::from_secs_f64(audio.position())));
                if audio.player.is_paused() {
                    MediaPlayback::Paused { progress }
                } else {
                    MediaPlayback::Playing { progress }
                }
            }
            None => MediaPlayback::Stopped,
        };
        self.controls.set_playback(playback)?;
        // Explicitly publish volume after handling SetVolume, as required by MPRIS.
        if self.volume != Some(app.volume) {
            self.controls.set_volume(f64::from(app.volume))?;
            self.volume = Some(app.volume);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{process::Command, time::Instant};

    // Run under an isolated bus: dbus-run-session -- cargo test desktop_dbus -- --ignored
    #[test]
    #[ignore = "requires dbus-run-session and gdbus"]
    fn desktop_dbus() {
        let mut desktop = Desktop::new().unwrap();
        desktop
            .controls
            .set_metadata(MediaMetadata {
                title: Some("Chapter 2"),
                duration: Some(Duration::from_secs(120)),
                ..Default::default()
            })
            .unwrap();
        desktop
            .controls
            .set_playback(MediaPlayback::Paused {
                progress: Some(MediaPosition(Duration::from_secs(42))),
            })
            .unwrap();
        let call = |method: &str, args: &[&str]| {
            Command::new("gdbus")
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.mpris.MediaPlayer2.cozyread",
                    "--object-path",
                    "/org/mpris/MediaPlayer2",
                    "--method",
                    method,
                ])
                .args(args)
                .output()
                .unwrap()
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = call(
                "org.freedesktop.DBus.Properties.Get",
                &["org.mpris.MediaPlayer2.Player", "Metadata"],
            );
            if output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("Chapter 2")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "MPRIS metadata not published: {:?}",
                output
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        let output = call(
            "org.freedesktop.DBus.Properties.Get",
            &["org.mpris.MediaPlayer2.Player", "Position"],
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("42000000"));
        assert!(
            call("org.mpris.MediaPlayer2.Player.PlayPause", &[])
                .status
                .success()
        );
        assert!(matches!(
            desktop.events.recv_timeout(Duration::from_secs(2)).unwrap(),
            MediaControlEvent::Toggle
        ));
        assert!(
            call("org.mpris.MediaPlayer2.Player.Seek", &["15000000"])
                .status
                .success()
        );
        assert!(
            matches!(desktop.events.recv_timeout(Duration::from_secs(2)).unwrap(), MediaControlEvent::SeekBy(souvlaki::SeekDirection::Forward, d) if d == Duration::from_secs(15))
        );
    }
}
