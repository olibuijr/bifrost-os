//! Background texture loading as a pure model (`NativeTextureLoader`, `NativeTextureInfo`). An app asks for a texture with
//! load options, the loader keeps one request per texture and its state, and the decoder reports the decoded size and alpha
//! when it finishes. The model decodes nothing, reads no file and uploads nothing: a caller decodes the image on its own
//! thread, reports the result with `complete`, and a renderer uploads the pixels later. The mip count follows
//! `texture_descriptor::full_mip_count`.

use crate::texture_descriptor::full_mip_count;
use std::collections::BTreeMap;

/// The largest side a loaded texture may have. Larger images are refused rather than uploaded.
pub const MAX_SIDE: usize = 16384;

/// How the alpha channel of a loaded texture is stored (`NativeTextureInfoAlphaState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaState {
    /// The image has no alpha channel.
    None,
    /// Colour is stored separately from alpha.
    Straight,
    /// Colour is already multiplied by alpha.
    Premultiplied,
}

/// The options an app passes when it asks for a texture (`NativeTextureLoader` options).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadOptions {
    /// Flip the rows so the first row is the bottom of the image.
    pub origin_bottom_left: bool,
    /// Store the colour in the sRGB encoding.
    pub srgb: bool,
    /// Multiply colour by alpha when the image is decoded.
    pub premultiply_alpha: bool,
    /// Build the full mip chain instead of a single level.
    pub generate_mipmaps: bool,
}

/// What a loaded texture is (`NativeTextureInfo`): its size, mip levels, alpha state and the options it was loaded with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureInfo {
    pub width: usize,
    pub height: usize,
    pub mip_levels: usize,
    pub alpha: AlphaState,
    pub origin_bottom_left: bool,
    pub srgb: bool,
}

/// Why a texture cannot be loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The decoded image has no pixels (a side is zero).
    Empty,
    /// A side is larger than `MAX_SIDE`. Carries the side given.
    TooLarge(usize),
    /// The decoder failed. Carries a short code the caller chose.
    Decode(u32),
}

/// The decoded size and alpha of an image, as the decoder reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub width: usize,
    pub height: usize,
    pub has_alpha: bool,
}

/// Builds the texture description for a decoded image.
pub fn info_for(decoded: Decoded, options: LoadOptions) -> Result<TextureInfo, LoadError> {
    if decoded.width == 0 || decoded.height == 0 {
        return Err(LoadError::Empty);
    }
    for side in [decoded.width, decoded.height] {
        if side > MAX_SIDE {
            return Err(LoadError::TooLarge(side));
        }
    }
    let alpha = match (decoded.has_alpha, options.premultiply_alpha) {
        (false, _) => AlphaState::None,
        (true, true) => AlphaState::Premultiplied,
        (true, false) => AlphaState::Straight,
    };
    let mip_levels = if options.generate_mipmaps { full_mip_count(decoded.width, decoded.height, 1) } else { 1 };
    Ok(TextureInfo {
        width: decoded.width,
        height: decoded.height,
        mip_levels,
        alpha,
        origin_bottom_left: options.origin_bottom_left,
        srgb: options.srgb,
    })
}

/// Identifies one texture request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequestId(u64);

/// Where a request stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestState {
    Pending,
    Loaded(TextureInfo),
    Failed(LoadError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Request {
    options: LoadOptions,
    state: RequestState,
}

/// The loader's table of requests. Requests stay until `take` removes them.
#[derive(Clone, Debug, Default)]
pub struct TextureLoader {
    next: u64,
    requests: BTreeMap<RequestId, Request>,
}

impl TextureLoader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a request. It is `Pending` until the decoder reports on it.
    pub fn enqueue(&mut self, options: LoadOptions) -> RequestId {
        let id = RequestId(self.next);
        self.next += 1;
        self.requests.insert(id, Request { options, state: RequestState::Pending });
        id
    }

    /// Reports a decoded image for a pending request and returns the new state. `None` when the request is unknown or
    /// already finished, so a late or duplicate report changes nothing.
    pub fn complete(&mut self, id: RequestId, decoded: Decoded) -> Option<RequestState> {
        self.finish(id, |options| match info_for(decoded, options) {
            Ok(info) => RequestState::Loaded(info),
            Err(error) => RequestState::Failed(error),
        })
    }

    /// Reports a decoder failure for a pending request.
    pub fn fail(&mut self, id: RequestId, error: LoadError) -> Option<RequestState> {
        self.finish(id, |_| RequestState::Failed(error))
    }

    fn finish(&mut self, id: RequestId, outcome: impl FnOnce(LoadOptions) -> RequestState) -> Option<RequestState> {
        let request = self.requests.get_mut(&id)?;
        if request.state != RequestState::Pending {
            return None;
        }
        request.state = outcome(request.options);
        Some(request.state)
    }

    /// The current state of a request, or `None` when it is unknown or was taken.
    pub fn state(&self, id: RequestId) -> Option<RequestState> {
        self.requests.get(&id).map(|r| r.state)
    }

    /// Removes a finished request and returns its state. A pending request stays in the table.
    pub fn take(&mut self, id: RequestId) -> Option<RequestState> {
        let state = self.requests.get(&id)?.state;
        if state == RequestState::Pending {
            return None;
        }
        self.requests.remove(&id).map(|r| r.state)
    }

    /// How many requests have not been reported on yet.
    pub fn pending_count(&self) -> usize {
        self.requests.values().filter(|r| r.state == RequestState::Pending).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(width: usize, height: usize) -> Decoded {
        Decoded { width, height, has_alpha: false }
    }

    #[test]
    fn a_plain_image_gets_one_level_and_no_alpha() {
        let info = info_for(rgb(64, 32), LoadOptions::default()).unwrap();
        assert_eq!(info.mip_levels, 1);
        assert_eq!(info.alpha, AlphaState::None);
        assert!(!info.origin_bottom_left && !info.srgb);
    }

    #[test]
    fn mipmaps_count_halvings_down_to_one_pixel() {
        let options = LoadOptions { generate_mipmaps: true, ..LoadOptions::default() };
        // 256 -> 128 -> ... -> 1 is nine halvings, so nine levels including the base.
        assert_eq!(info_for(rgb(256, 4), options).unwrap().mip_levels, 9);
        assert_eq!(info_for(rgb(1, 1), options).unwrap().mip_levels, 1);
    }

    #[test]
    fn alpha_is_premultiplied_only_when_asked() {
        let alpha = Decoded { width: 2, height: 2, has_alpha: true };
        let straight = info_for(alpha, LoadOptions::default()).unwrap();
        assert_eq!(straight.alpha, AlphaState::Straight);
        let premul = info_for(alpha, LoadOptions { premultiply_alpha: true, ..LoadOptions::default() }).unwrap();
        assert_eq!(premul.alpha, AlphaState::Premultiplied);
    }

    #[test]
    fn empty_and_oversized_images_are_refused() {
        assert_eq!(info_for(rgb(0, 8), LoadOptions::default()), Err(LoadError::Empty));
        assert_eq!(info_for(rgb(8, 0), LoadOptions::default()), Err(LoadError::Empty));
        assert_eq!(info_for(rgb(MAX_SIDE + 1, 8), LoadOptions::default()), Err(LoadError::TooLarge(MAX_SIDE + 1)));
        assert!(info_for(rgb(MAX_SIDE, 8), LoadOptions::default()).is_ok());
    }

    #[test]
    fn a_request_moves_from_pending_to_loaded() {
        let mut loader = TextureLoader::new();
        let id = loader.enqueue(LoadOptions::default());
        assert_eq!(loader.state(id), Some(RequestState::Pending));
        assert_eq!(loader.pending_count(), 1);
        let done = loader.complete(id, rgb(8, 8)).unwrap();
        assert!(matches!(done, RequestState::Loaded(TextureInfo { width: 8, height: 8, .. })));
        assert_eq!(loader.pending_count(), 0);
    }

    #[test]
    fn a_finished_request_ignores_a_second_report() {
        let mut loader = TextureLoader::new();
        let id = loader.enqueue(LoadOptions::default());
        loader.fail(id, LoadError::Decode(7)).unwrap();
        assert_eq!(loader.complete(id, rgb(8, 8)), None);
        assert_eq!(loader.state(id), Some(RequestState::Failed(LoadError::Decode(7))));
    }

    #[test]
    fn take_removes_only_finished_requests() {
        let mut loader = TextureLoader::new();
        let waiting = loader.enqueue(LoadOptions::default());
        let finished = loader.enqueue(LoadOptions::default());
        loader.complete(finished, rgb(0, 0));
        assert_eq!(loader.take(waiting), None);
        assert!(loader.take(finished).is_some());
        assert_eq!(loader.state(finished), None);
        assert_eq!(loader.state(waiting), Some(RequestState::Pending));
    }

    #[test]
    fn options_given_at_enqueue_apply_at_completion() {
        let mut loader = TextureLoader::new();
        let id = loader.enqueue(LoadOptions { origin_bottom_left: true, srgb: true, ..LoadOptions::default() });
        match loader.complete(id, rgb(4, 4)).unwrap() {
            RequestState::Loaded(info) => assert!(info.origin_bottom_left && info.srgb),
            other => panic!("expected a loaded texture, got {other:?}"),
        }
    }
}
