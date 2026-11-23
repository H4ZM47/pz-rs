//! Core screenshot requests, captured from one presented frame.
use pz_core::core::CoreRequest;
use pz_core::debug::{LocalTime, LogClock, SystemClock};
use pz_core::thumbnail::{stbi_write_png, THUMB_FILE};
use std::io::{self, ErrorKind};
use std::path::{Component, Path};

fn default_name() -> String {
    let clock = SystemClock::new();
    let now = clock.now_millis();
    let t = LocalTime::from_epoch(now, clock.local_offset_millis(now));
    format!(
        "screenshot_{:02}-{:02}-{:04}_{:02}-{:02}-{:02}.png",
        t.day, t.month, t.year, t.hour, t.minute, t.second
    )
}

pub(crate) fn save(
    request: CoreRequest,
    (w, h): (u32, u32),
    rgba: &[u8],
    cache: &Path,
    world: Option<&Path>,
) -> io::Result<()> {
    match request {
        CoreRequest::FullScreenshot(name) => {
            let name = name.unwrap_or_else(default_name);
            let name = Path::new(&name);
            if name.as_os_str().is_empty()
                || name
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
            {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    "invalid screenshot filename",
                ));
            }
            let dir = cache.join("Screenshots");
            std::fs::create_dir_all(&dir)?;
            std::fs::write(
                dir.join(name),
                stbi_write_png(rgba, w as usize, h as usize, 4, false),
            )
        }
        CoreRequest::Screenshot {
            x,
            y,
            width,
            height,
            ..
        } => {
            let Some(world) = world else { return Ok(()) };
            if width <= 0 || height <= 0 || width as u32 > w || height as u32 > h {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    "invalid screenshot dimensions",
                ));
            }
            // GL's origin is bottom left; the readback and the PNG both start at the top.
            let mut cropped = vec![0; width as usize * height as usize * 4];
            let left = i64::from(x);
            let top = i64::from(h) - i64::from(y) - i64::from(height);
            let start = left.max(0);
            let end = (left + i64::from(width)).min(i64::from(w));
            if start < end {
                let span = (end - start) as usize * 4;
                for (row, dst) in cropped.chunks_exact_mut(width as usize * 4).enumerate() {
                    let y = top + row as i64;
                    if !(0..i64::from(h)).contains(&y) {
                        continue;
                    }
                    let src = (y as usize * w as usize + start as usize) * 4;
                    let x = (start - left) as usize * 4;
                    dst[x..x + span].copy_from_slice(&rgba[src..src + span]);
                }
            }
            std::fs::write(
                world.join(THUMB_FILE),
                stbi_write_png(&cropped, width as usize, height as usize, 4, false),
            )
        }
        _ => unreachable!("only screenshot requests are drained"),
    }
}
