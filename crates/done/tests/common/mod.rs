//! Helpers shared by the native tests. Each native test file is its own process:
//! GTK binds to the thread that initializes it, and the harness gives every test
//! in one binary its own thread, so two GTK tests cannot share a binary.
#![allow(dead_code)]

use gtk4::{self as gtk, prelude::*};

pub fn descendants(widget: &gtk::Widget, output: &mut Vec<gtk::Widget>) {
    output.push(widget.clone());
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        descendants(&widget, output);
    }
}

pub fn border_box(widget: &impl IsA<gtk::Widget>) -> gtk::graphene::Rect {
    // compute_bounds is the border box core allocates; width()/height() exclude CSS padding.
    let widget = widget.upcast_ref::<gtk::Widget>();
    widget
        .compute_bounds(&widget.parent().unwrap())
        .expect("allocated widget has bounds")
}
