use super::*;
#[test]
fn review_retained_corner_damage_matches_full_repaint() {
    let (w,h)=(40,30); let color=pack(80,120,170,255);
    let mut retained=vec![color;w*h]; clip_window_corners(&mut retained,w,h,10.,None);
    let mut raw=vec![color;w*h]; let mut damage=crate::gfx::Damage::new(w,h);damage.add(30,20,10,10);
    for y in 20..30 {for x in 30..40 {raw[y*w+x]=pack(200,40,80,255);retained[y*w+x]=raw[y*w+x];}}
    clip_window_corners(&mut retained,w,h,10.,Some(&damage));
    clip_window_corners(&mut raw,w,h,10.,None);
    assert_eq!(retained,raw);
}
