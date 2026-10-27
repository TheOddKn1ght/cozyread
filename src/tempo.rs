//! Streaming waveform-similarity overlap-add (WSOLA) for speech.
//!
//! Samples within each grain retain their original sample rate. Only grain
//! placement changes with tempo; correlation aligns overlapping waveforms.
//! One shared alignment is used for every channel to preserve stereo phase.
use rodio::{ChannelCount, SampleRate, Source, source::SeekError};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
pub struct TempoControl {
    speed: Arc<AtomicU32>,
    timeline: Arc<Mutex<VecDeque<Segment>>>,
}
#[derive(Clone, Copy)]
struct Segment {
    output: f64,
    media: f64,
    speed: f64,
}
impl TempoControl {
    pub fn new(speed: f32) -> Self {
        Self {
            speed: Arc::new(AtomicU32::new(speed.clamp(0.5, 3.0).to_bits())),
            timeline: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
    pub fn set_speed(&self, speed: f32) {
        self.speed
            .store(speed.clamp(0.5, 3.0).to_bits(), Ordering::Relaxed);
    }
    pub fn position(&self, output: Duration) -> f64 {
        let output = output.as_secs_f64();
        let timeline = self.timeline.lock().unwrap();
        let Some(segment) = timeline
            .iter()
            .rev()
            .find(|s| s.output <= output)
            .or(timeline.front())
        else {
            return output;
        };
        (segment.media + (output - segment.output).max(0.0) * segment.speed).max(0.0)
    }
    fn record(&self, output: f64, media: f64, speed: f64) {
        let mut timeline = self.timeline.lock().unwrap();
        timeline.push_back(Segment {
            output,
            media,
            speed,
        });
        // More than ten seconds of history; device read-ahead is normally <100ms.
        if timeline.len() > 1024 {
            timeline.pop_front();
        }
    }
}

pub struct Tempo<S> {
    input: S,
    control: TempoControl,
    channels: ChannelCount,
    rate: SampleRate,
    hop: usize,
    search: usize,
    buffer: VecDeque<f32>,
    base: usize,
    expected: f64,
    tail: Vec<f32>,
    output: Vec<f32>,
    cursor: usize,
    output_frames: usize,
    origin: f64,
    eof: bool,
    finished: bool,
}
impl<S: Source> Tempo<S> {
    pub fn new(input: S, control: TempoControl) -> Self {
        let channels = input.channels();
        let rate = input.sample_rate();
        let hop = (rate.get() as usize / 50).max(1); // 40ms grains, 20ms overlap.
        let search = (rate.get() as usize / 200).max(1); // ±5ms waveform alignment.
        control.timeline.lock().unwrap().clear();
        Self {
            input,
            control,
            channels,
            rate,
            hop,
            search,
            buffer: VecDeque::new(),
            base: 0,
            expected: 0.0,
            tail: Vec::new(),
            output: Vec::new(),
            cursor: 0,
            output_frames: 0,
            origin: 0.0,
            eof: false,
            finished: false,
        }
    }
    fn frames(&self) -> usize {
        self.buffer.len() / self.channels.get() as usize
    }
    fn sample(&self, frame: usize, channel: usize) -> f32 {
        self.buffer[(frame - self.base) * self.channels.get() as usize + channel]
    }
    fn fill(&mut self, until: usize) {
        let channels = self.channels.get() as usize;
        while !self.eof && self.base + self.frames() < until {
            for _ in 0..channels {
                match self.input.next() {
                    Some(value) => self.buffer.push_back(value),
                    None => {
                        self.eof = true;
                        break;
                    }
                }
            }
        }
        self.buffer.truncate(self.frames() * channels);
    }
    fn best_match(&self, expected: usize, end: usize) -> usize {
        if self.tail.is_empty() {
            return expected;
        }
        let low = expected.saturating_sub(self.search).max(self.base);
        let high = (expected + self.search).min(end.saturating_sub(self.hop * 2));
        if low >= high {
            return expected;
        }
        let channels = self.channels.get() as usize;
        // Use all channels, sampling correlation at ~8kHz to bound CPU work.
        let stride = (self.rate.get() as usize / 8000).max(1);
        let score = |candidate: usize| {
            let (mut dot, mut energy) = (0.0_f64, 0.0_f64);
            for frame in (0..self.hop).step_by(stride) {
                for channel in 0..channels {
                    let a = self.tail[frame * channels + channel] as f64;
                    let b = self.sample(candidate + frame, channel) as f64;
                    dot += a * b;
                    energy += b * b;
                }
            }
            dot / (energy + 1e-12).sqrt()
        };
        let mut best = expected.clamp(low, high);
        let mut best_score = score(best);
        // Coarse search followed by sample-accurate refinement.
        for candidate in (low..=high).step_by(stride) {
            let value = score(candidate);
            if value > best_score {
                best = candidate;
                best_score = value;
            }
        }
        let coarse = best;
        for candidate in coarse.saturating_sub(stride).max(low)..=(coarse + stride).min(high) {
            let value = score(candidate);
            if value > best_score {
                best = candidate;
                best_score = value;
            }
        }
        best
    }
    fn grain(&mut self) -> bool {
        if self.finished {
            return false;
        }
        let expected = self.expected.round() as usize;
        self.fill(expected + self.search + 2 * self.hop);
        let end = self.base + self.frames();
        if expected >= end {
            self.finished = true;
            return false;
        }
        let channels = self.channels.get() as usize;
        let speed = f32::from_bits(self.control.speed.load(Ordering::Relaxed)) as f64;
        let start = if end >= expected + self.search + 2 * self.hop {
            self.best_match(expected, end)
        } else {
            expected
        };
        // Limit the final output to the remaining media duration at this tempo.
        let count = if self.eof {
            self.hop
                .min(((end as f64 - self.expected) / speed).ceil().max(0.0) as usize)
        } else {
            self.hop
        };
        self.output.clear();
        self.cursor = 0;
        for frame in 0..count {
            let mix = frame as f32 / self.hop as f32;
            for channel in 0..channels {
                let sample = if start + frame < end {
                    self.sample(start + frame, channel)
                } else {
                    0.0
                };
                let sample = if self.tail.is_empty() {
                    sample
                } else {
                    self.tail[frame * channels + channel] * (1.0 - mix) + sample * mix
                };
                self.output.push(sample);
            }
        }
        self.tail.clear();
        for frame in 0..self.hop {
            for channel in 0..channels {
                self.tail.push(if start + self.hop + frame < end {
                    self.sample(start + self.hop + frame, channel)
                } else {
                    0.0
                });
            }
        }
        self.control.record(
            self.origin + self.output_frames as f64 / self.rate.get() as f64,
            self.origin + self.expected / self.rate.get() as f64,
            speed,
        );
        self.output_frames += count;
        self.expected += count as f64 * speed;
        let discard_until = (self.expected.floor() as usize)
            .saturating_sub(self.search)
            .min(end);
        let discard = discard_until.saturating_sub(self.base);
        self.buffer.drain(..discard * channels);
        self.base += discard;
        !self.output.is_empty()
    }
}
impl<S: Source> Iterator for Tempo<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.cursor == self.output.len() && !self.grain() {
            return None;
        }
        let value = self.output[self.cursor];
        self.cursor += 1;
        Some(value)
    }
}
impl<S: Source> Source for Tempo<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    // Output duration changes dynamically. Media duration is held by Audio.
    fn total_duration(&self) -> Option<Duration> {
        None
    }
    fn try_seek(&mut self, pos: Duration) -> std::result::Result<(), SeekError> {
        self.input.try_seek(pos)?;
        self.buffer.clear();
        self.tail.clear();
        self.output.clear();
        self.base = 0;
        self.expected = 0.0;
        self.cursor = 0;
        self.output_frames = 0;
        self.origin = pos.as_secs_f64();
        self.eof = false;
        self.finished = false;
        self.control.timeline.lock().unwrap().clear();
        self.control.record(
            self.origin,
            self.origin,
            f32::from_bits(self.control.speed.load(Ordering::Relaxed)) as f64,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    fn tone(channels: u16) -> SamplesBuffer {
        let samples: Vec<_> = (0..24000)
            .flat_map(|i| {
                let value = (std::f32::consts::TAU * 220.0 * i as f32 / 8000.0).sin();
                (0..channels).map(move |ch| if ch == 0 { value } else { -value })
            })
            .collect();
        SamplesBuffer::new(
            ChannelCount::new(channels).unwrap(),
            SampleRate::new(8000).unwrap(),
            samples,
        )
    }
    #[test]
    fn tempo_changes_duration_without_changing_pitch() {
        for speed in [0.5, 1.0, 1.5, 2.0, 3.0] {
            let output: Vec<_> = Tempo::new(tone(1), TempoControl::new(speed)).collect();
            assert!(
                (output.len() as f64 - 24000.0 / speed as f64).abs() <= 2.0,
                "speed {speed}: {} samples",
                output.len()
            );
            let middle = &output[1600..output.len() - 1600];
            let crossings = middle
                .windows(2)
                .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
                .count();
            let hz = crossings as f64 * 8000.0 / middle.len() as f64;
            assert!((hz - 220.0).abs() < 3.0, "speed {speed}: {hz}Hz");
            assert!(output.iter().all(|s| s.is_finite() && s.abs() <= 1.001));
        }
    }
    #[test]
    fn stereo_alignment_seek_and_clock_survive_speed_changes() {
        let control = TempoControl::new(1.0);
        let mut source = Tempo::new(tone(2), control.clone());
        let first: Vec<_> = source.by_ref().take(8000).collect();
        assert!(
            first
                .as_chunks::<2>()
                .0
                .iter()
                .all(|p| (p[0] + p[1]).abs() < 1e-6)
        );
        control.set_speed(2.0);
        let next: Vec<_> = source.by_ref().take(8000).collect();
        assert!(
            next.as_chunks::<2>()
                .0
                .iter()
                .all(|p| (p[0] + p[1]).abs() < 1e-6)
        );
        assert!((control.position(Duration::from_secs(1)) - 1.5).abs() < 0.05);
        source.try_seek(Duration::from_secs(1)).unwrap();
        source.by_ref().take(1600).for_each(drop);
        assert!((control.position(Duration::from_millis(1100)) - 1.2).abs() < 0.001);
    }
    #[test]
    fn player_clock_measures_media_time_and_freezes_when_paused() {
        let control = TempoControl::new(2.0);
        let (player, mut output) = rodio::Player::new();
        player.append(Tempo::new(tone(1), control.clone()));
        output.by_ref().take(4000).for_each(drop);
        assert!((control.position(player.get_pos()) - 1.0).abs() < 0.03);
        player.pause();
        // Allow the player's 5ms control interval to observe the pause.
        output.by_ref().take(80).for_each(drop);
        let stopped = control.position(player.get_pos());
        output.by_ref().take(800).for_each(drop);
        assert!((control.position(player.get_pos()) - stopped).abs() < 0.001);
        player.play();
        output.by_ref().take(800).for_each(drop);
        assert!(control.position(player.get_pos()) > stopped + 0.15);
    }
    #[test]
    fn unity_speed_is_transparent_and_short_input_ends() {
        let expected: Vec<_> = tone(1).collect();
        let actual: Vec<_> = Tempo::new(tone(1), TempoControl::new(1.0)).collect();
        assert_eq!(expected.len(), actual.len());
        assert!(
            expected
                .iter()
                .zip(actual)
                .all(|(a, b)| (a - b).abs() < 1e-5)
        );
        for len in [0, 1, 30, 159, 160, 200] {
            let source = SamplesBuffer::new(
                ChannelCount::new(1).unwrap(),
                SampleRate::new(8000).unwrap(),
                vec![0.0; len],
            );
            let out: Vec<_> = Tempo::new(source, TempoControl::new(0.5)).collect();
            assert_eq!(out.len(), len * 2);
        }
    }
}
