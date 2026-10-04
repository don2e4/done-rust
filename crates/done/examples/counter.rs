use done::prelude::*;

#[derive(Default)]
struct Counter {
    count: i32,
}

fn view(s: &Counter) -> impl View<Counter> {
    column((
        text(format!("Clicked {} times", s.count)).size(24.0),
        row((
            button("−").on_click(|s: &mut Counter| s.count -= 1),
            button("+").on_click(|s: &mut Counter| s.count += 1),
        ))
        .gap(8.0),
    ))
    .padding(16.0)
    .align(Align::Center)
}

fn main() -> done::Result<()> {
    done::app("Counter", Counter::default(), view).run()
}
