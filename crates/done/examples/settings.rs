//! A settings form of controlled native inputs. Every edit flows through the
//! state; a value the state rejects or adjusts is shown as the state has it.
use done::prelude::*;

const THEMES: [&str; 3] = ["System", "Light", "Dark"];
const AVATAR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/assets/avatar.png");

struct Settings {
    name: String,
    notifications: bool,
    volume: f64,
    theme: usize,
    saved: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            name: "Ada".into(),
            notifications: true,
            volume: 60.0,
            theme: 0,
            saved: None,
        }
    }
}

fn save(s: &mut Settings) {
    s.saved = Some(format!(
        "Saved: {}, {} theme, volume {:.0}%",
        s.name.trim(),
        THEMES[s.theme],
        s.volume
    ));
}

fn view(s: &Settings) -> impl View<Settings> {
    let named = !s.name.trim().is_empty();
    let complete = [named, s.notifications, s.theme != 0]
        .iter()
        .filter(|done| **done)
        .count() as f64
        / 3.0;
    column((
        row((
            // A stack overlays its children: the badge sits on the avatar.
            stack((image(AVATAR).label("Avatar"), text("●").size(14.0))).align(Align::End),
            column((
                text(if named {
                    s.name.trim().to_owned()
                } else {
                    "Anonymous".to_owned()
                })
                .size(20.0),
                text("Profile settings"),
            ))
            .gap(2.0),
        ))
        .gap(12.0)
        .align(Align::Center),
        column((
            text("Display name"),
            text_input(s.name.clone())
                .placeholder("Your name")
                .label("Display name")
                // At most 24 characters: a longer edit is reverted natively.
                .on_input(|s: &mut Settings, value| {
                    if value.chars().count() <= 24 {
                        s.name = value.to_owned();
                    }
                }),
            (!named).then(|| text("A display name is required")),
        ))
        .gap(4.0)
        .align(Align::Stretch),
        checkbox("Desktop notifications", s.notifications)
            .on_toggle(|s: &mut Settings, on| s.notifications = on),
        row((
            text("Volume"),
            slider(s.volume, 0.0..=100.0)
                .step(1.0)
                .label("Volume")
                .on_change(|s: &mut Settings, volume| s.volume = volume.round())
                .grow(1.0),
            text(format!("{:.0}%", s.volume)),
        ))
        .gap(8.0)
        .align(Align::Center),
        row((
            text("Theme").grow(1.0),
            dropdown(THEMES, s.theme)
                .label("Theme")
                .on_select(|s: &mut Settings, theme| s.theme = theme),
        ))
        .gap(8.0)
        .align(Align::Center),
        column((
            text("Profile completeness"),
            progress(complete).label("Profile completeness"),
        ))
        .gap(4.0)
        .align(Align::Stretch),
        row((
            button("Reset").on_click(|s: &mut Settings| *s = Settings::default()),
            button("Save").on_click(save),
        ))
        .gap(8.0),
        s.saved.clone().map(text),
    ))
    .gap(14.0)
    .padding(16.0)
    .align(Align::Stretch)
}

fn main() -> done::Result<()> {
    done::app("Settings", Settings::default(), view)
        .size(380.0, 520.0)
        .run()
}
