//! Generates everything derived from `design/tokens.json`:
//!
//! - `generated/tokens.hex.json`: the tokens with `{ "oklch", "hex" }` in place of every OKLCH string
//! - `generated/contrast.md`: the WCAG contrast checks from the `contrast` group
//! - `generated/swatches.svg` and `.png`: the swatch sheet with the logo on dark and light
//! - `logo/*.svg` and `logo/png/icon-<size>.png`: the mark in the accent colour
//!
//! `--check` writes nothing and fails when a text output is stale. Both modes fail when a
//! contrast check is below its minimum.

use serde_json::{Map, Value, json};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const ICON_SIZES: [u32; 8] = [16, 32, 48, 64, 128, 256, 512, 1024];

fn main() -> ExitCode {
    let check = std::env::args().any(|a| a == "--check");
    let design = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    match run(&design, check) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(design: &Path, check: bool) -> Result<(), String> {
    let src = std::fs::read_to_string(design.join("tokens.json")).map_err(|e| e.to_string())?;
    let tokens: Value = serde_json::from_str(&src).map_err(|e| format!("tokens.json: {e}"))?;
    let palette = Palette(palette_hex(&tokens)?);
    let hex = |name| palette.hex(name);

    let (contrast, failures) = contrast_report(&tokens, &palette)?;
    let (bg, fg, accent) = (hex("background")?, hex("foreground")?, hex("accent")?);

    let mut text: Vec<(PathBuf, String)> = vec![
        (
            design.join("generated/tokens.hex.json"),
            pretty(&with_hex(&tokens)?),
        ),
        (design.join("generated/contrast.md"), contrast),
        (
            design.join("generated/swatches.svg"),
            swatch_sheet(&tokens, &palette)?,
        ),
        (design.join("logo/mark.svg"), mark_svg(accent)),
        (design.join("logo/mark-mono.svg"), mark_svg("currentColor")),
        (design.join("logo/mark-on-dark.svg"), tile_svg(bg, accent)),
        (design.join("logo/mark-on-light.svg"), tile_svg(fg, accent)),
    ];

    if check {
        let stale: Vec<String> = text
            .iter()
            .filter(|(p, s)| std::fs::read_to_string(p).ok().as_deref() != Some(s.as_str()))
            .map(|(p, _)| p.strip_prefix(design).unwrap_or(p).display().to_string())
            .collect();
        if !stale.is_empty() {
            return Err(format!("stale, run the generator: {}", stale.join(", ")));
        }
    } else {
        std::fs::create_dir_all(design.join("logo/png")).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(design.join("generated")).map_err(|e| e.to_string())?;
        for (path, body) in text.drain(..) {
            std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let tile = tile_svg(bg, accent);
        for size in ICON_SIZES {
            render(
                &tile,
                size,
                &design.join(format!("logo/png/icon-{size}.png")),
            )?;
        }
        let sheet = std::fs::read_to_string(design.join("generated/swatches.svg"))
            .map_err(|e| e.to_string())?;
        render(&sheet, 1920, &design.join("generated/swatches.png"))?;
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("contrast below minimum: {}", failures.join(", ")))
    }
}

// ---------------------------------------------------------------- colour

/// Parses `oklch(L C H)` or `oklch(L C H / A%)`; L in 0..1, H in degrees.
fn parse_oklch(s: &str) -> Option<(f64, f64, f64, Option<f64>)> {
    let inner = s.trim().strip_prefix("oklch(")?.strip_suffix(')')?;
    let (lch, alpha) = match inner.split_once('/') {
        Some((lch, a)) => {
            let a = a.trim();
            let a = match a.strip_suffix('%') {
                Some(p) => p.parse::<f64>().ok()? / 100.0,
                None => a.parse::<f64>().ok()?,
            };
            (lch, Some(a))
        }
        None => (inner, None),
    };
    let v: Vec<f64> = lch
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    match v[..] {
        [l, c, h] => Some((l, c, h, alpha)),
        _ => None,
    }
}

/// OKLCH to linear sRGB (Björn Ottosson's OKLab matrices).
fn oklch_to_linear_srgb(l: f64, c: f64, h: f64) -> [f64; 3] {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    [
        4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3,
        -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701_0 * s3,
    ]
}

fn encode_srgb(x: f64) -> u8 {
    let x = x.clamp(0.0, 1.0);
    let v = if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round() as u8
}

/// `#rrggbb`, or `#rrggbbaa` when the value carries an alpha. Out-of-gamut channels are clamped.
fn oklch_to_hex(s: &str) -> Option<String> {
    let (l, c, h, alpha) = parse_oklch(s)?;
    let [r, g, b] = oklch_to_linear_srgb(l, c, h).map(encode_srgb);
    Some(match alpha {
        Some(a) => format!(
            "#{r:02x}{g:02x}{b:02x}{:02x}",
            (a.clamp(0.0, 1.0) * 255.0).round() as u8
        ),
        None => format!("#{r:02x}{g:02x}{b:02x}"),
    })
}

/// WCAG 2.x relative luminance of `#rrggbb`.
fn luminance(hex: &str) -> f64 {
    let ch = |i: usize| {
        let v = u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(1) + 0.7152 * ch(3) + 0.0722 * ch(5)
}

fn contrast(a: &str, b: &str) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

// ---------------------------------------------------------------- tokens

/// Every OKLCH string becomes `{ "oklch": …, "hex": … }`; everything else is copied.
fn with_hex(v: &Value) -> Result<Value, String> {
    Ok(match v {
        Value::String(s) if s.starts_with("oklch(") => {
            json!({ "oklch": s, "hex": oklch_to_hex(s).ok_or(format!("bad colour {s}"))? })
        }
        Value::Object(m) => {
            let mut out = Map::new();
            for (k, v) in m {
                out.insert(k.clone(), with_hex(v)?);
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(with_hex).collect::<Result<_, _>>()?),
        other => other.clone(),
    })
}

/// Palette token name to `#rrggbb`.
struct Palette(Map<String, Value>);

impl Palette {
    fn hex(&self, name: &str) -> Result<&str, String> {
        self.0
            .get(name)
            .and_then(Value::as_str)
            .ok_or(format!("palette.{name} missing"))
    }
}

fn palette_hex(tokens: &Value) -> Result<Map<String, Value>, String> {
    let palette = tokens["palette"]
        .as_object()
        .ok_or("tokens.json has no palette")?;
    let mut out = Map::new();
    for (k, v) in palette.iter().filter(|(k, _)| !k.starts_with('$')) {
        let s = v.as_str().ok_or(format!("palette.{k} is not a string"))?;
        out.insert(
            k.clone(),
            Value::String(oklch_to_hex(s).ok_or(format!("palette.{k}: bad colour {s}"))?),
        );
    }
    Ok(out)
}

fn contrast_report(tokens: &Value, palette: &Palette) -> Result<(String, Vec<String>), String> {
    let hex = |name| palette.hex(name);
    let checks = tokens["contrast"]["checks"]
        .as_array()
        .ok_or("tokens.json has no contrast.checks")?;
    let mut md = String::from(
        "<!-- Generated by design/tools from design/tokens.json. Do not edit. -->\n\n\
         # Contrast (WCAG 2.x)\n\n\
         4.5 = AA for text, 3.0 = AA for UI components and large text.\n\n\
         | Foreground | Background | Ratio | Minimum | Result |\n|---|---|---|---|---|\n",
    );
    let mut failures = Vec::new();
    for c in checks {
        let (fg, bg) = (
            c["fg"].as_str().unwrap_or(""),
            c["bg"].as_str().unwrap_or(""),
        );
        let min = c["min"].as_f64().ok_or("contrast check without min")?;
        let ratio = contrast(hex(fg)?, hex(bg)?);
        let ok = ratio >= min;
        if !ok {
            failures.push(format!("{fg} on {bg} {ratio:.2} < {min}"));
        }
        let _ = writeln!(
            md,
            "| `{fg}` {} | `{bg}` {} | {ratio:.2}:1 | {min}:1 | {} |",
            hex(fg)?,
            hex(bg)?,
            if ok { "pass" } else { "FAIL" }
        );
    }
    Ok((md, failures))
}

fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s
}

// ---------------------------------------------------------------- logo

/// The Exponential mark (circle with three curves cut out) in a 100×100 box.
fn mark_defs(id: &str, stroke: f64) -> String {
    format!(
        r#"<clipPath id="{id}-clip"><circle cx="50" cy="50" r="50"/></clipPath>
    <mask id="{id}-mask">
      <rect x="-11" y="-11" width="122" height="122" fill="white"/>
      <g clip-path="url(#{id}-clip)" stroke="black" stroke-width="{stroke}" fill="none">
        <path d="M -5.87 62.01 C 39.09 65.44 48.72 28.71 49.03 -6.21"/>
        <path d="M -5.07 86.00 C 53.78 84.42 71.13 37.29 73.00 -5.09"/>
        <path d="M -4.27 109.99 C 68.46 103.40 93.55 45.86 96.98 -3.98"/>
      </g>
    </mask>"#
    )
}

fn mark_svg(fill: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 100 100">
  <!-- Generated by design/tools from design/tokens.json. Do not edit. -->
  <defs>
    {}
  </defs>
  <circle cx="50" cy="50" r="50" fill="{fill}" mask="url(#mark-mask)"/>
</svg>
"#,
        mark_defs("mark", 3.5)
    )
}

/// App-icon tile: Apple icon grid (824/1024 square, 185 corner radius), mark padded inside,
/// the same geometry as Exponential's `logo-macos.svg`.
fn tile_svg(plate: &str, fill: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <!-- Generated by design/tools from design/tokens.json. Do not edit. -->
  <defs>
    {}
  </defs>
  <rect x="100" y="100" width="824" height="824" rx="185" fill="{plate}"/>
  <g transform="translate(192 192) scale(6.4)">
    <circle cx="50" cy="50" r="50" fill="{fill}" mask="url(#mark-mask)"/>
  </g>
</svg>
"#,
        mark_defs("mark", 6.0)
    )
}

// ---------------------------------------------------------------- swatch sheet

fn swatch_sheet(tokens: &Value, palette_hex: &Palette) -> Result<String, String> {
    let hex = |name| palette_hex.hex(name);
    let palette = tokens["palette"].as_object().ok_or("no palette")?;
    let (bg, fg, mfg, border, accent) = (
        hex("background")?,
        hex("foreground")?,
        hex("mutedForeground")?,
        hex("border")?,
        hex("accent")?,
    );
    let font = r#"font-family="Inter, Helvetica Neue, Helvetica, Arial, sans-serif""#;
    let names: Vec<&String> = palette.keys().filter(|k| !k.starts_with('$')).collect();
    let (cols, cell_w, cell_h, x0, y0) = (4usize, 216.0, 112.0, 64.0, 360.0);
    let rows = names.len().div_ceil(cols);
    let height = y0 + rows as f64 * cell_h + 32.0;

    let mut s = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="960" height="{height}" viewBox="0 0 960 {height}">
  <!-- Generated by design/tools from design/tokens.json. Do not edit. -->
  <defs>
    {}
  </defs>
  <rect width="960" height="{height}" fill="{bg}"/>
  <text x="64" y="72" {font} font-size="32" font-weight="600" fill="{fg}">Rerusted palette</text>
  <text x="64" y="104" {font} font-size="15" fill="{mfg}">design/tokens.json · dark</text>
"#,
        mark_defs("mark", 6.0)
    );
    // The logo on dark and on light.
    for (i, (plate, label)) in [(bg, "on dark"), (fg, "on light")].iter().enumerate() {
        let x = 64.0 + i as f64 * 232.0;
        let _ = write!(
            s,
            r#"  <g transform="translate({x} 136) scale(0.1875)"><rect x="100" y="100" width="824" height="824" rx="185" fill="{plate}" stroke="{border}" stroke-width="6"/><g transform="translate(192 192) scale(6.4)"><circle cx="50" cy="50" r="50" fill="{accent}" mask="url(#mark-mask)"/></g></g>
  <text x="{tx}" y="340" {font} font-size="13" fill="{mfg}">{label}</text>
"#,
            tx = x + 20.0
        );
    }
    for (i, name) in names.iter().enumerate() {
        let (x, y) = (
            x0 + (i % cols) as f64 * cell_w,
            y0 + (i / cols) as f64 * cell_h,
        );
        let oklch = palette[name.as_str()].as_str().unwrap_or("");
        let _ = write!(
            s,
            r#"  <rect x="{x}" y="{y}" width="200" height="44" rx="10" fill="{h}" stroke="{border}"/>
  <text x="{x}" y="{ty}" {font} font-size="13" font-weight="600" fill="{fg}">{name}</text>
  <text x="{x}" y="{ty2}" {font} font-size="12" fill="{mfg}">{h} · {oklch}</text>
"#,
            h = hex(name)?,
            ty = y + 64.0,
            ty2 = y + 82.0,
        );
    }
    s.push_str("</svg>\n");
    Ok(s)
}

// ---------------------------------------------------------------- raster

fn render(svg: &str, width: u32, out: &Path) -> Result<(), String> {
    let mut opt = resvg::usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_str(svg, &opt).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = width as f32 / size.width();
    let height = (size.height() * scale).round() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or("empty pixmap")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .save_png(out)
        .map_err(|e| format!("{}: {e}", out.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hex column of the #RUST-5 token table.
    #[test]
    fn matches_the_rust_5_table() {
        for (oklch, hex) in [
            ("oklch(0.145 0.012 35)", "#0f0807"),
            ("oklch(0.205 0.016 35)", "#1e1512"),
            ("oklch(0.269 0.02 35)", "#2f2320"),
            ("oklch(0.42 0.04 36)", "#61463e"),
            ("oklch(0.708 0.03 40)", "#b29b93"),
            ("oklch(0.985 0.004 60)", "#fcf9f7"),
            ("oklch(0.56 0.15 42)", "#ba4f1f"),
        ] {
            assert_eq!(oklch_to_hex(oklch).as_deref(), Some(hex), "{oklch}");
        }
    }

    #[test]
    fn alpha_and_contrast() {
        assert_eq!(
            oklch_to_hex("oklch(1 0 0 / 10%)").as_deref(),
            Some("#ffffff1a")
        );
        assert!((contrast("#ffffff", "#000000") - 21.0).abs() < 1e-9);
    }
}
