//! Parity check for `pz_core::thumbnail`: decodes a `thumb.png` written by
//! the game (`stbi_write_png`, flipped), turns it back into the bottom-up
//! RGBA rows `glReadPixels` gave, encodes it again and compares the bytes.
//!
//! `cargo run -p pz-core --example thumb_reencode -- <thumb.png> [out.png]`

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input = std::fs::read(&args[1]).expect("read thumb.png");
    let dec = png::Decoder::new(std::io::Cursor::new(&input));
    let mut reader = dec.read_info().expect("png header");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("png frame");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let (w, h) = (info.width as usize, info.height as usize);
    let stride = w * 4;
    // PNG rows are top-down; GL rows bottom-up.
    let mut gl = Vec::with_capacity(stride * h);
    for y in (0..h).rev() {
        gl.extend_from_slice(&buf[y * stride..(y + 1) * stride]);
    }
    let out = pz_core::thumbnail::stbi_write_png(&gl, w, h, 4, true);
    if let Some(p) = args.get(2) {
        std::fs::write(p, &out).expect("write");
    }
    let same = out == input;
    let first = out.iter().zip(&input).position(|(a, b)| a != b);
    println!(
        "{w}x{h} input={} rust={} identical={same} first_diff={first:?}",
        input.len(),
        out.len()
    );
    if !same {
        std::process::exit(1);
    }
}
