//! The video test card: ten seconds of side-scrolling pixel-art gameplay at the 256x224 of a
//! 16-bit console, drawn here one frame at a time, to see a look on moving pixel art.
//!
//! Everything in it is drawn in this module and is original: the level, the round robot that
//! runs through it, the gems, the font. It copies, traces and imitates no game.
//!
//! What it shows is chosen for what a CRT does to it: a sky of dithered bands, three layers
//! scrolling at different speeds, hard-edged bricks and 1-pixel details, saturated reds and
//! blues, a sprite moving against a fast-scrolling background, and small text in the HUD.
//!
//! The camera crosses the level once per loop, and everything else repeats within a loop, so
//! the last frame runs into the first. Only the HUD's counters start over, as a game's demo does.
//! Drawing uses integers only, so a frame is the same on every platform.
use image::{Rgba, RgbaImage};

/// The picture's size.
pub const SIZE: (u32, u32) = (256, 224);
/// Frames per second.
pub const FPS: u32 = 60;
/// Frames in one loop: ten seconds. Frame `FRAMES` is frame 0 again.
pub const FRAMES: u64 = 600;

/// Frame `index`, counting from 0; any index, as the clip repeats.
pub fn frame(index: u64) -> RgbaImage {
    let frame = (index % FRAMES) as i32;
    let mut canvas = Canvas(RgbaImage::new(SIZE.0, SIZE.1));
    let collected = scene(&mut canvas, frame);
    hud(&mut canvas, collected, frame);
    canvas.0
}

/// Draws everything but the HUD at `frame`, and returns how many gems have been collected.
fn scene(canvas: &mut Canvas, frame: i32) -> i32 {
    let scroll = frame * SPEED;
    sky(canvas);
    far_hills(canvas, scroll / 4);
    clouds(canvas, scroll * 3 / 8);
    near_hills(canvas, scroll / 2);
    ground(canvas, scroll);
    platforms(canvas, scroll);
    // The runner's place in the level, which runs past its end towards the loop's end.
    let runner = scroll + RUNNER_X;
    let collected = gems(canvas, scroll, runner, frame);
    robot(canvas, runner, frame);
    collected
}

type Rgb = [u8; 3];

const WIDTH: i32 = SIZE.0 as i32;
const HEIGHT: i32 = SIZE.1 as i32;
/// How far the camera moves each frame, in pixels.
const SPEED: i32 = 2;
/// The level's width, which the camera crosses once per loop.
const LEVEL: i32 = SPEED * FRAMES as i32;
/// The HUD strip's height.
const HUD: i32 = 26;
/// Where the ground's surface is.
const GROUND: i32 = 176;
const TILE: i32 = 16;
/// Where the runner's middle is on screen.
const RUNNER_X: i32 = 72;

/// The picture being drawn. Drawing outside it is ignored.
struct Canvas(RgbaImage);

impl Canvas {
    fn put(&mut self, x: i32, y: i32, color: Rgb) {
        if (0..WIDTH).contains(&x) && (0..HEIGHT).contains(&y) {
            let [r, g, b] = color;
            self.0.put_pixel(x as u32, y as u32, Rgba([r, g, b, 255]));
        }
    }
}

/// A 4x4 ordered dither: a pixel shows the next colour when its entry is below the level, of 16.
const BAYER: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

fn dithered(x: i32, y: i32, level: i32) -> bool {
    BAYER[y.rem_euclid(4) as usize][x.rem_euclid(4) as usize] < level
}

/// Bands of sky from the top down, each dithering into the next over its last eight rows.
const SKY: [Rgb; 6] = [
    [80, 128, 224],
    [104, 156, 236],
    [128, 180, 244],
    [152, 200, 248],
    [176, 220, 252],
    [204, 236, 255],
];
const SKY_BAND: i32 = 27;

fn sky(canvas: &mut Canvas) {
    for y in HUD..HEIGHT {
        let row = y - HUD;
        let band = (row / SKY_BAND).min(SKY.len() as i32 - 1);
        let into = row - band * SKY_BAND - (SKY_BAND - 8);
        for x in 0..WIDTH {
            let next = band + 1 < SKY.len() as i32 && into >= 0 && dithered(x, y, (into + 1) * 2);
            canvas.put(x, y, SKY[(band + next as i32) as usize]);
        }
    }
}

/// A rounded hill: its middle and half its width across its layer, and how tall it is.
struct Hill {
    middle: i32,
    half_width: i32,
    height: i32,
}

/// How tall a layer of `hills`, repeating every `period`, stands at `x`.
fn hill_height(hills: &[Hill], period: i32, x: i32) -> i32 {
    hills
        .iter()
        .map(|hill| {
            // The nearest repeat of the hill.
            let dx = (x - hill.middle + period / 2).rem_euclid(period) - period / 2;
            let (w, h) = (hill.half_width, hill.height);
            if dx.abs() >= w {
                return 0;
            }
            // A half-ellipse: h * sqrt(1 - (dx / w)^2), in integers.
            isqrt(h * h * (w * w - dx * dx) / (w * w))
        })
        .max()
        .unwrap_or(0)
}

fn isqrt(n: i32) -> i32 {
    let mut root = 0;
    while (root + 1) * (root + 1) <= n {
        root += 1;
    }
    root
}

const FAR_PERIOD: i32 = LEVEL / 4;
const FAR_HILLS: [Hill; 3] = [
    Hill {
        middle: 40,
        half_width: 72,
        height: 60,
    },
    Hill {
        middle: 150,
        half_width: 52,
        height: 40,
    },
    Hill {
        middle: 228,
        half_width: 66,
        height: 72,
    },
];

/// Distant blue hills, a quarter as fast as the ground, with a dithered rim of light. They
/// reach the bottom of the picture, which shows through the pits.
fn far_hills(canvas: &mut Canvas, offset: i32) {
    let base = 170;
    for x in 0..WIDTH {
        let top = base - hill_height(&FAR_HILLS, FAR_PERIOD, x + offset);
        for y in top..HEIGHT {
            let depth = y - top;
            let color = if depth == 0 || depth < 4 && dithered(x + offset, y, 8 - depth * 2) {
                [196, 220, 248]
            } else {
                [136, 172, 224]
            };
            canvas.put(x, y, color);
        }
    }
}

const CLOUD_PERIOD: i32 = LEVEL * 3 / 8;
/// Where each cloud is across its layer and how high, and whether it is the small kind.
const CLOUDS: [(i32, i32, bool); 3] = [(40, 52, false), (210, 78, true), (330, 42, false)];
/// A cloud's puffs, as circles around its middle, and the flat bottom they are cut off at.
const PUFFS: [(i32, i32, i32); 4] = [(0, 0, 11), (-13, 4, 8), (13, 3, 9), (25, 6, 6)];
const CLOUD_BOTTOM: i32 = 9;

fn in_cloud(dx: i32, dy: i32, small: bool) -> bool {
    // A small cloud is the same shape at two thirds of the size.
    let (dx, dy) = if small {
        (dx * 3 / 2, dy * 3 / 2)
    } else {
        (dx, dy)
    };
    dy <= CLOUD_BOTTOM
        && PUFFS
            .iter()
            .any(|&(x, y, r)| (dx - x) * (dx - x) + (dy - y) * (dy - y) <= r * r)
}

/// White clouds with a pale underside and a blue outline, three eighths as fast as the ground.
fn clouds(canvas: &mut Canvas, offset: i32) {
    for &(middle, height, small) in &CLOUDS {
        let left = (middle - offset).rem_euclid(CLOUD_PERIOD);
        for x0 in [left, left - CLOUD_PERIOD] {
            for dy in -12..=CLOUD_BOTTOM {
                for dx in -22..=32 {
                    if !in_cloud(dx, dy, small) {
                        continue;
                    }
                    let edge = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|&(ex, ey)| !in_cloud(dx + ex, dy + ey, small));
                    let color = if edge {
                        [136, 176, 236]
                    } else if dy >= 5 || dy >= 3 && dithered(dx, dy, 8) {
                        [212, 228, 252]
                    } else {
                        [255, 255, 255]
                    };
                    canvas.put(x0 + dx, height + dy, color);
                }
            }
        }
    }
}

const NEAR_PERIOD: i32 = LEVEL / 2;
const NEAR_HILLS: [Hill; 5] = [
    Hill {
        middle: 60,
        half_width: 58,
        height: 46,
    },
    Hill {
        middle: 172,
        half_width: 42,
        height: 30,
    },
    Hill {
        middle: 300,
        half_width: 76,
        height: 58,
    },
    Hill {
        middle: 424,
        half_width: 48,
        height: 36,
    },
    Hill {
        middle: 526,
        half_width: 60,
        height: 50,
    },
];

/// Terraced green hills half as fast as the ground, outlined and lit from the left, their
/// terraces dithered into each other. They reach the bottom of the picture, which shows
/// through the pits.
fn near_hills(canvas: &mut Canvas, offset: i32) {
    let base = 186;
    for x in 0..WIDTH {
        let across = x + offset;
        let height = hill_height(&NEAR_HILLS, NEAR_PERIOD, across);
        if height == 0 {
            continue;
        }
        let top = base - height;
        // Which way the slope faces: rising to the right is lit.
        let lit = hill_height(&NEAR_HILLS, NEAR_PERIOD, across + 3) > height;
        for y in top..HEIGHT {
            let depth = y - top;
            let terrace = y / 6 % 2 == 1;
            let blended = y % 6 == 0 && dithered(across, y, 8);
            let color = if depth == 0 {
                [32, 104, 56]
            } else if depth < 3 && lit {
                [168, 240, 128]
            } else if terrace != blended {
                [72, 176, 80]
            } else {
                [104, 204, 96]
            };
            canvas.put(x, y, color);
        }
    }
}

/// The pits in the ground, as ranges of tiles across the level.
const PITS: [(i32, i32); 2] = [(21, 23), (52, 55)];

fn pit(tile: i32) -> bool {
    let tile = tile.rem_euclid(LEVEL / TILE);
    PITS.iter().any(|&(from, to)| (from..to).contains(&tile))
}

/// The ground at the camera's speed: a row of grass over earth, and three of bricks, with
/// flowers here and there.
fn ground(canvas: &mut Canvas, scroll: i32) {
    for tile in scroll / TILE..=(scroll + WIDTH) / TILE {
        if pit(tile) {
            continue;
        }
        let left = tile * TILE - scroll;
        // Which tile of the level it is, as the camera runs past the level's end.
        let tile = tile.rem_euclid(LEVEL / TILE);
        let (open_left, open_right) = (pit(tile - 1), pit(tile + 1));
        for tx in 0..TILE {
            let edge = tx == 0 && open_left || tx == TILE - 1 && open_right;
            for y in GROUND..HEIGHT {
                let ty = y - GROUND;
                let color = if edge && ty > 0 {
                    [64, 32, 24]
                } else if ty < TILE {
                    grass(tx, ty, tile)
                } else {
                    brick(tile * TILE + tx, ty - TILE)
                };
                canvas.put(left + tx, y, color);
            }
        }
        let seed = tile * 37 + 11;
        if seed % 5 == 0 && !open_left && !open_right {
            let petal = if seed % 2 == 0 {
                [248, 56, 80]
            } else {
                [255, 216, 64]
            };
            flower(canvas, left + 4 + seed % 7, petal);
        }
    }
}

fn grass(tx: i32, ty: i32, tile: i32) -> Rgb {
    let pebble = (tx * 3 + ty * 5 + tile * 7).rem_euclid(23);
    match ty {
        0 => [184, 244, 112],
        1..=3 if ty == 3 && (tx + tile).rem_euclid(4) == 0 => [48, 144, 48],
        1..=3 => [88, 196, 64],
        // Grass hanging over the earth.
        4 if (tx * 7 + tile * 3).rem_euclid(5) < 2 => [48, 144, 48],
        4 | 5 => [152, 96, 56],
        _ if pebble == 0 => [244, 200, 140],
        _ if pebble == 1 => [156, 100, 60],
        _ => [216, 156, 96],
    }
}

/// Bricks eight rows tall and a tile wide, each course offset by half a brick.
fn brick(x: i32, y: i32) -> Rgb {
    let (course, row) = (y / 8, y % 8);
    let across = (x + course % 2 * 8).rem_euclid(TILE);
    if row == 0 || across == 0 {
        [88, 32, 40]
    } else if row == 1 || across == 1 {
        [244, 140, 100]
    } else if row == 7 || across == TILE - 1 {
        [152, 44, 40]
    } else {
        [212, 76, 56]
    }
}

fn flower(canvas: &mut Canvas, x: i32, petal: Rgb) {
    for y in GROUND - 3..GROUND {
        canvas.put(x, y, [40, 128, 48]);
    }
    canvas.put(x + 1, GROUND - 2, [40, 128, 48]);
    for (dx, dy) in [(0, -6), (-1, -5), (1, -5), (0, -4)] {
        canvas.put(x + dx, GROUND + dy, petal);
    }
    canvas.put(x, GROUND - 5, [255, 255, 255]);
}

/// Floating platforms of blue metal blocks: the first tile, how many, and the top's height.
const PLATFORMS: [(i32, i32, i32); 3] = [(30, 4, 128), (36, 4, 96), (63, 3, 120)];

fn platforms(canvas: &mut Canvas, scroll: i32) {
    for &(tile, tiles, top) in &PLATFORMS {
        for repeat in [0, LEVEL] {
            for block in 0..tiles {
                let left = (tile + block) * TILE + repeat - scroll;
                if (-TILE..WIDTH).contains(&left) {
                    metal_block(canvas, left, top);
                }
            }
        }
    }
}

fn metal_block(canvas: &mut Canvas, left: i32, top: i32) {
    let rivet = |x: i32, y: i32| [3, 12].contains(&x) && [3, 12].contains(&y);
    let shadow = |x: i32, y: i32| rivet(x - 1, y - 1);
    for ty in 0..TILE {
        for tx in 0..TILE {
            let color = if ty == 0 || tx == 0 {
                [152, 192, 255]
            } else if ty == TILE - 1 || tx == TILE - 1 {
                [24, 36, 120]
            } else if rivet(tx, ty) {
                [224, 236, 255]
            } else if shadow(tx, ty) || ty == TILE - 2 || tx == TILE - 2 {
                [40, 68, 176]
            } else {
                [64, 108, 236]
            };
            canvas.put(left + tx, top + ty, color);
        }
    }
}

/// A jump from `from` to `to` across the level, from feet at `start` to feet at `land`,
/// arcing up to `height` above the straight line between them.
struct Jump {
    from: i32,
    to: i32,
    start: i32,
    land: i32,
    height: i32,
}

/// The runner's jumps, in order. It starts and ends the level on the ground.
const JUMPS: [Jump; 7] = [
    // Over the first pit.
    Jump {
        from: 300,
        to: 404,
        start: GROUND,
        land: GROUND,
        height: 44,
    },
    // Up onto the platforms, and down again.
    Jump {
        from: 436,
        to: 500,
        start: GROUND,
        land: 128,
        height: 28,
    },
    Jump {
        from: 528,
        to: 596,
        start: 128,
        land: 96,
        height: 24,
    },
    Jump {
        from: 632,
        to: 728,
        start: 96,
        land: GROUND,
        height: 20,
    },
    // Over the second pit.
    Jump {
        from: 792,
        to: 920,
        start: GROUND,
        land: GROUND,
        height: 52,
    },
    // Onto the last platform and off it.
    Jump {
        from: 960,
        to: 1016,
        start: GROUND,
        land: 120,
        height: 24,
    },
    Jump {
        from: 1052,
        to: 1124,
        start: 120,
        land: GROUND,
        height: 16,
    },
];

/// Where the runner's feet are at `x` across the level, and whether it is in the air.
fn feet(x: i32) -> (i32, bool) {
    let x = x.rem_euclid(LEVEL);
    let mut standing = GROUND;
    for jump in &JUMPS {
        if x < jump.from {
            break;
        }
        if x < jump.to {
            // A parabola from start to land: t = n / d of the way along.
            let (n, d) = (x - jump.from, jump.to - jump.from);
            let line = jump.start + (jump.land - jump.start) * n / d;
            return (line - 4 * jump.height * n * (d - n) / (d * d), true);
        }
        standing = jump.land;
    }
    (standing, false)
}

/// The gems, by their middle across the level and their height. Most are on the runner's
/// way and are collected; the two under the high platform are not.
const GEMS: [(i32, i32); 15] = [
    (168, 164),
    (188, 164),
    (208, 164),
    (352, 118),
    (504, 116),
    (522, 116),
    (612, 84),
    (600, 164),
    (620, 164),
    (760, 164),
    (780, 164),
    (856, 114),
    (1032, 108),
    (1150, 164),
    (1170, 164),
];

/// Whether the runner passes through a gem at `x` and `y`.
fn on_the_way(x: i32, y: i32) -> bool {
    let (feet, _) = feet(x);
    (feet - ROBOT_HEIGHT - 4..=feet + 2).contains(&y)
}

/// How far ahead of its middle the runner reaches a gem.
const REACH: i32 = 6;

/// Draws the gems in view, spinning, and a sparkle where one was just collected. Returns how
/// many the runner has collected since the loop began.
fn gems(canvas: &mut Canvas, scroll: i32, runner: i32, frame: i32) -> i32 {
    let reached = runner + REACH;
    let mut collected = 0;
    for (index, &(x, y)) in GEMS.iter().enumerate() {
        let way = on_the_way(x, y);
        // Gems near the level's start come round again before the loop ends.
        for at in [x, x + LEVEL] {
            let taken = way && reached >= at;
            // Those already behind the runner as the loop begins were collected in the last.
            if taken && at > RUNNER_X + REACH {
                collected += 1;
            }
            let screen = at - scroll;
            if !(-8..WIDTH + 8).contains(&screen) {
                continue;
            }
            if !taken {
                let phase = (frame / 3 + index as i32) % 8;
                gem(canvas, screen, y, phase, index % 2 == 0);
            } else if reached - at < 16 * SPEED {
                sparkle(canvas, screen, y, (reached - at) / SPEED);
            }
        }
    }
    collected
}

/// A gem turning: `phase` of 8, half a turn, then the same again from its other side.
fn gem(canvas: &mut Canvas, x: i32, y: i32, phase: i32, red: bool) {
    let (light, face, dark, outline): (Rgb, Rgb, Rgb, Rgb) = if red {
        ([255, 168, 184], [236, 32, 64], [140, 16, 40], [64, 8, 24])
    } else {
        ([168, 208, 255], [40, 96, 248], [20, 40, 144], [8, 16, 72])
    };
    let half_width = [4, 4, 3, 1, 0, 1, 3, 4][phase as usize];
    // The lit side swaps as the other face comes round.
    let flip = if phase >= 4 { -1 } else { 1 };
    let half_height = 5;
    let inside = |dx: i32, dy: i32| {
        if half_width == 0 {
            dx == 0 && dy.abs() <= half_height
        } else {
            dx.abs() * half_height + dy.abs() * half_width <= half_width * half_height
        }
    };
    for dy in -half_height..=half_height {
        for dx in -half_width..=half_width {
            if !inside(dx, dy) {
                continue;
            }
            let edge = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|&(ex, ey)| !inside(dx + ex, dy + ey));
            let color = if edge && half_width > 0 {
                outline
            } else if dx * flip == -1 && dy == -1 {
                [255, 255, 255]
            } else if dx * flip < 0 {
                light
            } else if dx == 0 {
                face
            } else {
                dark
            };
            canvas.put(x + dx, y + dy, color);
        }
    }
}

/// A star that grows and fades out over 16 frames.
fn sparkle(canvas: &mut Canvas, x: i32, y: i32, age: i32) {
    let reach = 2 + age / 3;
    let color = if age % 4 < 2 {
        [255, 255, 255]
    } else {
        [255, 232, 96]
    };
    for step in reach / 2..=reach {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            canvas.put(x + dx * step, y + dy * step, color);
        }
    }
    if age < 8 {
        for (dx, dy) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
            canvas.put(x + dx * (reach / 2), y + dy * (reach / 2), color);
        }
    }
}

/// The runner, a round robot, above its feet. `.` is transparent.
const ROBOT: [&str; 16] = [
    "......kk........",
    ".....kyrk.......",
    ".....krrk.......",
    "......kk........",
    ".......k........",
    ".....kkkkkk.....",
    "...kkwwwwwwkk...",
    "..kwwwwwwwwwlk..",
    ".kwwwwwwwwwwllk.",
    ".kwkkkkkkkkkklk.",
    ".kwkvvvcvvcvklk.",
    ".kwkvvvcvvcvkbk.",
    ".kwkkkkkkkkkkbk.",
    ".kwwwwwwwwwwlbk.",
    "..kwwwwwwwwlbk..",
    "...kkllllllkk...",
];
/// The robot's height with its feet.
const ROBOT_HEIGHT: i32 = ROBOT.len() as i32 + 2;

fn robot_color(key: u8) -> Option<Rgb> {
    Some(match key {
        b'k' => [24, 24, 48],
        b'w' => [240, 244, 252],
        b'l' => [176, 188, 220],
        b'b' => [88, 104, 176],
        b'v' => [32, 48, 96],
        b'c' => [88, 236, 248],
        b'r' => [240, 40, 56],
        b'y' => [255, 236, 128],
        _ => return None,
    })
}

/// The runner at `x` across the level: striding on the ground in four steps, bobbing on the
/// two where a foot is lifted, or with its feet tucked in the air.
fn robot(canvas: &mut Canvas, x: i32, frame: i32) {
    let (feet_y, airborne) = feet(x);
    let left = RUNNER_X - 8;
    let top = feet_y - ROBOT_HEIGHT;
    // Each foot's left edge and how far it is lifted.
    let step = frame / 5 % 4;
    let (back, front, bob) = match (airborne, step) {
        (true, _) => ((4, 1), (9, 1), 0),
        (false, 0 | 2) => ((2, 0), (10, 0), 0),
        (false, 1) => ((5, 1), (8, 0), -1),
        (false, _) => ((5, 0), (8, 1), -1),
    };
    for (row, keys) in ROBOT.iter().enumerate() {
        for (column, &key) in keys.as_bytes().iter().enumerate() {
            if let Some(color) = robot_color(key) {
                canvas.put(left + column as i32, top + row as i32 + bob, color);
            }
        }
    }
    let foot_top = top + ROBOT.len() as i32;
    for (foot, lift) in [back, front] {
        for dx in 0..4 {
            canvas.put(left + foot + dx, foot_top - lift, [248, 132, 40]);
            canvas.put(left + foot + dx, foot_top + 1 - lift, [24, 24, 48]);
        }
        // A leg under the body when it bobs up.
        if bob < 0 && lift == 0 {
            canvas.put(left + foot + 1, foot_top - 1, [24, 24, 48]);
            canvas.put(left + foot + 2, foot_top - 1, [24, 24, 48]);
        }
    }
}

/// The HUD along the top: the score, the gems collected and the time left.
fn hud(canvas: &mut Canvas, collected: i32, frame: i32) {
    for y in 0..HUD {
        let color = if y == HUD - 1 {
            [112, 112, 176]
        } else {
            [16, 16, 40]
        };
        for x in 0..WIDTH {
            canvas.put(x, y, color);
        }
    }
    // Inside the part of the picture a TV's overscan leaves, as games kept their HUDs.
    let (label, value, top) = ([255, 208, 64], [255, 255, 255], 12);
    let score = format!("{:06}", 4200 + 100 * collected);
    text(canvas, 24, top, "SCORE", label);
    text(canvas, 56, top, &score, value);
    gem(canvas, 120, top + 3, 0, true);
    text(canvas, 128, top, &format!("x{collected:02}"), value);
    text(canvas, 168, top, "TIME", label);
    let time = 312 - frame / FPS as i32;
    text(canvas, 196, top, &time.to_string(), value);
}

/// `text` in the 5x7 font, with a shadow below and to the right.
fn text(canvas: &mut Canvas, x: i32, y: i32, text: &str, color: Rgb) {
    for (index, character) in text.chars().enumerate() {
        let Some((_, rows)) = FONT.iter().find(|(key, _)| *key == character) else {
            continue;
        };
        let left = x + index as i32 * 6;
        for (dy, row) in rows.iter().enumerate() {
            for (dx, &bit) in row.as_bytes().iter().enumerate() {
                if bit == b'#' {
                    let (px, py) = (left + dx as i32, y + dy as i32);
                    canvas.put(px + 1, py + 1, [0, 0, 16]);
                    canvas.put(px, py, color);
                }
            }
        }
    }
}

/// The HUD's font: the digits and the letters it uses, 5 by 7.
const FONT: [(char, [&str; 7]); 19] = [
    (
        '0',
        [
            ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.",
        ],
    ),
    (
        '1',
        [
            "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
    ),
    (
        '2',
        [
            ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
        ],
    ),
    (
        '3',
        [
            "#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###.",
        ],
    ),
    (
        '4',
        [
            "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
        ],
    ),
    (
        '5',
        [
            "#####", "#....", "####.", "....#", "....#", "#...#", ".###.",
        ],
    ),
    (
        '6',
        [
            "..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###.",
        ],
    ),
    (
        '7',
        [
            "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...",
        ],
    ),
    (
        '8',
        [
            ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
        ],
    ),
    (
        '9',
        [
            ".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##..",
        ],
    ),
    (
        'C',
        [
            ".###.", "#...#", "#....", "#....", "#....", "#...#", ".###.",
        ],
    ),
    (
        'E',
        [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
    ),
    (
        'I',
        [
            ".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
    ),
    (
        'M',
        [
            "#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#",
        ],
    ),
    (
        'O',
        [
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
    ),
    (
        'R',
        [
            "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
        ],
    ),
    (
        'S',
        [
            ".####", "#....", "#....", ".###.", "....#", "....#", "####.",
        ],
    ),
    (
        'T',
        [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
    ),
    (
        'x',
        [
            ".....", ".....", "#...#", ".#.#.", "..#..", ".#.#.", "#...#",
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_the_same_every_time_and_loop_without_a_seam() {
        assert_eq!(frame(0).dimensions(), SIZE);
        assert_eq!(frame(0), frame(FRAMES));
        assert_eq!(frame(137), frame(137 + 3 * FRAMES));
        assert_eq!(frame(401), frame(401));
        assert!(frame(0).pixels().all(|p| p[3] == 255), "opaque");
        // It moves: every frame differs from the one before, the last from the first too.
        for index in [1, 150, 400, FRAMES] {
            assert_ne!(frame(index - 1), frame(index), "frame {index}");
        }
        // The scene carries on past the loop's end into its start: drawn where the camera
        // has got to after a whole loop, it is the first frame's. Only the HUD starts over.
        let scene_at = |frame| {
            let mut canvas = Canvas(RgbaImage::new(SIZE.0, SIZE.1));
            scene(&mut canvas, frame);
            canvas.0
        };
        assert!(scene_at(0) == scene_at(FRAMES as i32), "a seam at the loop");
    }

    #[test]
    fn the_runner_lands_on_what_it_jumps_to_and_collects_gems_on_its_way() {
        // Each jump starts where the last one landed.
        let mut standing = GROUND;
        for jump in &JUMPS {
            assert_eq!(jump.start, standing, "the jump from {}", jump.from);
            assert_eq!(feet(jump.from), (jump.start, true));
            standing = jump.land;
        }
        assert_eq!(standing, GROUND, "the loop ends where it starts");
        // It never stands over a pit, and stands on a platform only where there is one.
        for x in 0..LEVEL {
            let (y, airborne) = feet(x);
            if airborne {
                continue;
            }
            let tile = x / TILE;
            if y == GROUND {
                assert!(
                    !pit(tile) && !pit((x - 6) / TILE) && !pit((x + 6) / TILE),
                    "{x}"
                );
            } else {
                assert!(
                    PLATFORMS.iter().any(|&(first, tiles, top)| top == y
                        && (first * TILE..(first + tiles) * TILE).contains(&x)),
                    "{x} at {y}"
                );
            }
        }
        let on_the_way = GEMS.iter().filter(|&&(x, y)| on_the_way(x, y)).count();
        assert_eq!(on_the_way, GEMS.len() - 2);
        let collected = |index: u64| {
            let frame = index as i32;
            gems(
                &mut Canvas(RgbaImage::new(SIZE.0, SIZE.1)),
                frame * SPEED,
                frame * SPEED + RUNNER_X,
                frame,
            )
        };
        assert_eq!(collected(0), 0);
        assert_eq!(collected(FRAMES - 1), on_the_way as i32);
    }
}
