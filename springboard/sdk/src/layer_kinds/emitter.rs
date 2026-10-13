//! Emitter content: particles born from cells at a rate, inside a shape, drawn as circles, squares or images. The
//! kinematics of a cell (`birth_rate`, `lifetime`, `velocity`, emission direction) reuse `bifrost_render::emitter::Cell`;
//! this module adds the appearance (scale, spin, colour and their ranges and speeds), acceleration, the emitter shape and
//! mode, the draw order and the rasteriser. The simulation is deterministic for a seed.
use super::raster::{pack_premul, sample, unpack};
use bifrost_render::emitter::Cell;
use bifrost_render::gfx::{self, Image};

/// What a particle looks like.
#[derive(Clone, Debug, PartialEq)]
pub enum CellShape {
    Circle,
    Square,
    /// A premultiplied image stretched over the particle's square and tinted by the particle colour.
    Image(Image),
}

/// The region new particles start in, centred on `EmitterContent::position`, sized by `EmitterContent::size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EmitterShape {
    #[default]
    Point,
    /// Along the x axis, `size.0` long.
    Line,
    Rectangle,
    /// A circle of diameter `min(size.0, size.1)`.
    Circle,
}

/// Where in the shape particles start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EmitterMode {
    /// At the shape's defining points: the ends of a line, the corners of a rectangle, the centre of a circle.
    Points,
    /// On the outline.
    Outline,
    /// Inside the shape.
    #[default]
    Surface,
    /// Inside the shape (the same as `Surface` in a flat image).
    Volume,
}

/// The order particles are drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RenderOrder {
    #[default]
    Unordered,
    /// The oldest particle is drawn first, so the youngest is on top.
    OldestFirst,
    /// The oldest particle is drawn last, so it is on top.
    OldestLast,
    /// Particles higher in the image are drawn first.
    BackToFront,
    /// Colours add up instead of covering each other.
    Additive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmitterCell {
    pub name: String,
    pub enabled: bool,
    /// Rate, lifetime, velocity and emission direction (radians) with their ranges.
    pub base: Cell,
    /// Added to the velocity every second, in points per second squared.
    pub acceleration: (f32, f32),
    /// Diameter multiplier, its range (full width) and its change per second.
    pub scale: f32,
    pub scale_range: f32,
    pub scale_speed: f32,
    /// Rotation in radians per second, and the full width of its range.
    pub spin: f32,
    pub spin_range: f32,
    /// Straight RGBA 0 to 1, the full width of its random range per channel, and its change per second.
    pub color: [f32; 4],
    pub color_range: [f32; 4],
    pub color_speed: [f32; 4],
    /// Diameter (side of the square) at scale 1, in points.
    pub size: f32,
    pub shape: CellShape,
}

impl Default for EmitterCell {
    fn default() -> Self {
        EmitterCell {
            name: String::new(),
            enabled: true,
            base: Cell::default(),
            acceleration: (0.0, 0.0),
            scale: 1.0,
            scale_range: 0.0,
            scale_speed: 0.0,
            spin: 0.0,
            spin_range: 0.0,
            color: [1.0; 4],
            color_range: [0.0; 4],
            color_speed: [0.0; 4],
            size: 8.0,
            shape: CellShape::Circle,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmitterParticle {
    pub pos: (f32, f32),
    pub vel: (f32, f32),
    pub age: f32,
    pub life: f32,
    pub scale: f32,
    pub spin: f32,
    pub angle: f32,
    pub color: [f32; 4],
    pub cell: usize,
    /// Birth order across the emitter.
    pub seq: u64,
}

#[derive(Clone, Debug)]
pub struct EmitterContent {
    pub position: (f32, f32),
    pub size: (f32, f32),
    pub shape: EmitterShape,
    pub mode: EmitterMode,
    pub order: RenderOrder,
    /// Multipliers over every cell.
    pub birth_rate: f32,
    pub lifetime: f32,
    pub velocity: f32,
    pub scale: f32,
    pub spin: f32,
    pub cells: Vec<EmitterCell>,
    pub particles: Vec<EmitterParticle>,
    accum: Vec<f32>,
    state: u32,
    seq: u64,
}

impl EmitterContent {
    pub fn new(cells: Vec<EmitterCell>, seed: u32) -> Self {
        let accum = vec![0.0; cells.len()];
        EmitterContent {
            position: (0.0, 0.0),
            size: (0.0, 0.0),
            shape: EmitterShape::Point,
            mode: EmitterMode::Surface,
            order: RenderOrder::Unordered,
            birth_rate: 1.0,
            lifetime: 1.0,
            velocity: 1.0,
            scale: 1.0,
            spin: 1.0,
            cells,
            particles: Vec::new(),
            accum,
            state: seed,
            seq: 0,
        }
    }

    /// A uniform value in [0, 1).
    fn unit(&mut self) -> f32 {
        self.state = self.state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.state >> 8) as f32 / 16_777_216.0
    }

    fn centred(&mut self) -> f32 {
        self.unit() - 0.5
    }

    /// Advances the simulation by `dt` seconds (a negative `dt` counts as zero).
    pub fn step(&mut self, dt: f32) {
        let dt = dt.max(0.0);
        let cells = &self.cells;
        for p in self.particles.iter_mut() {
            let c = &cells[p.cell];
            p.age += dt;
            p.vel.0 += c.acceleration.0 * dt;
            p.vel.1 += c.acceleration.1 * dt;
            p.pos.0 += p.vel.0 * dt;
            p.pos.1 += p.vel.1 * dt;
            p.angle += p.spin * dt;
            p.scale = (p.scale + c.scale_speed * dt).max(0.0);
            for k in 0..4 {
                p.color[k] = (p.color[k] + c.color_speed[k] * dt).clamp(0.0, 1.0);
            }
        }
        self.particles.retain(|p| p.age < p.life);
        if self.accum.len() != self.cells.len() {
            self.accum.resize(self.cells.len(), 0.0);
        }
        for i in 0..self.cells.len() {
            if !self.cells[i].enabled {
                continue;
            }
            self.accum[i] += self.cells[i].base.birth_rate * self.birth_rate * dt;
            let n = self.accum[i].floor();
            self.accum[i] -= n;
            for _ in 0..(n as usize) {
                self.spawn(i);
            }
        }
    }

    /// Runs the simulation for `seconds` in steps of at most `step`, to start with particles already in flight.
    pub fn warm_up(&mut self, seconds: f32, step: f32) {
        let step = step.max(1e-3);
        let seconds = seconds.max(0.0);
        // Whole steps are counted, not subtracted from `seconds`: taking 0.1 off 1.0 in f32 leaves a sliver that
        // is too small to birth the last particle in `step`.
        let whole = (seconds / step).floor() as u32;
        for _ in 0..whole {
            self.step(step);
        }
        let rest = seconds - whole as f32 * step;
        if rest > 1e-6 {
            self.step(rest);
        }
    }

    fn start_point(&mut self) -> (f32, f32) {
        let (cx, cy) = self.position;
        let (w, h) = self.size;
        match self.shape {
            EmitterShape::Point => (cx, cy),
            EmitterShape::Line => match self.mode {
                EmitterMode::Points => (cx + if self.unit() < 0.5 { -w / 2.0 } else { w / 2.0 }, cy),
                _ => (cx + self.centred() * w, cy),
            },
            EmitterShape::Rectangle => match self.mode {
                EmitterMode::Points => {
                    let sx = if self.unit() < 0.5 { -0.5 } else { 0.5 };
                    let sy = if self.unit() < 0.5 { -0.5 } else { 0.5 };
                    (cx + sx * w, cy + sy * h)
                }
                EmitterMode::Outline => {
                    // A point along the perimeter.
                    let per = 2.0 * (w + h);
                    let mut d = self.unit() * per;
                    if d < w {
                        (cx - w / 2.0 + d, cy - h / 2.0)
                    } else {
                        d -= w;
                        if d < h {
                            (cx + w / 2.0, cy - h / 2.0 + d)
                        } else {
                            d -= h;
                            if d < w {
                                (cx + w / 2.0 - d, cy + h / 2.0)
                            } else {
                                (cx - w / 2.0, cy + h / 2.0 - (d - w))
                            }
                        }
                    }
                }
                _ => (cx + self.centred() * w, cy + self.centred() * h),
            },
            EmitterShape::Circle => {
                let r = w.min(h) / 2.0;
                let a = self.unit() * std::f32::consts::TAU;
                match self.mode {
                    EmitterMode::Points => (cx, cy),
                    EmitterMode::Outline => (cx + r * a.cos(), cy + r * a.sin()),
                    _ => {
                        let d = r * self.unit().sqrt();
                        (cx + d * a.cos(), cy + d * a.sin())
                    }
                }
            }
        }
    }

    fn spawn(&mut self, i: usize) {
        let cell = self.cells[i].clone();
        let pos = self.start_point();
        let life = ((cell.base.lifetime + self.centred() * cell.base.lifetime_range) * self.lifetime).max(0.0);
        let speed = (cell.base.velocity + self.centred() * cell.base.velocity_range) * self.velocity;
        let dir = cell.base.emission_longitude + self.centred() * cell.base.emission_range;
        let scale = ((cell.scale + self.centred() * cell.scale_range) * self.scale).max(0.0);
        let spin = (cell.spin + self.centred() * cell.spin_range) * self.spin;
        let mut color = cell.color;
        for k in 0..4 {
            color[k] = (color[k] + self.centred() * cell.color_range[k]).clamp(0.0, 1.0);
        }
        self.seq += 1;
        self.particles.push(EmitterParticle { pos, vel: (speed * dir.cos(), speed * dir.sin()), age: 0.0, life, scale, spin, angle: 0.0, color, cell: i, seq: self.seq });
    }

    /// The particles in drawing order.
    fn ordered(&self) -> Vec<&EmitterParticle> {
        let mut v: Vec<&EmitterParticle> = self.particles.iter().collect();
        match self.order {
            RenderOrder::OldestFirst | RenderOrder::Additive => v.sort_by_key(|p| p.seq),
            RenderOrder::OldestLast => v.sort_by_key(|p| std::cmp::Reverse(p.seq)),
            RenderOrder::BackToFront => v.sort_by(|a, b| a.pos.1.partial_cmp(&b.pos.1).unwrap_or(std::cmp::Ordering::Equal)),
            RenderOrder::Unordered => {}
        }
        v
    }

    /// Draws every live particle into a `w` x `h` image. `scale` is device pixels per point.
    pub fn render(&self, w: usize, h: usize, scale: f32) -> Image {
        let mut img = Image::new(w, h);
        let additive = self.order == RenderOrder::Additive;
        for p in self.ordered() {
            let cell = &self.cells[p.cell];
            let half = cell.size * p.scale * scale / 2.0;
            draw_particle(&mut img, (p.pos.0 * scale, p.pos.1 * scale), half, p.angle, p.color, &cell.shape, additive);
        }
        img
    }
}

fn draw_particle(img: &mut Image, c: (f32, f32), half: f32, angle: f32, color: [f32; 4], shape: &CellShape, additive: bool) {
    if half <= 0.0 || color[3] <= 0.0 {
        return;
    }
    // A rotated square's reach is the half diagonal.
    let reach = half * std::f32::consts::SQRT_2 + 1.0;
    let (x0, x1) = (((c.0 - reach).floor().max(0.0)) as i32, ((c.0 + reach).ceil().min(img.w as f32)) as i32);
    let (y0, y1) = (((c.1 - reach).floor().max(0.0)) as i32, ((c.1 + reach).ceil().min(img.h as f32)) as i32);
    let (sin, cos) = angle.sin_cos();
    for y in y0..y1 {
        for x in x0..x1 {
            let (dx, dy) = (x as f32 + 0.5 - c.0, y as f32 + 0.5 - c.1);
            // Into the particle's own axes.
            let (lx, ly) = (dx * cos + dy * sin, -dx * sin + dy * cos);
            let src: [f32; 4] = match shape {
                CellShape::Circle => {
                    let cov = (half - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
                    [color[0] * color[3] * cov, color[1] * color[3] * cov, color[2] * color[3] * cov, color[3] * cov]
                }
                CellShape::Square => {
                    let cov = (half - lx.abs() + 0.5).clamp(0.0, 1.0) * (half - ly.abs() + 0.5).clamp(0.0, 1.0);
                    [color[0] * color[3] * cov, color[1] * color[3] * cov, color[2] * color[3] * cov, color[3] * cov]
                }
                CellShape::Image(im) => {
                    if lx.abs() > half || ly.abs() > half {
                        continue;
                    }
                    let (iw, ih) = (im.w as f32, im.h as f32);
                    let s = sample(im, ((lx / half + 1.0) * 0.5 * iw).clamp(0.5, iw - 0.5), ((ly / half + 1.0) * 0.5 * ih).clamp(0.5, ih - 0.5));
                    [s[0] * color[0] * color[3], s[1] * color[1] * color[3], s[2] * color[2] * color[3], s[3] * color[3]]
                }
            };
            if src[3] <= 0.0 {
                continue;
            }
            let dst = &mut img.px[y as usize * img.w + x as usize];
            if additive {
                let d = unpack(*dst);
                *dst = pack_premul([d[0] + src[0], d[1] + src[1], d[2] + src[2], d[3] + src[3]]);
            } else {
                *dst = gfx::over(*dst, pack_premul(src));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_render::gfx::rgba;

    fn cell(rate: f32) -> EmitterCell {
        EmitterCell { base: Cell { birth_rate: rate, lifetime: 2.0, ..Default::default() }, ..Default::default() }
    }

    #[test]
    fn particles_are_born_at_the_rate_and_die_after_their_lifetime() {
        let mut e = EmitterContent::new(vec![cell(10.0)], 7);
        e.step(1.0);
        assert_eq!(e.particles.len(), 10);
        e.step(2.0);
        assert_eq!(e.particles.len(), 20, "the first ten reached their 2 s lifetime; twenty new ones were born");
        assert!(e.particles.iter().all(|p| p.age == 0.0));
    }

    #[test]
    fn disabled_cells_and_the_birth_multiplier_control_the_count() {
        let mut off = cell(10.0);
        off.enabled = false;
        let mut e = EmitterContent::new(vec![off], 1);
        e.step(1.0);
        assert!(e.particles.is_empty());
        let mut e = EmitterContent::new(vec![cell(10.0)], 1);
        e.birth_rate = 0.5;
        e.step(1.0);
        assert_eq!(e.particles.len(), 5);
    }

    #[test]
    fn velocity_direction_and_acceleration_move_particles() {
        let mut c = cell(1.0);
        c.base.velocity = 10.0;
        c.base.emission_longitude = 0.0;
        c.acceleration = (0.0, 20.0);
        let mut e = EmitterContent::new(vec![c], 3);
        e.step(1.0);
        assert_eq!(e.particles.len(), 1);
        e.step(1.0);
        let p = &e.particles[0];
        assert!((p.pos.0 - 10.0).abs() < 1e-3);
        assert!((p.pos.1 - 20.0).abs() < 1e-3, "accelerated down by 20 over the second step");
        assert!((p.vel.1 - 20.0).abs() < 1e-3);
    }

    #[test]
    fn scale_colour_and_spin_change_over_time() {
        let mut c = cell(1.0);
        c.scale = 1.0;
        c.scale_speed = -0.25;
        c.color_speed = [0.0, 0.0, 0.0, -0.5];
        c.spin = 2.0;
        let mut e = EmitterContent::new(vec![c], 3);
        e.step(1.0);
        e.step(1.0);
        let p = &e.particles[0];
        assert!((p.scale - 0.75).abs() < 1e-4);
        assert!((p.color[3] - 0.5).abs() < 1e-4);
        assert!((p.angle - 2.0).abs() < 1e-4);
    }

    #[test]
    fn ranges_vary_the_start_values_around_the_centre() {
        let mut c = cell(100.0);
        c.base.velocity = 10.0;
        c.base.velocity_range = 4.0;
        c.scale_range = 1.0;
        let mut e = EmitterContent::new(vec![c], 11);
        e.step(1.0);
        let speeds: Vec<f32> = e.particles.iter().map(|p| (p.vel.0 * p.vel.0 + p.vel.1 * p.vel.1).sqrt()).collect();
        assert!(speeds.iter().all(|&s| (8.0..=12.0).contains(&s)));
        assert!(speeds.iter().any(|&s| s < 9.0) && speeds.iter().any(|&s| s > 11.0));
        assert!(e.particles.iter().all(|p| (0.5..=1.5).contains(&p.scale)));
    }

    #[test]
    fn one_seed_gives_the_same_particles() {
        let run = |seed| {
            let mut c = cell(30.0);
            c.base.velocity = 5.0;
            c.base.velocity_range = 5.0;
            c.base.emission_range = 3.0;
            let mut e = EmitterContent::new(vec![c], seed);
            e.shape = EmitterShape::Rectangle;
            e.size = (10.0, 10.0);
            e.step(0.5);
            e.particles.iter().map(|p| p.pos).collect::<Vec<_>>()
        };
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(6));
    }

    #[test]
    fn the_shape_and_mode_choose_where_particles_start() {
        let make = |shape, mode| {
            let mut e = EmitterContent::new(vec![cell(200.0)], 9);
            e.position = (50.0, 50.0);
            e.size = (20.0, 10.0);
            e.shape = shape;
            e.mode = mode;
            e.step(1.0);
            e.particles.iter().map(|p| p.pos).collect::<Vec<_>>()
        };
        let line = make(EmitterShape::Line, EmitterMode::Surface);
        assert!(line.iter().all(|p| p.1 == 50.0 && (40.0..=60.0).contains(&p.0)));
        let ends = make(EmitterShape::Line, EmitterMode::Points);
        assert!(ends.iter().all(|p| p.0 == 40.0 || p.0 == 60.0));
        let outline = make(EmitterShape::Rectangle, EmitterMode::Outline);
        assert!(outline.iter().all(|p| (p.0 - 40.0).abs() < 1e-3 || (p.0 - 60.0).abs() < 1e-3 || (p.1 - 45.0).abs() < 1e-3 || (p.1 - 55.0).abs() < 1e-3));
        let inside = make(EmitterShape::Rectangle, EmitterMode::Volume);
        assert!(inside.iter().all(|p| (40.0..=60.0).contains(&p.0) && (45.0..=55.0).contains(&p.1)));
        let ring = make(EmitterShape::Circle, EmitterMode::Outline);
        assert!(ring.iter().all(|p| (((p.0 - 50.0).powi(2) + (p.1 - 50.0).powi(2)).sqrt() - 5.0).abs() < 1e-3), "diameter is the smaller side");
    }

    fn still(order: RenderOrder, colors: [[f32; 4]; 2]) -> EmitterContent {
        let mut e = EmitterContent::new(vec![EmitterCell { size: 8.0, shape: CellShape::Square, ..Default::default() }], 1);
        e.order = order;
        for (i, c) in colors.iter().enumerate() {
            e.particles.push(EmitterParticle { pos: (8.0, 8.0), vel: (0.0, 0.0), age: 0.0, life: 9.0, scale: 1.0, spin: 0.0, angle: 0.0, color: *c, cell: 0, seq: i as u64 + 1 });
        }
        e
    }

    #[test]
    fn a_particle_draws_around_its_position() {
        let e = still(RenderOrder::Unordered, [[1.0, 0.0, 0.0, 1.0], [0.0; 4]]);
        let img = e.render(16, 16, 1.0);
        assert_eq!(img.px[8 * 16 + 8], rgba(255, 0, 0, 255));
        assert_eq!(img.px[0], 0);
        assert_eq!(img.px[8 * 16 + 13], 0, "outside the 8 point square");
    }

    #[test]
    fn a_circle_cell_leaves_the_corners_of_its_box_empty() {
        let mut e = still(RenderOrder::Unordered, [[1.0, 1.0, 1.0, 1.0], [0.0; 4]]);
        e.cells[0].shape = CellShape::Circle;
        let img = e.render(16, 16, 1.0);
        assert_ne!(img.px[8 * 16 + 8], 0);
        assert_eq!(img.px[4 * 16 + 4], 0);
    }

    #[test]
    fn draw_order_decides_which_particle_is_on_top() {
        let red = [1.0, 0.0, 0.0, 1.0];
        let blue = [0.0, 0.0, 1.0, 1.0];
        let first = still(RenderOrder::OldestFirst, [red, blue]).render(16, 16, 1.0);
        let last = still(RenderOrder::OldestLast, [red, blue]).render(16, 16, 1.0);
        assert_eq!(first.px[8 * 16 + 8], rgba(0, 0, 255, 255), "the youngest is on top");
        assert_eq!(last.px[8 * 16 + 8], rgba(255, 0, 0, 255), "the oldest is on top");
    }

    #[test]
    fn additive_order_adds_colours() {
        let e = still(RenderOrder::Additive, [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]]);
        assert_eq!(e.render(16, 16, 1.0).px[8 * 16 + 8], rgba(255, 255, 0, 255));
    }

    #[test]
    fn an_image_cell_stretches_and_tints_its_image() {
        let mut e = still(RenderOrder::Unordered, [[1.0, 0.0, 0.0, 1.0], [0.0; 4]]);
        e.cells[0].shape = CellShape::Image(Image { w: 1, h: 1, px: vec![rgba(255, 255, 255, 255)] });
        assert_eq!(e.render(16, 16, 1.0).px[8 * 16 + 8], rgba(255, 0, 0, 255));
    }

    #[test]
    fn warm_up_advances_the_simulation() {
        let mut e = EmitterContent::new(vec![cell(10.0)], 1);
        e.warm_up(1.0, 0.1);
        assert_eq!(e.particles.len(), 10);
    }
}
