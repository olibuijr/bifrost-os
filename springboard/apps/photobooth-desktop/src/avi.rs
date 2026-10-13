//! The clip container: a Motion JPEG AVI (RIFF `AVI `, one video stream of `MJPG` frames and an `idx1` index). Every
//! frame is a complete JPEG, so any player that reads Motion JPEG AVI plays the clip without a video encoder.

/// Builds the file for `frames` (each one a complete JPEG of `w` by `h` pixels) played at `fps` frames per second.
pub fn mjpeg_avi(frames: &[Vec<u8>], w: u32, h: u32, fps: u32) -> Vec<u8> {
    let fps = fps.max(1);
    let count = frames.len() as u32;
    let largest = frames.iter().map(|f| f.len()).max().unwrap_or(0) as u32;

    let mut avih = Vec::with_capacity(56);
    le32(&mut avih, 1_000_000 / fps); // dwMicroSecPerFrame
    le32(&mut avih, 0); // dwMaxBytesPerSec
    le32(&mut avih, 0); // dwPaddingGranularity
    le32(&mut avih, 0x10); // dwFlags: AVIF_HASINDEX
    le32(&mut avih, count); // dwTotalFrames
    le32(&mut avih, 0); // dwInitialFrames
    le32(&mut avih, 1); // dwStreams
    le32(&mut avih, largest); // dwSuggestedBufferSize
    le32(&mut avih, w);
    le32(&mut avih, h);
    avih.extend_from_slice(&[0; 16]); // dwReserved[4]

    let mut strh = Vec::with_capacity(56);
    strh.extend_from_slice(b"vids");
    strh.extend_from_slice(b"MJPG");
    le32(&mut strh, 0); // dwFlags
    le16(&mut strh, 0); // wPriority
    le16(&mut strh, 0); // wLanguage
    le32(&mut strh, 0); // dwInitialFrames
    le32(&mut strh, 1); // dwScale
    le32(&mut strh, fps); // dwRate
    le32(&mut strh, 0); // dwStart
    le32(&mut strh, count); // dwLength
    le32(&mut strh, largest); // dwSuggestedBufferSize
    le32(&mut strh, u32::MAX); // dwQuality
    le32(&mut strh, 0); // dwSampleSize
    le16(&mut strh, 0); // rcFrame: left
    le16(&mut strh, 0); // top
    le16(&mut strh, w as u16); // right
    le16(&mut strh, h as u16); // bottom

    let mut strf = Vec::with_capacity(40); // BITMAPINFOHEADER
    le32(&mut strf, 40);
    le32(&mut strf, w);
    le32(&mut strf, h);
    le16(&mut strf, 1); // biPlanes
    le16(&mut strf, 24); // biBitCount
    strf.extend_from_slice(b"MJPG");
    le32(&mut strf, w * h * 3); // biSizeImage
    le32(&mut strf, 0);
    le32(&mut strf, 0);
    le32(&mut strf, 0);
    le32(&mut strf, 0);

    let mut strl = Vec::new();
    chunk(&mut strl, b"strh", &strh);
    chunk(&mut strl, b"strf", &strf);
    let mut hdrl = Vec::new();
    chunk(&mut hdrl, b"avih", &avih);
    list(&mut hdrl, b"strl", &strl);

    // Index offsets count from the `movi` fourcc, so the body starts at 4.
    let mut movi = Vec::new();
    let mut index = Vec::with_capacity(frames.len() * 16);
    for frame in frames {
        let offset = 4 + movi.len() as u32;
        chunk(&mut movi, b"00dc", frame);
        index.extend_from_slice(b"00dc");
        le32(&mut index, 0x10); // AVIIF_KEYFRAME: every frame is a JPEG
        le32(&mut index, offset);
        le32(&mut index, frame.len() as u32);
    }

    let mut body = Vec::new();
    list(&mut body, b"hdrl", &hdrl);
    list(&mut body, b"movi", &movi);
    chunk(&mut body, b"idx1", &index);

    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(b"RIFF");
    le32(&mut out, 4 + body.len() as u32);
    out.extend_from_slice(b"AVI ");
    out.extend_from_slice(&body);
    out
}

fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// A RIFF chunk: id, size, data, and a pad byte when the data length is odd.
fn chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(id);
    le32(out, data.len() as u32);
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

/// A RIFF `LIST` chunk of `kind` holding `body`.
fn list(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(b"LIST");
    le32(out, 4 + body.len() as u32);
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    fn find(b: &[u8], needle: &[u8]) -> Option<usize> {
        b.windows(needle.len()).position(|w| w == needle)
    }

    #[test]
    fn riff_size_covers_the_whole_file() {
        let frames = vec![vec![0xff, 0xd8, 0xff, 0xd9], vec![1, 2, 3]];
        let avi = mjpeg_avi(&frames, 16, 8, 10);
        assert_eq!(&avi[0..4], b"RIFF");
        assert_eq!(&avi[8..12], b"AVI ");
        assert_eq!(u32_at(&avi, 4) as usize, avi.len() - 8);
    }

    #[test]
    fn index_has_one_entry_per_frame_with_the_frame_sizes() {
        let frames = vec![vec![7; 4], vec![8; 5], vec![9; 6]];
        let avi = mjpeg_avi(&frames, 16, 8, 10);
        let idx = find(&avi, b"idx1").expect("idx1 chunk");
        assert_eq!(u32_at(&avi, idx + 4) as usize, 16 * frames.len());
        // the sizes are stored in the index entries, the third word of each 16-byte record
        let first = idx + 8;
        assert_eq!(&avi[first..first + 4], b"00dc");
        assert_eq!(u32_at(&avi, first + 12), 4);
        assert_eq!(u32_at(&avi, first + 16 + 12), 5);
        assert_eq!(u32_at(&avi, first + 32 + 12), 6);
    }

    #[test]
    fn frame_offsets_point_at_their_chunks() {
        let frames = vec![vec![1; 3], vec![2; 4]];
        let avi = mjpeg_avi(&frames, 16, 8, 10);
        let movi = find(&avi, b"movi").unwrap();
        let idx = find(&avi, b"idx1").unwrap();
        for (i, frame) in frames.iter().enumerate() {
            let entry = idx + 8 + 16 * i;
            let offset = movi + u32_at(&avi, entry + 8) as usize;
            assert_eq!(&avi[offset..offset + 4], b"00dc");
            assert_eq!(u32_at(&avi, offset + 4) as usize, frame.len());
            assert_eq!(&avi[offset + 8..offset + 8 + frame.len()], &frame[..]);
        }
    }

    #[test]
    fn header_records_the_size_and_rate() {
        let avi = mjpeg_avi(&[vec![0; 2]], 640, 480, 10);
        let avih = find(&avi, b"avih").unwrap();
        assert_eq!(u32_at(&avi, avih + 8), 100_000); // microseconds per frame
        assert_eq!(u32_at(&avi, avih + 8 + 32), 640);
        assert_eq!(u32_at(&avi, avih + 8 + 36), 480);
        assert!(find(&avi, b"MJPG").is_some());
    }

    #[test]
    fn an_empty_clip_is_still_a_valid_container() {
        let avi = mjpeg_avi(&[], 16, 8, 0);
        assert_eq!(u32_at(&avi, 4) as usize, avi.len() - 8);
        assert!(find(&avi, b"idx1").is_some());
    }
}
