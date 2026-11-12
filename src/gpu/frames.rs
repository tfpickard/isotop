//! Test only: real views' frames through the GPU. Draw calls that commute draw the pixels of the
//! recorded ones, a full read back gives the picks the CPU rasterizer gives, buffers stop growing,
//! and a pipelined frame comes back whole a frame late. They need a GPU (or lavapipe, with
//! `ISOTOP_ALLOW_SOFTWARE_GPU=1`), so they are ignored unless asked for.

use super::{Gpu, Order, Picks};
use crate::app::Renderer;
use crate::app::fixtures::{all_available, options};
use crate::cli::{Power, RendererChoice};
use crate::render::Frame;
use crate::render::shared::NONE;
use crate::views::View;
use crate::{App, demo_snapshot, warm_up};

fn gpu() -> Gpu {
    Gpu::new(wgpu::PowerPreference::LowPower)
        .expect("a GPU (hardware, or lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1)")
}

/// An app on `name`, set up as a headless `--demo --time 12` run of `width` pixels is, with no
/// frame drawn yet.
fn app_on(name: &str, width: u32) -> (App, crate::cli::Options) {
    let view = View::all()
        .find(|view| view.name() == name)
        .expect("a view of that name");
    let width_text = width.to_string();
    let options = options(&[
        "--demo",
        "--time",
        "12",
        "--renderer",
        "cpu",
        "--width",
        &width_text,
        "--output",
        "frames.png",
        "--view",
        name,
    ]);
    let mut app = App::new(
        view,
        demo_snapshot(
            view.info(),
            options.time - options.sample_seconds(),
            options.time,
            options.processes as usize,
        ),
        all_available(),
    );
    warm_up(&mut app, &options, None);
    app.animation = options.time as f32;
    (app, options)
}

/// The display list of the headless frame of `name`, recorded and not rasterized.
fn recorded(name: &str, width: u32) -> Frame {
    let (mut app, options) = app_on(name, width);
    app.draw(
        options.width,
        options.headless_height(),
        options.limit as usize,
    )
}

#[test]
#[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
fn draw_calls_that_commute_draw_the_pixels_and_picks_of_the_recorded_ones_in_every_view() {
    let mut gpu = gpu();
    let (mut recorded_calls, mut planned_calls) = (0, 0);
    for view in View::all() {
        let mut frame = recorded(view.name(), 320);
        gpu.set_order(Order::Recorded);
        gpu.render(&mut frame, Picks::Everywhere)
            .expect("the GPU renders");
        let (pixels, picks) = (frame.pixels.clone(), frame.picks.clone());
        let before = gpu.stats();
        gpu.set_order(Order::Commuting);
        gpu.render(&mut frame, Picks::Everywhere)
            .expect("the GPU renders");
        let after = gpu.stats();
        assert!(frame.pixels == pixels, "{}: pixels differ", view.name());
        assert!(frame.picks == picks, "{}: picks differ", view.name());
        assert_eq!(
            before.recorded_draw_calls, after.recorded_draw_calls,
            "the same list"
        );
        assert_eq!(before.draw_calls, before.recorded_draw_calls);
        assert!(after.draw_calls <= before.draw_calls);
        println!(
            "{:<12} {:>7} items {:>6} draw calls as recorded {:>6} planned",
            view.name(),
            after.items,
            before.draw_calls,
            after.draw_calls
        );
        recorded_calls += before.draw_calls;
        planned_calls += after.draw_calls;
    }
    assert!(planned_calls < recorded_calls, "some view saves draw calls");
}

#[test]
#[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
fn a_full_read_back_gives_the_picks_the_cpu_rasterizer_gives() {
    let mut gpu = gpu();
    for name in ["city", "orbit", "flow", "cells", "hive", "plumbing"] {
        let mut cpu = recorded(name, 480);
        cpu.rasterize();
        let mut hardware = recorded(name, 480);
        gpu.render(&mut hardware, Picks::Everywhere)
            .expect("the GPU renders");
        assert_eq!(hardware.picks.len(), cpu.picks.len());
        let differing = (0..cpu.picks.len())
            .filter(|&i| cpu.picks[i] != hardware.picks[i])
            .count();
        let picked = cpu.picks.iter().filter(|&&pick| pick != NONE).count();
        let gpu_picked = hardware.picks.iter().filter(|&&pick| pick != NONE).count();
        println!(
            "{name}: picks differ in {differing} of {} pixels ({picked} picked on the CPU, {gpu_picked} on the GPU)",
            cpu.picks.len()
        );
        assert!(picked > 0, "{name}: the CPU frame has something to pick");
        // The two rasterizers agree to a pixel at an edge, not at every pixel of one: the same
        // share the field parity test allows its window.
        assert!(
            differing * 200 <= cpu.picks.len(),
            "{name}: the picks differ in {differing} pixels"
        );
    }
}

#[test]
#[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
fn a_read_back_around_the_pointer_is_the_full_read_back_inside_its_window() {
    let mut gpu = gpu();
    let mut frame = recorded("city", 480);
    gpu.render(&mut frame, Picks::Everywhere)
        .expect("the GPU renders");
    let full = frame.picks.clone();
    let pointer = [200.0, 120.0];
    gpu.render(&mut frame, Picks::Around(pointer))
        .expect("the GPU renders");
    let width = frame.width as usize;
    let mut inside = 0;
    for (index, (&window, &whole)) in frame.picks.iter().zip(&full).enumerate() {
        let (x, y) = ((index % width) as f32, (index / width) as f32);
        if (x - pointer[0]).abs() < 79.0 && (y - pointer[1]).abs() < 79.0 {
            inside += 1;
            assert_eq!(window, whole, "at ({x}, {y})");
        }
    }
    assert!(inside > 10_000);
    gpu.render(&mut frame, Picks::Nowhere)
        .expect("the GPU renders");
    assert!(frame.picks.iter().all(|&pick| pick == NONE));
}

#[test]
#[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
fn no_buffer_is_made_again_once_the_frames_stop_growing() {
    let mut gpu = gpu();
    let (mut app, options) = app_on("flow", 480);
    let mut frame = None::<Frame>;
    let mut seen = Vec::new();
    for step in 0..40 {
        app.animation = options.time as f32 + step as f32 * 0.1;
        if let Some(earlier) = frame.take() {
            app.scene.spare = earlier.release();
        }
        let mut next = app.draw(
            options.width,
            options.headless_height(),
            options.limit as usize,
        );
        gpu.render(&mut next, Picks::Around([100.0, 100.0]))
            .expect("the GPU renders");
        frame = Some(next);
        seen.push((
            gpu.buffers_created(),
            gpu.staging.capacities(),
            gpu.staging.planner.capacities(),
            gpu.slots.capacities(),
        ));
    }
    // Nothing shrinks and nothing is made again each frame: after ten frames of warm-up the
    // lists and buffers grow by doubling only when a busier frame needs it, a handful of times
    // in thirty frames at most, and the GPU buffers never again.
    let settled = &seen[10..];
    let changes = settled.windows(2).filter(|pair| pair[0] != pair[1]).count();
    for pair in settled.windows(2) {
        let [(_, a, b, c), (_, d, e, f)] = [pair[0], pair[1]];
        let grew = |before: &[usize], after: &[usize]| {
            before
                .iter()
                .zip(after)
                .all(|(before, after)| after >= before)
        };
        assert!(grew(&a, &d) && grew(&b, &e), "a list shrank");
        assert!(c[0] <= f[0] && c[1] <= f[1], "a readback buffer shrank");
    }
    assert!(changes <= 4, "{changes} of 30 frames grew something");
    assert_eq!(
        settled.first().map(|seen| seen.0),
        settled.last().map(|seen| seen.0),
        "a GPU buffer was made again after warm-up"
    );
}

#[test]
#[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
fn two_frames_fly_at_once_and_a_third_waits_for_one_to_land() {
    let mut gpu = gpu();
    let mut frames = [0, 1, 2].map(|_| recorded("city", 160));
    let first = gpu.submit(&frames[0], Picks::Nowhere).expect("one flies");
    let second = gpu.submit(&frames[1], Picks::Nowhere).expect("two fly");
    assert!(
        gpu.submit(&frames[2], Picks::Nowhere).is_err(),
        "a third has no buffer to land in"
    );
    gpu.collect(first, &mut frames[0]).expect("the first lands");
    let third = gpu
        .submit(&frames[2], Picks::Nowhere)
        .expect("a buffer is free again");
    // A frame dropped without landing waits for the GPU and frees its buffer.
    drop(second);
    drop(third);
    let fourth = gpu
        .submit(&frames[1], Picks::Nowhere)
        .expect("both are free");
    gpu.collect(fourth, &mut frames[1]).expect("it lands");
    assert_eq!(frames[0].pixels.len(), 160 * 90 * 3);
}

/// A renderer on the GPU that runs a frame behind.
fn pipelined_renderer() -> Renderer {
    let mut renderer = Renderer::choose(RendererChoice::Gpu, Power::Low)
        .expect("a GPU (hardware, or lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1)");
    assert!(renderer.set_pipelined(true).is_none());
    renderer
}

/// What a frame that is drawn at once looks like, to hold a pipelined one to.
fn at_once(frame: &mut Frame) -> (Vec<u8>, Vec<u32>) {
    let mut renderer = Renderer::choose(RendererChoice::Gpu, Power::Low).expect("a GPU");
    renderer = renderer.reading_every_pick(true);
    renderer.rasterize(frame, None);
    (frame.pixels.clone(), frame.picks.clone())
}

#[test]
#[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
fn a_pipelined_frame_comes_back_whole_while_the_next_is_recorded() {
    let mut renderer = pipelined_renderer().reading_every_pick(true);
    let names = ["city", "orbit", "flow", "cells"];
    let expected: Vec<_> = names
        .iter()
        .map(|name| at_once(&mut recorded(name, 240)))
        .collect();
    let identities: Vec<_> = names
        .iter()
        .map(|name| recorded(name, 240).identities)
        .collect();
    let mut returned = Vec::new();
    for name in names {
        let ready = renderer.overlap(recorded(name, 240), None);
        returned.push(ready);
    }
    // The first frame has nothing before it; each call after hands back the call before.
    assert!(returned[0].is_none());
    for (index, ready) in returned.iter().enumerate().skip(1) {
        let frame = ready.as_ref().expect("a frame comes back");
        assert!(
            frame.pixels == expected[index - 1].0 && frame.picks == expected[index - 1].1,
            "call {index} returned the frame of call {}, finished",
            index - 1
        );
        assert!(
            frame.identities == identities[index - 1],
            "and it is whole: its identities are its own"
        );
    }
    // The last frame is still in flight; flushing finishes it, and only once.
    let last = renderer.flush().expect("the last frame was in flight");
    assert!(last.pixels == expected[3].0 && last.picks == expected[3].1);
    assert!(renderer.flush().is_none());
}

#[test]
#[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
fn leaving_the_pipeline_finishes_the_frame_in_flight_and_then_draws_at_once() {
    let mut renderer = pipelined_renderer();
    assert!(renderer.overlap(recorded("city", 240), None).is_none());
    let flushed = renderer.set_pipelined(false).expect("the frame in flight");
    assert!(!flushed.pixels.is_empty());
    // Not pipelined, a frame is drawn at once and comes straight back.
    let frame = renderer
        .overlap(recorded("orbit", 240), None)
        .expect("it comes straight back");
    assert!(!frame.pixels.is_empty());
    assert!(renderer.flush().is_none());
    // Back in the pipeline, with nothing in flight, the first frame again has nothing before it.
    assert!(renderer.set_pipelined(true).is_none());
    assert!(renderer.overlap(recorded("city", 240), None).is_none());
}

#[test]
#[ignore = "needs a GPU; run with --ignored on one, or on lavapipe with ISOTOP_ALLOW_SOFTWARE_GPU=1"]
fn a_pipeline_that_meets_a_new_size_returns_each_frame_at_its_own() {
    let mut renderer = pipelined_renderer();
    assert!(renderer.overlap(recorded("city", 320), None).is_none());
    let first = renderer
        .overlap(recorded("city", 200), None)
        .expect("the first comes back");
    assert_eq!(first.width, 320);
    assert_eq!(first.pixels.len(), 320 * 180 * 3);
    let second = renderer.flush().expect("the second");
    assert_eq!(second.width, 200);
    assert_eq!(second.pixels.len(), 200 * 112 * 3);
}

/// Nothing here needs a GPU: a CPU renderer has no frame to run behind.
#[test]
fn a_cpu_renderer_hands_every_frame_straight_back() {
    let mut renderer = Renderer::cpu();
    assert!(renderer.set_pipelined(true).is_none());
    let frame = renderer
        .overlap(recorded("city", 160), None)
        .expect("a frame comes back at once");
    assert_eq!(frame.pixels.len(), 160 * 90 * 3);
    assert!(renderer.flush().is_none());
    assert!(renderer.draw_stats().is_none());
}
