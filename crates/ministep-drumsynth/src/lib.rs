//! MiniStep's built-in drum synth.
//!
//! [`start`] opens the default audio output and returns an [`AudioOutput`]
//! (keep it alive) and a [`SynthSink`] to hand to the engine. The transport
//! thread calls [`SynthSink::trigger`] with grid times; the audio callback
//! renders each hit on its exact sample.

pub mod dsp;
pub mod mixer;
pub mod voice;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Sender, bounded};
use ministep_core::clock::monotonic;
use ministep_core::voice::{DrumVoiceSink, VoiceParams};

pub use mixer::{Hit, Mixer};
pub use voice::render;

/// Thread-safe trigger handle for the engine.
pub struct SynthSink {
    tx: Sender<Hit>,
    volume: Arc<AtomicU32>,
}

impl DrumVoiceSink for SynthSink {
    fn trigger(&self, at: f64, voice: VoiceParams, velocity: u8) {
        let _ = self.tx.try_send(Hit { at, params: voice, velocity });
    }
}

impl SynthSink {
    pub fn set_volume(&self, v: f32) {
        self.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
}

/// The running audio stream; dropping it stops sound.
pub struct AudioOutput {
    _stream: cpal::Stream,
    pub device: String,
    pub sample_rate: u32,
    pub latency_ms: f64,
}

/// Open the default (or named) output device.
pub fn start(device_name: Option<&str>) -> Result<(AudioOutput, Arc<SynthSink>), String> {
    let host = cpal::default_host();
    let device = match device_name {
        Some(name) => host
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.description().ok().is_some_and(|desc| desc.name() == name))
            .ok_or(format!("no audio output named {name:?}"))?,
        None => host.default_output_device().ok_or("no audio output device")?,
    };
    let device_label = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "audio output".into());
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let sample_rate = supported.sample_rate();
    let channels = usize::from(supported.channels());
    let block = 128u32;
    let config = cpal::StreamConfig { channels: supported.channels(), sample_rate, buffer_size: cpal::BufferSize::Fixed(block) };
    // Two blocks of headroom plus a margin for callback jitter.
    let latency = (2.0 * f64::from(block) / f64::from(sample_rate) + 0.002).clamp(0.004, 0.03);
    let (tx, rx) = bounded::<Hit>(1024);
    let volume = Arc::new(AtomicU32::new(0.8f32.to_bits()));
    let vol = Arc::clone(&volume);
    let build = |config: cpal::StreamConfig| {
        let rx = rx.clone();
        let vol = Arc::clone(&vol);
        let mut mixer_state = Mixer::new(sample_rate as f32, latency);
        device.build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                mixer_state.master = f32::from_bits(vol.load(Ordering::Relaxed));
                mixer_state.process(data, channels, monotonic(), Some(&rx));
            },
            |e| eprintln!("ministep: audio error: {e}"),
            None,
        )
    };
    // Some devices refuse a fixed buffer size; fall back to their default.
    let stream = match build(config) {
        Ok(s) => s,
        Err(_) => build(cpal::StreamConfig { buffer_size: cpal::BufferSize::Default, ..config }).map_err(|e| e.to_string())?,
    };
    stream.play().map_err(|e| e.to_string())?;
    Ok((
        AudioOutput { _stream: stream, device: device_label, sample_rate, latency_ms: latency * 1000.0 },
        Arc::new(SynthSink { tx, volume }),
    ))
}

/// Names of the available audio outputs.
pub fn output_devices() -> Vec<String> {
    cpal::default_host()
        .output_devices()
        .map(|ds| ds.filter_map(|d| d.description().ok().map(|x| x.name().to_string())).collect())
        .unwrap_or_default()
}
