//! The handwritten GTK4 baseline for the Linux hello-world measurements.

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use gtk4::{self as gtk, glib, prelude::*};

    gtk::init()?;
    let main_loop = glib::MainLoop::new(None, false);
    let window = gtk::Window::builder()
        .title("Hello")
        .default_width(320)
        .default_height(200)
        .build();
    let label = gtk::Label::new(Some("Hello, world!"));
    label.set_xalign(0.0);
    label.set_yalign(0.0);
    window.set_child(Some(&label));
    let exit_loop = main_loop.clone();
    window.connect_close_request(move |_| {
        exit_loop.quit();
        glib::Propagation::Proceed
    });
    #[cfg(feature = "probe")]
    done_gtk4::probe::install(&window);
    window.present();
    main_loop.run();
    window.destroy();
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("raw_hello is the GTK4 baseline for Linux measurements");
}
