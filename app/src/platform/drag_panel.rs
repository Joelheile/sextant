//! A small floating panel beside System Settings that holds Sextant's icon. The user drags the
//! icon into a Privacy & Security list, the way macOS adds an app it has not asked about yet. The
//! panel follows the System Settings window and closes once access is granted, the window closes,
//! or another pane opens.

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSCursor, NSDragOperation, NSDraggingContext, NSDraggingItem,
    NSDraggingSession, NSDraggingSource, NSEvent, NSImageView, NSPanel, NSRunningApplication,
    NSTextAlignment, NSTextField, NSView, NSWindowStyleMask, NSWorkspace,
};
use objc2_core_foundation::{CFDictionary, CFNumber, CFNumberType, CFString, CGRect};
use objc2_core_graphics::{
    CGDisplayBounds, CGMainDisplayID, CGRectMakeWithDictionaryRepresentation,
    CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID, kCGWindowBounds,
    kCGWindowLayer, kCGWindowOwnerPID,
};
use objc2_foundation::{NSArray, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};

const SETTINGS_ID: &str = "com.apple.systempreferences";
const TICK: Duration = Duration::from_millis(250);
/// How long to wait for System Settings to show its window before giving up.
const OPEN_TICKS: u32 = 40;
const WIDTH: f64 = 280.0;
const HEIGHT: f64 = 150.0;
const ICON: f64 = 64.0;
const GAP: f64 = 12.0;

/// Bumped by every `show`, so an older panel's tracking thread knows to stop.
static FLOW: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static OPEN: RefCell<Option<Retained<NSPanel>>> = const { RefCell::new(None) };
}

struct Tile {
    path: String,
}

define_class!(
    // SAFETY: NSImageView has no subclassing requirements and `DragTile` does not implement Drop.
    #[unsafe(super(NSImageView, objc2_app_kit::NSControl, NSView, objc2_app_kit::NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Tile]
    struct DragTile;

    impl DragTile {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        // Swallows the click so NSImageView does not start its own tracking loop.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {}

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.begin_drag(event);
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::openHandCursor());
        }
    }

    unsafe impl NSObjectProtocol for DragTile {}

    unsafe impl NSDraggingSource for DragTile {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operations(
            &self,
            _session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> NSDragOperation {
            NSDragOperation::Copy
        }
    }
);

impl DragTile {
    fn new(path: String, frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Tile { path });
        // SAFETY: initWithFrame: is NSView's designated initializer.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Drags the app bundle as a file URL, which is what the Privacy & Security lists accept.
    fn begin_drag(&self, event: &NSEvent) {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&self.ivars().path));
        let item = NSDraggingItem::initWithPasteboardWriter(
            NSDraggingItem::alloc(),
            ProtocolObject::from_ref(&*url),
        );
        let icon = self.image();
        // SAFETY: the contents are an NSImage or nil, as NSDraggingItem expects.
        unsafe {
            item.setDraggingFrame_contents(
                self.bounds(),
                icon.as_deref().map(|image| image as &AnyObject),
            );
        }
        self.beginDraggingSessionWithItems_event_source(
            &NSArray::from_retained_slice(&[item]),
            event,
            ProtocolObject::from_ref(self),
        );
    }
}

/// The app bundle that holds `exe`, or `exe` itself in a dev build that has no bundle.
fn app_path(exe: &Path) -> String {
    exe.ancestors()
        .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))
        .unwrap_or(exe)
        .to_string_lossy()
        .into_owned()
}

/// Opens the panel with `hint` under it and keeps it beside System Settings until `granted`
/// returns true, the System Settings window closes, or `show` runs again.
pub fn show(hint: String, granted: fn() -> bool) {
    let flow = FLOW.fetch_add(1, Ordering::SeqCst) + 1;
    let path = app_path(&std::env::current_exe().unwrap_or_default());
    DispatchQueue::main().exec_async(move || open(path, &hint));
    thread::spawn(move || follow(flow, granted));
}

fn follow(flow: u64, granted: fn() -> bool) {
    let mut seen = false;
    let mut waited = 0;
    loop {
        thread::sleep(TICK);
        if FLOW.load(Ordering::SeqCst) != flow {
            return;
        }
        let frame = settings_frame();
        waited += 1;
        let gone = frame.is_none() && (seen || waited > OPEN_TICKS);
        if gone || granted() {
            DispatchQueue::main().exec_async(move || {
                if FLOW.load(Ordering::SeqCst) == flow {
                    close();
                }
            });
            return;
        }
        if let Some(frame) = frame {
            seen = true;
            let origin = beside(frame, screen_height());
            DispatchQueue::main().exec_async(move || place(origin));
        }
    }
}

fn open(path: String, hint: &str) {
    let mtm = MainThreadMarker::new().expect("dispatched to the main queue");
    close();
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::UtilityWindow
        | NSWindowStyleMask::NonactivatingPanel;
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT));
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        style,
        NSBackingStoreType::Buffered,
        false,
    );
    // SAFETY: `OPEN` owns the panel, so AppKit must not release it on close.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setTitle(&NSString::from_str("Sextant"));
    panel.setFloatingPanel(true);
    panel.setHidesOnDeactivate(false);
    panel.setBecomesKeyOnlyIfNeeded(true);

    let content = NSView::initWithFrame(NSView::alloc(mtm), frame);
    let icon_frame = NSRect::new(
        NSPoint::new((WIDTH - ICON) / 2.0, HEIGHT - ICON - GAP),
        NSSize::new(ICON, ICON),
    );
    let tile = DragTile::new(path.clone(), icon_frame, mtm);
    tile.setImage(Some(
        &NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path)),
    ));
    content.addSubview(&tile);

    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(hint), mtm);
    label.setAlignment(NSTextAlignment::Center);
    label.setFrame(NSRect::new(
        NSPoint::new(GAP, GAP),
        NSSize::new(WIDTH - 2.0 * GAP, HEIGHT - ICON - 3.0 * GAP),
    ));
    content.addSubview(&label);

    panel.setContentView(Some(&content));
    if let Some(frame) = settings_frame() {
        let (x, y) = beside(frame, screen_height());
        panel.setFrameOrigin(NSPoint::new(x, y));
    } else {
        panel.center();
    }
    panel.orderFrontRegardless();
    OPEN.with(|open| *open.borrow_mut() = Some(panel));
}

fn place((x, y): (f64, f64)) {
    OPEN.with(|open| {
        if let Some(panel) = open.borrow().as_ref() {
            panel.setFrameOrigin(NSPoint::new(x, y));
        }
    });
}

fn close() {
    if let Some(panel) = OPEN.with(|open| open.borrow_mut().take()) {
        panel.close();
    }
}

fn screen_height() -> f64 {
    CGDisplayBounds(CGMainDisplayID()).size.height
}

/// The panel's bottom-left corner in AppKit coordinates: centred under the System Settings window,
/// or to its right when there is no room below. `settings` is in Quartz coordinates, whose origin
/// is the top left of the main display.
fn beside(settings: CGRect, screen_height: f64) -> (f64, f64) {
    let below_top = settings.origin.y + settings.size.height + GAP;
    if below_top + HEIGHT <= screen_height {
        let x = settings.origin.x + (settings.size.width - WIDTH) / 2.0;
        (x, screen_height - below_top - HEIGHT)
    } else {
        let x = settings.origin.x + settings.size.width + GAP;
        (x, screen_height - settings.origin.y - settings.size.height)
    }
}

/// The frame of the main System Settings window, if one is on screen.
fn settings_frame() -> Option<CGRect> {
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(
        SETTINGS_ID,
    ));
    let pid = apps.iter().next()?.processIdentifier();
    let list = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        kCGNullWindowID,
    )?;
    (0..list.count()).find_map(|index| {
        // SAFETY: the list holds dictionaries; each value read is a CFNumber, a dictionary or null.
        unsafe {
            let entry = (list.value_at_index(index) as *const CFDictionary).as_ref()?;
            let number = |key: &CFString| {
                let value =
                    (entry.value((key as *const CFString).cast()) as *const CFNumber).as_ref()?;
                let mut out = 0_i32;
                value
                    .value(
                        CFNumberType::SInt32Type,
                        (&mut out as *mut i32).cast::<c_void>(),
                    )
                    .then_some(out)
            };
            if number(kCGWindowOwnerPID)? != pid || number(kCGWindowLayer)? != 0 {
                return None;
            }
            let bounds = (entry.value((kCGWindowBounds as *const CFString).cast())
                as *const CFDictionary)
                .as_ref()?;
            let mut rect = CGRect::default();
            CGRectMakeWithDictionaryRepresentation(Some(bounds), &mut rect).then_some(rect)
        }
    })
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGSize};

    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> CGRect {
        CGRect::new(CGPoint::new(x, y), CGSize::new(width, height))
    }

    #[test]
    fn sits_centred_under_the_window_when_there_is_room() {
        let (x, y) = beside(rect(100.0, 50.0, 700.0, 500.0), 1000.0);
        assert_eq!(x, 100.0 + (700.0 - WIDTH) / 2.0);
        assert_eq!(y, 1000.0 - (50.0 + 500.0 + GAP) - HEIGHT);
    }

    #[test]
    fn moves_to_the_right_when_the_window_reaches_the_bottom() {
        let (x, y) = beside(rect(100.0, 300.0, 700.0, 650.0), 1000.0);
        assert_eq!(x, 100.0 + 700.0 + GAP);
        assert_eq!(y, 1000.0 - 300.0 - 650.0);
    }

    #[test]
    fn drags_the_bundle_not_the_binary() {
        let exe = Path::new("/Applications/Sextant.app/Contents/MacOS/sextant");
        assert_eq!(app_path(exe), "/Applications/Sextant.app");
    }

    #[test]
    fn drags_the_binary_in_a_dev_build() {
        let exe = Path::new("/repo/target/debug/sextant");
        assert_eq!(app_path(exe), "/repo/target/debug/sextant");
    }
}
