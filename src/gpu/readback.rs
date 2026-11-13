//! Getting a frame back from the GPU: which picks to read, the buffers they are copied into, and
//! the frame in flight between the submit that starts the copy and the collect that finishes it.
//!
//! A frame can be read back at once (`Gpu::render` submits it and collects it) or collected a
//! frame later (`Renderer::pipelined`), so the GPU draws one frame while the CPU records the
//! next. Two buffers take turns, so a frame being mapped never blocks the one being copied.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use crate::render::Frame;
use crate::render::shared::NONE;

/// How much of the pick buffer a read back brings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Picks {
    /// None: the frame's picks are all `NONE`.
    Nowhere,
    /// A window around this frame position, which is as far as hover and clicks look.
    Around([f32; 2]),
    /// Every pixel's pick, for a headless frame whose picks are checked or compared.
    Everywhere,
}

/// Picking is read back only in a window around the pointer: hover and clicks never look further.
pub(super) const PICK_WINDOW: u32 = 160;
const PICK_ROW: u32 = (PICK_WINDOW * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
    * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// Where a frame's colour and picks sit in the readback buffer: the colour rows from 0, then the
/// pick rows from `pick_offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    pub(super) width: u32,
    pub(super) height: u32,
    /// Bytes per colour row, padded to wgpu's copy alignment.
    pub(super) row: u32,
    pub(super) pick_origin: [u32; 2],
    /// The pixels of the pick buffer that are copied: none, a window or all of them.
    pub(super) pick_size: [u32; 2],
    pub(super) pick_row: u32,
    pub(super) pick_offset: u64,
    /// The bytes the buffer holds.
    pub(super) bytes: u64,
}

impl Layout {
    pub(super) fn new(width: u32, height: u32, picks: Picks) -> Self {
        let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let pick_offset = u64::from(row) * u64::from(height);
        let (pick_origin, pick_size, pick_row) = match picks {
            Picks::Nowhere => ([0, 0], [0, 0], 0),
            Picks::Around([x, y]) => {
                let size = [PICK_WINDOW.min(width), PICK_WINDOW.min(height)];
                let corner = |at: f32, extent: u32, limit: u32| {
                    (at as i64 - i64::from(extent) / 2).clamp(0, i64::from(limit - extent)) as u32
                };
                (
                    [corner(x, size[0], width), corner(y, size[1], height)],
                    size,
                    PICK_ROW,
                )
            }
            Picks::Everywhere => ([0, 0], [width, height], row),
        };
        Self {
            width,
            height,
            row,
            pick_origin,
            pick_size,
            pick_row,
            pick_offset,
            bytes: pick_offset + u64::from(pick_row) * u64::from(pick_size[1]),
        }
    }
}

/// One of the two buffers frames are read back into, and whether a frame in flight holds it.
#[derive(Default)]
struct Slot {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    busy: Arc<AtomicBool>,
}

/// The readback buffers, which take turns and only grow.
#[derive(Default)]
pub(super) struct Slots {
    slots: [Slot; 2],
    next: usize,
    /// How many buffers have been created, which stops growing once the frames have.
    pub(super) created: u64,
}

impl Slots {
    /// The next buffer, with room for `bytes`, and the flag that says it is in use. Fails when
    /// both buffers hold frames that have not been collected.
    pub(super) fn acquire(
        &mut self,
        device: &wgpu::Device,
        bytes: u64,
    ) -> Result<(wgpu::Buffer, Arc<AtomicBool>), String> {
        let slot = &mut self.slots[self.next];
        if slot.busy.load(Ordering::Acquire) {
            return Err("two frames are already in flight on the GPU".to_owned());
        }
        if slot.capacity < bytes || slot.buffer.is_none() {
            slot.capacity = bytes.max(slot.capacity);
            slot.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: slot.capacity,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
            self.created += 1;
        }
        slot.busy.store(true, Ordering::Release);
        self.next = 1 - self.next;
        let buffer = slot.buffer.clone().ok_or("the readback buffer exists")?;
        Ok((buffer, Arc::clone(&slot.busy)))
    }

    /// The capacities of the two buffers.
    #[cfg(test)]
    pub(super) fn capacities(&self) -> [u64; 2] {
        [self.slots[0].capacity, self.slots[1].capacity]
    }
}

/// A frame submitted to the GPU whose colour and picks are being copied into a buffer. Finishing
/// it (`Gpu::collect`) waits for the GPU if it has not finished, and fills the frame; dropping it
/// waits and lets the buffer go, so the buffer is free for the next frame either way.
pub(crate) struct Flight {
    device: wgpu::Device,
    buffer: wgpu::Buffer,
    busy: Arc<AtomicBool>,
    submission: wgpu::SubmissionIndex,
    mapped: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    layout: Layout,
    /// Whether the buffer has been waited for and unmapped.
    settled: bool,
}

impl Flight {
    /// Starts mapping `buffer`, which `submission` copies the frame into.
    pub(super) fn new(
        device: wgpu::Device,
        buffer: wgpu::Buffer,
        busy: Arc<AtomicBool>,
        submission: wgpu::SubmissionIndex,
        layout: Layout,
    ) -> Self {
        let (sender, mapped) = mpsc::channel();
        buffer
            .slice(..layout.bytes)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        Self {
            device,
            buffer,
            busy,
            submission,
            mapped,
            layout,
            settled: false,
        }
    }

    /// Waits for the GPU to finish the frame, and says whether its buffer is mapped.
    fn wait(&mut self) -> Result<(), String> {
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(self.submission.clone()),
                timeout: None,
            })
            .map_err(|error| format!("GPU poll: {error}"))?;
        self.mapped
            .recv()
            .map_err(|error| format!("GPU readback: {error}"))?
            .map_err(|error| format!("GPU readback: {error}"))
    }

    /// Waits for the frame and fills `frame`'s pixels and picks from it. `scratch` is where the
    /// mapped memory is copied first.
    pub(super) fn finish(mut self, frame: &mut Frame, scratch: &mut Vec<u8>) -> Result<(), String> {
        let waited = self.wait();
        let converted = waited.and_then(|()| {
            let mapped = self
                .buffer
                .slice(..self.layout.bytes)
                .get_mapped_range()
                .map_err(|error| format!("GPU readback: {error}"))?;
            // Mapped readback memory can be uncached (Intel); one bulk copy into cached memory is
            // hundreds of times faster than reading it piecemeal during the conversion below.
            copy_mapped(&mapped, scratch);
            drop(mapped);
            convert(&self.layout, scratch, frame);
            Ok(())
        });
        self.release(converted.is_ok());
        converted
    }

    /// Lets the buffer go: unmapped when it was mapped, and free for the next frame.
    fn release(&mut self, mapped: bool) {
        if mapped {
            self.buffer.unmap();
        }
        self.settled = true;
        self.busy.store(false, Ordering::Release);
    }
}

impl Drop for Flight {
    fn drop(&mut self) {
        if !self.settled {
            let mapped = self.wait().is_ok();
            self.release(mapped);
        }
    }
}

/// Fills `frame`'s pixels from the colour rows of `data` and its picks from the pick rows: `NONE`
/// everywhere the readback did not bring.
fn convert(layout: &Layout, data: &[u8], frame: &mut Frame) {
    let row = layout.row as usize;
    let (w, h) = (layout.width as usize, layout.height as usize);
    frame.pixels.resize(w * h * 3, 0);
    frame.picks.clear();
    frame.picks.resize(w * h, NONE);
    for y in 0..h {
        let colors = &data[y * row..y * row + w * 4];
        for (pixel, rgba) in frame.pixels[y * w * 3..(y + 1) * w * 3]
            .as_chunks_mut::<3>()
            .0
            .iter_mut()
            .zip(colors.as_chunks::<4>().0)
        {
            pixel.copy_from_slice(&rgba[..3]);
        }
    }
    let [columns, rows] = layout.pick_size.map(|extent| extent as usize);
    let base = layout.pick_offset as usize;
    for wy in 0..rows {
        let source = &data[base + wy * layout.pick_row as usize..][..columns * 4];
        let start = (layout.pick_origin[1] as usize + wy) * w + layout.pick_origin[0] as usize;
        for (pick, bytes) in frame.picks[start..start + columns]
            .iter_mut()
            .zip(source.as_chunks::<4>().0)
        {
            *pick = u32::from_le_bytes(*bytes);
        }
    }
}

/// Copies GPU readback memory into `out`. Mapped readback memory may be write-combined (Intel
/// integrated GPUs), where ordinary loads are uncached and slow; SSE4.1 streaming loads read it at
/// close to normal memory speed.
fn copy_mapped(source: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(source.len());
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("sse4.1")
        && (source.as_ptr() as usize).is_multiple_of(16)
    {
        let whole = source.len() / 16 * 16;
        // SAFETY: SSE4.1 is available, the source is 16-byte aligned and every load stays within
        // `whole` bytes of it; the destination has capacity for `source.len()` bytes and its length
        // is set only after those bytes are written.
        unsafe {
            stream_copy(source.as_ptr(), out.as_mut_ptr(), whole);
            out.set_len(whole);
        }
        out.extend_from_slice(&source[whole..]);
        return;
    }
    out.extend_from_slice(source);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn stream_copy(source: *const u8, target: *mut u8, length: usize) {
    use std::arch::x86_64::{__m128i, _mm_storeu_si128, _mm_stream_load_si128};
    for offset in (0..length).step_by(16) {
        // SAFETY: the caller guarantees alignment, SSE4.1 support and in-bounds offsets.
        unsafe {
            let chunk = _mm_stream_load_si128(source.add(offset) as *const __m128i);
            _mm_storeu_si128(target.add(offset) as *mut __m128i, chunk);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pick_window_follows_the_pointer_and_stays_inside_the_frame() {
        let size = |picks| Layout::new(640, 360, picks);
        let around = size(Picks::Around([320.0, 180.0]));
        assert_eq!(around.pick_size, [160, 160]);
        assert_eq!(around.pick_origin, [240, 100]);
        for pointer in [[0.0, 0.0], [-50.0, 700.0], [639.0, 359.0], [f32::NAN, 1e9]] {
            let layout = size(Picks::Around(pointer));
            assert!(layout.pick_origin[0] + layout.pick_size[0] <= 640);
            assert!(layout.pick_origin[1] + layout.pick_size[1] <= 360);
        }
        let small = Layout::new(100, 60, Picks::Around([50.0, 30.0]));
        assert_eq!(small.pick_size, [100, 60]);
        assert_eq!(small.pick_origin, [0, 0]);
    }

    #[test]
    fn a_full_read_back_holds_every_row_of_colour_and_of_picks() {
        let layout = Layout::new(120, 80, Picks::Everywhere);
        assert_eq!(layout.pick_size, [120, 80]);
        assert_eq!(layout.pick_origin, [0, 0]);
        assert_eq!(layout.row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
        assert_eq!(layout.pick_row, layout.row);
        assert_eq!(layout.bytes, 2 * u64::from(layout.row) * 80);
        let none = Layout::new(120, 80, Picks::Nowhere);
        assert_eq!(none.bytes, u64::from(none.row) * 80);
    }

    #[test]
    fn converting_places_colour_and_picks_where_the_layout_says() {
        use crate::render::{Backdrop, Buffers, Sky};
        let sky = Sky {
            backdrop: Backdrop::Dusk,
            haze: [0.0; 3],
            time: 0.0,
        };
        for picks in [
            Picks::Nowhere,
            Picks::Around([10.0, 5.0]),
            Picks::Everywhere,
        ] {
            let layout = Layout::new(70, 20, picks);
            let mut data = vec![0u8; layout.bytes as usize];
            for y in 0..20usize {
                for x in 0..70usize {
                    data[y * layout.row as usize + x * 4..][..4]
                        .copy_from_slice(&[x as u8, y as u8, 7, 255]);
                }
            }
            for y in 0..layout.pick_size[1] as usize {
                for x in 0..layout.pick_size[0] as usize {
                    let (fx, fy) = (
                        layout.pick_origin[0] as usize + x,
                        layout.pick_origin[1] as usize + y,
                    );
                    data[layout.pick_offset as usize + y * layout.pick_row as usize + x * 4..][..4]
                        .copy_from_slice(&((fy * 70 + fx) as u32).to_le_bytes());
                }
            }
            let mut frame = Frame::new(70, 20, sky, Buffers::default());
            convert(&layout, &data, &mut frame);
            for y in 0..20usize {
                for x in 0..70usize {
                    assert_eq!(frame.pixels[(y * 70 + x) * 3..][..3], [x as u8, y as u8, 7]);
                    let inside = x >= layout.pick_origin[0] as usize
                        && x < (layout.pick_origin[0] + layout.pick_size[0]) as usize
                        && y >= layout.pick_origin[1] as usize
                        && y < (layout.pick_origin[1] + layout.pick_size[1]) as usize;
                    let wanted = if inside { (y * 70 + x) as u32 } else { NONE };
                    assert_eq!(frame.picks[y * 70 + x], wanted, "{picks:?} at ({x}, {y})");
                }
            }
        }
    }
}
