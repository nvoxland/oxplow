//! Composite the project's `iconTint` behind the app icon so concurrent
//! oxplow windows are tellable apart in the Dock (tsk246).
//!
//! # Why this is needed at all
//!
//! Tauri already pushes the embedded `bundle.icon` in as the macOS dock icon,
//! but **only for dev builds** — `tauri::app`'s `RuntimeRunEvent::Ready` arm is
//! `#[cfg(all(dev, target_os = "macos"))]`, because an unbundled binary has no
//! `.app` to take an icon from. That is exactly why a `./target/debug/oxplow`
//! looks identical to an installed one: both end up showing the same art.
//!
//! # Ordering
//!
//! [`apply`] must run on `RunEvent::Ready` **after** Tauri's own call, or Tauri
//! overwrites the tint. That ordering is free rather than fragile: Tauri sets
//! the icon while mapping its internal `Ready` and only then hands `Ready` to
//! the app's run callback, so anything we do there is strictly later.
//!
//! # Scope
//!
//! macOS only. The rest of the app treats a tint as advisory, so every failure
//! path here degrades to "stock icon" rather than surfacing an error — a
//! cosmetic aid must never be able to stop a window from opening.

/// Paint `tint` behind the current application icon and install the result.
///
/// No-op when `tint` is `None`, unparseable, or the platform has no icon to
/// start from. Safe to call more than once; each call re-derives from the
/// *current* icon, so callers should pass the same tint rather than stacking.
#[cfg(target_os = "macos")]
pub fn apply(tint: Option<&str>) {
    let Some((r, g, b)) = tint.and_then(oxplow_config::parse_hex_rgb) else {
        return;
    };

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSColor, NSCompositingOperation, NSImage, NSRectFill};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    // Called from the Tauri run callback, which is the main thread. There is no
    // safe way to prove that to the type system here, and a wrong answer would
    // be a UI-thread violation rather than a tint bug — so bail instead of
    // asserting if the marker can't be obtained.
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("icon tint skipped: not on the main thread");
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(icon) = app.applicationIconImage() else {
        return;
    };

    let size = icon.size();
    if size.width <= 0.0 || size.height <= 0.0 {
        return;
    }

    // The glyph is inset so the tint reads as a border around it rather than a
    // wash behind an edge-to-edge icon.
    let inset = 0.12;
    let glyph = NSRect::new(
        NSPoint::new(size.width * inset, size.height * inset),
        NSSize::new(
            size.width * (1.0 - inset * 2.0),
            size.height * (1.0 - inset * 2.0),
        ),
    );
    let source = NSRect::new(NSPoint::new(0.0, 0.0), size);

    // Block-based drawing rather than lockFocus/unlockFocus: the latter is
    // deprecated (and resolution-dependent), and this crate builds with
    // warnings denied.
    let handler = RcBlock::new(move |rect: NSRect| {
        let color = NSColor::colorWithSRGBRed_green_blue_alpha(
            r as f64 / 255.0,
            g as f64 / 255.0,
            b as f64 / 255.0,
            1.0,
        );
        color.setFill();
        NSRectFill(rect);
        icon.drawInRect_fromRect_operation_fraction(
            glyph,
            source,
            NSCompositingOperation::SourceOver,
            1.0,
        );
        Bool::YES
    });
    let tinted = NSImage::imageWithSize_flipped_drawingHandler(size, false, &handler);
    unsafe { app.setApplicationIconImage(Some(&tinted)) };
}

/// Non-macOS builds have no dock to tint.
#[cfg(not(target_os = "macos"))]
pub fn apply(_tint: Option<&str>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_is_a_no_op_without_a_usable_tint() {
        // The guard has to come before any AppKit call, or a project with no
        // `iconTint` would touch the shared NSApplication for nothing — and
        // this test would need a running app to pass.
        apply(None);
        apply(Some("not-a-colour"));
        apply(Some(""));
    }
}
