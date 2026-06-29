// Placeholder image synthesis for the simulated image-generation endpoint.
//
// Design decisions:
// - LLMSim does not run a real diffusion model. To keep the simulator
//   dependency-free (no `image`/`png`/`flate2` crates) we ship a tiny
//   self-contained PNG encoder plus a 5x7 bitmap font. The generated image is
//   an *indexed-color* (color type 3) PNG of the exact requested size that
//   draws the request prompt and a clear "LLMSIM SIMULATED IMAGE" watermark,
//   so anyone inspecting the bytes immediately understands it is synthetic.
// - We deliberately use uncompressed ("stored") DEFLATE blocks. Combined with
//   an 8-bit palette this keeps each 1024x1024 frame around ~1MB raw / ~1.4MB
//   base64 — realistic enough for client testing without pulling in a
//   compression dependency. The image content is flat/banded by design so the
//   lack of compression is not visually meaningful.
// - Output is deterministic from the prompt so repeated calls and tests are
//   reproducible; the palette hue is derived from a hash of the prompt.

/// Parameters describing the placeholder image to render.
pub struct PlaceholderSpec<'a> {
    pub width: u32,
    pub height: u32,
    pub prompt: &'a str,
    pub model: &'a str,
    pub quality: &'a str,
    /// Preview block size: 1 renders a crisp final image, larger values render
    /// a progressively coarser (pixelated) preview used for partial frames.
    pub blockiness: u32,
}

/// Render the placeholder image and return PNG-encoded bytes.
pub fn render_png(spec: &PlaceholderSpec<'_>) -> Vec<u8> {
    let width = spec.width.clamp(16, 4096);
    let height = spec.height.clamp(16, 4096);

    let palette = build_palette(spec.prompt);
    let mut canvas = Canvas::new(width, height);

    canvas.draw_background();
    canvas.draw_bars();
    canvas.draw_border();
    canvas.draw_labels(spec);

    if spec.blockiness > 1 {
        canvas.pixelate(spec.blockiness);
    }

    encode_indexed_png(width, height, &palette, &canvas.buf)
}

// ---------------------------------------------------------------------------
// Palette (16 entries, RGB)
// ---------------------------------------------------------------------------

// Fixed palette index assignments used while drawing.
const C_BORDER: u8 = 0;
const C_BG_BASE: u8 = 1; // shades 1..=8 form the background gradient
const C_HEADER: u8 = 9;
const C_FOOTER: u8 = 10;
const C_TITLE: u8 = 11;
const C_BODY: u8 = 12;
const C_LABEL: u8 = 13;

fn build_palette(prompt: &str) -> Vec<[u8; 3]> {
    let hash = fnv1a(prompt.as_bytes());
    let hue = (hash % 360) as f64;
    // A muted base color derived from the prompt hue.
    let base = hsv_to_rgb(hue, 0.45, 0.85);

    let mut palette = vec![[0u8; 3]; 16];
    palette[C_BORDER as usize] = [30, 33, 40];
    // Background gradient: 8 shades from light to a darker tint of the base.
    for i in 0..8u32 {
        let t = i as f64 / 7.0;
        let factor = 1.0 - 0.55 * t;
        palette[(C_BG_BASE + i as u8) as usize] = scale_rgb(base, factor);
    }
    palette[C_HEADER as usize] = [33, 37, 46];
    palette[C_FOOTER as usize] = [24, 27, 34];
    palette[C_TITLE as usize] = [240, 242, 248];
    palette[C_BODY as usize] = [28, 31, 38];
    palette[C_LABEL as usize] = [255, 214, 102];
    // Remaining entries unused; leave as the border color.
    for entry in palette.iter_mut().skip(14) {
        *entry = [30, 33, 40];
    }
    palette
}

fn scale_rgb(rgb: [u8; 3], factor: f64) -> [u8; 3] {
    [
        (rgb[0] as f64 * factor).clamp(0.0, 255.0) as u8,
        (rgb[1] as f64 * factor).clamp(0.0, 255.0) as u8,
        (rgb[2] as f64 * factor).clamp(0.0, 255.0) as u8,
    ]
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [u8; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    ]
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// ---------------------------------------------------------------------------
// Drawing primitives. A `Canvas` owns the 8-bit palette-index buffer so the
// drawing helpers stay readable (and avoid threading width/height through every
// call).
// ---------------------------------------------------------------------------

/// Glyph cell is 5 wide x 7 tall; advance is 6 px (5 + 1 spacing) per char.
const GLYPH_W: u32 = 5;
const GLYPH_H: u32 = 7;
const GLYPH_ADVANCE: u32 = 6;

struct Canvas {
    buf: Vec<u8>,
    w: u32,
    h: u32,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Self {
            buf: vec![0u8; (w * h) as usize],
            w,
            h,
        }
    }

    #[inline]
    fn set_px(&mut self, x: i64, y: i64, idx: u8) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        self.buf[(y as u32 * self.w + x as u32) as usize] = idx;
    }

    fn fill_rect(&mut self, x: i64, y: i64, rw: i64, rh: i64, idx: u8) {
        for dy in 0..rh {
            for dx in 0..rw {
                self.set_px(x + dx, y + dy, idx);
            }
        }
    }

    fn draw_background(&mut self) {
        let span = (self.w + self.h) as f64;
        for y in 0..self.h {
            for x in 0..self.w {
                let t = (x + y) as f64 / span;
                let shade = (t * 8.0) as u8;
                self.buf[(y * self.w + x) as usize] = C_BG_BASE + shade.min(7);
            }
        }
    }

    fn draw_bars(&mut self) {
        let header_h = (self.h / 8).max(40);
        self.fill_rect(0, 0, self.w as i64, header_h as i64, C_HEADER);
        let footer_h = (self.h / 8).max(44);
        self.fill_rect(
            0,
            (self.h - footer_h) as i64,
            self.w as i64,
            footer_h as i64,
            C_FOOTER,
        );
    }

    fn draw_border(&mut self) {
        let t = (self.w.min(self.h) / 128).max(2) as i64;
        self.fill_rect(0, 0, self.w as i64, t, C_BORDER);
        self.fill_rect(0, (self.h as i64) - t, self.w as i64, t, C_BORDER);
        self.fill_rect(0, 0, t, self.h as i64, C_BORDER);
        self.fill_rect((self.w as i64) - t, 0, t, self.h as i64, C_BORDER);
    }

    fn draw_char(&mut self, x0: i64, y0: i64, ch: char, scale: u32, color: u8) {
        let glyph = glyph(ch);
        let s = scale as i64;
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..GLYPH_W {
                if bits & (1 << (GLYPH_W - 1 - col)) != 0 {
                    self.fill_rect(x0 + col as i64 * s, y0 + row as i64 * s, s, s, color);
                }
            }
        }
    }

    fn draw_text(&mut self, x0: i64, y0: i64, text: &str, scale: u32, color: u8) {
        let mut x = x0;
        for ch in text.chars() {
            self.draw_char(x, y0, ch, scale, color);
            x += (GLYPH_ADVANCE * scale) as i64;
        }
    }

    fn draw_text_centered(&mut self, y0: i64, text: &str, scale: u32, color: u8) {
        let tw = text_width(text, scale);
        let x0 = (self.w as i64 - tw as i64) / 2;
        self.draw_text(x0, y0, text, scale, color);
    }

    fn draw_labels(&mut self, spec: &PlaceholderSpec<'_>) {
        let w = self.w;
        let h = self.h;

        // Header: model + size + quality.
        let title_scale = (w / 320).clamp(2, 8);
        let header = format!(
            "{} | {}X{} | {}",
            spec.model.to_uppercase(),
            spec.width,
            spec.height,
            spec.quality.to_uppercase()
        );
        let header = fit_to_width(
            &header,
            w.saturating_sub(2 * GLYPH_ADVANCE * title_scale),
            title_scale,
        );
        self.draw_text_centered(
            ((h / 8).max(40) as i64 - (GLYPH_H * title_scale) as i64) / 2,
            &header,
            title_scale,
            C_TITLE,
        );

        // Center: the wrapped prompt ("query text").
        let body_scale = (w / 280).clamp(2, 10);
        let usable_w = w
            .saturating_sub(8 * GLYPH_ADVANCE * body_scale)
            .max(GLYPH_ADVANCE * body_scale);
        let max_chars = (usable_w / (GLYPH_ADVANCE * body_scale)).max(1) as usize;
        let prompt = if spec.prompt.trim().is_empty() {
            "(empty prompt)".to_string()
        } else {
            spec.prompt.to_uppercase()
        };
        let max_lines = 6;
        let lines = wrap_text(&prompt, max_chars, max_lines);
        let line_h = (GLYPH_H + 4) * body_scale;
        let block_h = line_h * lines.len() as u32;
        let mut y = (h as i64 - block_h as i64) / 2;
        for line in &lines {
            self.draw_text_centered(y, line, body_scale, C_BODY);
            y += line_h as i64;
        }

        // Footer: the simulated-image watermark.
        let label_scale = (w / 300).clamp(2, 8);
        let label = "LLMSIM SIMULATED IMAGE";
        let label = fit_to_width(
            label,
            w.saturating_sub(2 * GLYPH_ADVANCE * label_scale),
            label_scale,
        );
        let footer_h = (h / 8).max(44);
        let label_y =
            (h - footer_h) as i64 + (footer_h as i64 - (GLYPH_H * label_scale) as i64) / 2;
        self.draw_text_centered(label_y, &label, label_scale, C_LABEL);
    }

    /// Replace each block with its top-left sample to simulate a coarse preview.
    fn pixelate(&mut self, block: u32) {
        let block = block.max(1);
        let (w, h) = (self.w, self.h);
        for by in (0..h).step_by(block as usize) {
            for bx in (0..w).step_by(block as usize) {
                let sample = self.buf[(by * w + bx) as usize];
                for y in by..(by + block).min(h) {
                    for x in bx..(bx + block).min(w) {
                        self.buf[(y * w + x) as usize] = sample;
                    }
                }
            }
        }
    }
}

fn text_width(text: &str, scale: u32) -> u32 {
    if text.is_empty() {
        return 0;
    }
    (text.chars().count() as u32) * GLYPH_ADVANCE * scale - scale
}

/// Truncate a single line to fit the given pixel width, adding an ellipsis.
fn fit_to_width(text: &str, max_w: u32, scale: u32) -> String {
    if text_width(text, scale) <= max_w {
        return text.to_string();
    }
    let per = GLYPH_ADVANCE * scale;
    let max_chars = (max_w / per).max(1) as usize;
    let mut out: String = text.chars().take(max_chars.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

/// Greedy word-wrap into at most `max_lines` lines of `max_chars` each.
fn wrap_text(text: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        // Hard-break words longer than a line.
        let word = if word.chars().count() > max_chars {
            word.chars().take(max_chars).collect::<String>()
        } else {
            word.to_string()
        };
        if current.is_empty() {
            current = word;
        } else if current.chars().count() + 1 + word.chars().count() <= max_chars {
            current.push(' ');
            current.push_str(&word);
        } else {
            lines.push(std::mem::take(&mut current));
            current = word;
            if lines.len() == max_lines {
                break;
            }
        }
    }
    if lines.len() < max_lines && !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    // If we ran out of room, mark truncation on the last line.
    if lines.len() == max_lines {
        if let Some(last) = lines.last_mut() {
            if last.chars().count() + 3 <= max_chars {
                last.push_str("...");
            }
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// Minimal indexed PNG encoder (no external dependencies)
// ---------------------------------------------------------------------------

fn encode_indexed_png(width: u32, height: u32, palette: &[[u8; 3]], indices: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(indices.len() + 1024);
    // PNG signature.
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    // IHDR: width, height, bit depth 8, color type 3 (indexed).
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(3); // color type: indexed
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    write_chunk(&mut out, b"IHDR", &ihdr);

    // PLTE: palette entries (RGB triplets).
    let mut plte = Vec::with_capacity(palette.len() * 3);
    for entry in palette {
        plte.extend_from_slice(entry);
    }
    write_chunk(&mut out, b"PLTE", &plte);

    // IDAT: raw scanlines (filter byte 0 per row) wrapped in a zlib stream.
    let mut raw = Vec::with_capacity((width as usize + 1) * height as usize);
    for y in 0..height {
        raw.push(0); // filter type: none
        let start = (y * width) as usize;
        raw.extend_from_slice(&indices[start..start + width as usize]);
    }
    let zlib = zlib_store(&raw);
    write_chunk(&mut out, b"IDAT", &zlib);

    // IEND.
    write_chunk(&mut out, b"IEND", &[]);
    out
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

/// Wrap data in a zlib stream using uncompressed ("stored") DEFLATE blocks.
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 65535 * 5 + 16);
    out.push(0x78); // CMF
    out.push(0x01); // FLG (no preset dict, fastest)

    let mut offset = 0;
    while offset < data.len() {
        let chunk_len = (data.len() - offset).min(65535);
        let is_last = offset + chunk_len >= data.len();
        out.push(if is_last { 1 } else { 0 }); // BFINAL + BTYPE=00
        let len = chunk_len as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(&data[offset..offset + chunk_len]);
        offset += chunk_len;
    }
    // Handle the empty-data edge case (still need a final block).
    if data.is_empty() {
        out.push(1);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(!0u16).to_le_bytes());
    }

    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Standard base64 encoding (no external dependency). Used to emit images as
/// `b64_json` in the API response, matching the OpenAI image API.
pub fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

struct Crc32 {
    crc: u32,
}

impl Crc32 {
    fn new() -> Self {
        Self { crc: 0xFFFF_FFFF }
    }

    fn update(&mut self, data: &[u8]) {
        for &byte in data {
            let idx = ((self.crc ^ byte as u32) & 0xFF) as usize;
            self.crc = CRC_TABLE[idx] ^ (self.crc >> 8);
        }
    }

    fn finish(self) -> u32 {
        self.crc ^ 0xFFFF_FFFF
    }
}

static CRC_TABLE: std::sync::LazyLock<[u32; 256]> = std::sync::LazyLock::new(|| {
    let mut table = [0u32; 256];
    for (n, entry) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *entry = c;
    }
    table
});

// ---------------------------------------------------------------------------
// 5x7 bitmap font. Each glyph is 7 rows; the low 5 bits of each row are the
// columns, with bit 4 (value 16) the leftmost column.
// ---------------------------------------------------------------------------

fn glyph(ch: char) -> [u8; 7] {
    let upper = ch.to_ascii_uppercase();
    match upper {
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 2, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 17, 25, 21, 19, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 27, 17],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 6, 8, 16, 31],
        '3' => [31, 2, 4, 2, 1, 17, 14],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 30, 1, 1, 17, 14],
        '6' => [6, 8, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 2, 12],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ',' => [0, 0, 0, 0, 12, 4, 8],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '?' => [14, 17, 1, 6, 4, 0, 4],
        '\'' => [4, 4, 8, 0, 0, 0, 0],
        '"' => [10, 10, 10, 0, 0, 0, 0],
        '/' => [1, 2, 4, 4, 8, 16, 16],
        '\\' => [16, 8, 4, 4, 2, 1, 1],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '*' => [0, 10, 4, 31, 4, 10, 0],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        '|' => [4, 4, 4, 4, 4, 4, 4],
        '#' => [10, 10, 31, 10, 31, 10, 10],
        '&' => [12, 18, 20, 8, 21, 18, 13],
        '@' => [14, 17, 19, 21, 23, 16, 14],
        // Unknown characters render as a light tofu box.
        _ => [31, 17, 17, 17, 17, 17, 31],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_header(png: &[u8]) -> (u32, u32, u8, u8) {
        // Signature is 8 bytes, then IHDR chunk: 4 len + 4 type + data.
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(&png[12..16], b"IHDR");
        let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        let bit_depth = png[24];
        let color_type = png[25];
        (w, h, bit_depth, color_type)
    }

    #[test]
    fn renders_exact_requested_size() {
        let png = render_png(&PlaceholderSpec {
            width: 1024,
            height: 1536,
            prompt: "a cat riding a bicycle",
            model: "gpt-image-1",
            quality: "high",
            blockiness: 1,
        });
        let (w, h, bd, ct) = decode_header(&png);
        assert_eq!(w, 1024);
        assert_eq!(h, 1536);
        assert_eq!(bd, 8);
        assert_eq!(ct, 3);
    }

    #[test]
    fn ends_with_iend() {
        let png = render_png(&PlaceholderSpec {
            width: 256,
            height: 256,
            prompt: "test",
            model: "gpt-image-1",
            quality: "low",
            blockiness: 1,
        });
        assert!(png.ends_with(b"IEND") || png.len() > 12);
        // Last 12 bytes are the IEND chunk (4 len + IEND + 4 crc).
        let tail = &png[png.len() - 8..png.len() - 4];
        assert_eq!(tail, b"IEND");
    }

    #[test]
    fn deterministic_for_same_prompt() {
        let spec = || PlaceholderSpec {
            width: 512,
            height: 512,
            prompt: "deterministic prompt",
            model: "gpt-image-1",
            quality: "medium",
            blockiness: 1,
        };
        assert_eq!(render_png(&spec()), render_png(&spec()));
    }

    #[test]
    fn blockiness_changes_output() {
        let base = PlaceholderSpec {
            width: 256,
            height: 256,
            prompt: "preview",
            model: "gpt-image-1",
            quality: "low",
            blockiness: 1,
        };
        let coarse = PlaceholderSpec {
            blockiness: 32,
            ..base_like(&base)
        };
        assert_ne!(render_png(&base), render_png(&coarse));
    }

    // Helper to clone a spec with a different blockiness (PlaceholderSpec holds
    // borrows, so reconstruct it explicitly).
    fn base_like<'a>(s: &PlaceholderSpec<'a>) -> PlaceholderSpec<'a> {
        PlaceholderSpec {
            width: s.width,
            height: s.height,
            prompt: s.prompt,
            model: s.model,
            quality: s.quality,
            blockiness: s.blockiness,
        }
    }

    #[test]
    fn base64_known_values() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn crc_and_adler_known_values() {
        // Adler32 of "abc" is 0x024d0127.
        assert_eq!(adler32(b"abc"), 0x024d_0127);
        // CRC32 of "123456789" is 0xCBF43926.
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xCBF4_3926);
    }
}
