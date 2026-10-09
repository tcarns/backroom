//! Audio processing that doesn't touch devices or the network:
//! - `TxPipeline`: mic samples (48 kHz mono) -> noise suppression -> voice gate -> Opus packets
//! - `Receiver` + `Mixer`: Opus packets from others -> decode -> jitter buffer -> one mixed signal
//! - `Resampler`: converts between the device's sample rate and 48 kHz

use nnnoiseless::DenoiseState;
use proto::{FRAME_SAMPLES, SAMPLE_RATE, VOICE_DATA, VOICE_END};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- resampler

/// Streaming linear-interpolation resampler. Plenty for voice.
pub struct Resampler {
    step: f64,
    pos: f64,
    prev: f32,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        Self {
            step: in_rate as f64 / out_rate as f64,
            pos: 0.0,
            prev: 0.0,
        }
    }
    pub fn is_passthrough(&self) -> bool {
        (self.step - 1.0).abs() < 1e-9
    }
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.is_passthrough() {
            out.extend_from_slice(input);
            return;
        }
        for &x in input {
            while self.pos <= 1.0 {
                out.push(self.prev + (x - self.prev) * self.pos as f32);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.prev = x;
        }
    }
}

// ---------------------------------------------------------------- shared controls

/// Settings and state shared between the UI thread and the audio threads.
/// Plain atomics, so the audio callbacks never wait on a lock for these.
pub struct VoiceControls {
    pub in_voice: AtomicBool,
    pub muted: AtomicBool,
    pub deafened: AtomicBool,
    pub push_to_talk: AtomicBool,
    pub ptt_held: AtomicBool,
    pub noise_suppression: AtomicBool,
    /// Voice activation threshold in dBFS, stored as f32 bits.
    threshold_db: AtomicU32,
    /// Latest mic level in dBFS (after noise suppression), for the meter.
    level_db: AtomicU32,
    /// True while we're sending voice.
    pub talking: AtomicBool,
}

impl Default for VoiceControls {
    fn default() -> Self {
        Self {
            in_voice: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            deafened: AtomicBool::new(false),
            push_to_talk: AtomicBool::new(false),
            ptt_held: AtomicBool::new(false),
            noise_suppression: AtomicBool::new(true),
            threshold_db: AtomicU32::new((-45.0f32).to_bits()),
            level_db: AtomicU32::new((-100.0f32).to_bits()),
            talking: AtomicBool::new(false),
        }
    }
}

impl VoiceControls {
    pub fn threshold_db(&self) -> f32 {
        f32::from_bits(self.threshold_db.load(Ordering::Relaxed))
    }
    pub fn set_threshold_db(&self, v: f32) {
        self.threshold_db.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn level_db(&self) -> f32 {
        f32::from_bits(self.level_db.load(Ordering::Relaxed))
    }
    fn set_level_db(&self, v: f32) {
        self.level_db.store(v.to_bits(), Ordering::Relaxed);
    }
}

pub fn rms_db(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return -100.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    let rms = (sum / samples.len() as f32).sqrt();
    if rms <= 1e-5 {
        -100.0
    } else {
        20.0 * rms.log10()
    }
}

// ---------------------------------------------------------------- sending

const VA_HANG_FRAMES: u32 = 20; // keep sending 400 ms after speech stops
const PTT_HANG_FRAMES: u32 = 3; // 60 ms after the key is released
pub const BITRATE: i32 = 32_000;

pub struct TxPipeline {
    pending: Vec<f32>,
    denoise: Box<DenoiseState<'static>>,
    denoise_in: [f32; DenoiseState::FRAME_SIZE],
    denoise_out: [f32; DenoiseState::FRAME_SIZE],
    encoder: opus::Encoder,
    packet: Vec<u8>,
    sending: bool,
    hang: u32,
    prev_frame: Vec<f32>,
    have_prev: bool,
    seq: u32,
}

impl TxPipeline {
    pub fn new() -> Result<Self, String> {
        let mut encoder =
            opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip)
                .map_err(|e| e.to_string())?;
        let _ = encoder.set_bitrate(opus::Bitrate::Bits(BITRATE));
        let _ = encoder.set_vbr(true);
        Ok(Self {
            pending: Vec::with_capacity(FRAME_SAMPLES * 4),
            denoise: DenoiseState::new(),
            denoise_in: [0.0; DenoiseState::FRAME_SIZE],
            denoise_out: [0.0; DenoiseState::FRAME_SIZE],
            encoder,
            packet: vec![0u8; proto::MAX_OPUS_PACKET],
            sending: false,
            hang: 0,
            prev_frame: vec![0.0; FRAME_SAMPLES],
            have_prev: false,
            seq: 0,
        })
    }

    /// Feed 48 kHz mono samples. `send` receives finished voice frames
    /// (already in the client-to-server wire format).
    pub fn push(&mut self, samples: &[f32], ctl: &VoiceControls, send: &mut dyn FnMut(Vec<u8>)) {
        self.pending.extend_from_slice(samples);
        while self.pending.len() >= FRAME_SAMPLES {
            let mut frame: Vec<f32> = self.pending.drain(..FRAME_SAMPLES).collect();
            self.process_frame(&mut frame, ctl, send);
        }
    }

    fn process_frame(
        &mut self,
        frame: &mut [f32],
        ctl: &VoiceControls,
        send: &mut dyn FnMut(Vec<u8>),
    ) {
        // Noise suppression (RNNoise) works on 480-sample chunks scaled like 16-bit audio.
        let mut voice_prob = 1.0f32;
        if ctl.noise_suppression.load(Ordering::Relaxed) {
            voice_prob = 0.0;
            for chunk in frame.chunks_mut(DenoiseState::FRAME_SIZE) {
                for (d, s) in self.denoise_in.iter_mut().zip(chunk.iter()) {
                    *d = *s * 32768.0;
                }
                let p = self
                    .denoise
                    .process_frame(&mut self.denoise_out, &self.denoise_in);
                voice_prob = voice_prob.max(p);
                for (s, d) in chunk.iter_mut().zip(self.denoise_out.iter()) {
                    *s = (*d / 32768.0).clamp(-1.0, 1.0);
                }
            }
        }
        let level = rms_db(frame);
        ctl.set_level_db(level);

        let allowed = ctl.in_voice.load(Ordering::Relaxed)
            && !ctl.muted.load(Ordering::Relaxed)
            && !ctl.deafened.load(Ordering::Relaxed);
        let want = if !allowed {
            self.hang = 0;
            false
        } else if ctl.push_to_talk.load(Ordering::Relaxed) {
            if ctl.ptt_held.load(Ordering::Relaxed) {
                self.hang = PTT_HANG_FRAMES;
                true
            } else if self.hang > 0 {
                self.hang -= 1;
                true
            } else {
                false
            }
        } else {
            let loud = level > ctl.threshold_db();
            // With noise suppression on, also require RNNoise to think it's a voice,
            // unless it's clearly loud.
            let speech = loud && (voice_prob > 0.25 || level > ctl.threshold_db() + 15.0);
            if speech {
                self.hang = VA_HANG_FRAMES;
                true
            } else if self.hang > 0 {
                self.hang -= 1;
                true
            } else {
                false
            }
        };

        if want {
            if !self.sending {
                self.sending = true;
                ctl.talking.store(true, Ordering::Relaxed);
                // Send the frame before the gate opened too, so the first syllable isn't clipped.
                if self.have_prev && !ctl.push_to_talk.load(Ordering::Relaxed) {
                    let prev = std::mem::take(&mut self.prev_frame);
                    self.encode_and_send(&prev, send);
                    self.prev_frame = prev;
                }
            }
            self.encode_and_send(frame, send);
        } else if self.sending {
            self.sending = false;
            ctl.talking.store(false, Ordering::Relaxed);
            self.seq = self.seq.wrapping_add(1);
            send(proto::encode_client_voice(VOICE_END, self.seq, &[]));
        }
        self.prev_frame.copy_from_slice(frame);
        self.have_prev = true;
    }

    fn encode_and_send(&mut self, frame: &[f32], send: &mut dyn FnMut(Vec<u8>)) {
        match self.encoder.encode_float(frame, &mut self.packet) {
            Ok(n) if n > 0 => {
                self.seq = self.seq.wrapping_add(1);
                send(proto::encode_client_voice(
                    VOICE_DATA,
                    self.seq,
                    &self.packet[..n],
                ));
            }
            _ => {}
        }
    }

    pub fn is_sending(&self) -> bool {
        self.sending
    }
}

// ---------------------------------------------------------------- receiving

/// Frames to collect before starting playback of someone (absorbs network jitter).
const PREBUFFER_FRAMES: usize = 3;
/// If more than this piles up, drop the oldest to keep delay down.
const MAX_BUFFER_FRAMES: usize = 12;
const TRIM_TO_FRAMES: usize = 4;
const SPEAKING_HOLD: Duration = Duration::from_millis(300);

struct Stream {
    queue: VecDeque<f32>,
    playing: bool,
    ended: bool,
    last_packet: Instant,
    underruns: u32,
}

/// Per-person playback buffers, mixed into one signal by the speaker callback.
pub struct Mixer {
    streams: HashMap<u32, Stream>,
    gains: HashMap<u32, f32>,
    system: VecDeque<f32>,
    pub deafened: bool,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            streams: HashMap::new(),
            gains: HashMap::new(),
            system: VecDeque::new(),
            deafened: false,
        }
    }
}

impl Mixer {
    pub fn push_pcm(&mut self, sender: u32, pcm: &[f32]) {
        let s = self.streams.entry(sender).or_insert_with(|| Stream {
            queue: VecDeque::with_capacity(FRAME_SAMPLES * MAX_BUFFER_FRAMES),
            playing: false,
            ended: false,
            last_packet: Instant::now(),
            underruns: 0,
        });
        s.queue.extend(pcm.iter().copied());
        s.last_packet = Instant::now();
        s.ended = false;
        if !s.playing && s.queue.len() >= FRAME_SAMPLES * PREBUFFER_FRAMES {
            s.playing = true;
        }
        if s.queue.len() > FRAME_SAMPLES * MAX_BUFFER_FRAMES {
            let extra = s.queue.len() - FRAME_SAMPLES * TRIM_TO_FRAMES;
            s.queue.drain(..extra);
        }
    }

    /// The sender stopped talking: play out whatever is buffered.
    pub fn end(&mut self, sender: u32) {
        if let Some(s) = self.streams.get_mut(&sender) {
            s.ended = true;
            s.playing = true;
        }
    }

    pub fn remove(&mut self, sender: u32) {
        self.streams.remove(&sender);
    }

    pub fn retain(&mut self, keep: impl Fn(u32) -> bool) {
        self.streams.retain(|id, _| keep(*id));
    }

    pub fn clear(&mut self) {
        self.streams.clear();
    }

    /// Volume for one person, 0.0 to 2.0 (0 = muted for me).
    pub fn set_gain(&mut self, sender: u32, gain: f32) {
        self.gains.insert(sender, gain);
    }

    pub fn play_system(&mut self, samples: &[f32]) {
        if self.system.len() < SAMPLE_RATE as usize * 2 {
            self.system.extend(samples.iter().copied());
        }
    }

    /// Fill `out` (48 kHz mono) with everyone mixed together.
    pub fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        for (id, s) in self.streams.iter_mut() {
            if !s.playing {
                continue;
            }
            let gain = if self.deafened {
                0.0
            } else {
                *self.gains.get(id).unwrap_or(&1.0)
            };
            let n = out.len().min(s.queue.len());
            for (o, x) in out.iter_mut().zip(s.queue.drain(..n)) {
                *o += x * gain;
            }
            if s.queue.is_empty() {
                s.playing = false;
                if !s.ended && n < out.len() {
                    s.underruns += 1;
                }
            }
        }
        let n = out.len().min(self.system.len());
        for (o, x) in out.iter_mut().zip(self.system.drain(..n)) {
            *o += x;
        }
        for o in out.iter_mut() {
            *o = o.clamp(-1.0, 1.0);
        }
    }

    /// Who is audible right now.
    pub fn speaking(&self) -> Vec<u32> {
        let now = Instant::now();
        self.streams
            .iter()
            .filter(|(_, s)| !s.ended && now.duration_since(s.last_packet) < SPEAKING_HOLD)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Times each person's audio ran dry mid-sentence since the last call (network hiccups).
    pub fn take_underruns(&mut self) -> Vec<(u32, u32)> {
        let mut v = Vec::new();
        for (id, s) in self.streams.iter_mut() {
            if s.underruns > 0 {
                v.push((*id, s.underruns));
                s.underruns = 0;
            }
        }
        v
    }
}

/// Opus decoders for each person; turns voice frames into PCM for the mixer.
pub struct Receiver {
    decoders: HashMap<u32, (opus::Decoder, u32)>,
    pcm: Vec<f32>,
}

impl Default for Receiver {
    fn default() -> Self {
        Self {
            decoders: HashMap::new(),
            pcm: vec![0.0; FRAME_SAMPLES * 6],
        }
    }
}

pub enum Decoded<'a> {
    Audio(&'a [f32]),
    End,
    Nothing,
}

impl Receiver {
    /// Returns true if this frame starts or ends someone's talking (the UI should refresh).
    pub fn handle(
        &mut self,
        kind: u8,
        sender: u32,
        seq: u32,
        payload: &[u8],
        mixer: &parking_lot::Mutex<Mixer>,
    ) -> bool {
        match self.decode(kind, sender, seq, payload) {
            Decoded::Audio(pcm) => {
                let mut m = mixer.lock();
                let was_speaking = m
                    .streams
                    .get(&sender)
                    .map(|s| !s.ended && s.last_packet.elapsed() < SPEAKING_HOLD)
                    .unwrap_or(false);
                m.push_pcm(sender, pcm);
                !was_speaking
            }
            Decoded::End => {
                mixer.lock().end(sender);
                true
            }
            Decoded::Nothing => false,
        }
    }

    pub fn decode(&mut self, kind: u8, sender: u32, seq: u32, payload: &[u8]) -> Decoded<'_> {
        if kind == VOICE_END {
            if let Some(d) = self.decoders.get_mut(&sender) {
                d.1 = seq;
            }
            return Decoded::End;
        }
        if kind != VOICE_DATA {
            return Decoded::Nothing;
        }
        let entry = match self.decoders.entry(sender) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(v) => {
                match opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono) {
                    Ok(d) => v.insert((d, seq.wrapping_sub(1))),
                    Err(_) => return Decoded::Nothing,
                }
            }
        };
        let (decoder, last_seq) = entry;
        let gap = seq.wrapping_sub(*last_seq);
        if gap == 0 || gap > u32::MAX / 2 {
            return Decoded::Nothing; // duplicate or old
        }
        let mut filled = 0;
        // A few frames went missing (the server skipped them for a slow link): let Opus fill the gap.
        if (2..=4).contains(&gap) {
            for _ in 0..gap - 1 {
                if let Ok(n) =
                    decoder.decode_float(&[], &mut self.pcm[filled..filled + FRAME_SAMPLES], false)
                {
                    filled += n;
                }
            }
        }
        *last_seq = seq;
        match decoder.decode_float(
            payload,
            &mut self.pcm[filled..filled + FRAME_SAMPLES],
            false,
        ) {
            Ok(n) => Decoded::Audio(&self.pcm[..filled + n]),
            Err(_) => Decoded::Nothing,
        }
    }

    pub fn retain(&mut self, keep: impl Fn(u32) -> bool) {
        self.decoders.retain(|id, _| keep(*id));
    }
}

// ---------------------------------------------------------------- sounds

/// Short two-note chimes for joining and leaving, synthesized at 48 kHz.
pub fn chime(kind: &str) -> Vec<f32> {
    let notes: &[(f32, f32, f32)] = match kind {
        "join" => &[(523.25, 0.0, 0.09), (783.99, 0.08, 0.16)],
        "leave" => &[(783.99, 0.0, 0.09), (523.25, 0.08, 0.16)],
        "mute" => &[(392.0, 0.0, 0.07)],
        "unmute" => &[(587.33, 0.0, 0.07)],
        "message" => &[(880.0, 0.0, 0.05), (1174.66, 0.05, 0.08)],
        _ => &[],
    };
    let sr = SAMPLE_RATE as f32;
    let total = notes.iter().map(|(_, s, d)| s + d).fold(0.0, f32::max);
    let mut out = vec![0.0f32; (total * sr) as usize + 1];
    for &(freq, start, dur) in notes {
        let s0 = (start * sr) as usize;
        let n = (dur * sr) as usize;
        for i in 0..n {
            let t = i as f32 / sr;
            let attack = (t / 0.015).min(1.0);
            let release = ((dur - t) / dur).max(0.0).powf(2.0);
            let v = (std::f32::consts::TAU * freq * t).sin() * 0.12 * attack * release;
            if let Some(o) = out.get_mut(s0 + i) {
                *o += v;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    fn sine(freq: f32, n: usize, amp: f32, phase0: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                (std::f32::consts::TAU * freq * (i + phase0) as f32 / SAMPLE_RATE as f32).sin()
                    * amp
            })
            .collect()
    }

    #[test]
    fn resampler_lengths() {
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        r.process(&vec![0.5; 44_100], &mut out);
        assert!((out.len() as i64 - 48_000).abs() <= 2, "{}", out.len());
        let mut r = Resampler::new(48_000, 44_100);
        let mut out = Vec::new();
        r.process(&vec![0.5; 48_000], &mut out);
        assert!((out.len() as i64 - 44_100).abs() <= 2, "{}", out.len());
    }

    #[test]
    fn gate_and_round_trip() {
        let ctl = VoiceControls::default();
        ctl.in_voice.store(true, Ordering::Relaxed);
        ctl.noise_suppression.store(false, Ordering::Relaxed);
        let mut tx = TxPipeline::new().unwrap();
        let mut frames = Vec::new();
        // Silence sends nothing.
        tx.push(&vec![0.0; FRAME_SAMPLES * 10], &ctl, &mut |f| {
            frames.push(f)
        });
        assert!(frames.is_empty());
        // A loud tone opens the gate.
        tx.push(&sine(440.0, FRAME_SAMPLES * 25, 0.3, 0), &ctl, &mut |f| {
            frames.push(f)
        });
        assert!(frames.len() >= 25, "{}", frames.len());
        assert!(ctl.talking.load(Ordering::Relaxed));
        // Silence again: hang time, then an END frame.
        tx.push(&vec![0.0; FRAME_SAMPLES * 30], &ctl, &mut |f| {
            frames.push(f)
        });
        assert_eq!(frames.last().unwrap()[0], VOICE_END);
        assert!(!ctl.talking.load(Ordering::Relaxed));

        // Decode everything and check we get a 440 Hz tone back.
        let mixer = Mutex::new(Mixer::default());
        let mut rx = Receiver::default();
        // Arrive one frame per 20 ms, with the speaker pulling 20 ms each time, like real playback.
        let mut played = Vec::new();
        let mut chunk = vec![0.0; FRAME_SAMPLES];
        for f in &frames {
            let (k, seq, p) = proto::parse_client_voice(f).unwrap();
            rx.handle(k, 7, seq, p, &mixer);
            mixer.lock().mix(&mut chunk);
            played.extend_from_slice(&chunk);
        }
        let out = &played[..FRAME_SAMPLES * 25];
        let level = rms_db(&out[FRAME_SAMPLES * 5..]);
        assert!(level > -20.0, "level {level}");
        // Zero crossings ~ 2 * 440 per second.
        let crossings = out[FRAME_SAMPLES * 5..]
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count() as f32;
        let secs = (out.len() - FRAME_SAMPLES * 5) as f32 / SAMPLE_RATE as f32;
        let freq = crossings / secs;
        assert!((freq - 440.0).abs() < 25.0, "freq {freq}");
    }

    #[test]
    fn muted_sends_nothing() {
        let ctl = VoiceControls::default();
        ctl.in_voice.store(true, Ordering::Relaxed);
        ctl.muted.store(true, Ordering::Relaxed);
        let mut tx = TxPipeline::new().unwrap();
        let mut frames = 0;
        tx.push(&sine(440.0, FRAME_SAMPLES * 20, 0.3, 0), &ctl, &mut |_| {
            frames += 1
        });
        assert_eq!(frames, 0);
    }

    #[test]
    fn push_to_talk() {
        let ctl = VoiceControls::default();
        ctl.in_voice.store(true, Ordering::Relaxed);
        ctl.push_to_talk.store(true, Ordering::Relaxed);
        let mut tx = TxPipeline::new().unwrap();
        let mut frames = 0;
        tx.push(&sine(440.0, FRAME_SAMPLES * 20, 0.3, 0), &ctl, &mut |_| {
            frames += 1
        });
        assert_eq!(frames, 0, "nothing without the key");
        ctl.ptt_held.store(true, Ordering::Relaxed);
        tx.push(&vec![0.0; FRAME_SAMPLES * 5], &ctl, &mut |_| frames += 1);
        assert_eq!(frames, 5, "even quiet audio goes out while the key is held");
    }

    #[test]
    fn noise_suppression_keeps_voice_like_tone_gate_shut_on_hiss() {
        let ctl = VoiceControls::default();
        ctl.in_voice.store(true, Ordering::Relaxed);
        let mut tx = TxPipeline::new().unwrap();
        let mut frames = 0;
        // Quiet steady hiss (pseudo-random) should be suppressed and not open the gate.
        let mut x: u32 = 1;
        let hiss: Vec<f32> = (0..FRAME_SAMPLES * 100)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 - 0.5) * 0.01
            })
            .collect();
        tx.push(&hiss, &ctl, &mut |_| frames += 1);
        assert!(frames < 10, "hiss opened the gate for {frames} frames");
    }

    #[test]
    fn jitter_buffer_waits_then_plays() {
        let mut m = Mixer::default();
        m.push_pcm(1, &vec![0.5; FRAME_SAMPLES]);
        let mut out = vec![0.0; FRAME_SAMPLES];
        m.mix(&mut out);
        assert_eq!(out[0], 0.0, "should wait for the prebuffer");
        m.push_pcm(1, &vec![0.5; FRAME_SAMPLES * 2]);
        m.mix(&mut out);
        assert!((out[0] - 0.5).abs() < 1e-6);
        m.set_gain(1, 0.0);
        m.mix(&mut out);
        assert_eq!(out[0], 0.0, "muted for me");
    }
}
