use crate::tempo::{Tempo, TempoControl};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::{fs::File, path::Path, time::Duration};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub struct Audio {
    _device: MixerDeviceSink,
    pub player: Player,
    pub duration: Option<Duration>,
    tempo: TempoControl,
}
impl Audio {
    pub fn open() -> Result<Self> {
        let mut device = DeviceSinkBuilder::open_default_sink()?;
        device.log_on_drop(false);
        let player = Player::connect_new(device.mixer());
        Ok(Self {
            _device: device,
            player,
            duration: None,
            tempo: TempoControl::new(1.0),
        })
    }
    pub fn load(
        &mut self,
        path: &Path,
        position: f64,
        speed: f32,
        volume: f32,
        paused: bool,
    ) -> Result<()> {
        let source = Decoder::try_from(File::open(path)?)?;
        let duration = source.total_duration();
        self.player.clear();
        self.player.pause();
        self.duration = duration;
        self.tempo.set_speed(speed);
        self.player.set_volume(volume);
        self.player.append(Tempo::new(source, self.tempo.clone()));
        self.seek_to(if position.is_finite() {
            position.max(0.0)
        } else {
            0.0
        })?;
        if !paused {
            self.player.play();
        }
        Ok(())
    }
    pub fn position(&self) -> f64 {
        let position = self.tempo.position(self.player.get_pos());
        self.duration
            .map_or(position, |d| position.min(d.as_secs_f64()))
    }
    pub fn set_speed(&self, speed: f32) {
        self.tempo.set_speed(speed);
    }
    pub fn seek_to(&self, seconds: f64) -> Result<()> {
        let target = seconds.max(0.0);
        self.player.try_seek(Duration::from_secs_f64(
            target.min(self.duration.map_or(target, |d| d.as_secs_f64())),
        ))?;
        Ok(())
    }
    pub fn seek(&self, seconds: f64) -> Result<()> {
        self.seek_to(self.position() + seconds)
    }
}
