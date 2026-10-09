//! Microphone and speaker streams (WASAPI on Windows, via cpal).
//! The mic callback runs the whole send pipeline; the speaker callback pulls from the mixer.

use crate::net::{VoiceSender, Wake};
use crate::voice::{Mixer, Resampler, TxPipeline, VoiceControls};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use parking_lot::Mutex;
use proto::SAMPLE_RATE;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub type ErrorHook = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Default, Clone)]
pub struct DeviceList {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

pub fn list_devices() -> DeviceList {
    let host = cpal::default_host();
    let mut list = DeviceList::default();
    if let Ok(devs) = host.input_devices() {
        list.inputs = devs.filter_map(|d| d.name().ok()).collect();
    }
    if let Ok(devs) = host.output_devices() {
        list.outputs = devs.filter_map(|d| d.name().ok()).collect();
    }
    list.inputs.dedup();
    list.outputs.dedup();
    list
}

fn pick_input(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name {
        if let Ok(mut devs) = host.input_devices() {
            if let Some(d) = devs.find(|d| d.name().map(|n| n == name).unwrap_or(false)) {
                return Some(d);
            }
        }
    }
    host.default_input_device()
}

fn pick_output(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name {
        if let Ok(mut devs) = host.output_devices() {
            if let Some(d) = devs.find(|d| d.name().map(|n| n == name).unwrap_or(false)) {
                return Some(d);
            }
        }
    }
    host.default_output_device()
}

pub struct Input {
    _stream: cpal::Stream,
    pub device: String,
    pub rate: u32,
}

pub struct Output {
    _stream: cpal::Stream,
    pub device: String,
    pub rate: u32,
}

pub fn start_input(
    name: Option<&str>,
    ctl: Arc<VoiceControls>,
    voice: VoiceSender,
    wake: Wake,
    on_error: ErrorHook,
) -> Result<Input, String> {
    let device =
        pick_input(name).ok_or("No microphone found. Plug one in, then pick it in settings.")?;
    let device_name = device.name().unwrap_or_else(|_| "Microphone".into());
    let supported = device
        .default_input_config()
        .map_err(|e| format!("Couldn't open the microphone ({e})."))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate.0;
    let pipeline = TxPipeline::new()?;
    let state = InputState {
        channels: config.channels as usize,
        resampler: Resampler::new(rate, SAMPLE_RATE),
        mono: Vec::new(),
        at48k: Vec::new(),
        pipeline,
        ctl,
        voice,
        wake,
    };
    let err = move |e: cpal::StreamError| on_error(format!("Microphone stopped: {e}"));
    let stream = match format {
        SampleFormat::F32 => build_input::<f32>(&device, &config, state, err),
        SampleFormat::I16 => build_input::<i16>(&device, &config, state, err),
        SampleFormat::U16 => build_input::<u16>(&device, &config, state, err),
        SampleFormat::I32 => build_input::<i32>(&device, &config, state, err),
        other => {
            return Err(format!(
                "Your microphone uses an unsupported sample format ({other:?})."
            ))
        }
    }
    .map_err(|e| format!("Couldn't start the microphone ({e})."))?;
    stream
        .play()
        .map_err(|e| format!("Couldn't start the microphone ({e})."))?;
    Ok(Input {
        _stream: stream,
        device: device_name,
        rate,
    })
}

struct InputState {
    channels: usize,
    resampler: Resampler,
    mono: Vec<f32>,
    at48k: Vec<f32>,
    pipeline: TxPipeline,
    ctl: Arc<VoiceControls>,
    voice: VoiceSender,
    wake: Wake,
}

impl InputState {
    fn process<T: Sample>(&mut self, data: &[T])
    where
        f32: FromSample<T>,
    {
        self.mono.clear();
        let ch = self.channels.max(1);
        for frame in data.chunks(ch) {
            let sum: f32 = frame.iter().map(|s| f32::from_sample(*s)).sum();
            self.mono.push(sum / ch as f32);
        }
        self.at48k.clear();
        self.resampler.process(&self.mono, &mut self.at48k);
        let was_talking = self.ctl.talking.load(Ordering::Relaxed);
        let voice = &self.voice;
        self.pipeline
            .push(&self.at48k, &self.ctl, &mut |frame| voice.send(frame));
        if self.ctl.talking.load(Ordering::Relaxed) != was_talking {
            (self.wake)();
        }
    }
}

fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut state: InputState,
    err: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    device.build_input_stream(config, move |data: &[T], _| state.process(data), err, None)
}

pub fn start_output(
    name: Option<&str>,
    mixer: Arc<Mutex<Mixer>>,
    on_error: ErrorHook,
) -> Result<Output, String> {
    let device = pick_output(name).ok_or("No speakers or headphones found.")?;
    let device_name = device.name().unwrap_or_else(|_| "Speakers".into());
    let supported = device
        .default_output_config()
        .map_err(|e| format!("Couldn't open your speakers ({e})."))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate.0;
    let state = OutputState {
        channels: config.channels as usize,
        resampler: Resampler::new(SAMPLE_RATE, rate),
        chunk: vec![0.0; 480],
        scratch: Vec::new(),
        pending: VecDeque::new(),
        mixer,
    };
    let err = move |e: cpal::StreamError| on_error(format!("Speakers stopped: {e}"));
    let stream = match format {
        SampleFormat::F32 => build_output::<f32>(&device, &config, state, err),
        SampleFormat::I16 => build_output::<i16>(&device, &config, state, err),
        SampleFormat::U16 => build_output::<u16>(&device, &config, state, err),
        SampleFormat::I32 => build_output::<i32>(&device, &config, state, err),
        other => {
            return Err(format!(
                "Your speakers use an unsupported sample format ({other:?})."
            ))
        }
    }
    .map_err(|e| format!("Couldn't start your speakers ({e})."))?;
    stream
        .play()
        .map_err(|e| format!("Couldn't start your speakers ({e})."))?;
    Ok(Output {
        _stream: stream,
        device: device_name,
        rate,
    })
}

struct OutputState {
    channels: usize,
    resampler: Resampler,
    chunk: Vec<f32>,
    scratch: Vec<f32>,
    pending: VecDeque<f32>,
    mixer: Arc<Mutex<Mixer>>,
}

impl OutputState {
    fn fill<T: Sample + FromSample<f32>>(&mut self, data: &mut [T]) {
        let ch = self.channels.max(1);
        let frames = data.len() / ch;
        while self.pending.len() < frames {
            // Mix 10 ms at a time at 48 kHz, then convert to the device rate.
            self.mixer.lock().mix(&mut self.chunk);
            self.scratch.clear();
            self.resampler.process(&self.chunk, &mut self.scratch);
            self.pending.extend(self.scratch.iter().copied());
        }
        for frame in data.chunks_mut(ch) {
            let v = self.pending.pop_front().unwrap_or(0.0);
            for s in frame.iter_mut() {
                *s = T::from_sample(v);
            }
        }
    }
}

fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut state: OutputState,
    err: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    device.build_output_stream(config, move |data: &mut [T], _| state.fill(data), err, None)
}
