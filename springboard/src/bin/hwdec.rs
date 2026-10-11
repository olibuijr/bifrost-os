//! Hardware decode benchmark: decodes an H.264/HEVC elementary stream (Annex-B) with
//! libavcodec (VA-API when the render node has it, else software; `SPRINGBOARD_VIDEO_SW=1`
//! forces software), reports fps and can dump one frame as NV12.
//!
//! hwdec [--dump N FILE.nv12] [--max FRAMES] STREAM.h264|.h265

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("hwdec needs Linux");
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(e) = desktop() {
        eprintln!("hwdec: {e}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
fn desktop() -> std::io::Result<()> {
    use springboard::media::hwdec::{ffmpeg, nal, vaapi, Codec, Pixels, VideoDecoder};
    use std::io::Write;
    use std::time::Instant;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut dump, mut max, mut path) = (None, u64::MAX, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dump" => {
                let n: u64 = it.next().and_then(|s| s.parse().ok()).ok_or(std::io::ErrorKind::InvalidInput)?;
                dump = Some((n, it.next().cloned().ok_or(std::io::ErrorKind::InvalidInput)?));
            }
            "--max" => max = it.next().and_then(|s| s.parse().ok()).ok_or(std::io::ErrorKind::InvalidInput)?,
            p => path = Some(p.to_string()),
        }
    }
    let path = path.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "usage: hwdec [--dump N FILE.nv12] [--max FRAMES] STREAM.h264|.h265"))?;
    let codec = if path.ends_with(".h265") || path.ends_with(".hevc") { Codec::Hevc } else { Codec::H264 };
    let data = std::fs::read(&path)?;
    let nals = nal::split_annexb(&data);
    let units = nal::access_units(codec, &nals);
    println!("{path}: {} NAL units, {} access units, {codec:?}", nals.len(), units.len());
    println!("vaapi: {}", vaapi::probe().map_or("unavailable".to_string(), |s| s.summary()));

    let mut dec = ffmpeg::Decoder::open(codec)?;
    dec.configure(codec, &[], 0, 0)?;
    println!("{}", dec.describe());

    let (mut frames, mut size) = (0u64, (0, 0));
    let t0 = Instant::now();
    let mut take = |dec: &mut ffmpeg::Decoder, frames: &mut u64| -> std::io::Result<()> {
        while let Some(f) = dec.pull()? {
            size = (f.width, f.height);
            if let (Some((n, file)), Pixels::Nv12 { y, uv, stride }) = (&dump, &f.pixels) {
                if *n == *frames {
                    let mut w = std::fs::File::create(file)?;
                    w.write_all(y)?;
                    w.write_all(uv)?;
                    println!("dumped frame {frames} ({}x{}, stride {stride}, pts {} us) to {file}", f.width, f.height, f.pts_us);
                }
            }
            *frames += 1;
        }
        Ok(())
    };
    for (i, au) in units.iter().enumerate() {
        if frames >= max {
            break;
        }
        dec.push(au, i as i64 * 33_333)?;
        take(&mut dec, &mut frames)?;
    }
    dec.finish()?;
    take(&mut dec, &mut frames)?;
    let secs = t0.elapsed().as_secs_f64();
    println!("{frames} frames {}x{} in {:.0} ms: {:.1} fps", size.0, size.1, secs * 1000.0, frames as f64 * 1.0 / secs);
    Ok(())
}
