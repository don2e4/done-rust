#![cfg(target_os = "linux")]

use done::prelude::*;
use gtk4::{self as gtk, glib, prelude::*};
use std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration};

mod common;
use common::{border_box, descendants};

const AVATAR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/assets/avatar.png");

#[test]
#[ignore = "requires a desktop display and window manager; run with --ignored --test-threads=1"]
fn native_images_load_local_files_and_report_failures() {
    // Missing and undecodable files fail the run before the main loop starts.
    let missing = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("missing.png");
    let corrupt = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("corrupt.png");
    std::fs::write(&corrupt, b"not an image").unwrap();
    for path in [missing, corrupt] {
        let shown = path.clone();
        let error = done::app("Bad image", (), move |_: &()| image(shown.clone()))
            .run()
            .unwrap_err();
        assert!(
            error.to_string().starts_with(&path.display().to_string()),
            "the error names the file: {error}"
        );
    }

    let checked = Rc::new(Cell::new(false));
    let observed = checked.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let Some(window) = gtk::Window::list_toplevels()
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::Window>().ok())
        else {
            return glib::ControlFlow::Continue;
        };
        if !window.is_mapped() || window.width() == 0 {
            return glib::ControlFlow::Continue;
        }
        let mut widgets = Vec::new();
        descendants(window.upcast_ref(), &mut widgets);
        let picture = widgets
            .iter()
            .find_map(|widget| widget.downcast_ref::<gtk::Picture>())
            .expect("a native picture");
        let paintable = picture.paintable().expect("decoded image");
        assert_eq!(
            (paintable.intrinsic_width(), paintable.intrinsic_height()),
            (64, 64)
        );
        let bounds = border_box(picture);
        assert!(bounds.width() >= 64.0 && bounds.height() >= 64.0);
        assert_eq!(picture.alternative_text().as_deref(), Some("Avatar"));
        assert_eq!(picture.accessible_role(), gtk::AccessibleRole::Img);
        observed.set(true);
        window.close();
        glib::ControlFlow::Break
    });
    let watchdog = glib::timeout_add_local_once(Duration::from_secs(5), || {
        panic!("native image check did not complete within five seconds");
    });
    done::app("Image check", (), |_: &()| {
        column((image(AVATAR).label("Avatar"),)).padding(8.0)
    })
    .run()
    .unwrap();
    watchdog.remove();
    assert!(checked.get());
}
