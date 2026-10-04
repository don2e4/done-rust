use done::prelude::*;

fn main() -> done::Result<()> {
    done::app("Hello", (), |_: &()| text("Hello, world!")).run()
}
