//! Self-contained Photos Favorites preview; touches no existing photo library.
//! cargo run --example photos_favorites_preview
use springboard::font::Font;
use springboard::gfx::{pack, Surface};
use springboard::pngenc::encode_rgb;
use springboard::shell::photos::{Album, Photos};
use springboard::shell::ui::Ui;
use springboard::shell::Event;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const W: usize = 1200;
const H: usize = 1920;

fn tap(photos: &mut Photos, ui: &Ui, x: f32, y: f32, t: f64) {
    photos.touch(ui, Event::Down(x, y), t);
    photos.touch(ui, Event::Up(x, y), t + 30.0);
}

fn settle(photos: &mut Photos, ui: &Ui) {
    photos.tick(ui, 0.0);
    for _ in 0..10 {
        springboard::bare::sys::wait(&photos.fds(), 50);
        photos.pump(ui);
    }
}

fn save(photos: &mut Photos, ui: &Ui, path: &Path) {
    let mut pixels = vec![0; W * H];
    photos.draw(
        &mut Surface {
            w: W,
            h: H,
            stride: W,
            px: &mut pixels,
            y0: 0,
        },
        ui,
        0.0,
    );
    std::fs::write(path, encode_rgb(W, H, &pixels)).unwrap();
    eprintln!("{}", path.display());
}

fn picture(path: &Path, index: usize) {
    let (w, h) = (320, 240);
    let palette = [
        (70, 124, 172),
        (165, 103, 96),
        (76, 137, 121),
        (124, 97, 157),
        (61, 125, 148),
        (179, 124, 65),
    ];
    let (red, green, blue) = palette[index];
    let mut pixels = vec![0; w * h];
    for y in 0..h {
        for x in 0..w {
            let light = 1.0 - y as f32 / h as f32 * 0.45;
            pixels[y * w + x] = pack(
                (red as f32 * light) as u8,
                (green as f32 * light) as u8,
                (blue as f32 * light) as u8,
                255,
            );
        }
    }
    let mut surface = Surface {
        w,
        h,
        stride: w,
        px: &mut pixels,
        y0: 0,
    };
    surface.fill_rrect(
        210.0 - index as f32 * 12.0,
        30.0,
        45.0,
        45.0,
        23.0,
        pack(246, 214, 153, 255),
    );
    surface.fill_rrect(
        -50.0,
        110.0 + index as f32 * 3.0,
        260.0,
        260.0,
        130.0,
        pack(33, 63, 69, 255),
    );
    surface.fill_rrect(130.0, 140.0, 290.0, 290.0, 145.0, pack(25, 48, 61, 255));
    std::fs::write(path, encode_rgb(w, h, &pixels)).unwrap();
    let date = 1_790_000_000 + index as u64 * 4 * 86_400;
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(date))
        .unwrap();
}

fn main() {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = Path::new("target").join(format!("photos-favorites-preview-{id}"));
    let pictures = root.join("Pictures");
    std::fs::create_dir_all(&pictures).unwrap();
    for i in 0..6 {
        picture(&pictures.join(format!("Photo-{}.png", i + 1)), i);
    }
    let font = Box::leak(std::fs::read("assets/SFUI.ttf").unwrap().into_boxed_slice());
    let ui = Ui::new(Font::parse(font).unwrap(), W as f32 / 768.0);
    let mut photos = Photos::new(
        &ui,
        W as i32,
        H as i32,
        root.join("data"),
        vec![pictures],
        0,
    )
    .unwrap();
    settle(&mut photos, &ui);
    let paths: Vec<_> = photos
        .photos()
        .iter()
        .map(|photo| photo.path.clone())
        .collect();
    let favorites = [0, 2, 3, 5];
    for index in favorites {
        assert!(photos.show(&ui, &paths[index]));
        tap(
            &mut photos,
            &ui,
            W as f32 / 2.0,
            H as f32 - ui.pt(53.0),
            1000.0 + index as f64 * 100.0,
        );
    }
    photos.select_album(&ui, Album::Favorites);
    settle(&mut photos, &ui);
    save(&mut photos, &ui, &root.join("favorites.png"));
    tap(&mut photos, &ui, ui.pt(40.0), ui.pt(42.0), 3000.0);
    save(&mut photos, &ui, &root.join("albums.png"));
    for index in favorites {
        assert!(photos.show(&ui, &paths[index]));
        tap(
            &mut photos,
            &ui,
            W as f32 / 2.0,
            H as f32 - ui.pt(53.0),
            4000.0 + index as f64 * 100.0,
        );
    }
    photos.select_album(&ui, Album::Favorites);
    save(&mut photos, &ui, &root.join("favorites-empty.png"));
}
