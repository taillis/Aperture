//! `showcase` layout: full-bleed game poster / hero background with transparent
//! game logo on the left and translucent telemetry cards on the right.

use super::*;
use image::imageops::FilterType;

const GLASS_BG: Rgb = (10, 12, 18);
const CARD_LIGHT: Rgb = (242, 245, 250);
const CARD_MUTED: Rgb = (182, 190, 206);
const CARD_DIM: Rgb = (132, 140, 158);

/// Prepares or reuses a cached, Cover-fitted, dimmed + scrimmed background layer for `showcase`.
pub(super) fn prepare_bg(
    cache: &mut Option<(String, Layer)>,
    d: &Data,
    poster_path: &str,
    bg_dim: u32,
    w: u32,
    h: u32,
) {
    if w == 0 || h == 0 {
        return;
    }
    let poster_trimmed = poster_path.trim();
    let key = if !poster_trimmed.is_empty() {
        format!("file:{poster_trimmed}:{bg_dim}:{w}x{h}")
    } else if let Some(game) = d.game {
        if game.hero.is_some() {
            format!("hero:{}:{bg_dim}:{w}x{h}", game.key)
        } else if game.poster.is_some() {
            format!("poster:{}:{bg_dim}:{w}x{h}", game.key)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    if key.is_empty() {
        *cache = None;
        return;
    }
    if cache
        .as_ref()
        .is_some_and(|(k, layer)| k == &key && layer.w == w as usize && layer.h == h as usize)
    {
        return;
    }

    let rgb_opt = if !poster_trimmed.is_empty() {
        load_custom_poster_rgb(poster_trimmed, w, h)
    } else if let Some(game) = d.game {
        game.hero
            .as_ref()
            .or(game.poster.as_ref())
            .and_then(|gp| fit_cover_rgb(&gp.rgb, gp.width, gp.height, w, h))
    } else {
        None
    };

    let Some(mut rgb) = rgb_opt else {
        *cache = None;
        return;
    };

    apply_dim_and_scrim(&mut rgb, w, h, bg_dim);
    let mut layer = Layer::new(w, h);
    layer.fill_opaque_rgb(&rgb);
    *cache = Some((key, layer));
}

fn load_custom_poster_rgb(path: &str, tw: u32, th_px: u32) -> Option<Vec<u8>> {
    let resolved = if std::path::Path::new(path).is_file() {
        std::path::PathBuf::from(path)
    } else {
        std::env::current_exe().ok()?.parent()?.join(path)
    };
    let mut reader = image::ImageReader::open(&resolved).ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    reader.limits(limits);
    let img = reader.decode().ok()?.to_rgb8();
    let (sw, sh) = img.dimensions();
    fit_cover_rgb(img.as_raw(), sw, sh, tw, th_px)
}

fn fit_cover_rgb(src: &[u8], sw: u32, sh: u32, tw: u32, th_px: u32) -> Option<Vec<u8>> {
    if sw == 0 || sh == 0 || tw == 0 || th_px == 0 || src.len() < (sw * sh * 3) as usize {
        return None;
    }
    let img = image::RgbImage::from_raw(sw, sh, src[..(sw * sh * 3) as usize].to_vec())?;
    let scale = (tw as f32 / sw as f32).max(th_px as f32 / sh as f32);
    let rw = (sw as f32 * scale).ceil().max(tw as f32) as u32;
    let rh = (sh as f32 * scale).ceil().max(th_px as f32) as u32;
    let resized = image::imageops::resize(&img, rw, rh, FilterType::Triangle);
    let cx = (rw - tw) / 2;
    let cy = (rh - th_px) / 2;
    let cropped = image::imageops::crop_imm(&resized, cx, cy, tw, th_px).to_image();
    Some(cropped.into_raw())
}

/// Applies user dimming + cinematic left-to-right readability scrim onto RGB888 buffer.
fn apply_dim_and_scrim(rgb: &mut [u8], w: u32, h: u32, bg_dim: u32) {
    let dim_f = (100 - bg_dim.min(100)) as f32 / 100.0;
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    for y in 0..h {
        let fy = y as f32 / hf;
        let edge_v = ((0.5 - fy).abs() * 2.0).powi(2) * 0.24;
        for x in 0..w {
            let fx = x as f32 / wf;
            // Stronger dark scrim under the left logo stage, gentler gradient under right telemetry.
            let scrim = if fx < 0.44 {
                0.62 - 0.22 * (fx / 0.44)
            } else {
                0.40 - 0.14 * ((fx - 0.44) / 0.56)
            };
            let keep = dim_f * (1.0 - (scrim + edge_v).clamp(0.0, 0.88));
            let i = ((y * w + x) * 3) as usize;
            rgb[i] = (rgb[i] as f32 * keep) as u8;
            rgb[i + 1] = (rgb[i + 1] as f32 * keep) as u8;
            rgb[i + 2] = (rgb[i + 2] as f32 * keep) as u8;
        }
    }
}

/// Paints the `showcase` dashboard.
pub(super) fn paint(
    p: &mut Painter,
    w: f32,
    h: f32,
    sc: &Scale,
    d: &Data,
    now: &chrono::DateTime<Local>,
    hist: (&VecDeque<f32>, &VecDeque<f32>, &VecDeque<f32>),
    cover: Option<&Cover>,
    bg_layer: Option<&Layer>,
) {
    let k = sc.k;
    if let Some(bg) = bg_layer {
        p.layer.copy_from(bg);
    } else {
        paint_fallback_backdrop(p, w, h);
    }

    let m = (16.0 * k).round();
    let gap = (12.0 * k).round();
    let left_w = ((w - 2.0 * m - gap) * 0.365).round();
    let right_x = m + left_w + gap;
    let right_w = w - m - right_x;
    let col_w = ((right_w - gap) / 2.0).floor();
    let row_h = ((h - 2.0 * m - gap) / 2.0).floor();

    paint_left_stage(p, (m, m, left_w, h - 2.0 * m), sc, d, now, cover);

    // Right 2x2 translucent glass grid: GAME FPS, CPU, GPU, SYSTEM
    paint_fps_card(p, (right_x, m, col_w, row_h), sc, d);
    paint_cpu_card(p, (right_x + col_w + gap, m, right_w - col_w - gap, row_h), sc, d, hist.0);
    paint_gpu_card(p, (right_x, m + row_h + gap, col_w, h - 2.0 * m - row_h - gap), sc, d);
    paint_system_card(
        p,
        (right_x + col_w + gap, m + row_h + gap, right_w - col_w - gap, h - 2.0 * m - row_h - gap),
        sc,
        d,
    );
}

fn paint_fallback_backdrop(p: &mut Painter, w: f32, h: f32) {
    let c0 = th().bg;
    let c1 = lerp_rgb(th().bg, th().cpu[0], 0.22);
    let c2 = lerp_rgb(th().bg, th().gpu[0], 0.16);
    let wi = w.ceil() as i32;
    let hi = h.ceil() as i32;
    for y in 0..hi {
        let fy = y as f32 / h.max(1.0);
        for x in 0..wi {
            let fx = x as f32 / w.max(1.0);
            let t = (fx * 0.65 + (1.0 - fy) * 0.35).clamp(0.0, 1.0);
            let c = lerp_rgb(lerp_rgb(c0, c1, fx), c2, t * 0.55);
            p.layer.over(x, y, c, 255);
        }
    }
}

fn glass_card(p: &mut Painter, r: R, k: f32, accent: Rgb) {
    let r = (r.0.round(), r.1.round(), r.2.round(), r.3.round());
    let rad = 16.0 * k;
    p.layer.fill_rrect_solid(r, rad, GLASS_BG, 188);
    p.layer.stroke_rrect(r, rad, 1.0, WHITE, 36);
    capsule(
        p,
        (r.0 + rad, r.1 + 1.0),
        (r.0 + r.2 - rad, r.1 + 1.0),
        0.7,
        accent,
        155,
    );
}

fn glass_header(p: &mut Painter, r: R, k: f32, accent: Rgb, title: &str, right: &str) -> f32 {
    let pd = 16.0 * k;
    let cap = 12.5 * k;
    let base = r.1 + pd + cap;
    circle(p, r.0 + pd + 4.0 * k, base - cap / 2.0, 4.0 * k, accent, 255);
    spaced(p, r.0 + pd + 15.0 * k, base, title, cap, W::Medium, CARD_MUTED, 1.8 * k);
    if !right.is_empty() {
        let rw = p.width(right, cap, W::Regular);
        p.text(r.0 + r.2 - pd - rw, base, right, cap, W::Regular, CARD_DIM, 255);
    }
    base
}

fn paint_left_stage(
    p: &mut Painter,
    r: R,
    sc: &Scale,
    d: &Data,
    now: &chrono::DateTime<Local>,
    cover: Option<&Cover>,
) {
    let k = sc.k;
    let (x, y, w, h) = r;
    let pd = 14.0 * k;
    let stats = STATS.with(|s| s.borrow().clone());

    // Top row: status pill on left, clock + weather on right.
    let (pill_text, pill_col) = if let Some(game) = d.game {
        let src = if game.source == "Detected" {
            "IN GAME".to_string()
        } else {
            format!("IN GAME · {}", game.source.to_ascii_uppercase())
        };
        (src, th().fps)
    } else if stats.gaming {
        ("IN GAME".to_string(), th().fps)
    } else {
        ("SHOWCASE".to_string(), th().cpu[0])
    };

    let pcap = 11.5 * k;
    let pill_tw = spaced_width(p, &pill_text, pcap, W::Medium, 1.4 * k);
    let pill_w = (pill_tw + 38.0 * k).min(w * 0.62);
    let pill_h = 28.0 * k;
    p.layer
        .fill_rrect_solid((x + pd, y + 4.0 * k, pill_w, pill_h), pill_h / 2.0, GLASS_BG, 200);
    p.layer
        .stroke_rrect((x + pd, y + 4.0 * k, pill_w, pill_h), pill_h / 2.0, 1.0, pill_col, 130);
    circle(p, x + pd + 13.0 * k, y + 4.0 * k + pill_h / 2.0, 4.0 * k, pill_col, 255);
    spaced(
        p,
        x + pd + 24.0 * k,
        y + 4.0 * k + pill_h / 2.0 + pcap * 0.45,
        &pill_text,
        pcap,
        W::Medium,
        CARD_LIGHT,
        1.4 * k,
    );

    // Clock & weather top-right of left stage
    let time_str = clock_parts(now).0;
    let wx_str = d
        .weather
        .map(|wt| format!("  ·  {:.0}°", wt.temp_in(d.fahrenheit).round() + 0.0))
        .unwrap_or_default();
    let top_meta = format!("{time_str}{wx_str}");
    let mcap = 15.0 * k;
    let mw = p.width(&top_meta, mcap, W::Medium);
    p.text(
        x + w - pd - mw,
        y + 4.0 * k + pill_h / 2.0 + mcap * 0.45,
        &top_meta,
        mcap,
        W::Medium,
        CARD_LIGHT,
        245,
    );

    // Bottom footer area (reserved when background music is playing alongside a game).
    let footer_h = if d.track.is_some() { 40.0 * k } else { 10.0 * k };
    let center_top = y + 44.0 * k;
    let center_h = (h - 44.0 * k - footer_h).max(80.0 * k);

    if let Some(game) = d.game {
        if let Some(logo) = &game.logo {
            draw_transparent_logo(p, logo, (x + pd, center_top, w - 2.0 * pd, center_h - 32.0 * k));
            let sub = ellipsize(p, &game.title, sc.m, W::Medium, w - 2.0 * pd);
            p.text(
                x + pd,
                center_top + center_h - 6.0 * k,
                &sub,
                sc.m,
                W::Medium,
                CARD_MUTED,
                235,
            );
        } else {
            // Cover thumbnail (when available) + bold stacked title.
            let mut text_x = x + pd;
            let mut text_w = w - 2.0 * pd;
            if let Some(c) = cover {
                let cs = (center_h * 0.72).min(130.0 * k).round();
                let cy = center_top + (center_h - cs) / 2.0;
                blit_cover(p, c, text_x, cy, cs, 14.0 * k);
                p.layer.stroke_rrect((text_x, cy, cs, cs), 14.0 * k, 1.0, WHITE, 48);
                text_x += cs + 18.0 * k;
                text_w = (x + w - pd - text_x).max(60.0 * k);
            }
            let title = ellipsize(p, &game.title, sc.xl * 0.92, W::Medium, text_w);
            let tcap = p.fit(&title, sc.xl * 0.92, text_w, W::Medium);
            let cy = center_top + center_h * 0.52;
            p.text(text_x, cy, &title, tcap, W::Medium, CARD_LIGHT, 255);
            let exe_sub = ellipsize(p, &format!("{}.exe", game.exe_name), sc.s, W::Regular, text_w);
            p.text(text_x, cy + 16.0 * k + sc.s, &exe_sub, sc.s, W::Regular, CARD_DIM, 230);
        }
    } else {
        // Idle showcase: clock centerpiece + date + optional now-playing line.
        let tcap = p.fit(&time_str, sc.xl * 1.6, w - 2.0 * pd, W::Light);
        let tbase = center_top + center_h * 0.54;
        p.text(x + pd, tbase, &time_str, tcap, W::Light, CARD_LIGHT, 255);
        let date = crate::tiles::nice_case(&i18n::day_month_year(now));
        let dline = format!("{}  ·  {}", i18n::weekday(now), date);
        let dline = ellipsize(p, &dline, sc.m, W::Regular, w - 2.0 * pd);
        p.text(x + pd, tbase + 18.0 * k + sc.m, &dline, sc.m, W::Regular, CARD_MUTED, 235);
    }

    if let Some(track) = d.track {
        // Compact glass music pill at bottom-left (shown if music is playing alongside a game).
        let py = y + h - 32.0 * k;
        let line = if track.artist.trim().is_empty() {
            track.title.clone()
        } else {
            format!("{} — {}", track.title, track.artist)
        };
        let label = ellipsize(p, &line, sc.s, W::Regular, w - 2.0 * pd - 46.0 * k);
        let pw = (p.width(&label, sc.s, W::Regular) + 42.0 * k).min(w - 2.0 * pd);
        p.layer.fill_rrect_solid((x + pd, py, pw, 28.0 * k), 14.0 * k, GLASS_BG, 188);
        p.layer.stroke_rrect((x + pd, py, pw, 28.0 * k), 14.0 * k, 1.0, WHITE, 28);
        music_note(p, x + pd + 15.0 * k, py + 14.0 * k, 6.2 * k, th().cpu[0]);
        p.text(x + pd + 28.0 * k, py + 14.0 * k + sc.s * 0.42, &label, sc.s, W::Regular, CARD_MUTED, 240);
    }
}

fn draw_transparent_logo(p: &mut Painter, logo: &crate::game_info::GameLogo, box_r: R) {
    let (bx, by, bw, bh) = box_r;
    if bw < 16.0 || bh < 16.0 || logo.width == 0 || logo.height == 0 {
        return;
    }
    let scale = (bw / logo.width as f32).min(bh / logo.height as f32).min(1.65);
    let dw = (logo.width as f32 * scale).round().max(1.0) as u32;
    let dh = (logo.height as f32 * scale).round().max(1.0) as u32;
    let Some(src) = image::RgbaImage::from_raw(logo.width, logo.height, logo.rgba.clone()) else {
        return;
    };
    let resized = image::imageops::resize(&src, dw, dh, FilterType::Triangle);
    let ox = bx.round() as i32;
    let oy = (by + (bh - dh as f32) / 2.0).round() as i32;

    // Subtle dark drop-shadow pass for white/bright logos over busy art.
    let raw = resized.as_raw();
    for yy in 0..dh as i32 {
        for xx in 0..dw as i32 {
            let a = raw[((yy as u32 * dw + xx as u32) * 4 + 3) as usize];
            if a > 20 {
                p.layer.over(ox + xx + 2, oy + yy + 3, (0, 0, 0), a / 3);
            }
        }
    }
    p.layer.blit_rgba(ox, oy, dw, dh, raw);
}

fn paint_fps_card(p: &mut Painter, r: R, sc: &Scale, d: &Data) {
    let k = sc.k;
    let accent = th().fps;
    glass_card(p, r, k, accent);
    let stats = STATS.with(|s| s.borrow().clone());
    let pd = 16.0 * k;
    let hb = glass_header(
        p,
        r,
        k,
        accent,
        "GAME FPS",
        &if stats.gaming || d.game.is_some() {
            mmss(stats.secs)
        } else {
            "IDLE".to_string()
        },
    );

    let (x, y, w, h) = r;
    let fps_val = d.gpu.fps.filter(|&f| f > 0).map(|f| f.to_string()).unwrap_or_else(|| "--".into());
    let cap = fit_vu(p, &fps_val, "FPS", sc.xl * 1.05, (w - 2.0 * pd) * 0.44, W::Medium);
    let vbase = hb + 12.0 * k + cap;
    value_unit(p, x + pd, vbase, &fps_val, "FPS", cap, W::Medium, CARD_LIGHT);

    // FPS sparkline on right half of upper card
    let sx = x + pd + (w - 2.0 * pd) * 0.48;
    let sw = x + w - pd - sx;
    if sw > 40.0 * k {
        let hist: VecDeque<f32> = stats.fps.iter().copied().collect();
        let peak = hist.iter().cloned().fold(60.0f32, f32::max) * 1.1;
        sparkline(
            p,
            (sx, hb + 8.0 * k, sw, (vbase - hb - 6.0 * k).max(26.0 * k)),
            &hist,
            peak,
            [accent, lerp_rgb(accent, WHITE, 0.45)],
            k,
        );
    }

    // Bottom 3 session columns: AVG | 1% LOW | MAX
    let row_y = y + h - pd - sc.s - 22.0 * k;
    let cols = [
        ("AVG", if stats.avg > 0.0 { format!("{:.0}", stats.avg) } else { "--".into() }),
        ("1% LOW", if stats.low1 > 0.0 { format!("{:.0}", stats.low1) } else { "--".into() }),
        ("MAX", if stats.max > 0.0 { format!("{:.0}", stats.max) } else { "--".into() }),
    ];
    let cw = (w - 2.0 * pd) / 3.0;
    for (i, (lbl, val)) in cols.iter().enumerate() {
        let cx = x + pd + i as f32 * cw;
        spaced(p, cx, row_y, lbl, 10.5 * k, W::Medium, CARD_DIM, 1.2 * k);
        p.text(cx, row_y + 20.0 * k, val, sc.m * 1.05, W::Medium, CARD_LIGHT, 255);
    }
}

fn paint_cpu_card(p: &mut Painter, r: R, sc: &Scale, d: &Data, hist: &VecDeque<f32>) {
    let k = sc.k;
    let colors = th().cpu;
    glass_card(p, r, k, colors[0]);
    let short_name = ellipsize(p, d.cpu_name, 12.0 * k, W::Regular, r.2 * 0.56);
    let hb = glass_header(p, r, k, colors[0], "CPU", &short_name);
    let (x, y, w, h) = r;
    let pd = 16.0 * k;

    let pct = format!("{:.0}", d.cpu_pct);
    let cap = fit_vu(p, &pct, "%", sc.xl * 1.05, (w - 2.0 * pd) * 0.44, W::Medium);
    let vbase = hb + 12.0 * k + cap;
    value_unit(p, x + pd, vbase, &pct, "%", cap, W::Medium, CARD_LIGHT);

    let sx = x + pd + (w - 2.0 * pd) * 0.48;
    let sw = x + w - pd - sx;
    if sw > 40.0 * k {
        sparkline(p, (sx, hb + 8.0 * k, sw, (vbase - hb - 6.0 * k).max(26.0 * k)), hist, 100.0, colors, k);
    }

    let row_y = y + h - pd - sc.s - 22.0 * k;
    let temp = d.cpu_temp.map(|t| format!("{t:.0}°C")).unwrap_or_else(|| "--".into());
    let pwr = d.cpu_power.map(|w| format!("{w:.0}W")).unwrap_or_else(|| "--".into());
    let clk = d.cpu_mhz.map(|m| format!("{:.2} GHz", m as f32 / 1000.0)).unwrap_or_else(|| "--".into());
    let cols = [("TEMP", temp), ("POWER", pwr), ("CLOCK", clk)];
    let cw = (w - 2.0 * pd) / 3.0;
    for (i, (lbl, val)) in cols.iter().enumerate() {
        let cx = x + pd + i as f32 * cw;
        spaced(p, cx, row_y, lbl, 10.5 * k, W::Medium, CARD_DIM, 1.2 * k);
        let vc = if i == 0 && d.cpu_temp.is_some_and(|t| t >= 85.0) { HOT } else { CARD_LIGHT };
        p.text(cx, row_y + 20.0 * k, val, sc.m * 1.05, W::Medium, vc, 255);
    }
}

fn paint_gpu_card(p: &mut Painter, r: R, sc: &Scale, d: &Data) {
    let k = sc.k;
    let colors = th().gpu;
    glass_card(p, r, k, colors[0]);
    let gpu_name = d.gpu.name.as_deref().unwrap_or("Graphics");
    let short_name = ellipsize(p, gpu_name, 12.0 * k, W::Regular, r.2 * 0.56);
    let hb = glass_header(p, r, k, colors[0], "GPU", &short_name);
    let (x, y, w, h) = r;
    let pd = 16.0 * k;

    let pct = d.gpu_pct.map(|v| format!("{v:.0}")).unwrap_or_else(|| "--".into());
    let unit = if d.gpu_pct.is_some() { "%" } else { "" };
    let cap = fit_vu(p, &pct, unit, sc.xl * 1.05, (w - 2.0 * pd) * 0.44, W::Medium);
    let vbase = hb + 12.0 * k + cap;
    value_unit(p, x + pd, vbase, &pct, unit, cap, W::Medium, CARD_LIGHT);

    // VRAM summary + bar on right half
    let sx = x + pd + (w - 2.0 * pd) * 0.48;
    let sw = x + w - pd - sx;
    if let (Some(used), Some(total)) = (d.gpu.vram_used_mb, d.gpu.vram_total_mb) {
        if total > 0 && sw > 40.0 * k {
            spaced(p, sx, hb + 18.0 * k, "VRAM", 10.5 * k, W::Medium, CARD_DIM, 1.2 * k);
            let vram = format!("{}/{} GB", gb(used.max(0) as u64), gb(total.max(0) as u64));
            p.text(sx, hb + 39.0 * k, &vram, sc.m, W::Medium, CARD_LIGHT, 255);
            hbar(p, sx, hb + 48.0 * k, sw, 6.0 * k, Some(used as f32 / total as f32), colors);
        }
    }

    let row_y = y + h - pd - sc.s - 22.0 * k;
    let temp = d.gpu.temp_edge_c.map(|t| format!("{t}°C")).unwrap_or_else(|| "--".into());
    let pwr = d.gpu.power_w.map(|w| format!("{w:.0}W")).unwrap_or_else(|| "--".into());
    let clk = d.gpu.clock_mhz.map(|c| format!("{c} MHz")).unwrap_or_else(|| "--".into());
    let cols = [("TEMP", temp), ("POWER", pwr), ("CLOCK", clk)];
    let cw = (w - 2.0 * pd) / 3.0;
    for (i, (lbl, val)) in cols.iter().enumerate() {
        let cx = x + pd + i as f32 * cw;
        spaced(p, cx, row_y, lbl, 10.5 * k, W::Medium, CARD_DIM, 1.2 * k);
        let vc = if i == 0 && d.gpu.temp_edge_c.is_some_and(|t| t >= 85) { HOT } else { CARD_LIGHT };
        p.text(cx, row_y + 20.0 * k, val, sc.m * 1.05, W::Medium, vc, 255);
    }
}

fn paint_system_card(p: &mut Painter, r: R, sc: &Scale, d: &Data) {
    let k = sc.k;
    let colors = th().ram;
    glass_card(p, r, k, colors[0]);
    let hb = glass_header(p, r, k, colors[0], "SYSTEM", &format!("UP {}", mmss(d.uptime_s)));
    let (x, _y, w, _h) = r;
    let pd = 16.0 * k;
    let inner = w - 2.0 * pd;
    let col_w = (inner - 14.0 * k) / 2.0;

    // Row 1: RAM | DISK
    let r1 = hb + 18.0 * k;
    spaced(p, x + pd, r1, "MEMORY", 10.5 * k, W::Medium, CARD_DIM, 1.2 * k);
    let ram_txt = format!("{} / {} GB", gb(d.ram_used_mb), gb(d.ram_total_mb));
    let rcap = p.fit(&ram_txt, sc.m * 1.05, col_w, W::Medium);
    p.text(x + pd, r1 + 22.0 * k, &ram_txt, rcap, W::Medium, CARD_LIGHT, 255);
    if d.ram_total_mb > 0 {
        hbar(p, x + pd, r1 + 31.0 * k, col_w, 5.0 * k, Some(d.ram_used_mb as f32 / d.ram_total_mb as f32), th().ram);
    }

    let dx = x + pd + col_w + 14.0 * k;
    let ssd_lbl = d.disk.map(|dk| format!("SSD {}", dk.label)).unwrap_or_else(|| "STORAGE".into());
    spaced(p, dx, r1, &ssd_lbl, 10.5 * k, W::Medium, CARD_DIM, 1.2 * k);
    let ssd_txt = d
        .disk
        .map(|dk| format!("{:.0} / {:.0} GB", dk.used_gb, dk.total_gb))
        .unwrap_or_else(|| "--".into());
    let dcap = p.fit(&ssd_txt, sc.m * 1.05, col_w, W::Medium);
    p.text(dx, r1 + 22.0 * k, &ssd_txt, dcap, W::Medium, CARD_LIGHT, 255);
    if let Some(dk) = d.disk {
        if dk.total_gb > 0.0 {
            hbar(p, dx, r1 + 31.0 * k, col_w, 5.0 * k, Some((dk.used_gb / dk.total_gb) as f32), th().ssd);
        }
    }

    // Row 2: NETWORK | FANS
    let r2 = r1 + 56.0 * k;
    spaced(p, x + pd, r2, "NETWORK", 10.5 * k, W::Medium, CARD_DIM, 1.2 * k);
    let dn = rate(d.net_kb.0);
    let up = rate(d.net_kb.1);
    let net_txt = format!("↓{} {}  ↑{} {}", dn.0, dn.1, up.0, up.1);
    let ncap = p.fit(&net_txt, sc.m, col_w, W::Medium);
    p.text(x + pd, r2 + 22.0 * k, &net_txt, ncap, W::Medium, CARD_LIGHT, 255);

    spaced(p, dx, r2, "COOLING", 10.5 * k, W::Medium, CARD_DIM, 1.2 * k);
    let fan_txt = match (d.cpu_fan_rpm, d.gpu.fan_rpm) {
        (Some(c), Some(g)) => format!("CPU {c}  GPU {g}"),
        (Some(c), None) => format!("CPU {c} RPM"),
        (None, Some(g)) => format!("GPU {g} RPM"),
        (None, None) => d
            .fans
            .first()
            .map(|f| format!("{} RPM", f.rpm))
            .unwrap_or_else(|| "--".into()),
    };
    let fcap = p.fit(&fan_txt, sc.m, col_w, W::Medium);
    p.text(dx, r2 + 22.0 * k, &fan_txt, fcap, W::Medium, CARD_LIGHT, 255);
}
