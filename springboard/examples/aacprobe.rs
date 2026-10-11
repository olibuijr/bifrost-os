//! Decodes the first audio packets of an MP4 with the AAC decoder and prints every error.
//! Run with `cargo run --release --example aacprobe -- file.mp4`.
use springboard::media::mp4::{Codec, Demuxer, Kind};
use springboard::media::source::MemSource;
use symphonia_core::codecs::audio::well_known::CODEC_ID_AAC;
use symphonia_core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia_core::packet::PacketRef;
use symphonia_core::units::{Duration, Timestamp};

fn main() {
    let path = std::env::args().nth(1).expect("file.mp4");
    let mut src = MemSource::new(std::fs::read(path).unwrap());
    let mut d = Demuxer::open(&mut src).unwrap();
    let Codec::Aac { asc, .. } = d.audio().unwrap().codec.clone() else { panic!("not AAC") };
    println!("asc {asc:02x?}");
    let mut params = AudioCodecParameters::new();
    params.for_codec(CODEC_ID_AAC).with_extra_data(asc.clone().into());
    let mut dec = symphonia_codec_aac::AacDecoder::try_new(&params, &AudioDecoderOptions::default()).expect("try_new");
    let (mut ok, mut err) = (0, 0);
    let mut first_err = None;
    let mut n = 0;
    while let Ok(Some(s)) = d.next(Kind::Audio, &mut src) {
        let p = PacketRef::new(0, Timestamp::new(0), Duration::new(1024), &s.data);
        match dec.decode_ref(&p) {
            Ok(buf) => {
                ok += 1;
                if ok == 1 {
                    println!("first ok: {} channels, rate {}, {} frames", buf.spec().channels().count(), buf.spec().rate(), buf.frames());
                }
            }
            Err(e) => {
                err += 1;
                if first_err.is_none() {
                    first_err = Some(format!("{e} (packet {n}, {} bytes, head {:02x?})", s.data.len(), &s.data[..s.data.len().min(12)]));
                }
            }
        }
        n += 1;
        if n >= 400 {
            break;
        }
    }
    println!("{n} packets: {ok} decoded, {err} errors; first error: {first_err:?}");
}
