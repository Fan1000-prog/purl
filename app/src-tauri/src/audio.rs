//! Mic capture via cpal (CoreAudio). The cpal Stream is !Send, so it lives on
//! a dedicated thread; `Recorder` talks to that thread over channels.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no input device found — check microphone permission in System Settings")]
    NoDevice,
    #[error("audio stream error: {0}")]
    Stream(String),
    #[error("recording produced no audio")]
    Empty,
}

/// What the capture thread hands back: samples, sample rate, channel count.
type Capture = Result<(Vec<f32>, u32, u16), AudioError>;

pub struct Recorder {
    stop_tx: mpsc::Sender<()>,
    handle: JoinHandle<Capture>,
    level: Arc<AtomicU32>,
}

/// Root-mean-square of a sample chunk — the live "loudness" for the UI.
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

impl Recorder {
    /// Start capturing from the default input device on a dedicated thread.
    pub fn start() -> Result<Recorder, AudioError> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();
        let level = Arc::new(AtomicU32::new(0));
        let level_in = Arc::clone(&level);

        let handle = std::thread::spawn(move || {
            use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

            let host = cpal::default_host();
            let device = match host.default_input_device() {
                Some(d) => d,
                None => {
                    let _ = ready_tx.send(Err(AudioError::NoDevice));
                    return Err(AudioError::NoDevice);
                }
            };
            let config = match device.default_input_config() {
                Ok(c) => c,
                Err(e) => {
                    let err = AudioError::Stream(e.to_string());
                    let _ = ready_tx.send(Err(AudioError::Stream(e.to_string())));
                    return Err(err);
                }
            };
            let sample_rate = config.sample_rate().0;
            let channels = config.channels();
            let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&samples);

            let err_fn = |e| eprintln!("audio stream error: {e}");
            let level_f32 = Arc::clone(&level_in);
            let level_i16 = Arc::clone(&level_in);
            let stream = match config.sample_format() {
                cpal::SampleFormat::F32 => device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _| {
                        level_f32.store(rms(data).to_bits(), Ordering::Relaxed);
                        sink.lock().unwrap().extend_from_slice(data)
                    },
                    err_fn,
                    None,
                ),
                cpal::SampleFormat::I16 => device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let mut guard = sink.lock().unwrap();
                        let start = guard.len();
                        guard.extend(data.iter().map(|&s| s as f32 / i16::MAX as f32));
                        level_i16.store(rms(&guard[start..]).to_bits(), Ordering::Relaxed);
                    },
                    err_fn,
                    None,
                ),
                other => {
                    let err = AudioError::Stream(format!("unsupported sample format {other:?}"));
                    let _ = ready_tx.send(Err(AudioError::Stream(format!(
                        "unsupported sample format {other:?}"
                    ))));
                    return Err(err);
                }
            }
            .map_err(|e| AudioError::Stream(e.to_string()))?;

            stream
                .play()
                .map_err(|e| AudioError::Stream(e.to_string()))?;
            let _ = ready_tx.send(Ok(()));

            // Block until stop() (or Recorder drop closes the channel).
            let _ = stop_rx.recv();
            drop(stream);

            let data = std::mem::take(&mut *samples.lock().unwrap());
            Ok((data, sample_rate, channels))
        });

        // Surface device/permission failures immediately instead of at stop().
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Recorder {
                stop_tx,
                handle,
                level,
            }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                let err = handle
                    .join()
                    .map(|r| r.err())
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| AudioError::Stream("recorder thread died".into()));
                Err(err)
            }
        }
    }

    /// Handle for polling the live input level (RMS, ~0.0–1.0) from any thread.
    pub fn level_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.level)
    }

    /// Stop capturing and discard all audio — for cancelled dictations.
    pub fn cancel(self) {
        let _ = self.stop_tx.send(());
        let _ = self.handle.join();
    }

    /// Stop capturing and return the audio as 16 kHz mono f32, resampled in
    /// memory. Nothing touches the disk.
    pub fn stop_samples(self) -> Result<Vec<f32>, AudioError> {
        let _ = self.stop_tx.send(());
        let (raw, sample_rate, channels) = self
            .handle
            .join()
            .map_err(|_| AudioError::Stream("recorder thread panicked".into()))??;

        let mono = downmix(&raw, channels);
        let resampled = resample_linear(&mono, sample_rate, TARGET_SAMPLE_RATE);
        if resampled.is_empty() {
            return Err(AudioError::Empty);
        }
        Ok(resampled)
    }
}

fn downmix(interleaved: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let n = channels as usize;
    interleaved
        .chunks_exact(n)
        .map(|frame| frame.iter().sum::<f32>() / n as f32)
        .collect()
}

fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = ((input.len() as f64) / ratio).floor() as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = *input.get(idx + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, secs: f32, freq: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn rms_of_silence_is_zero_and_scales_with_amplitude() {
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(rms(&[0.0, 0.0]), 0.0);
        let quiet = rms(&sine(16_000, 0.1, 440.0));
        let loud_samples: Vec<f32> = sine(16_000, 0.1, 440.0).iter().map(|s| s * 2.0).collect();
        let loud = rms(&loud_samples);
        assert!(loud > quiet * 1.9 && loud < quiet * 2.1);
    }

    #[test]
    fn downmix_averages_stereo() {
        let stereo = vec![1.0, 0.0, 0.5, 0.5];
        assert_eq!(downmix(&stereo, 2), vec![0.5, 0.5]);
    }

    #[test]
    fn resample_48k_to_16k_thirds_length() {
        let input = sine(48_000, 1.0, 440.0);
        let out = resample_linear(&input, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn resample_same_rate_is_identity() {
        let input = sine(16_000, 0.1, 440.0);
        assert_eq!(resample_linear(&input, 16_000, 16_000), input);
    }
}
