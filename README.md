# TorTTY — Torbie TTY 🐈

### A fast, GPU-accelerated terminal emulator.

![Torbie, the cat: a grey and orange tabby sitting](assets/torbie-readme.png)

**A terminal should feel instant.**

TorTTY is a modern terminal emulator built for speed, responsiveness and a clean experience. It combines GPU-accelerated rendering with a lightweight, native application to deliver a smooth terminal experience, even when handling large amounts of output.

Whether you're coding, managing your system, or working with command-line tools, TorTTY stays out of your way and lets you focus on what matters.

## ✨ Features

- **GPU-accelerated rendering** — Smooth, responsive drawing powered by modern graphics APIs.
- **Lightweight** — A compact standalone application with minimal runtime dependencies.
- **Multiple tabs** — Keep different shells and tasks organized in one window.
- **Workspaces** — Organize your terminal sessions into separate workspaces.
- **Kitty keyboard protocol** — Enhanced keyboard reporting for applications that support it.
- **Kitty graphics protocol** — Display images directly inside the terminal.
- **256-color support** — ANSI colors and the extended 256-color palette.
- **Unicode support** — Wide characters, CJK text and emoji rendering.
- **Text selection and clipboard** — Select, copy and paste text naturally.
- **Mouse support** — Full terminal mouse reporting for interactive applications.
- **Scrollback history** — Navigate through previous terminal output.
- **Live configuration** — Customize fonts, colors, cursor, scrollback and more, with automatic configuration reload.

## 🚀 Getting Started

### Requirements

- Linux (X11)
- A supported GPU and graphics driver (Vulkan or OpenGL)
- A system monospaced font

Linux X11 is the currently tested environment.

### Installation

The easiest way to install TorTTY is using the installation script:

```bash
./install.sh --from-release
```

Alternatively, build and install directly from source:

```bash
git clone https://github.com/bazoocaze/tortty.git
cd tortty
./install.sh
```

The installer places the executable in `~/.local/bin` and creates a desktop menu entry.

### Run

Simply launch:

```bash
tortty
```

Or start a specific application:

```bash
tortty --cmd 'htop'
tortty --cmd 'vim'
tortty --cmd 'mc'
```

To run the tree you are editing instead of the installed binary:

```bash
./run.sh   # builds the debug binary and runs it
```

## 🎨 Customization

TorTTY works out of the box with sensible defaults. No configuration file is required.

For customization, create:

```text
~/.config/tortty/tortty.conf
```

Example:

```ini
font_family JetBrains Mono
font_size 13
scrollback 20000

foreground #d6d6d6
background #1e1e2e
cursor_color #f5e0dc

cursor_shape block
cursor_blink off

ligatures standard
```

Configuration changes are detected automatically. You don't need to restart the terminal.

See [Configuration Reference](docs/CONFIG.md) for all available options.

## 📖 Documentation

- [Features](docs/FEATURES.md)
- [Configuration Reference](docs/CONFIG.md)
- [Installation Guide](docs/INSTALL.md)
- [Known Limitations](docs/LIMITATIONS.md)

## 🗺️ Roadmap

TorTTY is actively evolving. Some of the areas being explored include:

- Split terminal panes.
- Search through terminal history.
- Hyperlink detection and interaction.
- More advanced image support.
- Persistent workspaces and sessions.
- Additional platform support.

## 📜 License

MIT License. See [LICENSE](LICENSE).
