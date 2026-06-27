//! Audio playback for streamed PCM chunks.
//!
//! The server streams **mono float32 little-endian PCM @ 24 kHz** over the WS
//! `/stream` socket (see `server.py::_produce_kokoro`, which emits
//! `arr.astype(np.float32).tobytes()`, and the GPU path which converts its
//! int16 frames back to float32 before forwarding). The Python client reads
//! these as `np.frombuffer(chunk, dtype=np.float32)`. We do the same here.
//!
//! Output goes to the system default device via cpal (WASAPI / CoreAudio /
//! ALSA). Because most backends won't open a 24 kHz stream, we open the
//! device's native rate and resample 24 kHz -> device rate on the fly (this is
//! exactly what the Python `Player` does with `np.interp`). Playback speed is
//! folded into the same resample ratio.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const SRC_RATE: f64 = 24_000.0;

/// Streaming linear resampler (mono). Keeps fractional state across chunks so
/// there are no per-chunk boundary clicks.
struct LinearResampler {
    /// Input samples consumed per output sample = (src/dst) * speed.
    ratio: f64,
    /// Position of the next output sample, in input-sample units.
    next_pos: f64,
    /// Index of the most recent input sample (`s_cur`). Starts at -1.
    in_index: i64,
    s_prev: f32,
    s_cur: f32,
}

impl LinearResampler {
    fn new(ratio: f64) -> Self {
        LinearResampler {
            ratio: if ratio.is_finite() && ratio > 0.0 { ratio } else { 1.0 },
            next_pos: 0.0,
            in_index: -1,
            s_prev: 0.0,
            s_cur: 0.0,
        }
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for &x in input {
            self.in_index += 1;
            self.s_prev = self.s_cur;
            self.s_cur = x;
            // Emit every output sample whose position falls in
            // [in_index - 1, in_index].
            while self.next_pos <= self.in_index as f64 {
                let base = (self.in_index - 1) as f64;
                let frac = (self.next_pos - base) as f32;
                let v = self.s_prev + (self.s_cur - self.s_prev) * frac;
                out.push(v);
                self.next_pos += self.ratio;
            }
        }
    }
}

/// Plays mono float32 @ 24 kHz, resampled to the default device's native rate.
pub struct AudioPlayer {
    buf: Arc<Mutex<VecDeque<f32>>>,
    channels: usize,
    volume: f32,
    resampler: LinearResampler,
    // Held to keep the stream alive; dropped (stopping playback) in `finish`.
    _stream: cpal::Stream,
}

impl AudioPlayer {
    /// Open the default output device and start an (initially silent) stream.
    pub fn new(speed: f64, volume: f64) -> Result<AudioPlayer, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no default output device".to_string())?;
        let supported = device
            .default_output_config()
            .map_err(|e| format!("default_output_config: {e}"))?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let dst_rate = config.sample_rate.0 as f64;
        let channels = config.channels as usize;

        let buf: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let cb_buf = buf.clone();

        let err_fn = |e| config_err_log(&format!("audio stream error: {e}"));

        // The callback pulls interleaved device-rate samples out of `buf`,
        // padding with silence when we've not buffered enough yet.
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let mut b = cb_buf.lock().unwrap();
                    for s in data.iter_mut() {
                        *s = b.pop_front().unwrap_or(0.0);
                    }
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    let mut b = cb_buf.lock().unwrap();
                    for s in data.iter_mut() {
                        let v = b.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0);
                        *s = (v * 32767.0) as i16;
                    }
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_output_stream(
                &config,
                move |data: &mut [u16], _| {
                    let mut b = cb_buf.lock().unwrap();
                    for s in data.iter_mut() {
                        let v = b.pop_front().unwrap_or(0.0).clamp(-1.0, 1.0);
                        *s = (((v * 0.5) + 0.5) * 65535.0) as u16;
                    }
                },
                err_fn,
                None,
            ),
            other => return Err(format!("unsupported sample format: {other:?}")),
        }
        .map_err(|e| format!("build_output_stream: {e}"))?;

        stream.play().map_err(|e| format!("stream.play: {e}"))?;

        let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
        let ratio = (SRC_RATE / dst_rate) * speed;

        Ok(AudioPlayer {
            buf,
            channels: channels.max(1),
            volume: volume.clamp(0.0, 1.0) as f32,
            resampler: LinearResampler::new(ratio),
            _stream: stream,
        })
    }

    /// Queue a chunk of mono float32 @ 24 kHz samples for playback.
    pub fn write(&mut self, mono_24k: &[f32]) {
        if mono_24k.is_empty() {
            return;
        }
        let mut resampled = Vec::with_capacity(mono_24k.len() * 2);
        self.resampler.process(mono_24k, &mut resampled);
        let mut b = self.buf.lock().unwrap();
        for v in resampled {
            let s = v * self.volume;
            for _ in 0..self.channels {
                b.push_back(s);
            }
        }
    }

    /// Block until the buffered audio has drained, then a short device-latency
    /// tail, then stop the stream. Capped so a stuck device can't hang forever.
    pub fn finish(self) {
        let start = Instant::now();
        loop {
            let remaining = self.buf.lock().unwrap().len();
            if remaining == 0 {
                break;
            }
            if start.elapsed() > Duration::from_secs(120) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Let the last buffered frames actually reach the speakers.
        std::thread::sleep(Duration::from_millis(250));
        // `self` (and `_stream`) dropped here -> stream stops.
    }
}

fn config_err_log(msg: &str) {
    crate::config::hook_log(msg);
}
