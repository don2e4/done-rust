#!/usr/bin/env python3
"""Record the done demo video on a private virtual display, in this machine's
own desktop: ghostty with the user's config, fish with the user's config and
prompt, the Openbox "Lilac" theme from the user's rc.xml, and the wallpaper.

This is the author's recording script, not a portable tool. It reads the
author's desktop configuration (~/.config/ghostty, ~/.config/openbox/rc.xml and
its wallpaper, ~/.Xresources, fish's CachyOS configuration), expects this
repository at ~/done, and creates and removes ~/hello. On another machine it
needs those adjusted.

Timeline: an empty directory, install the framework, create an app, add a
component, run it, a native window opens, and the punchline. done has no browser
or webview, so the "browser opens" step of a web-framework demo is a native
window here.

Needs Xvfb, openbox, ghostty, fish, feh, xrdb, xdotool, ffmpeg, and fc-match.
Writes demo/done-demo.mp4, plus scratch files and frame checks under
target/demo/. Nothing appears on the real desktop: ghostty is started with
--gtk-single-instance=false, so it cannot hand the window to the ghostty that
is already running there.

Disclosed setup, none of it visible in the video:
- A dedicated cargo target directory is warmed first by building the same app,
  so the recorded `cargo run` compiles only the app itself. Cargo runs offline
  against the local registry cache, without debug info.
- fish runs with --private (no history read or written, so no suggestions from
  the user's history), without the fastfetch greeting (it prints system
  details), and with the system autopair plugin's bindings and the CachyOS !/$
  history shortcuts replaced by plain typing at the first prompt, so typed code
  arrives exactly as typed.
"""
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "target" / "demo"
TARGET = WORK / "cargo-target"
# Outside the repository: `cargo init` inside a workspace's directory tree adds
# the new package to that workspace's members.
PROJECT = Path.home() / "hello"
REHEARSAL = Path.home() / ".cache" / "done-demo"
OUTPUT = ROOT / "demo" / "done-demo.mp4"
DISPLAY = ":78"
WIDTH, HEIGHT = 1920, 1080
LEAD_IN = 1.5  # seconds recorded before the timeline starts
LENGTH = 29.0
TERMINAL_CLASS = "com.mitchellh.ghostty"

HELLO = """echo 'use done::prelude::*;

fn main() -> done::Result<()> {
    done::app("Hello", (), |_: &()| text("Hello, world!")).run()
}' > src/main.rs"""

COUNTER = """echo 'use done::prelude::*;

fn counter(n: i32) -> impl View<i32> {
    column((
        text(format!("Clicked {n} times")).size(24.0),
        button("+1").on_click(|n: &mut i32| *n += 1),
    ))
    .gap(8.0)
    .padding(16.0)
    .align(Align::Center)
}

fn main() -> done::Result<()> {
    done::app("Hello", 0, |n: &i32| counter(*n)).run()
}' > src/main.rs"""

CAPTIONS = [
    (0.0, 3.0, "an empty directory"),
    (3.0, 6.0, "install the framework"),
    (6.0, 12.0, "create an app"),
    (12.0, 17.0, "add a component"),
    (17.0, 20.0, "run it"),
    (20.0, 24.0, "a native window opens: real GTK widgets, no browser"),
    (24.0, LENGTH, "wait, that's the whole framework?"),
]

# Lilac palette (~/.config/openbox/RICE.md).
INK = "0x232846"
SURFACE = "0xEEF1FF"

FISH_INIT = """function fish_greeting; end
# The system autopair plugin rebinds quote and bracket keys during start-up, and
# the CachyOS config binds ! and $ to history shortcuts; restore plain typing at
# the first prompt so typed code arrives verbatim.
function __demo_plain_keys --on-event fish_prompt
    for c in '(' ')' '[' ']' '{' '}' '"' "'" '!' '$'
        bind -- $c self-insert
    end
    bind backspace backward-delete-char
    bind tab complete
    functions --erase __demo_plain_keys
end
"""

ENV = dict(
    os.environ,
    DISPLAY=DISPLAY,
    CARGO_TARGET_DIR=str(TARGET),
    CARGO_NET_OFFLINE="true",
    CARGO_PROFILE_DEV_DEBUG="0",
    # Silence Mesa's software-GL notice on the virtual display.
    EGL_LOG_LEVEL="fatal",
    LIBGL_DEBUG="quiet",
)


def run(*args, **kwargs):
    return subprocess.run(args, check=True, env=ENV, **kwargs)


def xdotool(*args):
    return subprocess.run(
        ["xdotool", *args], check=True, env=ENV, capture_output=True, text=True
    ).stdout.strip()


def warm_cache():
    """Build the final app once so the recording compiles only the app crate."""
    rehearsal = REHEARSAL / "hello"
    shutil.rmtree(REHEARSAL, ignore_errors=True)
    rehearsal.mkdir(parents=True)
    run("cargo", "init", "-q", cwd=rehearsal)
    run("cargo", "add", "-q", "done", "--path", str(Path.home() / "done/crates/done"), cwd=rehearsal)
    source = COUNTER.split("'", 1)[1].rsplit("'", 1)[0] + "\n"
    (rehearsal / "src/main.rs").write_text(source)
    run("cargo", "build", "-q", cwd=rehearsal)
    shutil.rmtree(REHEARSAL)


def openbox_config():
    """The user's rc.xml (Lilac theme, fonts, keys) plus window placement."""
    rc = (Path.home() / ".config/openbox/rc.xml").read_text()
    rules = f"""<applications>
    <application class="{TERMINAL_CLASS}">
      <position force="yes"><x>120</x><y>96</y></position>
    </application>
    <application class="hello">
      <position force="yes"><x>1250</x><y>300</y></position>
    </application>"""
    if "<applications>" not in rc:
        sys.exit("unexpected rc.xml: no <applications> section")
    path = WORK / "openbox.xml"
    path.write_text(rc.replace("<applications>", rules, 1))
    return path


def type_lines(text, delay):
    """Type like a person: characters, then a real Return key per line."""
    for line in text.split("\n"):
        if line:
            subprocess.run(["xdotool", "type", "--delay", str(delay), "--", line], check=True, env=ENV)
        xdotool("key", "Return")


def wait_until(start, offset):
    remaining = start + offset - time.monotonic()
    if remaining > 0:
        time.sleep(remaining)


def window(pattern, by="--name", timeout=15.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        found = subprocess.run(
            ["xdotool", "search", "--onlyvisible", by, pattern],
            env=ENV, capture_output=True, text=True,
        ).stdout.split()
        if found:
            return found[-1]
        time.sleep(0.05)
    raise RuntimeError(f"no window matching {pattern!r}")


def glide(x, y, steps=12):
    """Move the pointer in visible steps rather than jumping."""
    current = dict(line.split("=") for line in xdotool("getmouselocation", "--shell").splitlines())
    x0, y0 = int(current["X"]), int(current["Y"])
    for step in range(1, steps + 1):
        xdotool("mousemove", str(x0 + (x - x0) * step // steps), str(y0 + (y - y0) * step // steps))
        time.sleep(0.02)


def framework_lines():
    sources = sorted((Path.home() / "done/crates").glob("*/src/*.rs"))
    return sum(len(path.read_text().splitlines()) for path in sources)


def caption_filter(font):
    filters = []
    for index, (start, end, text) in enumerate(CAPTIONS):
        path = WORK / f"caption-{index}.txt"
        path.write_text(text)
        last = index == len(CAPTIONS) - 1
        filters.append(
            f"drawtext=fontfile='{font}':textfile='{path}':"
            f"fontsize={50 if last else 32}:fontcolor={SURFACE}:box=1:"
            f"boxcolor={INK}@0.94:boxborderw=22:x=(w-text_w)/2:y=h-text_h-70:"
            f"enable='between(t,{start},{end})'"
        )
    filters.append(f"fade=t=out:st={LENGTH - 0.6}:d=0.6")
    return ",".join(filters)


def main():
    for tool in ("Xvfb", "openbox", "ghostty", "fish", "feh", "xrdb", "xdotool", "ffmpeg", "fc-match"):
        if shutil.which(tool) is None:
            sys.exit(f"missing {tool}")
    if PROJECT.exists():
        sys.exit(f"{PROJECT} already exists; the demo needs it to start empty")
    WORK.mkdir(parents=True, exist_ok=True)
    manifest = (ROOT / "Cargo.toml").read_bytes()

    def check_manifest():
        if (ROOT / "Cargo.toml").read_bytes() != manifest:
            (ROOT / "Cargo.toml").write_bytes(manifest)
            sys.exit("the workspace manifest changed during the demo; restored it")

    print("warming the build cache", flush=True)
    warm_cache()
    check_manifest()
    (WORK / "init.fish").write_text(FISH_INIT)
    rc = openbox_config()
    lines = framework_lines()
    processes = []
    PROJECT.mkdir()
    try:
        processes.append(subprocess.Popen(
            ["Xvfb", DISPLAY, "-screen", "0", f"{WIDTH}x{HEIGHT}x24", "-nolisten", "tcp"],
            env=ENV, stderr=subprocess.DEVNULL))
        time.sleep(1.0)
        xresources = Path.home() / ".Xresources"
        if xresources.exists():
            run("xrdb", "-merge", str(xresources))
        processes.append(subprocess.Popen(
            ["openbox", "--config-file", str(rc)], env=ENV, stderr=subprocess.DEVNULL))
        time.sleep(0.8)
        run("feh", "--no-fehbg", "--bg-scale", str(Path.home() / ".config/openbox/wallpaper.png"))
        processes.append(subprocess.Popen(
            ["ghostty", "--gtk-single-instance=false", f"--working-directory={PROJECT}",
             "--window-width=96", "--window-height=34",
             "-e", "fish", "--private", "-C", f"source {WORK / 'init.fish'}"],
            env=ENV, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        terminal = window(TERMINAL_CLASS, by="--class")
        xdotool("windowactivate", "--sync", terminal)
        # Park the pointer on the wallpaper, away from the terminal.
        xdotool("mousemove", "1700", "860")
        time.sleep(1.5)

        raw = WORK / "raw.mp4"
        recorder = subprocess.Popen(
            ["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-framerate", "30",
             "-video_size", f"{WIDTH}x{HEIGHT}", "-i", DISPLAY,
             "-c:v", "libx264", "-preset", "veryfast", "-crf", "16", "-pix_fmt", "yuv420p", str(raw)],
            env=ENV, stdin=subprocess.PIPE)
        start = time.monotonic() + LEAD_IN
        print("recording", flush=True)

        wait_until(start, 0.5)                                   # 0:00 empty directory
        type_lines("ls -A", 60)
        wait_until(start, 3.0)                                   # 0:03 install
        type_lines("cargo init -q && cargo add -q done --path ~/done/crates/done", 16)
        wait_until(start, 4.9)
        type_lines("ls", 60)
        wait_until(start, 6.0)                                   # 0:06 create the app
        type_lines(HELLO, 13)
        wait_until(start, 12.0)                                  # 0:12 add a component
        type_lines(COUNTER, 5)
        wait_until(start, 17.0)                                  # 0:17 run
        type_lines("cargo run -q &", 40)
        app = window("^Hello$")                                  # ~0:20 the window opens
        geometry = dict(line.split("=") for line in
                        xdotool("getwindowgeometry", "--shell", app).splitlines())
        x, y = int(geometry["X"]), int(geometry["Y"])
        # The column is centred in the 320x200 window: label, gap, then button.
        button = (x + 160, y + 71)
        wait_until(start, 20.4)
        glide(*button)
        for offset in (21.0, 21.8, 22.6):
            wait_until(start, offset)
            xdotool("click", "1")
        wait_until(start, 23.4)
        glide(button[0] + 70, button[1] + 95, steps=6)           # off the button
        xdotool("windowactivate", "--sync", terminal)            # back to the terminal
        wait_until(start, 24.0)                                  # 0:24 the punchline
        type_lines("# wait, that's the whole framework?", 22)
        type_lines("cat ~/done/crates/*/src/*.rs | wc -l", 22)
        wait_until(start, LENGTH + 0.5)
        recorder.communicate(b"q", timeout=30)
    finally:
        subprocess.run(["pkill", "-f", f"{TARGET}/debug/hello"], env=ENV)
        for process in reversed(processes):
            process.send_signal(signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
        shutil.rmtree(PROJECT, ignore_errors=True)

    font = subprocess.run(["fc-match", "-f", "%{file}", "Adwaita Sans"],
                          capture_output=True, text=True, check=True).stdout
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    # Capture starts about 0.1 s after ffmpeg launches.
    run("ffmpeg", "-y", "-loglevel", "error", "-ss", f"{LEAD_IN - 0.1:.2f}", "-i", str(raw),
        "-t", str(LENGTH), "-vf", caption_filter(font),
        "-c:v", "libx264", "-preset", "slow", "-crf", "18", "-pix_fmt", "yuv420p",
        "-movflags", "+faststart", str(OUTPUT))
    frames = WORK / "frames"
    shutil.rmtree(frames, ignore_errors=True)
    frames.mkdir()
    for mark in (0.2, 3.5, 5.6, 6.5, 11.5, 12.5, 16.8, 17.5, 20.3, 23.5, 25.0, 28.0):
        run("ffmpeg", "-loglevel", "error", "-ss", str(mark), "-i", str(OUTPUT),
            "-frames:v", "1", str(frames / f"t{mark:05.1f}.png"))
    check_manifest()
    print(f"framework lines at recording time: {lines}")
    print(f"wrote {OUTPUT}")


if __name__ == "__main__":
    main()
