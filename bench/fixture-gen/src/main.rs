use std::fs;
use mozjpeg::{ColorSpace, Compress, Marker};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn noise(&mut self, amp: i32) -> i32 { (self.next() % (2 * amp as u32 + 1)) as i32 - amp }
}

fn photo(w: usize, h: usize, seed: u64) -> Vec<u8> {
    let mut rng = Lcg(seed);
    let mut px = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let fx = x as f64 / w as f64;
            let fy = y as f64 / h as f64;
            let base = [
                120.0 + 90.0 * (fx * 6.0).sin() * (fy * 3.0).cos(),
                110.0 + 80.0 * (fy * 5.0 + fx * 2.0).sin(),
                100.0 + 100.0 * fx * (1.0 - fy),
            ];
            // A hard-edged disc, so there are edges for the metric to see.
            let (dx, dy) = (fx - 0.7, fy - 0.4);
            let disc = if dx * dx + dy * dy < 0.03 { 70.0 } else { 0.0 };
            for c in 0..3 {
                let v = base[c] + disc + rng.noise(7) as f64;
                px.push(v.clamp(0.0, 255.0) as u8);
            }
        }
    }
    px
}

fn write_png(path: &str, w: u32, h: u32, color: png::ColorType, data: &[u8], palette: Option<(&[u8], &[u8])>, icc: Option<&[u8]>) {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(color);
        e.set_depth(png::BitDepth::Eight);
        if let Some((pal, trns)) = palette {
            e.set_palette(pal.to_vec());
            if !trns.is_empty() { e.set_trns(trns.to_vec()); }
        }
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(data).unwrap();
        wr.finish().unwrap();
    }
    if let Some(icc) = icc {
        // iCCP: name, NUL, method 0, then stored-zlib data. Built by hand so the fixture carries a real chunk.
        let mut body = b"test\0\0".to_vec();
        let mut z = vec![0x78, 0x01, 0x01];
        z.extend_from_slice(&(icc.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(icc.len() as u16)).to_le_bytes());
        z.extend_from_slice(icc);
        let (mut a, mut b) = (1u32, 0u32);
        for &x in icc { a = (a + x as u32) % 65521; b = (b + a) % 65521; }
        z.extend_from_slice(&((b << 16) | a).to_be_bytes());
        body.extend_from_slice(&z);
        let mut chunk = (body.len() as u32).to_be_bytes().to_vec();
        let mut crc_in = b"iCCP".to_vec();
        crc_in.extend_from_slice(&body);
        chunk.extend_from_slice(b"iCCP");
        chunk.extend_from_slice(&body);
        chunk.extend_from_slice(&crc32(&crc_in).to_be_bytes());
        out.splice(33..33, chunk);
    }
    fs::write(path, out).unwrap();
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 { crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 }; }
    }
    !crc
}

fn exif_orientation(o: u8) -> Vec<u8> {
    // Orientation plus a GPS pointer, so the strip test has something to remove.
    let mut v = b"Exif\0\0MM\x00\x2A\x00\x00\x00\x08\x00\x02".to_vec();
    v.extend_from_slice(&[0x01, 0x12, 0x00, 0x03, 0, 0, 0, 1, 0, o, 0, 0]);
    v.extend_from_slice(&[0x88, 0x25, 0x00, 0x04, 0, 0, 0, 1, 0, 0, 0, 0x1A]);
    v.extend_from_slice(&[0, 0, 0, 0]);
    v.extend_from_slice(b"GPSLatitude=38.7223N GPSLongitude=9.1393W padding padding padding");
    v
}

fn write_jpeg(path: &str, w: usize, h: usize, gray: bool, data: &[u8], q: f32, icc: Option<&[u8]>, orientation: Option<u8>) {
    let mut c = Compress::new(if gray { ColorSpace::JCS_GRAYSCALE } else { ColorSpace::JCS_RGB });
    c.set_size(w, h);
    c.set_quality(q);
    // A deliberately wasteful encode: baseline, unoptimized tables, so there is room to improve.
    c.set_fastest_defaults();
    c.set_quality(q);
    let mut s = c.start_compress(Vec::new()).unwrap();
    if let Some(icc) = icc {
        // Written by hand: the mozjpeg crate's helper numbers ICC chunks from 0, the spec says 1.
        let mut p = b"ICC_PROFILE\0".to_vec();
        p.extend_from_slice(&[1, 1]);
        p.extend_from_slice(icc);
        s.write_marker(Marker::APP(2), &p);
    }
    if let Some(o) = orientation { s.write_marker(Marker::APP(1), &exif_orientation(o)); }
    s.write_marker(Marker::COM, b"made by the squish fixture generator");
    s.write_scanlines(data).unwrap();
    fs::write(path, s.finish().unwrap()).unwrap();
}

fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let p = |n: &str| format!("{dir}/{n}");
    let fake_icc: Vec<u8> = (0..400u32).map(|i| (i * 7 % 251) as u8).collect();

    let (w, h) = (192usize, 128usize);
    let rgb = photo(w, h, 1);
    write_png(&p("photo-rgb.png"), w as u32, h as u32, png::ColorType::Rgb, &rgb, None, None);
    write_png(&p("photo-icc.png"), w as u32, h as u32, png::ColorType::Rgb, &rgb, None, Some(&fake_icc));

    // Logo-like: a soft-edged disc on a transparent background.
    let (lw, lh) = (96usize, 96usize);
    let mut rgba = Vec::new();
    for y in 0..lh { for x in 0..lw {
        let (dx, dy) = (x as f64 - 48.0, y as f64 - 48.0);
        let d = (dx * dx + dy * dy).sqrt();
        let a = ((40.0 - d) * 12.0).clamp(0.0, 255.0) as u8;
        rgba.extend_from_slice(&[(200.0 - d * 2.0) as u8, (60.0 + d * 3.0) as u8, 180, a]);
    }}
    write_png(&p("logo-alpha.png"), lw as u32, lh as u32, png::ColorType::Rgba, &rgba, None, None);

    let gray: Vec<u8> = rgb.chunks(3).map(|c| ((c[0] as u32 + c[1] as u32 + c[2] as u32) / 3) as u8).collect();
    write_png(&p("photo-gray.png"), w as u32, h as u32, png::ColorType::Grayscale, &gray, None, None);

    // Already a 6-color palette image with one transparent entry.
    let pal = [255, 255, 255, 0, 0, 0, 220, 30, 30, 30, 160, 30, 30, 30, 220, 240, 200, 20];
    let trns = [0u8];
    let idx: Vec<u8> = (0..64 * 64).map(|i| ((i / 64 / 11 + i % 64 / 11) % 6) as u8).collect();
    write_png(&p("palette.png"), 64, 64, png::ColorType::Indexed, &idx, Some((&pal, &trns)), None);

    write_jpeg(&p("photo.jpg"), w, h, false, &rgb, 92.0, None, None);
    write_jpeg(&p("photo-meta.jpg"), w, h, false, &rgb, 92.0, Some(&fake_icc), Some(6));
    write_jpeg(&p("photo-gray.jpg"), w, h, true, &gray, 92.0, None, None);

    // APNG: a real PNG with an acTL chunk spliced in before IDAT.
    let mut apng = fs::read(p("palette.png")).unwrap();
    let mut actl = 8u32.to_be_bytes().to_vec();
    let mut body = b"acTL".to_vec();
    body.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, 0]);
    actl.extend_from_slice(&body);
    actl.extend_from_slice(&crc32(&body).to_be_bytes());
    apng.splice(33..33, actl);
    fs::write(p("animated.png"), apng).unwrap();
}
