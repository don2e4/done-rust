use crate::{abort_on_panic, Queue};
use done_core::{Event, Rect, Size};
use gtk4::{self as gtk, glib, prelude::*, subclass::prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Role {
    /// Places a row's, column's, or stack's children. Core sizes it.
    #[default]
    Group,
    /// Fills the window, reports resizes, and enforces the layout's minimum.
    Root,
    /// A scroll viewport's content, natively as large as its children's extent.
    Content,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Host {
        pub(super) children: RefCell<Vec<(gtk::Widget, Rect)>>,
        pub(super) role: Cell<Role>,
        pub(super) queue: RefCell<Option<Rc<Queue>>>,
        pub(super) minimum: Cell<Size>,
        pub(super) last_size: Cell<Option<Size>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Host {
        const NAME: &'static str = "DoneLayoutHost";
        type Type = super::Host;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }
    }

    impl ObjectImpl for Host {
        fn dispose(&self) {
            abort_on_panic(|| {
                while let Some(child) = self.obj().first_child() {
                    child.unparent();
                }
                self.children.borrow_mut().clear();
                self.queue.borrow_mut().take();
            });
        }
    }

    impl WidgetImpl for Host {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            // Core measures native leaves and sizes groups itself.
            let size = match self.role.get() {
                Role::Group => Size::default(),
                Role::Root => self.minimum.get(),
                Role::Content => {
                    self.children
                        .borrow()
                        .iter()
                        .fold(Size::default(), |extent, (_, rect)| Size {
                            width: extent.width.max(rect.x + rect.width),
                            height: extent.height.max(rect.y + rect.height),
                        })
                }
            };
            let extent = match orientation {
                gtk::Orientation::Horizontal => size.width,
                _ => size.height,
            }
            .ceil() as i32;
            (extent, extent, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _: i32) {
            abort_on_panic(|| {
                if self.role.get() == Role::Root {
                    let size = Size {
                        width: width as f32,
                        height: height as f32,
                    };
                    if self.last_size.replace(Some(size)) != Some(size) {
                        if let Some(queue) = self.queue.borrow().as_ref() {
                            queue.push(Event::Resized(size));
                        }
                    }
                }
                for (child, rect) in self.children.borrow().iter() {
                    // GTK expects a measurement before every allocation.
                    child.measure(gtk::Orientation::Horizontal, -1);
                    // Round edges, not sizes, so siblings never gap or overlap.
                    let (left, top) = (rect.x.round(), rect.y.round());
                    let right = (rect.x + rect.width).round();
                    let bottom = (rect.y + rect.height).round();
                    let transform =
                        gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(left, top));
                    child.allocate(
                        (right - left) as i32,
                        (bottom - top) as i32,
                        -1,
                        Some(transform),
                    );
                }
            });
        }

        fn focus(&self, direction: gtk::DirectionType) -> bool {
            abort_on_panic(|| {
                if !matches!(
                    direction,
                    gtk::DirectionType::TabForward | gtk::DirectionType::TabBackward
                ) {
                    return self.parent_focus(direction);
                }
                // Tab follows child order, which follows the description.
                let obj = self.obj();
                let current = obj.focus_child();
                let mut child = match direction {
                    gtk::DirectionType::TabBackward => current.or_else(|| obj.last_child()),
                    _ => current.or_else(|| obj.first_child()),
                };
                while let Some(candidate) = child {
                    if candidate.child_focus(direction) {
                        return true;
                    }
                    child = match direction {
                        gtk::DirectionType::TabBackward => candidate.prev_sibling(),
                        _ => candidate.next_sibling(),
                    };
                }
                false
            })
        }
    }
}

glib::wrapper! {
    pub struct Host(ObjectSubclass<imp::Host>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Host {
    pub(crate) fn new(role: Role) -> Self {
        let host: Self = glib::Object::new();
        host.imp().role.set(role);
        host
    }

    pub(crate) fn set_queue(&self, queue: Rc<Queue>) {
        *self.imp().queue.borrow_mut() = Some(queue);
    }

    pub(crate) fn add(&self, child: &gtk::Widget) {
        child.set_parent(self);
        self.imp()
            .children
            .borrow_mut()
            .push((child.clone(), Rect::default()));
        self.changed();
    }

    pub(crate) fn remove(&self, child: &gtk::Widget) {
        self.imp()
            .children
            .borrow_mut()
            .retain(|(widget, _)| widget != child);
        child.unparent();
        self.changed();
    }

    pub(crate) fn set_child_frame(&self, child: &gtk::Widget, rect: Rect) {
        let mut children = self.imp().children.borrow_mut();
        let Some((_, previous)) = children.iter_mut().find(|(widget, _)| widget == child) else {
            return;
        };
        if *previous != rect {
            *previous = rect;
            drop(children);
            self.changed();
        }
    }

    /// Move existing children into `order`; paint, focus, and accessibility
    /// order all follow GTK's child order.
    pub(crate) fn reorder(&self, order: &[gtk::Widget]) {
        for child in order {
            child.insert_before(self, None::<&gtk::Widget>);
        }
        let position: HashMap<&gtk::Widget, usize> = order
            .iter()
            .enumerate()
            .map(|(index, child)| (child, index))
            .collect();
        self.imp()
            .children
            .borrow_mut()
            .sort_by_key(|(widget, _)| position.get(widget).copied());
        self.queue_allocate();
    }

    pub(crate) fn set_minimum(&self, minimum: Size) {
        if self.imp().minimum.replace(minimum) != minimum {
            self.queue_resize();
        }
    }

    fn changed(&self) {
        match self.imp().role.get() {
            // Content extent is this host's own size request.
            Role::Content => self.queue_resize(),
            Role::Group | Role::Root => self.queue_allocate(),
        }
    }
}
