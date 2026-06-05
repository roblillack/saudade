//! svg — `include_svg!` versus `include_str!` + a runtime SVG rasterizer.
//!
//! Two ways to put an SVG icon on screen:
//!
//! * **`include_svg!`** reads the SVG *at compile time* and bakes it into a set
//!   of flattened polygons. At run time saudade only fills those polygons —
//!   there is no SVG parser, no `usvg`, no `resvg` anywhere in the binary.
//! * **`include_str!` + `resvg`** embeds the SVG *text* and parses + rasterizes
//!   it at run time (what control-panel's `Icon` currently does). Flexible, but
//!   it drags the whole `resvg`/`usvg`/`tiny-skia` tree into the program and
//!   re-parses the SVG every time a new pixel size is needed.
//!
//! The window draws the same six icons both ways (top row baked, bottom row
//! rasterized) plus a size ramp so you can eyeball the fidelity. On startup it
//! also prints a micro-benchmark of both methods to the console.
//!
//! ```console
//! $ cargo run --release --example svg     # --release: the numbers mean something
//! ```
//!
//! Note the two macros resolve paths differently: `include_str!` is relative to
//! *this source file* (`examples/`), while `include_svg!` — a stable-Rust proc
//! macro that can't see the call site's file — is relative to the crate root
//! (`CARGO_MANIFEST_DIR`). Hence the `examples/` prefix on the `include_svg!`
//! paths below.

use std::time::{Duration, Instant};

use resvg::{tiny_skia, usvg};
use saudade::{App, Color, Painter, Rect, SvgImage, Theme, Widget, WindowConfig, include_svg};

/// One icon available through both pipelines: the compile-time-baked polygons
/// and the raw SVG text the runtime rasterizer parses.
struct Icon {
    name: &'static str,
    /// Baked at compile time by `include_svg!` (path relative to the crate root).
    baked: SvgImage,
    /// The same SVG embedded as text for the `resvg` path (relative to this file).
    source: &'static str,
}

/// The six demo icons, paired so each can be drawn either way. The stroke-heavy
/// marks (sound, audio, network) exercise the macro's stroke-to-outline
/// expansion; colors exercises cubic Béziers + circles.
const ICONS: &[Icon] = &[
    Icon {
        name: "sound",
        baked: include_svg!("examples/assets/icons/sound.svg"),
        source: include_str!("assets/icons/sound.svg"),
    },
    Icon {
        name: "colors",
        baked: include_svg!("examples/assets/icons/colors.svg"),
        source: include_str!("assets/icons/colors.svg"),
    },
    Icon {
        name: "network",
        baked: include_svg!("examples/assets/icons/network.svg"),
        source: include_str!("assets/icons/network.svg"),
    },
    Icon {
        name: "printers",
        baked: include_svg!("examples/assets/icons/printers.svg"),
        source: include_str!("assets/icons/printers.svg"),
    },
    Icon {
        name: "audio",
        baked: include_svg!("examples/assets/icons/audio.svg"),
        source: include_str!("assets/icons/audio.svg"),
    },
    Icon {
        name: "display",
        baked: include_svg!("examples/assets/icons/display.svg"),
        source: include_str!("assets/icons/display.svg"),
    },
];

const W: i32 = 660;
const H: i32 = 430;

fn main() {
    benchmark();
    App::new(
        WindowConfig::new("include_svg! vs include_str!", W, H),
        Demo,
    )
    .with_theme(Theme::windows_31())
    .run();
}

// ---------------------------------------------------------------------------
// Runtime rasterizer (the `include_str!` + resvg side)
// ---------------------------------------------------------------------------

/// Rasterize SVG *text* into a `w × h` opaque ARGB buffer — a full parse plus
/// render, exactly what control-panel's `Icon` does for every new size.
fn rasterize_text(svg: &str, w: u32, h: u32) -> Vec<u32> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("valid demo SVG");
    rasterize_tree(&tree, w, h)
}

/// Render an already-parsed tree into a `w × h` opaque ARGB buffer. Anti-aliased
/// edges are composited over white (the workspace) so the icon sits cleanly on
/// the demo's white background, mirroring control-panel's compositing.
fn rasterize_tree(tree: &usvg::Tree, w: u32, h: u32) -> Vec<u32> {
    let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("non-zero size");
    let size = tree.size();
    let scale = (w as f32 / size.width()).min(h as f32 / size.height());
    let tx = (w as f32 - size.width() * scale) * 0.5;
    let ty = (h as f32 - size.height() * scale) * 0.5;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);
    resvg::render(tree, transform, &mut pixmap.as_mut());

    let mut out = vec![Color::WHITE.0; (w * h) as usize];
    for (px, chunk) in out.iter_mut().zip(pixmap.data().chunks_exact(4)) {
        let (r, g, b, a) = (chunk[0], chunk[1], chunk[2], chunk[3]);
        if a == 0 {
            continue;
        }
        // Source is premultiplied; composite over opaque white and store opaque.
        let inv = 255 - a as u32;
        let comp = |c: u8| (c as u32 + inv).min(255);
        *px = 0xFF00_0000 | (comp(r) << 16) | (comp(g) << 8) | comp(b);
    }
    out
}

/// Draw an icon the `resvg` way: at the painter's physical footprint (so it is
/// crisp at any DPI), then blit the raster onto the surface.
fn draw_rasterized(painter: &mut Painter, svg: &str, rect: Rect) {
    painter.physical(rect, |p, phys| {
        if phys.w <= 0 || phys.h <= 0 {
            return;
        }
        let raster = rasterize_text(svg, phys.w as u32, phys.h as u32);
        for py in 0..phys.h {
            let row = (py * phys.w) as usize;
            for px in 0..phys.w {
                p.pixel(phys.x + px, phys.y + py, Color(raster[row + px as usize]));
            }
        }
    });
}

// ---------------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------------

struct Demo;

impl Widget for Demo {
    fn bounds(&self) -> Rect {
        Rect::new(0, 0, W, H)
    }

    fn paint(&mut self, painter: &mut Painter, theme: &Theme) {
        painter.fill(Color::WHITE);

        painter.text(
            16,
            12,
            "include_svg!  vs  include_str! + resvg",
            14.0,
            theme.text,
        );
        painter.text(
            16,
            32,
            "Same six icons, two pipelines. They should look identical.",
            11.0,
            theme.disabled_text,
        );

        // Two labeled rows of the six icons, baked on top, rasterized below.
        let cell = 56;
        let gap = 44;
        let x0 = 150;
        let size = 40;
        let pad = (cell - size) / 2;

        let baked_y = 64;
        let rast_y = baked_y + cell + 20;
        painter.text(
            16,
            baked_y + cell / 2 - 14,
            "include_svg!",
            11.0,
            theme.text,
        );
        painter.text(
            16,
            baked_y + cell / 2,
            "(baked polygons)",
            10.0,
            theme.disabled_text,
        );
        painter.text(16, rast_y + cell / 2 - 14, "include_str!", 11.0, theme.text);
        painter.text(
            16,
            rast_y + cell / 2,
            "+ resvg (runtime)",
            10.0,
            theme.disabled_text,
        );

        for (i, icon) in ICONS.iter().enumerate() {
            let x = x0 + i as i32 * (cell + gap);
            painter.stroke_rect(Rect::new(x, baked_y, cell, cell), theme.shadow);
            painter.stroke_rect(Rect::new(x, rast_y, cell, cell), theme.shadow);
            icon.baked
                .draw(painter, Rect::new(x + pad, baked_y + pad, size, size));
            draw_rasterized(
                painter,
                icon.source,
                Rect::new(x + pad, rast_y + pad, size, size),
            );
            painter.text_centered(
                Rect::new(x, rast_y + cell + 2, cell, 14),
                icon.name,
                10.0,
                theme.disabled_text,
            );
        }

        // A size ramp of one icon, baked on top vs. rasterized below, so the
        // fidelity holds up across scales.
        let ramp = &ICONS[1]; // colors — Béziers + circles
        let ramp_y = rast_y + cell + 34;
        painter.etched_h_line(16, ramp_y - 10, W - 32, theme);
        painter.text(
            16,
            ramp_y,
            "size ramp — same baked geometry at every size:",
            11.0,
            theme.text,
        );
        let mut x = 150;
        let top = ramp_y + 18;
        for s in [16, 20, 24, 32, 40, 48, 64] {
            let baked_box = Rect::new(x, top, s, s);
            let rast_box = Rect::new(x + s + 12, top, s, s);
            ramp.baked.draw(painter, baked_box);
            draw_rasterized(painter, ramp.source, rast_box);
            x += (s + 12) * 2 + 10;
        }

        painter.text(
            16,
            H - 22,
            "Benchmark printed to the console — run with --release for real numbers.",
            10.0,
            theme.disabled_text,
        );
    }
}

// ---------------------------------------------------------------------------
// Benchmark
// ---------------------------------------------------------------------------

/// Time both pipelines across a range of pixel sizes and print a table.
///
/// * **baked fill** — `include_svg!` geometry filled by saudade (no parsing).
/// * **parse+render** — `usvg::Tree::from_str` + `resvg::render`: the real cost
///   of going from embedded SVG text to pixels at a fresh size (control-panel
///   pays this every time the scale factor changes).
/// * **render-only** — `resvg::render` with the tree pre-parsed once, to isolate
///   how much of the runtime cost is parsing.
fn benchmark() {
    let sizes: [u32; 5] = [16, 32, 64, 128, 256];

    println!(
        "\n  include_svg! vs runtime rasterization  ({} icons, avg per op)",
        ICONS.len()
    );
    if cfg!(debug_assertions) {
        println!("  [debug build — numbers are rough; use --release to compare for real]");
    }
    println!(
        "  {:>6}   {:>14}   {:>14}   {:>14}   {:>9}",
        "size", "baked fill", "parse+render", "render-only", "speedup",
    );
    println!("  {}", "-".repeat(72));

    for &size in &sizes {
        // Scale iteration counts to the work per op so each timing is stable
        // without the big sizes taking forever (especially in a debug build).
        let budget: u64 = if cfg!(debug_assertions) {
            400_000
        } else {
            6_000_000
        };
        let iters = (budget / (size as u64 * size as u64)).clamp(8, 4000) as u32;

        let mut baked = Duration::ZERO;
        let mut parse_render = Duration::ZERO;
        let mut render_only = Duration::ZERO;

        for icon in ICONS {
            let mut buf = vec![Color::WHITE.0; (size * size) as usize];
            let t = Instant::now();
            for _ in 0..iters {
                let mut p = Painter::new(&mut buf, size as i32, size as i32, 1.0, 0, 0, None, None);
                icon.baked
                    .draw(&mut p, Rect::new(0, 0, size as i32, size as i32));
            }
            baked += t.elapsed();

            let t = Instant::now();
            for _ in 0..iters {
                std::hint::black_box(rasterize_text(icon.source, size, size));
            }
            parse_render += t.elapsed();

            let tree = usvg::Tree::from_str(icon.source, &usvg::Options::default()).unwrap();
            let t = Instant::now();
            for _ in 0..iters {
                std::hint::black_box(rasterize_tree(&tree, size, size));
            }
            render_only += t.elapsed();
        }

        let ops = (ICONS.len() as u32 * iters) as f64;
        let us = |d: Duration| d.as_secs_f64() * 1e6 / ops;
        let (b, pr) = (us(baked), us(parse_render));
        println!(
            "  {:>5}px   {:>11.2} µs   {:>11.2} µs   {:>11.2} µs   {:>7.0}×",
            size,
            b,
            pr,
            us(render_only),
            pr / b,
        );
    }

    fidelity();

    println!(
        "\n  Plus: the baked path links no SVG crates at all — `resvg`/`usvg`/`tiny-skia`\n  \
         (a large dependency tree) stay entirely out of the binary.\n"
    );
}

/// Quantify how close the baked polygons land to resvg's own rasterization:
/// render each icon both ways over white at 64px and report the mean per-channel
/// difference. A few /255 is just anti-aliasing disagreement at edges — the
/// shapes themselves (including the stroke outlines kurbo expands) line up.
fn fidelity() {
    let size = 64u32;
    let mut total = 0.0;
    let mut worst = (0.0, "");
    for icon in ICONS {
        let mut baked = vec![Color::WHITE.0; (size * size) as usize];
        {
            let mut p = Painter::new(&mut baked, size as i32, size as i32, 1.0, 0, 0, None, None);
            icon.baked
                .draw(&mut p, Rect::new(0, 0, size as i32, size as i32));
        }
        let rast = rasterize_text(icon.source, size, size);

        let mut sum = 0u64;
        for (&x, &y) in baked.iter().zip(rast.iter()) {
            let (a, b) = (Color(x), Color(y));
            sum += a.red().abs_diff(b.red()) as u64;
            sum += a.green().abs_diff(b.green()) as u64;
            sum += a.blue().abs_diff(b.blue()) as u64;
        }
        let mean = sum as f64 / (baked.len() * 3) as f64;
        total += mean;
        if mean > worst.0 {
            worst = (mean, icon.name);
        }
    }
    let avg = total / ICONS.len() as f64;
    println!(
        "\n  fidelity vs resvg @ {size}px: mean Δ {:.2}/255 ({:.2}%); worst: {} at {:.2}/255",
        avg,
        avg / 255.0 * 100.0,
        worst.1,
        worst.0,
    );
}
