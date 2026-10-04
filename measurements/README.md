# Measurements

- [milestone1.py](milestone1.py): the measurement script. It needs Python 3
  (standard library only), a Linux desktop session and the probe builds; the
  [README's Measurements section](../README.md#measurements) summarizes its
  method. Raw results are written to `measurements/local/`, which is not
  committed.
- [MILESTONE_1.md](MILESTONE_1.md), [MILESTONE_2.md](MILESTONE_2.md),
  [MILESTONE_3.md](MILESTONE_3.md): development reports written at the end of
  each milestone. They are working records addressed to the project's author,
  kept as written: they include every sample, the machine they ran on, what was
  not verified, and the decisions that were open at the time.

To reproduce the hello-world measurements on Linux:

```sh
cargo build --release -p done --examples -p done-gtk4 --example raw_hello
cargo build --profile probe -p done --example hello -p done-gtk4 --example raw_hello --features done/probe,done-gtk4/probe
python3 measurements/milestone1.py
```

The script opens about 40 short-lived windows on the current display.
