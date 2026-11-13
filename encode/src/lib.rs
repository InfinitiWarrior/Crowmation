//! Video export: I420 frames in, H.264 MP4 out.
//!
//! Backends: NVENC (feature `nvenc`, preferred) and openh264 (always built,
//! the fallback for machines and CI runners without a hardware encoder).

#[cfg(feature = "nvenc")]
mod nvenc_load;
use std::sync::mpsc::Receiver;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Backend {
    /// NVENC if it is built in and works on this machine, else openh264.
    Auto,
    Nvenc,
    OpenH264,
}

pub struct Settings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Slower encoder presets for smaller files at the same visual quality.
    pub high_quality: bool,
    /// Total number of frames.
    pub frames: u64,
}

/// The order to render frames in, and how many interleaved lanes it has.
///
/// NVENC takes frames in order. openh264 uses one core per encoder, so the
/// video is cut into up to `OPENH264_THREADS` segments (whole keyframe
/// intervals), each with its own encoder, and frames are rendered round robin
/// across segments: lane k is segment k. Any order is valid input to
/// `encode_mp4`; this one keeps every encoder busy without buffering raw frames.
pub fn frame_order(s: &Settings, backend: Backend) -> (Vec<u64>, usize) {
    let nvenc = backend == Backend::Nvenc || (backend == Backend::Auto && nvenc_present());
    let seg = segment_len(s);
    if nvenc || seg >= s.frames {
        return ((0..s.frames).collect(), 1);
    }
    let lanes = s.frames.div_ceil(seg);
    let order = (0..seg).flat_map(|j| (0..lanes).map(move |k| k * seg + j)).filter(|f| *f < s.frames).collect();
    (order, lanes as usize)
}

/// Keyframe interval in frames.
fn gop(s: &Settings) -> u64 {
    s.fps as u64 * 2
}

/// Frames per openh264 segment: an even share, rounded up to whole keyframe intervals.
fn segment_len(s: &Settings) -> u64 {
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, OPENH264_THREADS) as u64;
    s.frames.div_ceil(workers).div_ceil(gop(s)).max(1) * gop(s)
}

#[cfg(feature = "nvenc")]
fn nvenc_present() -> bool {
    nvenc_load::missing().is_none()
}

#[cfg(not(feature = "nvenc"))]
fn nvenc_present() -> bool {
    false
}

/// Parallel openh264 encoders. More stops helping once rendering is the bottleneck.
const OPENH264_THREADS: usize = 8;

/// Receives encoded Annex B access units in frame order.
type Sink<'a> = dyn FnMut(&[u8]) -> Result<(), String> + 'a;

/// Encode `s.frames` I420 frames, received as (frame index, pixels) in the
/// order from `frame_order`, into an MP4 at `path`. Run it on its own thread
/// so encoding overlaps rendering. Returns the name of the backend used.
pub fn encode_mp4(path: &std::path::Path, s: &Settings, backend: Backend, frames: Receiver<(u64, Vec<u8>)>) -> Result<&'static str, String> {
    let file = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?);
    let mut mux = muxide::api::MuxerBuilder::new(file)
        .video(muxide::api::VideoCodec::H264, s.width, s.height, s.fps as f64)
        .build()
        .map_err(|e| format!("mp4: {e}"))?;
    let mut n = 0u64;
    let fps = s.fps as f64;
    let mut sink = |au: &[u8]| {
        if au.is_empty() {
            return Ok(());
        }
        mux.write_video(n as f64 / fps, au, is_keyframe(au)).map_err(|e| format!("mp4: {e}"))?;
        n += 1;
        Ok(())
    };
    let used = match backend {
        Backend::OpenH264 => openh264_loop(s, frames, &mut sink)?,
        Backend::Nvenc => nvenc_loop(s, frames, &mut sink).map_err(|e| e.message())?,
        Backend::Auto => match nvenc_loop(s, frames, &mut sink) {
            Ok(name) => name,
            Err(NvError::Unavailable(frames, why)) => {
                eprintln!("NVENC unavailable ({why}), using openh264");
                openh264_loop(s, frames, &mut sink)?
            }
            Err(NvError::Failed(e)) => return Err(e),
        },
    };
    mux.finish().map_err(|e| format!("mp4: {e}"))?;
    Ok(used)
}

enum NvError {
    /// NVENC could not start; hands the untouched frame channel back for the fallback.
    Unavailable(Receiver<(u64, Vec<u8>)>, String),
    /// Failed after frames were consumed; no fallback possible.
    #[allow(dead_code)]
    Failed(String),
}

impl NvError {
    fn message(self) -> String {
        match self {
            NvError::Unavailable(_, m) | NvError::Failed(m) => format!("NVENC: {m}"),
        }
    }
}

impl From<String> for NvError {
    fn from(m: String) -> Self {
        NvError::Failed(m)
    }
}

/// True if an Annex B access unit holds an IDR slice.
fn is_keyframe(au: &[u8]) -> bool {
    au.windows(4).any(|w| w[..3] == [0, 0, 1] && w[3] & 0x1f == 5)
}

fn planes(s: &Settings) -> (usize, usize) {
    let y = (s.width * s.height) as usize;
    (y, y / 4)
}

fn openh264_loop(s: &Settings, frames: Receiver<(u64, Vec<u8>)>, sink: &mut Sink) -> Result<&'static str, String> {
    use openh264::encoder::{BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, UsageType, VuiConfig};
    use std::collections::BTreeMap;
    use std::sync::mpsc;

    let gop = gop(s);
    let cfg = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .bitrate(BitRate::from_bps(s.width * s.height * s.fps / 8))
        .max_frame_rate(FrameRate::from_hz(s.fps as f32))
        .complexity(if s.high_quality { Complexity::High } else { Complexity::Medium })
        .vui(VuiConfig::bt709()) // matches the GPU conversion; untagged HD decodes as BT.601 in ffmpeg
        .adaptive_quantization(false)
        .background_detection(false)
        .num_threads(1)
        .intra_frame_period(IntraFramePeriod::from_num_frames(gop as u32));

    // One encoder per segment (see `frame_order`); results are written in frame order.
    let seg = segment_len(s);
    let workers = s.frames.div_ceil(seg).max(1) as usize;
    let (done_tx, done_rx) = mpsc::channel::<(u64, Result<Vec<u8>, String>)>();
    let mut inputs = Vec::new();
    for _ in 0..workers {
        let mut enc = Encoder::with_api_config(openh264::OpenH264API::from_source(), cfg).map_err(|e| format!("openh264: {e}"))?;
        let (tx, rx) = mpsc::sync_channel::<(u64, Vec<u8>)>(2);
        let done = done_tx.clone();
        let (w, h) = (s.width as usize, s.height as usize);
        let (ys, cs) = planes(s);
        std::thread::spawn(move || {
            for (i, f) in rx {
                if i % gop == 0 {
                    enc.force_intra_frame();
                }
                let yuv = openh264::formats::YUVSlices::new((&f[..ys], &f[ys..ys + cs], &f[ys + cs..]), (w, h), (w, w / 2, w / 2));
                let bits = enc.encode(&yuv).map(|b| b.to_vec()).map_err(|e| format!("openh264: {e}"));
                if done.send((i, bits)).is_err() {
                    return;
                }
            }
        });
        inputs.push(tx);
    }
    drop(done_tx);

    // Encoded frames arrive out of order; hand them to the sink in order.
    let mut ready: BTreeMap<u64, Result<Vec<u8>, String>> = BTreeMap::new();
    let mut next = 0u64;
    let mut flush = |ready: &mut BTreeMap<u64, Result<Vec<u8>, String>>, next: &mut u64| -> Result<(), String> {
        while let Some(r) = ready.remove(next) {
            sink(&r?)?;
            *next += 1;
        }
        Ok(())
    };
    for (i, f) in frames {
        inputs[((i / seg) as usize).min(workers - 1)].send((i, f)).map_err(|_| "openh264: encoder thread stopped")?;
        ready.extend(done_rx.try_iter());
        flush(&mut ready, &mut next)?;
    }
    drop(inputs);
    ready.extend(done_rx.iter());
    flush(&mut ready, &mut next)?;
    Ok("openh264")
}

#[cfg(not(feature = "nvenc"))]
fn nvenc_loop(_: &Settings, frames: Receiver<(u64, Vec<u8>)>, _: &mut Sink) -> Result<&'static str, NvError> {
    Err(NvError::Unavailable(frames, "built without the `nvenc` feature".into()))
}

#[cfg(feature = "nvenc")]
fn nvenc_loop(s: &Settings, frames: Receiver<(u64, Vec<u8>)>, sink: &mut Sink) -> Result<&'static str, NvError> {
    use nvidia_video_codec_sdk::sys::nvEncodeAPI::{
        NV_ENC_BUFFER_FORMAT::NV_ENC_BUFFER_FORMAT_IYUV, NV_ENC_CODEC_H264_GUID, NV_ENC_PARAMS_RC_MODE,
        NV_ENC_PRESET_P1_GUID, NV_ENC_PRESET_P5_GUID, NV_ENC_TUNING_INFO, NV_ENC_VUI_COLOR_PRIMARIES, NV_ENC_VUI_MATRIX_COEFFS,
        NV_ENC_VUI_TRANSFER_CHARACTERISTIC,
    };
    use nvidia_video_codec_sdk::{EncodePictureParams, Encoder, EncoderInitParams};

    // P1 runs about 2x faster than P5 on simple graphics, for roughly 10% larger files.
    let preset_guid = if s.high_quality { NV_ENC_PRESET_P5_GUID } else { NV_ENC_PRESET_P1_GUID };
    if let Some(why) = nvenc_load::missing() {
        return Err(NvError::Unavailable(frames, why));
    }
    // Everything up to the first frame can fail and fall back to openh264.
    let setup = || -> Result<_, String> {
        let ctx = cudarc::driver::CudaContext::new(0).map_err(|e| format!("CUDA: {e:?}"))?;
        let enc = Encoder::initialize_with_cuda(ctx).map_err(|e| format!("{e:?}"))?;
        let tuning = NV_ENC_TUNING_INFO::NV_ENC_TUNING_INFO_HIGH_QUALITY;
        let mut preset = enc.get_preset_config(NV_ENC_CODEC_H264_GUID, preset_guid, tuning).map_err(|e| format!("{e:?}"))?;
        let cfg = &mut preset.presetCfg;
        let gop = s.fps * 2;
        cfg.gopLength = gop;
        cfg.frameIntervalP = 1; // no B frames: one output per input, simple timestamps
        cfg.rcParams.rateControlMode = NV_ENC_PARAMS_RC_MODE::NV_ENC_PARAMS_RC_VBR;
        cfg.rcParams.averageBitRate = 0;
        cfg.rcParams.targetQuality = 20; // constant quality, lower is better
        unsafe {
            cfg.encodeCodecConfig.h264Config.idrPeriod = gop;
            cfg.encodeCodecConfig.h264Config.set_repeatSPSPPS(1); // every keyframe is self contained
            // Tag BT.709 limited range to match the GPU conversion.
            let vui = &mut cfg.encodeCodecConfig.h264Config.h264VUIParameters;
            vui.videoSignalTypePresentFlag = 1;
            vui.videoFullRangeFlag = 0;
            vui.colourDescriptionPresentFlag = 1;
            vui.colourPrimaries = NV_ENC_VUI_COLOR_PRIMARIES::NV_ENC_VUI_COLOR_PRIMARIES_BT709;
            vui.transferCharacteristics = NV_ENC_VUI_TRANSFER_CHARACTERISTIC::NV_ENC_VUI_TRANSFER_CHARACTERISTIC_BT709;
            vui.colourMatrix = NV_ENC_VUI_MATRIX_COEFFS::NV_ENC_VUI_MATRIX_COEFFS_BT709;
        }
        Ok((enc, preset))
    };
    let (enc, mut preset) = match setup() {
        Ok(x) => x,
        Err(e) => return Err(NvError::Unavailable(frames, e)),
    };
    let mut init = EncoderInitParams::new(NV_ENC_CODEC_H264_GUID, s.width, s.height);
    init.preset_guid(preset_guid)
        .tuning_info(NV_ENC_TUNING_INFO::NV_ENC_TUNING_INFO_HIGH_QUALITY)
        .framerate(s.fps, 1)
        .enable_picture_type_decision()
        .encode_config(&mut preset.presetCfg);
    let session = match enc.start_session(NV_ENC_BUFFER_FORMAT_IYUV, init) {
        Ok(x) => x,
        Err(e) => return Err(NvError::Unavailable(frames, format!("{e:?}"))),
    };
    let err = |e| format!("NVENC: {e:?}");
    // Several frames in flight, so copying frame N overlaps encoding of earlier ones.
    const DEPTH: usize = 4;
    let mut inputs = (0..DEPTH).map(|_| session.create_input_buffer()).collect::<Result<Vec<_>, _>>().map_err(err)?;
    let mut outputs = (0..DEPTH).map(|_| session.create_output_bitstream()).collect::<Result<Vec<_>, _>>().map_err(err)?;
    // Learn the row pitch the driver picked for the input buffers.
    drop(inputs[0].lock().map_err(err)?);
    let pitch = nvidia_video_codec_sdk::EncoderInput::pitch(&inputs[0]) as usize;
    let (w, h) = (s.width as usize, s.height as usize);
    let mut padded = vec![0u8; pitch * h * 3 / 2];
    let mut in_flight = std::collections::VecDeque::new();
    // NVENC gets frames in order (see `frame_order`).
    for (i, (_, f)) in frames.into_iter().enumerate() {
        let k = i % DEPTH;
        if in_flight.len() == DEPTH {
            let old: usize = in_flight.pop_front().unwrap();
            sink(outputs[old].lock().map_err(err)?.data())?;
        }
        let (ys, cs) = planes(s);
        // IYUV in a pitched buffer: Y rows at `pitch`, U and V rows at `pitch / 2`.
        for r in 0..h {
            padded[r * pitch..r * pitch + w].copy_from_slice(&f[r * w..(r + 1) * w]);
        }
        let (cw, cp) = (w / 2, pitch / 2);
        for (plane, src) in [(0, &f[ys..ys + cs]), (1, &f[ys + cs..])] {
            let base = pitch * h + plane * cp * (h / 2);
            for r in 0..h / 2 {
                padded[base + r * cp..base + r * cp + cw].copy_from_slice(&src[r * cw..(r + 1) * cw]);
            }
        }
        unsafe { inputs[k].lock().map_err(err)?.write(&padded) };
        session
            .encode_picture(&mut inputs[k], &mut outputs[k], EncodePictureParams { input_timestamp: i as u64, ..Default::default() })
            .map_err(err)?;
        in_flight.push_back(k);
    }
    for old in in_flight {
        sink(outputs[old].lock().map_err(err)?.data())?;
    }
    Ok("nvenc")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(backend: Backend) -> (String, std::path::PathBuf) {
        let (w, h) = (320usize, 240usize);
        let s = Settings { width: w as u32, height: h as u32, fps: 30, high_quality: false, frames: 30 };
        let path = std::env::temp_dir().join(format!("crowmation-{backend:?}-{}.mp4", std::process::id()));
        let (tx, rx) = std::sync::mpsc::channel();
        for i in 0..30u8 {
            let mut f = vec![16u8 + i * 4; w * h];
            f.extend(vec![128u8; w * h / 2]);
            tx.send((i as u64, f)).unwrap();
        }
        drop(tx);
        (encode_mp4(&path, &s, backend, rx).unwrap().to_string(), path)
    }

    #[test]
    fn openh264_writes_mp4() {
        let (name, path) = encode(Backend::OpenH264);
        assert_eq!(name, "openh264");
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"ftyp") && bytes.windows(4).any(|w| w == b"avcC"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn auto_picks_something() {
        let (name, path) = encode(Backend::Auto);
        assert!(name == "nvenc" || name == "openh264");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn keyframe_detection() {
        assert!(is_keyframe(&[0, 0, 0, 1, 0x67, 0, 0, 1, 0x65]));
        assert!(!is_keyframe(&[0, 0, 0, 1, 0x41, 1, 2]));
    }
}
