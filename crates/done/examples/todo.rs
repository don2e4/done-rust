//! A keyed collection: rows keep their native controls, and keyboard focus,
//! while items are added, removed, and reordered.
use done::prelude::*;

struct Todo {
    id: u64,
    title: String,
    done: bool,
}

struct Todos {
    items: Vec<Todo>,
    draft: String,
    next_id: u64,
}

impl Default for Todos {
    fn default() -> Self {
        let titles = ["Read ARCHITECTURE.md", "Try Tab, Space, and Enter"];
        Todos {
            items: titles
                .iter()
                .zip(0..)
                .map(|(title, id)| Todo {
                    id,
                    title: (*title).to_owned(),
                    done: false,
                })
                .collect(),
            draft: String::new(),
            next_id: titles.len() as u64,
        }
    }
}

fn add(s: &mut Todos) {
    let title = s.draft.trim();
    if !title.is_empty() {
        s.items.push(Todo {
            id: s.next_id,
            title: title.to_owned(),
            done: false,
        });
        s.next_id += 1;
        s.draft.clear();
    }
}

fn view(s: &Todos) -> impl View<Todos> {
    // Finished items sink below open ones; their rows move rather than rebuild.
    let mut items: Vec<&Todo> = s.items.iter().collect();
    items.sort_by_key(|todo| todo.done);
    let left = items.iter().filter(|todo| !todo.done).count();
    column((
        text("Todos").size(24.0),
        row((
            text_input(s.draft.clone())
                .placeholder("What needs doing?")
                .label("New todo")
                .on_input(|s: &mut Todos, value| s.draft = value.to_owned())
                .on_submit(add)
                .grow(1.0),
            button("Add").on_click(add),
        ))
        .gap(8.0),
        scroll(
            column(keyed(items.into_iter().map(|todo| (todo.id, item(todo)))))
                .gap(4.0)
                .align(Align::Stretch),
        )
        .grow(1.0),
        row((
            text(format!("{left} left")).grow(1.0),
            button("Clear done").on_click(|s: &mut Todos| s.items.retain(|todo| !todo.done)),
        ))
        .gap(8.0)
        .align(Align::Center),
    ))
    .gap(12.0)
    .padding(16.0)
    .align(Align::Stretch)
}

fn item(todo: &Todo) -> impl View<Todos> {
    let id = todo.id;
    row((
        checkbox(todo.title.clone(), todo.done)
            .on_toggle(move |s: &mut Todos, done| {
                if let Some(todo) = s.items.iter_mut().find(|todo| todo.id == id) {
                    todo.done = done;
                }
            })
            .grow(1.0),
        button("Remove").on_click(move |s: &mut Todos| s.items.retain(|todo| todo.id != id)),
    ))
    .gap(8.0)
    .align(Align::Center)
}

fn main() -> done::Result<()> {
    done::app("Todos", Todos::default(), view)
        .size(360.0, 420.0)
        .run()
}
