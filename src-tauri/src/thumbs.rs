//! Icons and thumbnails via the Windows Shell.
//!
//! `IShellItemImageFactory::GetImage` is what Explorer itself uses, so a photo
//! yields its actual picture, a video its poster frame, and anything else its
//! file-type icon — with no image-decoding dependency on our side. Results are
//! returned as PNG data URIs, which the app's CSP already allows (`img-src
//! 'self' data:`).

use base64::Engine;
use std::cell::Cell;
use std::path::Path;
use windows::core::HSTRING;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK,
    SIIGBF_ICONONLY,
};

thread_local! {
    static COM_READY: Cell<bool> = const { Cell::new(false) };
}

/// COM must be initialised once per thread before any shell call. Worker
/// threads are reused across requests, so this is cheap after the first hit.
pub fn ensure_com() {
    COM_READY.with(|ready| {
        if !ready.get() {
            // Ignore the result: RPC_E_CHANGED_MODE just means another
            // component already initialised this thread, which is fine.
            let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            ready.set(true);
        }
    });
}

/// Renders `path` at roughly `size` pixels and returns a PNG data URI.
/// `icon_only` skips content thumbnails and asks for the file-type/app icon.
pub fn shell_image(path: &Path, size: u32, icon_only: bool) -> Option<String> {
    if !path.exists() {
        return None;
    }
    ensure_com();

    let flags: SIIGBF = if icon_only {
        SIIGBF(SIIGBF_ICONONLY.0 | SIIGBF_BIGGERSIZEOK.0)
    } else {
        SIIGBF_BIGGERSIZEOK
    };

    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None) }.ok()?;
    let bitmap = unsafe {
        factory.GetImage(
            SIZE {
                cx: size as i32,
                cy: size as i32,
            },
            flags,
        )
    }
    .ok()?;

    let png = bitmap_to_png(bitmap, size);
    let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
    let png = png?;

    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

/// Copies a shell HBITMAP into straight-alpha RGBA and encodes it as PNG.
fn bitmap_to_png(bitmap: HBITMAP, max_size: u32) -> Option<Vec<u8>> {
    let mut info = BITMAP::default();
    let copied = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut info as *mut _ as *mut _),
        )
    };
    if copied == 0 || info.bmWidth <= 0 || info.bmHeight == 0 {
        return None;
    }
    let width = info.bmWidth as u32;
    let height = info.bmHeight.unsigned_abs();
    // Guard against a handler returning something absurd.
    if width > max_size * 8 || height > max_size * 8 {
        return None;
    }

    let mut header = BITMAPINFO::default();
    header.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        // Negative height requests top-down rows, matching PNG's order.
        biHeight: -(height as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };

    let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
    let dc = unsafe { CreateCompatibleDC(None) };
    let rows = unsafe {
        GetDIBits(
            dc,
            bitmap,
            0,
            height,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut header,
            DIB_RGB_COLORS,
        )
    };
    let _ = unsafe { DeleteDC(dc) };
    if rows == 0 {
        return None;
    }

    // The shell hands back BGRA with premultiplied alpha. Some thumbnail
    // handlers leave the alpha channel entirely zero for opaque images —
    // un-premultiplying that would erase the picture, so treat it as opaque.
    let opaque = pixels.chunks_exact(4).all(|p| p[3] == 0);
    for p in pixels.chunks_exact_mut(4) {
        let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
        if opaque {
            p[0] = r;
            p[1] = g;
            p[2] = b;
            p[3] = 255;
        } else if a == 0 {
            p[0] = 0;
            p[1] = 0;
            p[2] = 0;
            p[3] = 0;
        } else {
            let un = |c: u8| ((c as u32 * 255) / a as u32).min(255) as u8;
            p[0] = un(r);
            p[1] = un(g);
            p[2] = un(b);
            p[3] = a;
        }
    }

    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&pixels).ok()?;
    }
    Some(out)
}

/// Resolves the file whose icon represents an installed application.
///
/// The registry `DisplayIcon` is the authoritative source but comes in several
/// shapes (`path`, `"path",0`, `path,-102`). When it is missing or points at a
/// file that is gone, fall back to the install folder's most plausible
/// executable.
pub fn app_icon_source(
    display_icon: Option<&str>,
    install_location: Option<&str>,
    uninstall_string: Option<&str>,
) -> Option<std::path::PathBuf> {
    if let Some(raw) = display_icon {
        let trimmed = raw.trim();
        // Strip a trailing `,<index>` that is not part of the path.
        let without_index = match trimmed.rsplit_once(',') {
            Some((head, tail))
                if !head.is_empty()
                    && tail
                        .trim()
                        .trim_start_matches('-')
                        .chars()
                        .all(|c| c.is_ascii_digit()) =>
            {
                head
            }
            _ => trimmed,
        };
        let cleaned = without_index.trim().trim_matches('"');
        if !cleaned.is_empty() {
            let path = std::path::PathBuf::from(cleaned);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    if let Some(dir) = install_location {
        if let Some(exe) = best_exe_in(std::path::Path::new(dir.trim().trim_matches('"'))) {
            return Some(exe);
        }
    }

    // MSI packages usually record neither DisplayIcon nor InstallLocation in
    // the Uninstall key — their icon lives in the Windows Installer database.
    if let Some(raw) = uninstall_string {
        if let Some(code) = crate::uninstall::msi_product_code_public(raw) {
            if let Some(icon) = msi_property(&code, "ProductIcon") {
                let path = std::path::PathBuf::from(crate::uninstall::expand_env_vars_public(&icon));
                if path.is_file() {
                    return Some(path);
                }
            }
            if let Some(dir) = msi_property(&code, "InstallLocation") {
                let expanded = crate::uninstall::expand_env_vars_public(&dir);
                if let Some(exe) = best_exe_in(std::path::Path::new(expanded.trim())) {
                    return Some(exe);
                }
            }
        }
    }

    // Last resort: the uninstaller's own icon is still better than nothing.
    if let Some(raw) = uninstall_string {
        let (program, _) = crate::uninstall::split_command_line_public(raw);
        let path = std::path::PathBuf::from(program);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Reads a property for an installed MSI product (e.g. `ProductIcon`).
fn msi_property(product_code: &str, property: &str) -> Option<String> {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::System::ApplicationInstallationAndServicing::MsiGetProductInfoW;

    let product = HSTRING::from(product_code);
    let prop = HSTRING::from(property);

    // First call sizes the buffer; the returned count excludes the terminator.
    let mut len: u32 = 0;
    let rc = unsafe { MsiGetProductInfoW(&product, &prop, None, Some(&mut len)) };
    if rc != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize + 1];
    let mut cap = buf.len() as u32;
    let rc = unsafe {
        MsiGetProductInfoW(&product, &prop, Some(PWSTR(buf.as_mut_ptr())), Some(&mut cap))
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(cap as usize);
    let value = String::from_utf16_lossy(&buf).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Picks the executable in `dir` most likely to carry the app's branding:
/// prefers one whose name matches the folder, otherwise the largest.
fn best_exe_in(dir: &Path) -> Option<std::path::PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let folder = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let normalise = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    let folder_key = normalise(&folder);

    let mut best: Option<(bool, u64, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e.eq_ignore_ascii_case("exe")) != Some(true) {
            continue;
        }
        let stem = path
            .file_stem()
            .map(|s| normalise(&s.to_string_lossy()))
            .unwrap_or_default();
        // Installers and helpers rarely carry the product logo.
        if ["unins000", "uninstall", "uninst", "setup", "installer"]
            .iter()
            .any(|skip| stem.starts_with(skip))
        {
            continue;
        }
        let matches_folder = !folder_key.is_empty()
            && (folder_key.contains(&stem) || stem.contains(&folder_key));
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let candidate = (matches_folder, size, path);
        if best
            .as_ref()
            .map(|b| (b.0, b.1) < (candidate.0, candidate.1))
            .unwrap_or(true)
        {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_real_system_icon() {
        let notepad = Path::new(r"C:\Windows\System32\notepad.exe");
        if !notepad.is_file() {
            return; // Not present on this machine — nothing to assert.
        }
        let uri = shell_image(notepad, 32, true).expect("notepad should yield an icon");
        assert!(uri.starts_with("data:image/png;base64,"));
        let payload = uri.trim_start_matches("data:image/png;base64,");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .expect("valid base64");
        // PNG magic number.
        assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert!(bytes.len() > 100, "icon looks empty: {} bytes", bytes.len());
    }

    /// The point of using the shell imaging API is that a picture renders as
    /// *itself*, not as a generic file-type icon. Write a solid-red PNG, ask
    /// for a thumbnail, and confirm the pixels that come back are red.
    #[test]
    fn image_files_render_their_actual_contents() {
        let dir = std::env::temp_dir().join("storage doctor thumb test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("solid-red.png");

        let (w, h) = (200u32, 200u32);
        let mut raw = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            raw.extend_from_slice(&[220, 20, 20, 255]);
        }
        {
            let file = std::fs::File::create(&source).unwrap();
            let mut encoder = png::Encoder::new(file, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&raw)
                .unwrap();
        }

        let uri = shell_image(&source, 96, false).expect("image should produce a thumbnail");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(uri.trim_start_matches("data:image/png;base64,"))
            .unwrap();

        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert!(info.width >= 32, "thumbnail too small: {}px", info.width);

        // Sample the centre pixel.
        let channels = info.color_type.samples();
        let mid = ((info.height / 2) * info.line_size as u32 / channels as u32
            + info.width / 2) as usize
            * channels;
        let (r, g, b) = (buf[mid], buf[mid + 1], buf[mid + 2]);
        assert!(
            r > 150 && g < 100 && b < 100,
            "centre pixel is rgb({r},{g},{b}) — expected red, so the shell \
             returned a generic icon rather than the image itself"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Diagnostic: how many installed apps actually yield a logo, where it
    /// came from, how large the payload is and how long the batch takes.
    /// Run with `cargo test -- --ignored --nocapture icon_coverage`.
    #[test]
    #[ignore]
    fn icon_coverage_audit() {
        use rayon::prelude::*;

        let apps = crate::uninstall::list_installed();
        let started = std::time::Instant::now();

        let resolved: Vec<(String, Option<std::path::PathBuf>, &'static str)> = apps
            .iter()
            .map(|a| {
                let from_display = a
                    .display_icon
                    .as_deref()
                    .and_then(|d| app_icon_source(Some(d), None, None));
                if from_display.is_some() {
                    return (a.name.clone(), from_display, "DisplayIcon");
                }
                let from_dir = a
                    .install_location
                    .as_deref()
                    .and_then(|l| app_icon_source(None, Some(l), None));
                if from_dir.is_some() {
                    return (a.name.clone(), from_dir, "install folder");
                }
                let from_uninst = a
                    .uninstall_string
                    .as_deref()
                    .and_then(|u| app_icon_source(None, None, Some(u)));
                if from_uninst.is_some() {
                    return (a.name.clone(), from_uninst, "uninstaller exe");
                }
                (a.name.clone(), None, "none")
            })
            .collect();
        let resolve_ms = started.elapsed().as_millis();

        let render_started = std::time::Instant::now();
        let rendered: Vec<(String, &'static str, Option<usize>)> = resolved
            .par_iter()
            .map(|(name, path, origin)| {
                let bytes = path
                    .as_ref()
                    .and_then(|p| shell_image(p, 32, true))
                    .map(|uri| uri.len());
                (name.clone(), *origin, bytes)
            })
            .collect();
        let render_ms = render_started.elapsed().as_millis();

        let total = rendered.len();
        let with_icon = rendered.iter().filter(|r| r.2.is_some()).count();
        let payload: usize = rendered.iter().filter_map(|r| r.2).sum();

        let mut by_origin: std::collections::BTreeMap<&str, (usize, usize)> =
            std::collections::BTreeMap::new();
        for (_, origin, bytes) in &rendered {
            let e = by_origin.entry(origin).or_insert((0, 0));
            e.0 += 1;
            if bytes.is_some() {
                e.1 += 1;
            }
        }

        println!("\n=== app icon coverage ===");
        println!("apps listed:            {total}");
        println!("rendered an icon:       {with_icon}  ({:.0}%)", 100.0 * with_icon as f64 / total.max(1) as f64);
        println!("resolve time:           {resolve_ms} ms");
        println!("render time (parallel): {render_ms} ms");
        println!("payload (data URIs):    {:.0} KB total, {:.0} B avg",
            payload as f64 / 1024.0,
            payload as f64 / with_icon.max(1) as f64);
        println!("source of icon:");
        for (origin, (found, ok)) in &by_origin {
            println!("  {origin:<16} resolved {found:>3}, rendered {ok:>3}");
        }
        println!("apps with NO icon:");
        for (name, origin, bytes) in &rendered {
            if bytes.is_none() {
                println!("  - {name}  (source: {origin})");
            }
        }
    }

    #[test]
    fn missing_files_return_none() {
        assert!(shell_image(Path::new(r"C:\No Such File.png"), 64, false).is_none());
    }

    #[test]
    fn display_icon_index_suffix_is_stripped() {
        let notepad = r"C:\Windows\System32\notepad.exe";
        if !Path::new(notepad).is_file() {
            return;
        }
        assert_eq!(
            app_icon_source(Some(&format!("{notepad},0")), None, None),
            Some(std::path::PathBuf::from(notepad))
        );
        assert_eq!(
            app_icon_source(Some(&format!("\"{notepad}\",-102")), None, None),
            Some(std::path::PathBuf::from(notepad))
        );
        // A comma that is part of the name must survive.
        assert_eq!(app_icon_source(Some(r"C:\a,b\gone.exe"), None, None), None);
    }
}
