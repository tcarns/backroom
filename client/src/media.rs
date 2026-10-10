//! Playing videos and audio inside the app, with Windows' own Media Foundation
//! (the same decoders the Films & TV app uses), so TUFFcord carries no codecs.
//!
//! A [`Player`] runs on its own thread: it owns the media engine, which plays
//! the sound itself, and hands each new video frame over already scaled to the
//! size it's shown at. [`probe`] reads a video's size, length and one frame
//! (the poster) before it's sent.
//!
//! On other systems (only used for testing) there's no player: videos show
//! their poster and open in the system's player instead.

/// What a video or audio file is, worked out before sending it.
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    /// One frame, already turned the right way up and shrunk.
    pub poster: Option<image::RgbaImage>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// The file has been opened and its length is known.
    pub ready: bool,
    pub playing: bool,
    pub ended: bool,
    /// Seconds.
    pub position: f64,
    pub duration: f64,
    pub has_video: bool,
    /// The video's own size, once known.
    pub native: [u32; 2],
    /// Decoded by the graphics card (rather than the processor).
    pub gpu: bool,
    pub error: Option<String>,
}

pub fn supported() -> bool {
    cfg!(windows)
}

pub use imp::{probe, Player};

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod imp {
    use super::{Probe, Status};
    use crate::net::Wake;
    use eframe::egui::ColorImage;
    use parking_lot::Mutex;
    use std::path::Path;
    use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
    use std::sync::Arc;
    use std::time::Duration;
    use windows::core::{implement, BSTR, HSTRING};
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::*;
    use windows::Win32::System::Variant::VT_I8;

    enum Cmd {
        Play,
        Pause,
        Seek(f64),
        Volume(f32),
        Size(u32, u32),
    }

    #[derive(Default)]
    struct Shared {
        status: Status,
        frame: Option<ColorImage>,
    }

    pub struct Player {
        cmd: Sender<Cmd>,
        shared: Arc<Mutex<Shared>>,
    }

    impl Player {
        /// Start playing `path`. Video frames are delivered at `size` pixels
        /// (letterboxed to keep their shape).
        pub fn open(
            path: &Path,
            size: [u32; 2],
            volume: f32,
            wake: Wake,
        ) -> Result<Player, String> {
            let (tx, rx) = channel();
            let shared = Arc::new(Mutex::new(Shared::default()));
            let path = path.to_path_buf();
            let s = shared.clone();
            std::thread::Builder::new()
                .name("player".into())
                .spawn(move || {
                    let result = unsafe { run(&path, size, volume, &s, rx, &wake) };
                    if let Err(e) = result {
                        s.lock().status.error = Some(describe(&e));
                        wake();
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(Player { cmd: tx, shared })
        }
        pub fn play(&self) {
            let _ = self.cmd.send(Cmd::Play);
        }
        pub fn pause(&self) {
            let _ = self.cmd.send(Cmd::Pause);
        }
        pub fn seek(&self, seconds: f64) {
            let _ = self.cmd.send(Cmd::Seek(seconds.max(0.0)));
        }
        pub fn set_volume(&self, v: f32) {
            let _ = self.cmd.send(Cmd::Volume(v.clamp(0.0, 1.0)));
        }
        pub fn set_size(&self, size: [u32; 2]) {
            let _ = self.cmd.send(Cmd::Size(size[0], size[1]));
        }
        pub fn status(&self) -> Status {
            self.shared.lock().status.clone()
        }
        /// The newest frame, if there's one we haven't taken yet.
        pub fn take_frame(&self) -> Option<ColorImage> {
            self.shared.lock().frame.take()
        }
    }
    // Dropping the Player drops `cmd`; the thread notices and shuts the engine down.

    fn describe(e: &windows::core::Error) -> String {
        let code = e.code().0 as u32;
        match code {
            // MF_E_UNSUPPORTED_BYTESTREAM_TYPE, MF_E_TOPO_CODEC_NOT_FOUND, MF_E_INVALIDMEDIATYPE
            0xC00D36C4 | 0xC00D5212 | 0xC00D36B4 => {
                "Windows can't play this kind of file (a codec may be missing).".into()
            }
            _ => format!("Couldn't play this ({:#010x}).", code),
        }
    }

    #[implement(IMFMediaEngineNotify)]
    struct Notify {
        shared: Arc<Mutex<Shared>>,
        wake: Wake,
    }

    impl IMFMediaEngineNotify_Impl for Notify_Impl {
        fn EventNotify(
            &self,
            event: u32,
            _param1: usize,
            param2: u32,
        ) -> windows::core::Result<()> {
            let event = MF_MEDIA_ENGINE_EVENT(event as i32);
            {
                let mut s = self.shared.lock();
                if event == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA {
                    s.status.ready = true;
                } else if event == MF_MEDIA_ENGINE_EVENT_ENDED {
                    s.status.ended = true;
                    s.status.playing = false;
                } else if event == MF_MEDIA_ENGINE_EVENT_ERROR {
                    let e =
                        windows::core::Error::from_hresult(windows::core::HRESULT(param2 as i32));
                    s.status.error = Some(describe(&e));
                    s.status.playing = false;
                }
            }
            (self.wake)();
            Ok(())
        }
    }

    unsafe fn run(
        path: &Path,
        mut size: [u32; 2],
        volume: f32,
        shared: &Arc<Mutex<Shared>>,
        rx: Receiver<Cmd>,
        wake: &Wake,
    ) -> windows::core::Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
            let result = (|| -> windows::core::Result<()> {
                let factory: IMFMediaEngineClassFactory =
                    CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
                let mut attrs: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut attrs, 2)?;
                let attrs = attrs.unwrap();
                let notify: IMFMediaEngineNotify = Notify {
                    shared: shared.clone(),
                    wake: wake.clone(),
                }
                .into();
                attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
                // Frame-server mode: we ask for each frame (no window of its own).
                attrs.SetUINT32(
                    &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                    DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
                )?;
                // Let the graphics card decode when there is one (much lighter on
                // the processor); otherwise Windows decodes in software.
                let gpu = gpu::Gpu::new().ok();
                if let Some(g) = &gpu {
                    attrs.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &g.manager)?;
                }
                shared.lock().status.gpu = gpu.is_some();
                let engine = factory.CreateInstance(0, &attrs)?;
                let wic: IWICImagingFactory =
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
                engine.SetVolume(volume as f64)?;
                engine.SetSource(&BSTR::from(path.to_string_lossy().as_ref()))?;
                engine.Play()?;

                let mut bitmap: Option<(IWICBitmap, u32, u32)> = None;
                let mut textures: Option<gpu::Targets> = None;
                let mut last_pts = i64::MIN;
                let mut frames_shown = 0u32;
                let mut frame_error: Option<windows::core::Error> = None;
                let mut gave_up_on_picture = false;
                let mut last_wake = std::time::Instant::now();
                'outer: loop {
                    // Commands, waiting a little (about 120 checks a second).
                    let first = match rx.recv_timeout(Duration::from_millis(8)) {
                        Ok(c) => Some(c),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break 'outer,
                    };
                    for cmd in first.into_iter().chain(rx.try_iter()) {
                        match cmd {
                            Cmd::Play => {
                                if engine.IsEnded().as_bool() {
                                    let _ = engine.SetCurrentTime(0.0);
                                }
                                shared.lock().status.ended = false;
                                let _ = engine.Play();
                            }
                            Cmd::Pause => {
                                let _ = engine.Pause();
                            }
                            Cmd::Seek(t) => {
                                shared.lock().status.ended = false;
                                let _ = engine.SetCurrentTime(t);
                                last_pts = i64::MIN;
                            }
                            Cmd::Volume(v) => {
                                let _ = engine.SetVolume(v as f64);
                            }
                            Cmd::Size(w, h) => {
                                size = [w, h];
                                last_pts = i64::MIN;
                            }
                        }
                    }
                    let has_video = engine.HasVideo().as_bool();
                    if has_video && size[0] >= 2 && size[1] >= 2 && !gave_up_on_picture {
                        if let Ok(pts) = engine.OnVideoStreamTick() {
                            if pts != last_pts && pts >= 0 {
                                last_pts = pts;
                                let grabbed = match &gpu {
                                    Some(g) => g.grab(&engine, &mut textures, size),
                                    None => grab(&engine, &wic, &mut bitmap, size),
                                };
                                match grabbed {
                                    Ok(img) => {
                                        frames_shown += 1;
                                        shared.lock().frame = Some(img);
                                        wake();
                                    }
                                    Err(e) => frame_error = Some(e),
                                }
                            }
                        }
                        // Sound but no picture after a while: say so, so the app can
                        // offer the system's video player instead.
                        if frames_shown == 0 && engine.GetCurrentTime() > 1.5 {
                            gave_up_on_picture = true;
                            let why = frame_error
                                .as_ref()
                                .map(|e| format!(" ({:#010x})", e.code().0 as u32))
                                .unwrap_or_default();
                            let _ = engine.Pause();
                            shared.lock().status.error =
                                Some(format!("Couldn't show this video here{why}."));
                            wake();
                        }
                    }
                    let d = engine.GetDuration();
                    let playing = !engine.IsPaused().as_bool() && !engine.IsEnded().as_bool();
                    let mut native = [0u32; 2];
                    if has_video {
                        let _ =
                            engine.GetNativeVideoSize(Some(&mut native[0]), Some(&mut native[1]));
                    }
                    let changed = {
                        let mut s = shared.lock();
                        s.status.native = native;
                        let changed =
                            s.status.playing != playing || s.status.has_video != has_video;
                        s.status.has_video = has_video;
                        s.status.position = engine.GetCurrentTime();
                        s.status.duration = if d.is_finite() { d } else { 0.0 };
                        s.status.playing = playing;
                        changed
                    };
                    // Video frames wake the UI anyway; audio needs its seek bar moved.
                    if changed
                        || (playing
                            && !has_video
                            && last_wake.elapsed() > Duration::from_millis(250))
                    {
                        last_wake = std::time::Instant::now();
                        wake();
                    }
                }
                let _ = engine.Shutdown();
                Ok(())
            })();
            let _ = MFShutdown();
            CoUninitialize();
            result
        }
    }

    /// Decoding on the graphics card: a Direct3D 11 device shared with Media
    /// Foundation, and textures to copy each frame through.
    mod gpu {
        use eframe::egui::ColorImage;
        use windows::core::Interface;
        use windows::Win32::Foundation::{E_FAIL, HMODULE, RECT};
        use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
        use windows::Win32::Graphics::Direct3D11::*;
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
        };
        use windows::Win32::Media::MediaFoundation::*;

        pub struct Gpu {
            device: ID3D11Device,
            context: ID3D11DeviceContext,
            pub manager: IMFDXGIDeviceManager,
        }

        /// A texture the engine draws into, and one we can read back.
        pub struct Targets {
            target: ID3D11Texture2D,
            staging: ID3D11Texture2D,
            size: [u32; 2],
        }

        impl Gpu {
            pub unsafe fn new() -> windows::core::Result<Gpu> {
                unsafe {
                    let mut device = None;
                    let mut context = None;
                    D3D11CreateDevice(
                        None,
                        D3D_DRIVER_TYPE_HARDWARE,
                        HMODULE::default(),
                        D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                        None,
                        D3D11_SDK_VERSION,
                        Some(&mut device),
                        None,
                        Some(&mut context),
                    )?;
                    let device =
                        device.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
                    let context =
                        context.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
                    // Media Foundation uses the device from its own threads.
                    if let Ok(mt) = device.cast::<ID3D11Multithread>() {
                        let _ = mt.SetMultithreadProtected(true);
                    }
                    let mut token = 0u32;
                    let mut manager = None;
                    MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
                    let manager =
                        manager.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
                    manager.ResetDevice(&device, token)?;
                    Ok(Gpu {
                        device,
                        context,
                        manager,
                    })
                }
            }

            fn texture(
                &self,
                size: [u32; 2],
                staging: bool,
            ) -> windows::core::Result<ID3D11Texture2D> {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: size[0],
                    Height: size[1],
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: if staging {
                        D3D11_USAGE_STAGING
                    } else {
                        D3D11_USAGE_DEFAULT
                    },
                    BindFlags: if staging {
                        0
                    } else {
                        D3D11_BIND_RENDER_TARGET.0 as u32
                    },
                    CPUAccessFlags: if staging {
                        D3D11_CPU_ACCESS_READ.0 as u32
                    } else {
                        0
                    },
                    MiscFlags: 0,
                };
                let mut tex = None;
                unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut tex))? };
                tex.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))
            }

            /// The current frame, scaled to `size`.
            pub unsafe fn grab(
                &self,
                engine: &IMFMediaEngine,
                targets: &mut Option<Targets>,
                size: [u32; 2],
            ) -> windows::core::Result<ColorImage> {
                unsafe {
                    if !targets.as_ref().is_some_and(|t| t.size == size) {
                        *targets = Some(Targets {
                            target: self.texture(size, false)?,
                            staging: self.texture(size, true)?,
                            size,
                        });
                    }
                    let t = targets.as_ref().unwrap();
                    let [w, h] = size;
                    let rect = RECT {
                        left: 0,
                        top: 0,
                        right: w as i32,
                        bottom: h as i32,
                    };
                    let black = MFARGB {
                        rgbBlue: 0,
                        rgbGreen: 0,
                        rgbRed: 0,
                        rgbAlpha: 255,
                    };
                    engine.TransferVideoFrame(&t.target, None, &rect, Some(&black))?;
                    self.context.CopyResource(&t.staging, &t.target);
                    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                    self.context
                        .Map(&t.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                    let pitch = mapped.RowPitch as usize;
                    let data =
                        std::slice::from_raw_parts(mapped.pData as *const u8, pitch * h as usize);
                    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                    for y in 0..h as usize {
                        for px in data[y * pitch..y * pitch + w as usize * 4].chunks_exact(4) {
                            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                        }
                    }
                    self.context.Unmap(&t.staging, 0);
                    Ok(ColorImage::from_rgba_unmultiplied(
                        [w as usize, h as usize],
                        &rgba,
                    ))
                }
            }
        }
    }

    /// Copy the current frame, scaled to `size`, into an image for egui.
    unsafe fn grab(
        engine: &IMFMediaEngine,
        wic: &IWICImagingFactory,
        bitmap: &mut Option<(IWICBitmap, u32, u32)>,
        size: [u32; 2],
    ) -> windows::core::Result<ColorImage> {
        use windows::Win32::Foundation::E_FAIL;
        let fail = || windows::core::Error::from_hresult(E_FAIL);
        unsafe {
            let [w, h] = size;
            if !bitmap.as_ref().is_some_and(|b| b.1 == w && b.2 == h) {
                let b =
                    wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppBGRA, WICBitmapCacheOnLoad)?;
                *bitmap = Some((b, w, h));
            }
            let (bmp, _, _) = bitmap.as_ref().ok_or_else(fail)?;
            let rect = RECT {
                left: 0,
                top: 0,
                right: w as i32,
                bottom: h as i32,
            };
            let black = MFARGB {
                rgbBlue: 0,
                rgbGreen: 0,
                rgbRed: 0,
                rgbAlpha: 255,
            };
            engine.TransferVideoFrame(bmp, None, &rect, Some(&black))?;
            let lock = bmp.Lock(
                &WICRect {
                    X: 0,
                    Y: 0,
                    Width: w as i32,
                    Height: h as i32,
                },
                WICBitmapLockRead.0 as u32,
            )?;
            let stride = lock.GetStride()? as usize;
            let mut len = 0u32;
            let mut ptr = std::ptr::null_mut();
            lock.GetDataPointer(&mut len, &mut ptr)?;
            let data = std::slice::from_raw_parts(ptr, len as usize);
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row = data
                    .get(y * stride..y * stride + w as usize * 4)
                    .ok_or_else(fail)?;
                for px in row.chunks_exact(4) {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
            Ok(ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                &rgba,
            ))
        }
    }

    // ------------------------------------------------------------ probe

    /// Open the video silently in a player, skip a little way in, and take a frame.
    fn capture_with_player(path: &Path) -> Option<Probe> {
        use std::time::Instant;
        let player = Player::open(path, [0, 0], 0.0, Arc::new(|| {})).ok()?;
        let wait = |cond: &dyn Fn(&Status) -> bool| {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(6) {
                let s = player.status();
                if s.error.is_some() {
                    return None;
                }
                if cond(&s) {
                    return Some(s);
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            None
        };
        let st = wait(&|s| s.ready && s.native[0] > 0 && s.duration > 0.0)?;
        let [nw, nh] = st.native;
        let scale = (640.0 / nw.max(nh) as f32).min(1.0);
        let size = [
            ((nw as f32 * scale) as u32).max(2),
            ((nh as f32 * scale) as u32).max(2),
        ];
        player.seek((st.duration / 3.0).min(1.0));
        player.set_size(size);
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(6) {
            if let Some(img) = player.take_frame() {
                // Skip frames from before the seek: the engine may hand one over early.
                if player.status().position < 0.05 && st.duration > 1.0 {
                    continue;
                }
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                let poster =
                    image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, rgba)?;
                return Some(Probe {
                    width: nw,
                    height: nh,
                    duration_ms: (st.duration * 1000.0) as u64,
                    poster: Some(poster),
                });
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        None
    }

    /// Size, length and a poster frame of a video (or the length of audio).
    pub fn probe(path: &Path) -> Result<Probe, String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| e.to_string())?;
            let result = probe_inner(path).map_err(|e| e.message().to_string());
            let _ = MFShutdown();
            CoUninitialize();
            result
        }
    }

    unsafe fn probe_inner(path: &Path) -> windows::core::Result<Probe> {
        unsafe {
            // Windows' newer converter first (Windows 8 and later), then the older one.
            let mut best = None;
            for advanced in [true, false] {
                match probe_with(path, advanced) {
                    Ok(p) if p.poster.is_some() => return Ok(p),
                    Ok(p) => best = best.or(Some(Ok(p))),
                    Err(e) => best = best.or(Some(Err(e))),
                }
            }
            // Neither gave a frame: open it like the player does and take one.
            if let Some(mut p) = capture_with_player(path) {
                if let Some(Ok(b)) = &best {
                    if p.duration_ms == 0 {
                        p.duration_ms = b.duration_ms;
                    }
                }
                return Ok(p);
            }
            best.unwrap()
        }
    }

    unsafe fn probe_with(path: &Path, advanced: bool) -> windows::core::Result<Probe> {
        unsafe {
            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 1)?;
            let attrs = attrs.unwrap();
            // Let Media Foundation convert to plain RGB for us.
            if advanced {
                attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
            } else {
                attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
            }
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), &attrs)?;
            let mut probe = Probe::default();
            if let Ok(pv) = reader
                .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
            {
                let hns = pv.Anonymous.Anonymous.Anonymous.uhVal;
                probe.duration_ms = hns / 10_000;
            }
            let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
            let mt = MFCreateMediaType()?;
            mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            if reader.SetCurrentMediaType(video, None, &mt).is_err() {
                return Ok(probe); // audio only
            }
            let current = reader.GetCurrentMediaType(video)?;
            let packed = current.GetUINT64(&MF_MT_FRAME_SIZE)?;
            let (w, h) = ((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32);
            let stride = current
                .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                .map(|s| s as i32)
                .unwrap_or(w as i32 * 4);
            let rotation = reader
                .GetNativeMediaType(video, 0)
                .and_then(|t| t.GetUINT32(&MF_MT_VIDEO_ROTATION))
                .unwrap_or(0);
            // A frame a little way in is usually more telling than the first.
            if probe.duration_ms > 3000 {
                let mut pv = PROPVARIANT::default();
                (*pv.Anonymous.Anonymous).vt = VT_I8;
                (*pv.Anonymous.Anonymous).Anonymous.hVal = 10_000_000; // 1 s
                let _ = reader.SetCurrentPosition(&windows::core::GUID::zeroed(), &pv);
            }
            for _ in 0..60 {
                let mut flags = 0u32;
                let mut sample: Option<IMFSample> = None;
                reader.ReadSample(video, 0, None, Some(&mut flags), None, Some(&mut sample))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                let Some(sample) = sample else { continue };
                let buffer = sample.ConvertToContiguousBuffer()?;
                let mut ptr = std::ptr::null_mut();
                let mut len = 0u32;
                buffer.Lock(&mut ptr, None, Some(&mut len))?;
                let data = std::slice::from_raw_parts(ptr, len as usize);
                let row_bytes = w as usize * 4;
                let abs = stride.unsigned_abs() as usize;
                let mut img = image::RgbaImage::new(w, h);
                let mut ok = true;
                for y in 0..h as usize {
                    // A negative stride means the rows are stored bottom-up.
                    let src_row = if stride < 0 { h as usize - 1 - y } else { y };
                    let Some(row) = data.get(src_row * abs..src_row * abs + row_bytes) else {
                        ok = false;
                        break;
                    };
                    for (x, px) in row.chunks_exact(4).enumerate() {
                        img.put_pixel(x as u32, y as u32, image::Rgba([px[2], px[1], px[0], 255]));
                    }
                }
                let _ = buffer.Unlock();
                if ok {
                    let img = match rotation {
                        90 => image::imageops::rotate90(&img),
                        180 => image::imageops::rotate180(&img),
                        270 => image::imageops::rotate270(&img),
                        _ => img,
                    };
                    probe.width = img.width();
                    probe.height = img.height();
                    probe.poster = Some(img);
                }
                break;
            }
            if probe.poster.is_none() {
                let (pw, ph) = if rotation == 90 || rotation == 270 {
                    (h, w)
                } else {
                    (w, h)
                };
                probe.width = pw;
                probe.height = ph;
            }
            Ok(probe)
        }
    }
}

// ---------------------------------------------------------------- elsewhere

#[cfg(not(windows))]
mod imp {
    use super::{Probe, Status};
    use crate::net::Wake;
    use eframe::egui::ColorImage;
    use std::path::Path;

    pub struct Player;

    impl Player {
        pub fn open(
            _path: &Path,
            _size: [u32; 2],
            _volume: f32,
            _wake: Wake,
        ) -> Result<Player, String> {
            Err("Playing inside TUFFcord needs Windows.".into())
        }
        pub fn play(&self) {}
        pub fn pause(&self) {}
        pub fn seek(&self, _seconds: f64) {}
        pub fn set_volume(&self, _v: f32) {}
        pub fn set_size(&self, _size: [u32; 2]) {}
        pub fn status(&self) -> Status {
            Status::default()
        }
        pub fn take_frame(&self) -> Option<ColorImage> {
            None
        }
    }

    pub fn probe(_path: &Path) -> Result<Probe, String> {
        Err("Needs Windows.".into())
    }
}

/// A poster frame as a small JPEG to send along with a video.
pub fn poster_jpeg(img: &image::RgbaImage) -> Option<Vec<u8>> {
    let small = image::DynamicImage::ImageRgba8(img.clone())
        .thumbnail(640, 640)
        .to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 78)
        .encode_image(&small)
        .ok()?;
    (out.len() as u64 <= proto::files::MAX_POSTER).then_some(out)
}
