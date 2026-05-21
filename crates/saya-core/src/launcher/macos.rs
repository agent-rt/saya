use std::path::{Path, PathBuf};
use std::process::Command;

use objc2::AnyThread;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
use objc2_foundation::{NSDictionary, NSString};

pub fn launch(path: &Path) -> crate::Result<()> {
    let status = Command::new("/usr/bin/open")
        .arg(path)
        .status()
        .map_err(|e| crate::Error::Other(format!("spawn /usr/bin/open: {e}")))?;
    if !status.success() {
        return Err(crate::Error::Other(format!(
            "/usr/bin/open exited with {status}"
        )));
    }
    Ok(())
}

/// Returns image bytes for `path`, using a persistent on-disk cache at
/// `~/Library/Caches/Saya/icons/`.
///
/// Cold path prefers reading the `.icns` directly from the app bundle's
/// `Contents/Resources/` — that's the ground-truth artwork without paying
/// the NSWorkspace + LaunchServices roundtrip (which serialises across
/// threads and can take 100s of ms per call on a cold system). Falls back
/// to NSWorkspace + PNG encode if no .icns is found.
///
/// `NSImage(data:)` on the Swift side handles both `.icns` and `.png`
/// transparently, so the cache file format is opaque to consumers.
///
/// Writes are atomic (tmp + rename).
pub fn icon_png(path: &Path) -> crate::Result<Vec<u8>> {
    let cache_path = cache_path_for(path);
    if let Ok(bytes) = std::fs::read(&cache_path) {
        if !bytes.is_empty() {
            return Ok(bytes);
        }
        tracing::warn!(path = %cache_path.display(), "empty cache file; re-extracting");
    }
    let bytes = match read_bundle_icns(path) {
        Some(b) if !b.is_empty() => b,
        _ => extract_icon_png(path)?,
    };
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = cache_path.with_extension("png.tmp");
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        tracing::warn!(error = %e, path = %tmp.display(), "icon cache tmp write failed");
    } else if let Err(e) = std::fs::rename(&tmp, &cache_path) {
        tracing::warn!(error = %e, path = %cache_path.display(), "icon cache rename failed");
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(bytes)
}

/// Read the first `.icns` file inside the app bundle's Resources directory.
/// Returns `None` if the bundle layout isn't standard or the file is empty.
fn read_bundle_icns(app_path: &Path) -> Option<Vec<u8>> {
    let resources = app_path.join("Contents/Resources");
    let entries = std::fs::read_dir(&resources).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some("icns") {
            return std::fs::read(&p).ok();
        }
    }
    None
}

fn cache_path_for(path: &Path) -> PathBuf {
    crate::paths::icon_cache_dir().join(sanitize(path))
}

/// Path → filesystem-safe key. Real `.app` paths only contain letters,
/// digits, dots, hyphens and spaces, so this loses no information in
/// practice and stays human-readable for debugging.
fn sanitize(path: &Path) -> String {
    let s = path.to_string_lossy();
    let mut out = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' => out.push(ch),
            _ => out.push('_'),
        }
    }
    out.push_str(".png");
    out
}

fn extract_icon_png(path: &Path) -> crate::Result<Vec<u8>> {
    let path_str = path
        .to_str()
        .ok_or_else(|| crate::Error::Other("icon path must be utf-8".into()))?;

    let ns_path = NSString::from_str(path_str);
    let workspace = NSWorkspace::sharedWorkspace();
    let image = workspace.iconForFile(&ns_path);
    let tiff = image
        .TIFFRepresentation()
        .ok_or_else(|| crate::Error::Other("icon has no TIFF representation".into()))?;

    let bitmap = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)
        .ok_or_else(|| crate::Error::Other("failed to build NSBitmapImageRep".into()))?;

    let empty_props: objc2::rc::Retained<NSDictionary<NSString>> = NSDictionary::new();
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &empty_props)
    }
    .ok_or_else(|| crate::Error::Other("PNG representation failed".into()))?;

    Ok(png.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_icon_for_finder() {
        let path = std::path::PathBuf::from("/System/Library/CoreServices/Finder.app");
        if !path.exists() {
            return;
        }
        let bytes = icon_png(&path).expect("icon_png");
        assert!(bytes.len() > 100, "icon should be > 100 bytes, got {}", bytes.len());
        // Accept PNG (NSWorkspace fallback) or ICNS (direct bundle read).
        let png_magic: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let icns_magic: &[u8] = b"icns";
        assert!(
            bytes.starts_with(png_magic) || bytes.starts_with(icns_magic),
            "unexpected magic: {:02x?}",
            &bytes[..bytes.len().min(8)]
        );
    }

    #[test]
    fn sanitize_path_is_filesystem_safe() {
        let s = sanitize(Path::new("/Applications/Visual Studio Code.app"));
        assert_eq!(s, "_Applications_Visual_Studio_Code.app.png");
    }
}
